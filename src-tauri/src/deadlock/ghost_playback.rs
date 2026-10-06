use std::{
    fs,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::{cfg, hotkeys, match_safety, paths, route_recorder};

use std::fs::OpenOptions;
use std::io::Write;

const MAX_RECONSTRUCTION_GAP_MS: f64 = 100.0;
const RESIDUAL_FLOOR: f32 = 1.5;
const RESIDUAL_RELATIVE_TOLERANCE: f32 = 0.35;


#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RawPlaybackSample {
    pub(crate) t_ms: f64,
    pub(crate) position: [f32; 3],
    pub(crate) velocity: [f32; 3],
    pub(crate) yaw: Option<f32>,
    pub(crate) pitch: Option<f32>,
    pub(crate) coherent: bool,
    pub(crate) position_velocity_residual: Option<[f32; 3]>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RouteFile {
    samples: Vec<RouteFileSample>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RouteFileSample {
    t_ms: f64,
    position: [f32; 3],
    velocity: [f32; 3],
    yaw: Option<f32>,
    pitch: Option<f32>,
    #[serde(default)]
    diagnostics: RouteFileDiagnostics,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RouteFileDiagnostics {
    #[serde(default = "default_true")]
    coherent: bool,
    #[serde(default)]
    position_velocity_residual: Option<[f32; 3]>,
}

impl Default for RouteFileDiagnostics {
    fn default() -> Self {
        Self {
            coherent: true,
            position_velocity_residual: None,
        }
    }
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
enum PlaybackSourceQuality {
    Raw,
    RawIncoherent,
    ReconstructedIsolatedGlitch,
    InvalidRaw,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct PlaybackSample {
    timestamp_ms: f64,
    position: [f32; 3],
    yaw: Option<f32>,
    pitch: Option<f32>,
    confidence: f32,
    source_quality: PlaybackSourceQuality,
}

#[derive(Debug, Clone)]
struct PreparedRoute {
    samples: Vec<PlaybackSample>,
    duration_ms: f64,
    reconstructed_samples: usize,
    source: String,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GhostPlaybackStatus {
    playing: bool,
    elapsed_ms: f64,
    duration_ms: f64,
    current_sample: u64,
    sample_count: usize,
    reconstructed_samples: usize,
    route_source: Option<String>,
    looping: bool,
    stop_reason: Option<String>,
}

struct ActivePlayback {
    started_at: Instant,
    current_sample: Arc<AtomicU64>,
    stop: mpsc::Sender<()>,
    completion: mpsc::Receiver<Option<String>>,
    worker: JoinHandle<()>,
    route: PreparedRoute,
    looping: bool,
}

enum GhostState {
    Idle {
        route: Option<PreparedRoute>,
        stop_reason: Option<String>,
    },
    Playing(ActivePlayback),
}

static GHOST: Mutex<GhostState> = Mutex::new(GhostState::Idle {
    route: None,
    stop_reason: None,
});

pub fn load_route_json(path: String) -> Result<GhostPlaybackStatus, String> {
    let raw = fs::read_to_string(&path).map_err(|error| format!("Could not read {path}: {error}"))?;
    let route: RouteFile = serde_json::from_str(&raw)
        .map_err(|error| format!("Could not parse route JSON: {error}"))?;
    let input = route
        .samples
        .into_iter()
        .map(|sample| RawPlaybackSample {
            t_ms: sample.t_ms,
            position: sample.position,
            velocity: sample.velocity,
            yaw: sample.yaw,
            pitch: sample.pitch,
            coherent: sample.diagnostics.coherent,
            position_velocity_residual: sample.diagnostics.position_velocity_residual,
        })
        .collect();
    let prepared = prepare_route_playback(input, path)?;
    let mut state = GHOST.lock().map_err(|_| "Ghost playback lock poisoned".to_string())?;
    reap_finished(&mut state)?;
    if matches!(*state, GhostState::Playing(_)) {
        return Err("Stop Ghost playback before loading another route.".to_string());
    }
    *state = GhostState::Idle {
        route: Some(prepared),
        stop_reason: None,
    };
    status_from_state(&state)
}

pub fn start(looping: bool) -> Result<GhostPlaybackStatus, String> {
    match_safety::ensure_ghost_playback_allowed()?;
    if route_recorder::is_recording() {
        return Err("Stop route recording before starting Ghost playback.".to_string());
    }
    let mut state = GHOST.lock().map_err(|_| "Ghost playback lock poisoned".to_string())?;
    reap_finished(&mut state)?;
    if matches!(*state, GhostState::Playing(_)) {
        return Err("Ghost playback is already active.".to_string());
    }

    let loaded = match &*state {
        GhostState::Idle { route, .. } => route.clone(),
        GhostState::Playing(_) => unreachable!(),
    };
    let route = match loaded {
        Some(route) => route,
        None => prepare_route_playback(
            route_recorder::latest_playback_input()?,
            "latest recording".to_string(),
        )?,
    };
    ensure_playback_start(!route.samples.is_empty())?;

    let deadlock = paths::configured_deadlock_paths()
        .ok_or_else(|| "Deadlock directory is not configured".to_string())?;
    cfg::ensure_ghost_transport(&deadlock.cfg_file)?;
    write_cleanup_cfg(&deadlock.cfg_dir)?;
    hotkeys::activate_ghost_transport_from_ui()?;
    thread::sleep(Duration::from_millis(50));

    let worker_route = route.clone();
    let cfg_dir = deadlock.cfg_dir;
    let current_sample = Arc::new(AtomicU64::new(0));
    let worker_sample = current_sample.clone();
    let (stop_tx, stop_rx) = mpsc::channel();
    let (completion_tx, completion_rx) = mpsc::channel();
    let started_at = Instant::now();
    let worker = thread::Builder::new()
        .name("split-ghost-playback".to_string())
        .spawn(move || {
            let reason = run_worker(
                &worker_route,
                &cfg_dir,
                looping,
                started_at,
                worker_sample,
                stop_rx,
            );
            let _ = completion_tx.send(reason);
        })
        .map_err(|error| format!("Could not start Ghost playback: {error}"))?;

    *state = GhostState::Playing(ActivePlayback {
        started_at,
        current_sample,
        stop: stop_tx,
        completion: completion_rx,
        worker,
        route,
        looping,
    });
    status_from_state(&state)
}

pub fn stop() -> Result<GhostPlaybackStatus, String> {
    let mut state = GHOST.lock().map_err(|_| "Ghost playback lock poisoned".to_string())?;
    reap_finished(&mut state)?;
    let GhostState::Playing(active) = &*state else {
        return Err("Ghost playback is not active.".to_string());
    };
    let _ = active.stop.send(());
    let reason = active
        .completion
        .recv()
        .map_err(|_| "Ghost playback worker ended without a result".to_string())?;
    finish_active(&mut state, reason)?;
    status_from_state(&state)
}

pub fn status() -> Result<GhostPlaybackStatus, String> {
    let mut state = GHOST.lock().map_err(|_| "Ghost playback lock poisoned".to_string())?;
    reap_finished(&mut state)?;
    status_from_state(&state)
}

pub(crate) fn is_playing() -> bool {
    GHOST
        .lock()
        .map(|state| matches!(*state, GhostState::Playing(_)))
        .unwrap_or(true)
}

pub(crate) fn shutdown() {
    let playing = GHOST
        .lock()
        .map(|state| matches!(*state, GhostState::Playing(_)))
        .unwrap_or(false);
    if playing {
        let _ = stop();
    }
}

fn run_worker(
    route: &PreparedRoute,
    cfg_dir: &Path,
    looping: bool,
    mut cycle_started: Instant,
    current_sample: Arc<AtomicU64>,
    stop: mpsc::Receiver<()>,
) -> Option<String> {
    loop {
        let mut index = 0usize;

        while index < route.samples.len() {
            let deadline = cycle_started
                + Duration::from_secs_f64(route.samples[index].timestamp_ms / 1_000.0);

            let now = Instant::now();

            if now < deadline {
                match stop.recv_timeout(deadline - now) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                        cleanup_marker_if_safe(cfg_dir);
                        return None;
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                }
            } else if stop.try_recv().is_ok() {
                cleanup_marker_if_safe(cfg_dir);
                return None;
            }

            // We may have fallen behind while writing/executing the previous
            // frame. Always render the newest sample that should already have
            // happened instead of replaying every late sample.
            let elapsed_ms = cycle_started.elapsed().as_secs_f64() * 1_000.0;

            let mut render_index = index;

            while render_index + 1 < route.samples.len()
                && route.samples[render_index + 1].timestamp_ms <= elapsed_ms
            {
                render_index += 1;
            }

            let sample = &route.samples[render_index];
            let first_frame = index == 0;

            if let Err(error) = match_safety::ensure_ghost_playback_allowed() {
                let _ = write_cleanup_cfg(cfg_dir);
                return Some(error);
            }

            let frame_result = if first_frame {
                write_first_frame_cfg(cfg_dir, sample)
            } else {
                write_frame_cfg(cfg_dir, sample)
            };

            if let Err(error) = frame_result.and_then(|_| hotkeys::send_ghost_frame()) {
                let _ = write_cleanup_cfg(cfg_dir);
                return Some(error);
            }

            current_sample.store((render_index + 1) as u64, Ordering::Release);

            // Skip every sample that is already obsolete.
            index = render_index + 1;
        }

        if !looping {
            let _ = write_cleanup_cfg(cfg_dir);
            return None;
        }

        cycle_started = Instant::now();
        current_sample.store(0, Ordering::Release);
    }
}

fn cleanup_marker_if_safe(cfg_dir: &Path) {
    if match_safety::ensure_ghost_playback_allowed().is_ok()
        && write_cleanup_cfg(cfg_dir).is_ok()
    {
        let _ = hotkeys::send_ghost_frame();
    }
}

fn write_ghost_cfg(path: &Path, contents: &str) -> Result<(), String> {
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(path)
        .map_err(|error| format!("Could not open Ghost CFG: {error}"))?;

    file.write_all(contents.as_bytes())
        .map_err(|error| format!("Could not write Ghost CFG: {error}"))?;

    file.flush()
        .map_err(|error| format!("Could not flush Ghost CFG: {error}"))
}

fn write_first_frame_cfg(cfg_dir: &Path, sample: &PlaybackSample) -> Result<(), String> {
    let yaw = sample.yaw.unwrap_or(0.0);
    let [x, y, z] = sample.position;

    let contents = format!(
        "ent_fire split_ghost_marker Kill\n\
         ent_create prop_dynamic {{\"model\" \"models/heroes_staging/haze/haze.vmdl\" \"targetname\" \"split_ghost_marker\"}}\n\
         ent_fire split_ghost_marker SetAbsOrigin \"{x} {y} {z}\"\n\
         ent_fire split_ghost_marker SetAbsAngles \"0 {yaw} 0\"\n"
    );

    write_ghost_cfg(&cfg_dir.join("split_ghost_frame.cfg"), &contents)
        .map_err(|error| format!("Could not write Ghost first frame CFG: {error}"))
}

fn write_frame_cfg(cfg_dir: &Path, sample: &PlaybackSample) -> Result<(), String> {
    let yaw = sample.yaw.unwrap_or(0.0);
    let [x, y, z] = sample.position;

    let contents = format!(
        "ent_fire split_ghost_marker SetAbsOrigin \"{x} {y} {z}\"\n\
         ent_fire split_ghost_marker SetAbsAngles \"0 {yaw} 0\"\n"
    );

    write_ghost_cfg(&cfg_dir.join("split_ghost_frame.cfg"), &contents)
        .map_err(|error| format!("Could not write Ghost frame CFG: {error}"))
}

fn write_cleanup_cfg(cfg_dir: &Path) -> Result<(), String> {
    write_ghost_cfg(
        &cfg_dir.join("split_ghost_frame.cfg"),
        "ent_fire split_ghost_marker Kill\n",
    )
    .map_err(|error| format!("Could not write Ghost cleanup CFG: {error}"))
}

fn prepare_route_playback(
    raw: Vec<RawPlaybackSample>,
    source: String,
) -> Result<PreparedRoute, String> {
    validate_raw_timeline(&raw)?;
    let mut invalid = vec![false; raw.len()];
    for index in 1..raw.len().saturating_sub(1) {
        invalid[index] = isolated_position_glitch(raw[index - 1], raw[index], raw[index + 1]);
    }

    let first_time = raw[0].t_ms;
    let mut reconstructed_samples = 0;
    let samples = raw
        .iter()
        .enumerate()
        .map(|(index, sample)| {
            let can_reconstruct = invalid[index]
                && index > 0
                && index + 1 < raw.len()
                && !invalid[index - 1]
                && !invalid[index + 1]
                && raw[index + 1].t_ms - raw[index - 1].t_ms <= MAX_RECONSTRUCTION_GAP_MS;
            if can_reconstruct {
                reconstructed_samples += 1;
                let ratio = ((sample.t_ms - raw[index - 1].t_ms)
                    / (raw[index + 1].t_ms - raw[index - 1].t_ms)) as f32;
                PlaybackSample {
                    timestamp_ms: sample.t_ms - first_time,
                    position: lerp3(raw[index - 1].position, raw[index + 1].position, ratio),
                    yaw: interpolate_optional_angle(raw[index - 1].yaw, raw[index + 1].yaw, ratio),
                    pitch: interpolate_optional_angle(
                        raw[index - 1].pitch,
                        raw[index + 1].pitch,
                        ratio,
                    ),
                    confidence: 0.45,
                    source_quality: PlaybackSourceQuality::ReconstructedIsolatedGlitch,
                }
            } else {
                PlaybackSample {
                    timestamp_ms: sample.t_ms - first_time,
                    position: sample.position,
                    yaw: sample.yaw,
                    pitch: sample.pitch,
                    confidence: if invalid[index] {
                        0.25
                    } else if sample.coherent {
                        0.95
                    } else {
                        0.70
                    },
                    source_quality: if invalid[index] {
                        PlaybackSourceQuality::InvalidRaw
                    } else if sample.coherent {
                        PlaybackSourceQuality::Raw
                    } else {
                        PlaybackSourceQuality::RawIncoherent
                    },
                }
            }
        })
        .collect::<Vec<_>>();
    let duration_ms = samples.last().map_or(0.0, |sample| sample.timestamp_ms);
    Ok(PreparedRoute {
        samples,
        duration_ms,
        reconstructed_samples,
        source,
    })
}

fn isolated_position_glitch(
    previous: RawPlaybackSample,
    current: RawPlaybackSample,
    next: RawPlaybackSample,
) -> bool {
    let before = (current.t_ms - previous.t_ms) as f32 / 1_000.0;
    let after = (next.t_ms - current.t_ms) as f32 / 1_000.0;
    let total_ms = next.t_ms - previous.t_ms;
    if before <= 0.0 || after <= 0.0 || total_ms > MAX_RECONSTRUCTION_GAP_MS {
        return false;
    }
    let incoming = current
        .position_velocity_residual
        .map(vector_length)
        .unwrap_or_else(|| motion_residual(previous, current, before));
    let outgoing = motion_residual(current, next, after);
    let bridge = motion_residual(previous, next, before + after);
    incoming > motion_tolerance(previous.velocity, current.velocity, before)
        && outgoing > motion_tolerance(current.velocity, next.velocity, after)
        && bridge <= motion_tolerance(previous.velocity, next.velocity, before + after)
}

fn motion_residual(from: RawPlaybackSample, to: RawPlaybackSample, seconds: f32) -> f32 {
    let residual = std::array::from_fn(|axis| {
        let observed = to.position[axis] - from.position[axis];
        let average_velocity = (from.velocity[axis] + to.velocity[axis]) * 0.5;
        observed - average_velocity * seconds
    });
    vector_length(residual)
}

fn motion_tolerance(from: [f32; 3], to: [f32; 3], seconds: f32) -> f32 {
    let average_velocity = std::array::from_fn(|axis| (from[axis] + to[axis]) * 0.5);
    RESIDUAL_FLOOR + vector_length(average_velocity) * seconds * RESIDUAL_RELATIVE_TOLERANCE
}

fn validate_raw_timeline(raw: &[RawPlaybackSample]) -> Result<(), String> {
    if raw.len() < 2 {
        return Err("Ghost playback requires at least two route samples.".to_string());
    }
    for sample in raw {
        if !sample.t_ms.is_finite()
            || !sample.position.iter().all(|value| value.is_finite())
            || !sample.velocity.iter().all(|value| value.is_finite())
            || sample.yaw.is_some_and(|value| !value.is_finite())
            || sample.pitch.is_some_and(|value| !value.is_finite())
        {
            return Err("Ghost route contains NaN or infinite values.".to_string());
        }
    }
    if raw.windows(2).any(|pair| pair[0].t_ms >= pair[1].t_ms) {
        return Err("Ghost route timeline must be strictly monotone.".to_string());
    }
    Ok(())
}

fn interpolate_optional_angle(from: Option<f32>, to: Option<f32>, ratio: f32) -> Option<f32> {
    Some(interpolate_angle(from?, to?, ratio))
}

fn interpolate_angle(from: f32, to: f32, ratio: f32) -> f32 {
    let delta = (to - from + 180.0).rem_euclid(360.0) - 180.0;
    (from + delta * ratio).rem_euclid(360.0)
}

fn lerp3(from: [f32; 3], to: [f32; 3], ratio: f32) -> [f32; 3] {
    std::array::from_fn(|axis| from[axis] + (to[axis] - from[axis]) * ratio)
}

fn vector_length(value: [f32; 3]) -> f32 {
    value.into_iter().map(|axis| axis * axis).sum::<f32>().sqrt()
}

fn ensure_playback_start(has_route: bool) -> Result<(), String> {
    if !has_route {
        Err("No prepared Ghost route is available.".to_string())
    } else {
        Ok(())
    }
}

fn reap_finished(state: &mut GhostState) -> Result<(), String> {
    let reason = match state {
        GhostState::Playing(active) => match active.completion.try_recv() {
            Ok(reason) => Some(reason),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err("Ghost playback worker ended without a result".to_string())
            }
        },
        GhostState::Idle { .. } => None,
    };
    if let Some(reason) = reason {
        finish_active(state, reason)?;
    }
    Ok(())
}

fn finish_active(state: &mut GhostState, reason: Option<String>) -> Result<(), String> {
    let previous = std::mem::replace(
        state,
        GhostState::Idle {
            route: None,
            stop_reason: None,
        },
    );
    let GhostState::Playing(active) = previous else {
        return Ok(());
    };
    active
        .worker
        .join()
        .map_err(|_| "Ghost playback worker panicked".to_string())?;
    *state = GhostState::Idle {
        route: Some(active.route),
        stop_reason: reason,
    };
    Ok(())
}

fn status_from_state(state: &GhostState) -> Result<GhostPlaybackStatus, String> {
    Ok(match state {
        GhostState::Playing(active) => GhostPlaybackStatus {
            playing: true,
            elapsed_ms: active.started_at.elapsed().as_secs_f64() * 1_000.0,
            duration_ms: active.route.duration_ms,
            current_sample: active.current_sample.load(Ordering::Acquire),
            sample_count: active.route.samples.len(),
            reconstructed_samples: active.route.reconstructed_samples,
            route_source: Some(active.route.source.clone()),
            looping: active.looping,
            stop_reason: None,
        },
        GhostState::Idle { route, stop_reason } => GhostPlaybackStatus {
            playing: false,
            elapsed_ms: 0.0,
            duration_ms: route.as_ref().map_or(0.0, |route| route.duration_ms),
            current_sample: 0,
            sample_count: route.as_ref().map_or(0, |route| route.samples.len()),
            reconstructed_samples: route.as_ref().map_or(0, |route| route.reconstructed_samples),
            route_source: route.as_ref().map(|route| route.source.clone()),
            looping: false,
            stop_reason: stop_reason.clone(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(t_ms: f64, x: f32, velocity: f32) -> RawPlaybackSample {
        RawPlaybackSample {
            t_ms,
            position: [x, 0.0, 0.0],
            velocity: [velocity, 0.0, 0.0],
            yaw: Some(0.0),
            pitch: Some(0.0),
            coherent: true,
            position_velocity_residual: None,
        }
    }

    #[test]
    fn detects_and_reconstructs_one_impossible_point() {
        let raw = vec![sample(0.0, 0.0, 60.0), sample(16.0, -25.0, 60.0), sample(32.0, 1.92, 60.0)];
        assert!(isolated_position_glitch(raw[0], raw[1], raw[2]));
        let prepared = prepare_route_playback(raw, "test".into()).unwrap();
        assert_eq!(prepared.reconstructed_samples, 1);
        assert!((prepared.samples[1].position[0] - 0.96).abs() < 0.001);
    }

    #[test]
    fn keeps_fast_motion_when_velocity_explains_it() {
        let raw = vec![sample(0.0, 0.0, 1_000.0), sample(16.0, 16.0, 1_000.0), sample(32.0, 32.0, 1_000.0)];
        assert!(!isolated_position_glitch(raw[0], raw[1], raw[2]));
    }

    #[test]
    fn refuses_to_reconstruct_across_a_large_gap() {
        let raw = vec![sample(0.0, 0.0, 10.0), sample(100.0, -50.0, 10.0), sample(250.0, 2.5, 10.0)];
        assert!(!isolated_position_glitch(raw[0], raw[1], raw[2]));
        assert_eq!(prepare_route_playback(raw, "test".into()).unwrap().reconstructed_samples, 0);
    }

    #[test]
    fn yaw_interpolation_uses_the_short_wrap() {
        let middle = interpolate_angle(359.0, 1.0, 0.5);
        assert!(middle < 0.001 || (middle - 360.0).abs() < 0.001);
    }

    #[test]
    fn match_safety_blocks_playback_start() {
        match_safety::with_test_lock(true, || {
            assert_eq!(
                match_safety::ensure_ghost_playback_allowed().unwrap_err(),
                match_safety::MATCH_SAFETY_ERROR
            );
        });
    }

    #[test]
    fn timeline_must_be_strictly_monotone() {
        assert!(validate_raw_timeline(&[sample(10.0, 0.0, 0.0), sample(10.0, 1.0, 0.0)]).is_err());
    }
}
