use std::{
    fs::OpenOptions,
    io::Write,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;

use super::{match_safety, pawn, process, schema};

const TARGET_SAMPLE_RATE_HZ: u32 = 60;
const SAMPLE_INTERVAL: Duration = Duration::from_nanos(1_000_000_000 / 60);

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RouteSample {
    t_ms: f64,
    position: [f32; 3],
    velocity: [f32; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    yaw: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pitch: Option<f32>,
    diagnostics: RouteSampleDiagnostics,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct RouteSampleDiagnostics {
    scene_node: u64,
    local_origin: [f32; 3],
    absolute_origin: [f32; 3],
    wrapped_local_origin: [f32; 3],
    old_origin: [f32; 3],
    view_angles: [f32; 3],
    client_camera_angles: [f32; 3],
    angle_source_delta: [f32; 2],
    prediction_error: [f32; 3],
    prediction_error_time: f32,
    simulation_tick: u32,
    simulation_time: f32,
    coherent: bool,
    read_attempts: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    position_velocity_residual: Option<[f32; 3]>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
struct SamplingStats {
    effective_hz: f64,
    average_interval_ms: f64,
    max_interval_ms: f64,
    missed_deadlines: u64,
    three_read_samples: usize,
    incoherent_samples: usize,
    max_position_velocity_residual: f32,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RouteRecording {
    format: &'static str,
    version: u32,
    started_at: String,
    duration_ms: f64,
    target_sample_rate_hz: u32,
    sample_count: usize,
    position_source: &'static str,
    velocity_source: &'static str,
    view_angles_source: &'static str,
    sampling_stats: SamplingStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_reason: Option<String>,
    samples: Vec<RouteSample>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RouteRecordingStatus {
    recording: bool,
    elapsed_ms: f64,
    sample_count: u64,
    has_recording: bool,
    stop_reason: Option<String>,
}

struct WorkerCompletion {
    recording: RouteRecording,
}

struct ActiveRecording {
    started_at: Instant,
    sample_count: std::sync::Arc<AtomicU64>,
    stop: mpsc::Sender<()>,
    completion: mpsc::Receiver<WorkerCompletion>,
    worker: JoinHandle<()>,
}

enum RecorderState {
    Idle { last: Option<RouteRecording> },
    Recording(ActiveRecording),
}

static RECORDER: Mutex<RecorderState> = Mutex::new(RecorderState::Idle { last: None });

pub fn start_route_recording() -> Result<RouteRecordingStatus, String> {
    let startup_total = Instant::now();

    if super::ghost_playback::is_playing() {
        return Err("Stop Ghost playback before recording a route.".to_string());
    }

    let stage = Instant::now();
    let mut state = RECORDER
        .lock()
        .map_err(|_| "Route recorder lock poisoned".to_string())?;
    reap_finished(&mut state)?;
    ensure_can_start(matches!(*state, RecorderState::Recording(_)))?;
    println!(
        "[SPLIT][RouteRecorder][StartTiming] state_lock={:.2}ms",
        stage.elapsed().as_secs_f64() * 1_000.0
    );

    let stage = Instant::now();
    let pid = process::deadlock_pid().ok_or_else(|| "Deadlock is not running.".to_string())?;
    println!(
        "[SPLIT][RouteRecorder][StartTiming] deadlock_pid={:.2}ms",
        stage.elapsed().as_secs_f64() * 1_000.0
    );

    let stage = Instant::now();
    match_safety::ensure_route_recording_allowed()?;
    println!(
        "[SPLIT][RouteRecorder][StartTiming] match_safety={:.2}ms",
        stage.elapsed().as_secs_f64() * 1_000.0
    );

    let stage = Instant::now();
    let route_schema = schema::resolve_route_schema(pid)?;
    println!(
        "[SPLIT][RouteRecorder][StartTiming] resolve_route_schema={:.2}ms",
        stage.elapsed().as_secs_f64() * 1_000.0
    );

    let stage = Instant::now();
    let resolution = pawn::resolve_local_pawn(
        pid,
        route_schema.local_flag_offset,
        route_schema.pawn_handle_offset,
        route_schema.identity_size,
    )?;
    println!(
        "[SPLIT][RouteRecorder][StartTiming] resolve_local_pawn={:.2}ms",
        stage.elapsed().as_secs_f64() * 1_000.0
    );

    let started_at = Instant::now();
    let started_at_text = rfc3339_now()?;

    let stage = Instant::now();
    let first = {
        let reader = pawn::PawnTelemetryReader::new(pid, resolution, route_schema)?;
        sample_from_telemetry(started_at.elapsed(), reader.read_route_fast()?, None)?
    };
    println!(
        "[SPLIT][RouteRecorder][StartTiming] first_sample={:.2}ms",
        stage.elapsed().as_secs_f64() * 1_000.0
    );

    let sample_count = std::sync::Arc::new(AtomicU64::new(1));
    let worker_count = sample_count.clone();
    let (stop_tx, stop_rx) = mpsc::channel();
    let (completion_tx, completion_rx) = mpsc::channel();
    let worker = thread::Builder::new()
        .name("split-route-recorder".to_string())
        .spawn(move || {
            let reader = match pawn::PawnTelemetryReader::new(pid, resolution, route_schema) {
                Ok(reader) => reader,
                Err(error) => {
                    let recording = finish_recording(
                        started_at_text,
                        started_at.elapsed().as_secs_f64() * 1_000.0,
                        vec![first],
                        Some(error.clone()),
                        0,
                    );
                    eprintln!("[SPLIT][RouteRecorder] aborted reason={error}");
                    let _ = completion_tx.send(WorkerCompletion { recording });
                    return;
                }
            };
            run_worker(
                pid,
                reader,
                first,
                started_at,
                started_at_text,
                worker_count,
                stop_rx,
                completion_tx,
            )
        })
        .map_err(|error| format!("Could not start route recorder: {error}"))?;

    println!(
        "[SPLIT][RouteRecorder][StartTiming] total_before_ready={:.2}ms",
        startup_total.elapsed().as_secs_f64() * 1_000.0
    );
    println!("[SPLIT][RouteRecorder] started pid={pid}");
    // Keep recorder startup lean. The old experimental PawnScan walked every
    // Citadel pawn, resolved RTTI repeatedly and intentionally slept between
    // 50 samples. Because start_route_recording waits for this function to
    // return before the frontend leaves "Starting...", that diagnostic scan
    // could turn recorder startup into a multi-second (or much longer) stall.
    //
    // The scan is still available as a standalone diagnostic helper in
    // pawn.rs, but it must never run on the normal recording path.
    println!("[SPLIT][RouteRecorder] pawn=0x{:016X}", resolution.pawn);
    println!("[SPLIT][RouteRecorder] sampling target={TARGET_SAMPLE_RATE_HZ}Hz");

    *state = RecorderState::Recording(ActiveRecording {
        started_at,
        sample_count,
        stop: stop_tx,
        completion: completion_rx,
        worker,
    });
    status_from_state(&state)
}

pub fn stop_route_recording() -> Result<RouteRecordingStatus, String> {
    let mut state = RECORDER
        .lock()
        .map_err(|_| "Route recorder lock poisoned".to_string())?;
    reap_finished(&mut state)?;
    let RecorderState::Recording(active) = &mut *state else {
        return Err("No route recording is active.".to_string());
    };

    let _ = active.stop.send(());
    let completion = active
        .completion
        .recv()
        .map_err(|_| "Route recorder worker ended without a result".to_string())?;
    let previous = std::mem::replace(&mut *state, RecorderState::Idle { last: None });
    if let RecorderState::Recording(active) = previous {
        active
            .worker
            .join()
            .map_err(|_| "Route recorder worker panicked".to_string())?;
    }
    *state = RecorderState::Idle {
        last: Some(completion.recording),
    };
    status_from_state(&state)
}

pub fn get_route_recording_status() -> Result<RouteRecordingStatus, String> {
    let mut state = RECORDER
        .lock()
        .map_err(|_| "Route recorder lock poisoned".to_string())?;
    reap_finished(&mut state)?;
    status_from_state(&state)
}

pub(crate) fn is_recording() -> bool {
    RECORDER
        .lock()
        .map(|state| matches!(*state, RecorderState::Recording(_)))
        .unwrap_or(true)
}

pub fn save_route_recording_json(path: String) -> Result<(), String> {
    let mut state = RECORDER
        .lock()
        .map_err(|_| "Route recorder lock poisoned".to_string())?;
    reap_finished(&mut state)?;
    let RecorderState::Idle { last: Some(last) } = &*state else {
        return Err("No completed route recording is available.".to_string());
    };
    let json = serde_json::to_string_pretty(last)
        .map_err(|error| format!("Could not serialize route recording: {error}"))?;
    let destination = Path::new(&path);
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| format!("Could not create {}: {error}", destination.display()))?;
    file.write_all(json.as_bytes())
        .map_err(|error| format!("Could not write {}: {error}", destination.display()))?;
    file.sync_all()
        .map_err(|error| format!("Could not finish {}: {error}", destination.display()))
}

pub(crate) fn latest_playback_input(
) -> Result<Vec<super::ghost_playback::RawPlaybackSample>, String> {
    let mut state = RECORDER
        .lock()
        .map_err(|_| "Route recorder lock poisoned".to_string())?;
    reap_finished(&mut state)?;
    let RecorderState::Idle { last: Some(last) } = &*state else {
        return Err("No completed route recording is available for playback.".to_string());
    };
    Ok(last
        .samples
        .iter()
        .map(|sample| super::ghost_playback::RawPlaybackSample {
            t_ms: sample.t_ms,
            position: sample.position,
            velocity: sample.velocity,
            yaw: sample.yaw,
            pitch: sample.pitch,
            coherent: sample.diagnostics.coherent,
            position_velocity_residual: sample.diagnostics.position_velocity_residual,
        })
        .collect())
}

pub(crate) fn shutdown() {
    let active = RECORDER
        .lock()
        .map(|state| matches!(*state, RecorderState::Recording(_)))
        .unwrap_or(false);
    if active {
        let _ = stop_route_recording();
    }
}

fn run_worker(
    pid: u32,
    reader: pawn::PawnTelemetryReader,
    first: RouteSample,
    started_at: Instant,
    started_at_text: String,
    sample_count: std::sync::Arc<AtomicU64>,
    stop: mpsc::Receiver<()>,
    completion: mpsc::Sender<WorkerCompletion>,
) {
    let mut samples = vec![first];
    let mut next_sample = started_at + SAMPLE_INTERVAL;
    let mut stop_reason = None;
    let mut missed_deadlines = 0_u64;

    loop {
        let now = Instant::now();
        if now < next_sample {
            match stop.recv_timeout(next_sample - now) {
                Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
        } else if stop.try_recv().is_ok() {
            break;
        }

        if match_safety::is_locked() {
            stop_reason = Some("Live match safety lock enabled.".to_string());
            break;
        }

        let elapsed = started_at.elapsed();
        match reader.read_route_fast().and_then(|telemetry| {
            sample_from_telemetry(elapsed, telemetry, samples.last())
        })
        {
            Ok(sample) => {
                samples.push(sample);
                sample_count.store(samples.len() as u64, Ordering::Release);
            }
            Err(error) => {
                stop_reason = Some(if process::deadlock_pid() == Some(pid) {
                    error
                } else {
                    "Deadlock stopped.".to_string()
                });
                break;
            }
        }

        missed_deadlines += advance_deadline(&mut next_sample, Instant::now());
    }

    let duration_ms = started_at.elapsed().as_secs_f64() * 1_000.0;
    let recording = finish_recording(
        started_at_text,
        duration_ms,
        samples,
        stop_reason.clone(),
        missed_deadlines,
    );
    if let Some(reason) = &stop_reason {
        eprintln!("[SPLIT][RouteRecorder] aborted reason={reason}");
    } else {
        println!(
            "[SPLIT][RouteRecorder] stopped duration={duration_ms:.2}ms samples={}",
            recording.sample_count,
        );
    }
    println!(
        "[SPLIT][RouteRecorder] sampling effective={:.2}Hz avg={:.3}ms max={:.3}ms missed={} three_reads={} incoherent={}",
        recording.sampling_stats.effective_hz,
        recording.sampling_stats.average_interval_ms,
        recording.sampling_stats.max_interval_ms,
        recording.sampling_stats.missed_deadlines,
        recording.sampling_stats.three_read_samples,
        recording.sampling_stats.incoherent_samples,
    );
    let _ = completion.send(WorkerCompletion { recording });
}

fn advance_deadline(next_deadline: &mut Instant, now: Instant) -> u64 {
    *next_deadline += SAMPLE_INTERVAL;
    let mut missed = 0;
    while *next_deadline <= now {
        *next_deadline += SAMPLE_INTERVAL;
        missed += 1;
    }
    missed
}

fn sample_from_telemetry(
    elapsed: Duration,
    telemetry: pawn::PawnTelemetry,
    previous: Option<&RouteSample>,
) -> Result<RouteSample, String> {
    let t_ms = elapsed.as_secs_f64() * 1_000.0;
    let position_velocity_residual = previous.and_then(|previous| {
        let delta_seconds = ((t_ms - previous.t_ms) / 1_000.0) as f32;
        (delta_seconds > 0.0).then(|| {
            std::array::from_fn(|axis| {
                let observed = telemetry.position[axis] - previous.position[axis];
                let average_velocity =
                    (telemetry.velocity[axis] + previous.velocity[axis]) * 0.5;
                observed - average_velocity * delta_seconds
            })
        })
    });
    let sample = RouteSample {
        t_ms,
        position: telemetry.position,
        velocity: telemetry.velocity,
        pitch: telemetry.pitch,
        yaw: telemetry.yaw,
        diagnostics: RouteSampleDiagnostics {
            scene_node: telemetry.diagnostics.scene_node,
            local_origin: telemetry.diagnostics.local_origin,
            absolute_origin: telemetry.diagnostics.absolute_origin,
            wrapped_local_origin: telemetry.diagnostics.wrapped_local_origin,
            old_origin: telemetry.diagnostics.old_origin,
            view_angles: telemetry.diagnostics.view_angles,
            client_camera_angles: telemetry.diagnostics.client_camera_angles,
            angle_source_delta: [
                angular_delta(
                    telemetry.diagnostics.view_angles[0],
                    telemetry.diagnostics.client_camera_angles[0],
                ),
                angular_delta(
                    telemetry.diagnostics.view_angles[1],
                    telemetry.diagnostics.client_camera_angles[1],
                ),
            ],
            prediction_error: telemetry.diagnostics.prediction_error,
            prediction_error_time: telemetry.diagnostics.prediction_error_time,
            simulation_tick: telemetry.diagnostics.simulation_tick,
            simulation_time: telemetry.diagnostics.simulation_time,
            coherent: telemetry.diagnostics.coherent,
            read_attempts: telemetry.diagnostics.read_attempts,
            position_velocity_residual,
        },
    };
    validate_sample(&sample)?;
    Ok(sample)
}

fn validate_sample(sample: &RouteSample) -> Result<(), String> {
    let required_are_finite = sample.t_ms.is_finite()
        && sample.position.iter().all(|value| value.is_finite())
        && sample.velocity.iter().all(|value| value.is_finite());
    let optional_are_finite =
        sample.pitch.is_none_or(f32::is_finite) && sample.yaw.is_none_or(f32::is_finite);
    let diagnostics_are_finite = sample
        .diagnostics
        .local_origin
        .iter()
        .chain(sample.diagnostics.absolute_origin.iter())
        .chain(sample.diagnostics.wrapped_local_origin.iter())
        .chain(sample.diagnostics.old_origin.iter())
        .chain(sample.diagnostics.view_angles.iter())
        .chain(sample.diagnostics.client_camera_angles.iter())
        .chain(sample.diagnostics.angle_source_delta.iter())
        .chain(sample.diagnostics.prediction_error.iter())
        .all(|value| value.is_finite());
    let diagnostic_scalars_are_finite = sample.diagnostics.prediction_error_time.is_finite()
        && sample.diagnostics.simulation_time.is_finite()
        && sample
            .diagnostics
            .position_velocity_residual
            .is_none_or(|values| values.iter().all(|value| value.is_finite()));
    if required_are_finite
        && optional_are_finite
        && diagnostics_are_finite
        && diagnostic_scalars_are_finite
    {
        Ok(())
    } else {
        Err("Route sample contains NaN or infinite values.".to_string())
    }
}

fn finish_recording(
    started_at: String,
    duration_ms: f64,
    samples: Vec<RouteSample>,
    stop_reason: Option<String>,
    missed_deadlines: u64,
) -> RouteRecording {
    let sampling_stats = sampling_stats(&samples, missed_deadlines);
    RouteRecording {
        format: "split-route-debug",
        version: 1,
        started_at,
        duration_ms,
        target_sample_rate_hz: TARGET_SAMPLE_RATE_HZ,
        sample_count: samples.len(),
        position_source: "CGameSceneNode::m_vecAbsOrigin",
        velocity_source: "C_BaseEntity::m_vecAbsVelocity",
        view_angles_source: "C_CitadelPlayerPawn::m_angClientCamera",
        sampling_stats,
        stop_reason,
        samples,
    }
}

fn sampling_stats(samples: &[RouteSample], missed_deadlines: u64) -> SamplingStats {
    let intervals: Vec<f64> = samples
        .windows(2)
        .map(|pair| pair[1].t_ms - pair[0].t_ms)
        .collect();
    let interval_count = intervals.len();
    let total_interval_ms: f64 = intervals.iter().sum();
    let average_interval_ms = if interval_count == 0 {
        0.0
    } else {
        total_interval_ms / interval_count as f64
    };
    SamplingStats {
        effective_hz: if total_interval_ms > 0.0 {
            interval_count as f64 * 1_000.0 / total_interval_ms
        } else {
            0.0
        },
        average_interval_ms,
        max_interval_ms: intervals.into_iter().fold(0.0, f64::max),
        missed_deadlines,
        three_read_samples: samples
            .iter()
            .filter(|sample| sample.diagnostics.read_attempts == 3)
            .count(),
        incoherent_samples: samples
            .iter()
            .filter(|sample| !sample.diagnostics.coherent)
            .count(),
        max_position_velocity_residual: samples
            .iter()
            .filter_map(|sample| sample.diagnostics.position_velocity_residual)
            .map(vector_length)
            .fold(0.0, f32::max),
    }
}

fn angular_delta(left: f32, right: f32) -> f32 {
    (left - right + 180.0).rem_euclid(360.0) - 180.0
}

fn vector_length(value: [f32; 3]) -> f32 {
    value.into_iter().map(|axis| axis * axis).sum::<f32>().sqrt()
}

fn ensure_can_start(already_recording: bool) -> Result<(), String> {
    if already_recording {
        Err("A route recording is already active.".to_string())
    } else {
        Ok(())
    }
}

fn reap_finished(state: &mut RecorderState) -> Result<(), String> {
    let completion = match state {
        RecorderState::Recording(active) => match active.completion.try_recv() {
            Ok(completion) => Some(completion),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => {
                return Err("Route recorder worker ended without a result".to_string())
            }
        },
        RecorderState::Idle { .. } => None,
    };
    let Some(completion) = completion else {
        return Ok(());
    };

    let previous = std::mem::replace(state, RecorderState::Idle { last: None });
    if let RecorderState::Recording(active) = previous {
        active
            .worker
            .join()
            .map_err(|_| "Route recorder worker panicked".to_string())?;
    }
    *state = RecorderState::Idle {
        last: Some(completion.recording),
    };
    Ok(())
}

fn status_from_state(state: &RecorderState) -> Result<RouteRecordingStatus, String> {
    Ok(match state {
        RecorderState::Recording(active) => RouteRecordingStatus {
            recording: true,
            elapsed_ms: active.started_at.elapsed().as_secs_f64() * 1_000.0,
            sample_count: active.sample_count.load(Ordering::Acquire),
            has_recording: false,
            stop_reason: None,
        },
        RecorderState::Idle { last } => RouteRecordingStatus {
            recording: false,
            elapsed_ms: last.as_ref().map_or(0.0, |value| value.duration_ms),
            sample_count: last.as_ref().map_or(0, |value| value.sample_count as u64),
            has_recording: last.is_some(),
            stop_reason: last.as_ref().and_then(|value| value.stop_reason.clone()),
        },
    })
}

fn rfc3339_now() -> Result<String, String> {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("System clock is before the Unix epoch: {error}"))?;
    let total_seconds = since_epoch.as_secs();
    let days = (total_seconds / 86_400) as i64;
    let seconds = total_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        seconds / 3_600,
        (seconds % 3_600) / 60,
        seconds % 60,
        since_epoch.subsec_millis()
    ))
}

fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_sample(t_ms: f64) -> RouteSample {
        RouteSample {
            t_ms,
            position: [1.0, 2.0, 3.0],
            velocity: [4.0, 5.0, 6.0],
            yaw: Some(90.0),
            pitch: Some(-10.0),
            diagnostics: RouteSampleDiagnostics {
                scene_node: 0x1234,
                local_origin: [1.0, 2.0, 3.0],
                absolute_origin: [1.0, 2.0, 3.0],
                wrapped_local_origin: [1.0, 2.0, 3.0],
                old_origin: [1.0, 2.0, 3.0],
                view_angles: [-10.0, 90.0, 0.0],
                client_camera_angles: [-10.0, 90.0, 0.0],
                angle_source_delta: [0.0, 0.0],
                prediction_error: [0.0, 0.0, 0.0],
                prediction_error_time: 0.0,
                simulation_tick: 123,
                simulation_time: 2.0,
                coherent: true,
                read_attempts: 2,
                position_velocity_residual: None,
            },
        }
    }

    #[test]
    fn a_second_recording_is_refused() {
        assert!(ensure_can_start(false).is_ok());
        assert!(ensure_can_start(true).is_err());
    }

    #[test]
    fn samples_require_finite_values() {
        assert!(validate_sample(&valid_sample(0.0)).is_ok());
        let mut nan = valid_sample(1.0);
        nan.position[1] = f32::NAN;
        assert!(validate_sample(&nan).is_err());
        let mut infinite = valid_sample(2.0);
        infinite.velocity[2] = f32::INFINITY;
        assert!(validate_sample(&infinite).is_err());
    }

    #[test]
    fn timestamps_and_serialization_remain_monotonic_and_consistent() {
        let samples = vec![valid_sample(0.0), valid_sample(16.8), valid_sample(33.4)];
        assert!(samples.windows(2).all(|pair| pair[0].t_ms < pair[1].t_ms));
        let recording = finish_recording(
            "2026-10-06T12:00:00.000Z".to_string(),
            40.0,
            samples,
            None,
            2,
        );
        assert_eq!(recording.sample_count, 3);
        assert_eq!(recording.duration_ms, 40.0);
        let json = serde_json::to_string(&recording).unwrap();
        assert!(json.contains("\"format\":\"split-route-debug\""));
        assert!(json.contains("\"sampleCount\":3"));
        assert!(json.contains("\"missedDeadlines\":2"));
    }

    #[test]
    fn clean_stop_result_preserves_duration_and_count() {
        let recording = finish_recording(
            "2026-10-06T12:00:00.000Z".to_string(),
            25.5,
            vec![valid_sample(0.0), valid_sample(17.0)],
            None,
            0,
        );
        assert_eq!(recording.duration_ms, 25.5);
        assert_eq!(recording.sample_count, 2);
        assert_eq!(recording.stop_reason, None);
    }

    #[test]
    fn sampling_stats_use_real_timestamps() {
        let stats = sampling_stats(
            &[valid_sample(0.0), valid_sample(16.0), valid_sample(34.0)],
            1,
        );
        assert!((stats.effective_hz - 58.823_529).abs() < 0.001);
        assert_eq!(stats.average_interval_ms, 17.0);
        assert_eq!(stats.max_interval_ms, 18.0);
        assert_eq!(stats.missed_deadlines, 1);
        assert_eq!(stats.three_read_samples, 0);
        assert_eq!(stats.incoherent_samples, 0);
        assert_eq!(stats.max_position_velocity_residual, 0.0);
    }

    #[test]
    fn angular_delta_wraps_without_hiding_disagreement() {
        assert!((angular_delta(359.0, 1.0) + 2.0).abs() < f32::EPSILON);
        assert!((angular_delta(-10.0, -10.0)).abs() < f32::EPSILON);
    }

    #[test]
    fn deadline_scheduler_skips_late_slots_without_synthetic_samples() {
        let start = Instant::now();
        let mut next = start + SAMPLE_INTERVAL;
        assert_eq!(advance_deadline(&mut next, start + SAMPLE_INTERVAL * 3), 2);
        assert_eq!(next, start + SAMPLE_INTERVAL * 4);
    }

    #[test]
    fn utc_date_conversion_handles_epoch_and_known_date() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(20_367), (2025, 10, 6));
    }
}
