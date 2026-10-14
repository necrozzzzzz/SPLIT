//! Opt-in read-only bot motion tracing during native BOTREC playback.
//! Never modifies Deadlock memory, commands or replay timing.
use std::{fs::File, io::{BufWriter, Write}, thread, time::{Duration, Instant}};

use super::{match_safety, pawn, process, schema};

pub fn list_bot_replay_candidates() -> Result<Vec<u32>, String> {
    match_safety::ensure_route_recording_allowed()?;
    let pid = process::deadlock_pid().ok_or("Deadlock is not running.")?;
    let route_schema = schema::resolve_route_schema(pid)?;
    let local = pawn::resolve_local_pawn(
        pid, route_schema.local_flag_offset,
        route_schema.pawn_handle_offset, route_schema.identity_size,
    )?;
    pawn::list_other_citadel_pawns(pid, route_schema.identity_size, local.pawn)
}

pub fn record_bot_replay_diagnostic(
    entity_index: u32,
    duration_ms: u64,
    output_path: String,
) -> Result<String, String> {
    match_safety::ensure_route_recording_allowed()?;
    if entity_index == 0 || entity_index >= 32768 {
        return Err("A valid nonzero pawn entity index (1..32767) is required.".to_string());
    }
    if !(1000..=30000).contains(&duration_ms) {
        return Err("Duration must be between 1000 and 30000 ms.".to_string());
    }
    if output_path.trim().is_empty() {
        return Err("Choose a CSV output path.".to_string());
    }
    let pid = process::deadlock_pid().ok_or("Deadlock is not running.")?;
    let route_schema = schema::resolve_route_schema(pid)?;
    let local = pawn::resolve_local_pawn(
        pid,
        route_schema.local_flag_offset,
        route_schema.pawn_handle_offset,
        route_schema.identity_size,
    )?;
    let selected = pawn::resolve_entity_by_index(pid, entity_index, route_schema.identity_size)?;
    if selected == local.pawn {
        return Err("Selected entity is the local player, not the BOTREC bot.".to_string());
    }
    // Resolve/open/validate once; do not enumerate processes or modules per sample.
    let reader = pawn::PawnMotionReader::new(pid, entity_index, route_schema)?;
    reader.read()?;

    let file = File::create(&output_path)
        .map_err(|e| format!("Cannot create diagnostic CSV: {e}"))?;
    let mut out = BufWriter::new(file);
    writeln!(out, "elapsed_ms,entity_index,sim_tick,sim_time,pos_x,pos_y,pos_z,vel_x,vel_y,vel_z,speed_xy,derived_speed_xy,derived_vel_x,derived_vel_y,entity_flags,ground_handle,grounded_flag,read_error,sample_interval_ms,sim_delta_ms,sim_derived_speed_xy,sim_derived_vel_x,sim_derived_vel_y,read_duration_ms,sim_time_stable")
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    let interval = Duration::from_millis(16);
    let mut next = start;
    let mut good = 0u64;
    let mut failures = 0u64;
    let mut consecutive_failures = 0u64;
    let mut missed_deadlines = 0u64;
    let mut previous_position: Option<([f32; 3], f64, f64)> = None;
    let mut first_good_ms: Option<f64> = None;
    let mut last_good_ms = 0.0f64;
    let mut previous_capture_ms: Option<f64> = None;
    let mut max_interval_ms = 0.0f64;
    let mut max_read_ms = 0.0f64;
    while start.elapsed().as_millis() < duration_ms as u128 {
        let read_start_ms = start.elapsed().as_secs_f64() * 1000.0;
        let sample = reader.read();
        let read_end_ms = start.elapsed().as_secs_f64() * 1000.0;
        let read_duration_ms = read_end_ms - read_start_ms;
        // Midpoint timestamps bound the read instead of including setup time.
        let elapsed = (read_start_ms + read_end_ms) * 0.5;
        let sample_interval_ms = previous_capture_ms.map(|old| elapsed - old);
        if let Some(dt) = sample_interval_ms {
            max_interval_ms = max_interval_ms.max(dt);
        }
        previous_capture_ms = Some(elapsed);
        max_read_ms = max_read_ms.max(read_duration_ms);
        // Fixed-size rows also keep failed reads aligned with the CSV header.
        let mut row = vec![String::new(); 25];
        row[0] = format!("{elapsed:.3}");
        row[1] = entity_index.to_string();
        row[18] = sample_interval_ms.map(|dt| format!("{dt:.3}")).unwrap_or_default();
        row[23] = format!("{read_duration_ms:.3}");
        match sample {
            Ok((pos, vel, tick, sim_time, flags, ground, time_stable)) => {
                row[2] = tick.to_string();
                row[3] = format!("{sim_time:.6}");
                for axis in 0..3 {
                    row[4 + axis] = format!("{:.4}", pos[axis]);
                    row[7 + axis] = format!("{:.4}", vel[axis]);
                }
                row[10] = format!("{:.4}", vel[0].hypot(vel[1]));
                row[14] = flags.map(|v| v.to_string()).unwrap_or_default();
                row[15] = ground.map(|v| v.to_string()).unwrap_or_default();
                row[16] = flags.map(|v| if v & 1 != 0 { "1" } else { "0" }).unwrap_or("").to_string();
                row[24] = if time_stable { "1" } else { "0" }.to_string();
                if time_stable {
                    if let Some((old, old_ms, old_sim)) = previous_position {
                        if let Some((vx, vy)) = derive_motion(old, pos, (elapsed - old_ms) / 1000.0) {
                            row[11] = format!("{:.4}", vx.hypot(vy));
                            row[12] = format!("{vx:.4}");
                            row[13] = format!("{vy:.4}");
                        }
                        let sim_dt = sim_time as f64 - old_sim;
                        row[19] = format!("{:.3}", sim_dt * 1000.0);
                        // Cross-check only: client positions may be interpolated.
                        // A repeated or regressing simulation clock is never divided by.
                        if let Some((vx, vy)) = derive_motion(old, pos, sim_dt) {
                            row[20] = format!("{:.4}", vx.hypot(vy));
                            row[21] = format!("{vx:.4}");
                            row[22] = format!("{vy:.4}");
                        }
                    }
                    previous_position = Some((pos, elapsed, sim_time as f64));
                } else {
                    previous_position = None;
                }
                first_good_ms.get_or_insert(elapsed);
                last_good_ms = elapsed;
                good += 1;
                consecutive_failures = 0;
            }
            Err(error) => {
                row[17] = error.replace(',', " ").replace('\n', " ").replace('\r', " ");
                previous_position = None;
                failures += 1;
                consecutive_failures += 1;
            }
        }
        writeln!(out, "{}", row.join(",")).map_err(|e| e.to_string())?;
        if consecutive_failures > 20 {
            out.flush().map_err(|e| e.to_string())?;
            return Err("Bot telemetry stopped after 21 consecutive read errors. Check the CSV and restart the capture.".to_string());
        }
        // Skip missed slots rather than taking a burst of catch-up samples.
        next += interval;
        let now = Instant::now();
        while next <= now {
            next += interval;
            missed_deadlines += 1;
        }
        thread::sleep(next.saturating_duration_since(Instant::now()));
    }
    out.flush().map_err(|e| e.to_string())?;
    if good == 0 {
        return Err("No usable bot telemetry was captured.".to_string());
    }
    let effective_hz = first_good_ms
        .filter(|first| good > 1 && last_good_ms > *first)
        .map(|first| (good - 1) as f64 * 1000.0 / (last_good_ms - first))
        .unwrap_or(0.0);
    let summary = format!(
        "Bot replay trace: {output_path} ({good} samples, {failures} read errors). Actual {effective_hz:.2} Hz; max interval {max_interval_ms:.3} ms; max read {max_read_ms:.3} ms; missed slots {missed_deadlines}. Start time is the capture start, not the exact BOTREC tick."
    );
    println!("[SPLIT][BotTrace] {summary}");
    Ok(summary)
}

/// Position-derived average velocity over the supplied interval.
/// Reject invalid clocks, teleports and non-finite input; this is not an
/// instantaneous engine velocity and must not be labelled as one.
fn derive_motion(old: [f32; 3], pos: [f32; 3], dt: f64) -> Option<(f64, f64)> {
    if !dt.is_finite() || dt < 0.005 || dt > 0.25 {
        return None;
    }
    if !old.iter().chain(pos.iter()).all(|v| v.is_finite()) {
        return None;
    }
    let dx = pos[0] as f64 - old[0] as f64;
    let dy = pos[1] as f64 - old[1] as f64;
    let dz = pos[2] as f64 - old[2] as f64;
    if dx.hypot(dy).hypot(dz) > 150.0 {
        return None;
    }
    Some((dx / dt, dy / dt))
}

#[cfg(test)]
mod tests {
    use super::derive_motion;

    #[test]
    fn rejects_repeated_regressing_or_invalid_clocks() {
        for dt in [0.0, -0.016, 0.001, 0.5, f64::NAN, f64::INFINITY] {
            assert!(derive_motion([0.0; 3], [1.0, 0.0, 0.0], dt).is_none());
        }
    }

    #[test]
    fn rejects_teleports_and_nonfinite_positions() {
        assert!(derive_motion([0.0; 3], [151.0, 0.0, 0.0], 0.016).is_none());
        assert!(derive_motion([0.0; 3], [0.0, 0.0, 151.0], 0.016).is_none());
        assert!(derive_motion([0.0; 3], [f32::NAN, 0.0, 0.0], 0.016).is_none());
    }

    #[test]
    fn derives_known_motion_with_each_clock() {
        let old = [0.0; 3];
        let pos = [10.0, 0.0, 0.0];
        assert_eq!(derive_motion(old, pos, 0.04), Some((250.0, 0.0)));
        assert_eq!(derive_motion(old, pos, 0.03125), Some((320.0, 0.0)));
    }
}
