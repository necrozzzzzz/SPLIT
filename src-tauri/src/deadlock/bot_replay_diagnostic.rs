//! Opt-in read-only bot motion tracing during native BOTREC playback.
//! Never modifies Deadlock memory, commands or replay timing.
use std::{fs::File, io::{BufWriter, Write}, thread, time::{Duration, Instant}};

use super::{match_safety, pawn, process, schema};

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
    // Fail before creating a file if the supplied index is not a valid player pawn.
    pawn::read_pawn_motion_at_index(pid, entity_index, route_schema)?;

    let file = File::create(&output_path)
        .map_err(|e| format!("Cannot create diagnostic CSV: {e}"))?;
    let mut out = BufWriter::new(file);
    writeln!(out, "elapsed_ms,entity_index,sim_tick,sim_time,pos_x,pos_y,pos_z,vel_x,vel_y,vel_z,speed_xy,read_error")
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    let interval = Duration::from_millis(16);
    let mut next = start;
    let mut good = 0u64;
    let mut failures = 0u64;
    while start.elapsed().as_millis() < duration_ms as u128 {
        if process::deadlock_pid() != Some(pid) {
            return Err("Deadlock process changed during diagnostic.".to_string());
        }
        let elapsed = start.elapsed().as_secs_f64() * 1000.0;
        match pawn::read_pawn_motion_at_index(pid, entity_index, route_schema) {
            Ok((pos, vel, tick, sim_time)) => {
                let speed = vel[0].hypot(vel[1]);
                writeln!(out, "{elapsed:.3},{entity_index},{tick},{sim_time:.6},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{speed:.4},",
                    pos[0], pos[1], pos[2], vel[0], vel[1], vel[2])
                    .map_err(|e| e.to_string())?;
                good += 1;
            }
            Err(error) => {
                let safe_error = error.replace(',', " ").replace('\n', " ").replace('\r', " ");
                writeln!(out, "{elapsed:.3},{entity_index},,,,,,,,,,{safe_error}")
                    .map_err(|e| e.to_string())?;
                failures += 1;
                if failures > 20 && good == 0 {
                    return Err(format!("No valid bot samples. Check the entity index. Last error: {error}"));
                }
            }
        }
        next += interval;
        if next > Instant::now() {
            thread::sleep(next - Instant::now());
        }
    }
    out.flush().map_err(|e| e.to_string())?;
    if good == 0 {
        return Err("No usable bot telemetry was captured.".to_string());
    }
    Ok(format!("Bot replay trace: {output_path} ({good} samples, {failures} read errors). Start time is the API call, not the exact BOTREC tick."))
}
