//! Production Panorama Quick Access transport.
//!
//! This is intentionally separate from `panorama_bridge`, which remains the
//! experimental PNG-dimension transport used by the Borderless build.

use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use tauri::AppHandle;

const HOST: Ipv4Addr = Ipv4Addr::LOCALHOST;
const PORT: u16 = 32146;
const SLOT_COUNT: u8 = 8;
const FRAME_BYTES: usize = 16;
const FRAME_BITS: usize = FRAME_BYTES * 8;
const FRAME_PAYLOAD_BYTES: usize = 9;
const FRAME_MAGIC: u8 = 0xA0;
const FRAME_VERSION: u8 = 1;
const FRAME_END_FLAG: u8 = 0x08;
const MAX_MESSAGE_FRAMES: usize = 256;
const MAX_CACHED_MESSAGES: usize = 4;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static RUNNING: AtomicBool = AtomicBool::new(false);
static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static SERVER: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
static MESSAGE_CACHE: Mutex<Vec<FramedMessage>> = Mutex::new(Vec::new());

const BIT_ONE_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x04, 0x00, 0x00, 0x00, 0xb5, 0x1c, 0x0c,
    0x02, 0x00, 0x00, 0x00, 0x0b, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x64, 0xf8, 0x0f, 0x00,
    0x01, 0x05, 0x01, 0x01, 0x27, 0x18, 0xe3, 0x66, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44,
    0xae, 0x42, 0x60, 0x82,
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct FramedMessage {
    id: u16,
    frames: Vec<[u8; FRAME_BYTES]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BitRequest {
    message: u16,
    round: usize,
    bit: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Visibility {
    Hidden,
    Passive,
    Interactive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SlotState {
    index: u8,
    name: String,
    populated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PanoramaState {
    protocol_version: u8,
    sequence: u64,
    visibility: Visibility,
    active_preset: u8,
    active_preset_name: String,
    slots: Vec<SlotState>,
    can_undo: bool,
    can_redo: bool,
}

#[derive(Serialize)]
struct CompactSlot<'a>(u8, &'a str, u8);

#[derive(Serialize)]
struct CompactState<'a>(u8, u64, u8, u8, &'a str, Vec<CompactSlot<'a>>, u8, u8);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Action {
    LoadSlot(u8),
    SaveSlot(u8),
    Undo,
    Redo,
    PreviousPreset,
    NextPreset,
}

trait ActionExecutor {
    fn execute(&self, action: Action) -> Result<(), String>;
}

struct AppActionExecutor {
    app: AppHandle,
}

impl ActionExecutor for AppActionExecutor {
    fn execute(&self, action: Action) -> Result<(), String> {
        match action {
            Action::LoadSlot(slot) => crate::deadlock::load_slot_from_quick_access(slot),
            Action::SaveSlot(slot) => {
                crate::deadlock::save_slot_from_quick_access(self.app.clone(), slot)
            }
            Action::Undo => {
                let result = crate::deadlock::undo_last_action()?;
                crate::deadlock::emit_history_operation(&self.app, &result);
                Ok(())
            }
            Action::Redo => {
                let result = crate::deadlock::redo_last_action()?;
                crate::deadlock::emit_history_operation(&self.app, &result);
                Ok(())
            }
            Action::PreviousPreset => change_preset(&self.app, -1),
            Action::NextPreset => change_preset(&self.app, 1),
        }
    }
}

fn change_preset(app: &AppHandle, direction: i8) -> Result<(), String> {
    let names = crate::deadlock::get_preset_names()?;
    if names.is_empty() {
        return Err("No presets are available".to_string());
    }
    let current =
        usize::from(crate::deadlock::get_active_preset()?.saturating_sub(1)).min(names.len() - 1);
    let next = if direction < 0 {
        current.checked_sub(1).unwrap_or(names.len() - 1)
    } else {
        (current + 1) % names.len()
    };
    let preset = u8::try_from(next + 1).map_err(|_| "Preset index is too large".to_string())?;
    let slots = crate::deadlock::set_active_preset(preset)?;
    crate::ui::emit_to_main_if_present(app, "deadlock-slots", &slots);
    crate::ui::emit_to_main_if_present(app, "deadlock-preset", preset);
    crate::ui::emit_to_main_if_present(app, "deadlock-favorite-mode", false);
    Ok(())
}

fn next_sequence() -> u64 {
    SEQUENCE.fetch_add(1, Ordering::SeqCst).saturating_add(1)
}

fn visibility() -> Visibility {
    if crate::quick_access::is_interactive() {
        Visibility::Interactive
    } else if crate::quick_access::is_visible() {
        Visibility::Passive
    } else {
        Visibility::Hidden
    }
}

fn current_state() -> Result<PanoramaState, String> {
    let snapshots = crate::deadlock::get_slots()?;
    let metadata = crate::deadlock::get_slot_metadata()?;
    let active_preset = crate::deadlock::get_active_preset()?;
    let preset_names = crate::deadlock::get_preset_names()?;
    let history = crate::deadlock::get_history_state()?;

    let slots = snapshots
        .iter()
        .enumerate()
        .map(|(index, snapshot)| {
            let slot = u8::try_from(index + 1).unwrap_or(u8::MAX);
            let name = metadata
                .get(index)
                .map(|value| value.name.trim())
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| format!("Slot {slot}"));
            SlotState {
                index: slot,
                name,
                populated: snapshot.is_some(),
            }
        })
        .collect();

    let preset_index = usize::from(active_preset.saturating_sub(1));
    let active_preset_name = preset_names
        .get(preset_index)
        .cloned()
        .unwrap_or_else(|| format!("Preset {active_preset}"));

    Ok(PanoramaState {
        protocol_version: 1,
        sequence: next_sequence(),
        visibility: visibility(),
        active_preset,
        active_preset_name,
        slots,
        can_undo: history.can_undo,
        can_redo: history.can_redo,
    })
}

fn encode_state_payload(state: &PanoramaState) -> Result<Vec<u8>, String> {
    let visibility = match state.visibility {
        Visibility::Hidden => 0,
        Visibility::Passive => 1,
        Visibility::Interactive => 2,
    };
    let slots = state
        .slots
        .iter()
        .map(|slot| CompactSlot(slot.index, &slot.name, u8::from(slot.populated)))
        .collect();
    serde_json::to_vec(&CompactState(
        state.protocol_version,
        state.sequence,
        visibility,
        state.active_preset,
        &state.active_preset_name,
        slots,
        u8::from(state.can_undo),
        u8::from(state.can_redo),
    ))
    .map_err(|error| format!("Could not serialize Panorama state: {error}"))
}

fn crc16_ccitt(bytes: &[u8]) -> u16 {
    let mut crc = 0xffff_u16;
    for byte in bytes {
        crc ^= u16::from(*byte) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn build_frames(message_id: u16, payload: &[u8]) -> Result<Vec<[u8; FRAME_BYTES]>, String> {
    let frame_count = (payload.len().max(1) + FRAME_PAYLOAD_BYTES - 1) / FRAME_PAYLOAD_BYTES;
    if frame_count > MAX_MESSAGE_FRAMES {
        return Err(format!(
            "State payload requires too many frames: {frame_count}"
        ));
    }

    let mut frames = Vec::with_capacity(frame_count);
    for round in 0..frame_count {
        let start = round * FRAME_PAYLOAD_BYTES;
        let end = (start + FRAME_PAYLOAD_BYTES).min(payload.len());
        let chunk = &payload[start..end];
        let mut frame = [0_u8; FRAME_BYTES];
        let end_flag = u8::from(round + 1 == frame_count) * FRAME_END_FLAG;
        frame[0] = FRAME_MAGIC | FRAME_VERSION | end_flag;
        frame[1..3].copy_from_slice(&message_id.to_le_bytes());
        frame[3] = u8::try_from(round).map_err(|_| "Round index overflow".to_string())?;
        frame[4] = u8::try_from(chunk.len()).map_err(|_| "Payload length overflow".to_string())?;
        frame[5..5 + chunk.len()].copy_from_slice(chunk);
        let crc = crc16_ccitt(&frame[..14]);
        frame[14..16].copy_from_slice(&crc.to_le_bytes());
        frames.push(frame);
    }
    Ok(frames)
}

fn frame_bit(frame: &[u8; FRAME_BYTES], bit: usize) -> Result<bool, String> {
    if bit >= FRAME_BITS {
        return Err(format!("Invalid bit {bit}"));
    }
    Ok(frame[bit / 8] & (1 << (bit % 8)) != 0)
}

fn query_value(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|part| {
        let (candidate, value) = part.split_once('=').unwrap_or((part, ""));
        (candidate == key).then(|| value.to_string())
    })
}

fn parse_bit_request(query: &str) -> Result<BitRequest, String> {
    let parse = |key: &str| -> Result<usize, String> {
        let raw = query_value(query, key).ok_or_else(|| format!("Missing {key}"))?;
        raw.parse::<usize>()
            .map_err(|_| format!("Invalid {key} {raw}"))
    };
    let message = parse("message")?;
    let round = parse("round")?;
    let bit = parse("bit")?;
    if message == 0 || message > usize::from(u16::MAX) {
        return Err(format!("Invalid message {message}"));
    }
    if round >= MAX_MESSAGE_FRAMES {
        return Err(format!("Invalid round {round}"));
    }
    if bit >= FRAME_BITS {
        return Err(format!("Invalid bit {bit}"));
    }
    Ok(BitRequest {
        message: message as u16,
        round,
        bit,
    })
}

fn cached_frame_with<F>(
    cache: &mut Vec<FramedMessage>,
    request: BitRequest,
    create_payload: F,
) -> Result<[u8; FRAME_BYTES], String>
where
    F: FnOnce() -> Result<Vec<u8>, String>,
{
    let message_index =
        if let Some(index) = cache.iter().position(|item| item.id == request.message) {
            index
        } else {
            let frames = build_frames(request.message, &create_payload()?)?;
            if cache.len() >= MAX_CACHED_MESSAGES {
                cache.remove(0);
            }
            cache.push(FramedMessage {
                id: request.message,
                frames,
            });
            cache.len() - 1
        };

    cache[message_index]
        .frames
        .get(request.round)
        .copied()
        .ok_or_else(|| format!("Invalid round {}", request.round))
}

fn state_bit(request: BitRequest) -> Result<bool, String> {
    let mut cache = MESSAGE_CACHE
        .lock()
        .map_err(|_| "Panorama message cache lock poisoned".to_string())?;
    let frame = cached_frame_with(&mut cache, request, || {
        current_state().and_then(|state| encode_state_payload(&state))
    })?;
    frame_bit(&frame, request.bit)
}

fn parse_action(query: &str) -> Result<Action, String> {
    let name = query_value(query, "action").ok_or_else(|| "Missing action".to_string())?;
    let slot = || -> Result<u8, String> {
        let raw = query_value(query, "slot").ok_or_else(|| "Missing slot".to_string())?;
        let value = raw
            .parse::<u8>()
            .map_err(|_| format!("Invalid slot {raw}"))?;
        if !(1..=SLOT_COUNT).contains(&value) {
            return Err(format!("Invalid slot {value}"));
        }
        Ok(value)
    };

    match name.as_str() {
        "load_slot" => Ok(Action::LoadSlot(slot()?)),
        "save_slot" => Ok(Action::SaveSlot(slot()?)),
        "undo" => Ok(Action::Undo),
        "redo" => Ok(Action::Redo),
        "previous_preset" => Ok(Action::PreviousPreset),
        "next_preset" => Ok(Action::NextPreset),
        _ => Err(format!("Unknown action {name}")),
    }
}

fn route_action(executor: &impl ActionExecutor, query: &str) -> Result<(), String> {
    executor.execute(parse_action(query)?)
}

fn write_response(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let headers = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(headers.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn handle_connection(mut stream: TcpStream, app: &AppHandle) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let mut buffer = [0_u8; 8192];
    let Ok(count) = stream.read(&mut buffer) else {
        return;
    };
    let Ok(request) = std::str::from_utf8(&buffer[..count]) else {
        write_response(&mut stream, "400 Bad Request", "text/plain", b"Bad Request");
        return;
    };
    let mut parts = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();
    if method != "GET" || target.len() > 2048 {
        write_response(&mut stream, "400 Bad Request", "text/plain", b"Bad Request");
        return;
    }
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    match path {
        "/ipc/state-bit" => match parse_bit_request(query)
            .and_then(|request| state_bit(request).map(|is_one| (request, is_one)))
        {
            Ok((_request, true)) => write_response(&mut stream, "200 OK", "image/png", BIT_ONE_PNG),
            Ok((_request, false)) => write_response(&mut stream, "404 Not Found", "image/png", &[]),
            Err(error) => write_response(
                &mut stream,
                "400 Bad Request",
                "text/plain; charset=utf-8",
                error.as_bytes(),
            ),
        },
        "/action" => {
            let executor = AppActionExecutor { app: app.clone() };
            match route_action(&executor, query) {
                Ok(()) => write_response(&mut stream, "200 OK", "image/png", BIT_ONE_PNG),
                Err(error) => write_response(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain; charset=utf-8",
                    error.as_bytes(),
                ),
            }
        }
        _ => write_response(&mut stream, "404 Not Found", "text/plain", b"Not Found"),
    }
}

fn bind_listener(port: u16) -> Result<TcpListener, String> {
    TcpListener::bind(SocketAddr::new(IpAddr::V4(HOST), port))
        .map_err(|error| format!("Could not bind Panorama runtime to {HOST}:{port}: {error}"))
}

fn start_needed(server_running: bool) -> bool {
    !server_running
}

pub(crate) fn start(app: AppHandle) -> Result<(), String> {
    let mut server = SERVER
        .lock()
        .map_err(|_| "Panorama runtime thread lock poisoned".to_string())?;
    if !start_needed(server.is_some()) {
        return Ok(());
    }

    MESSAGE_CACHE
        .lock()
        .map_err(|_| "Panorama message cache lock poisoned".to_string())?
        .clear();

    let listener = bind_listener(PORT)?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("Could not configure Panorama runtime listener: {error}"))?;
    SHUTDOWN.store(false, Ordering::SeqCst);
    let handle = thread::Builder::new()
        .name("split-panorama-runtime".to_string())
        .spawn(move || {
            println!("[PanoramaRuntime] listening on http://{HOST}:{PORT}");
            while !SHUTDOWN.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, address)) if address.ip().is_loopback() => {
                        handle_connection(stream, &app);
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20));
                    }
                    Err(error) => {
                        eprintln!("[PanoramaRuntime] accept failed: {error}");
                        thread::sleep(Duration::from_millis(50));
                    }
                }
            }
            RUNNING.store(false, Ordering::SeqCst);
        })
        .map_err(|error| format!("Could not start Panorama runtime thread: {error}"))?;
    RUNNING.store(true, Ordering::SeqCst);
    *server = Some(handle);
    Ok(())
}

pub(crate) fn stop() -> Result<(), String> {
    SHUTDOWN.store(true, Ordering::SeqCst);
    let handle = SERVER
        .lock()
        .map_err(|_| "Panorama runtime thread lock poisoned".to_string())?
        .take();
    if let Some(handle) = handle {
        handle
            .join()
            .map_err(|_| "Panorama runtime thread panicked".to_string())?;
    }
    RUNNING.store(false, Ordering::SeqCst);
    MESSAGE_CACHE
        .lock()
        .map_err(|_| "Panorama message cache lock poisoned".to_string())?
        .clear();
    crate::quick_access::reset_panorama_state();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn sample_state(sequence: u64) -> PanoramaState {
        PanoramaState {
            protocol_version: 1,
            sequence,
            visibility: Visibility::Interactive,
            active_preset: 2,
            active_preset_name: "Routes: α & β".to_string(),
            slots: vec![SlotState {
                index: 1,
                name: "Mid / Bridge #1".to_string(),
                populated: true,
            }],
            can_undo: true,
            can_redo: false,
        }
    }

    #[test]
    fn compact_state_payload_preserves_unicode_and_fields() {
        let state = sample_state(42);
        let payload: serde_json::Value =
            serde_json::from_slice(&encode_state_payload(&state).unwrap()).unwrap();
        assert_eq!(payload[0], 1);
        assert_eq!(payload[1], 42);
        assert_eq!(payload[2], 2);
        assert_eq!(payload[3], 2);
        assert_eq!(payload[4], "Routes: α & β");
        assert_eq!(payload[5][0], serde_json::json!([1, "Mid / Bridge #1", 1]));
        assert_eq!(payload[6], 1);
        assert_eq!(payload[7], 0);
    }

    #[test]
    fn crc16_matches_ccitt_false_reference_vector() {
        assert_eq!(crc16_ccitt(b"123456789"), 0x29b1);
    }

    #[test]
    fn framing_preserves_payload_length_crc_and_end_marker() {
        let payload = b"abcdefghijklmnopqrst";
        let frames = build_frames(0x1234, payload).unwrap();
        assert_eq!(frames.len(), 3);

        let mut rebuilt = Vec::new();
        for (round, frame) in frames.iter().enumerate() {
            assert_eq!(frame[0] & 0xf0, FRAME_MAGIC);
            assert_eq!(frame[0] & 0x07, FRAME_VERSION);
            assert_eq!(u16::from_le_bytes([frame[1], frame[2]]), 0x1234);
            assert_eq!(usize::from(frame[3]), round);
            assert_eq!(
                u16::from_le_bytes([frame[14], frame[15]]),
                crc16_ccitt(&frame[..14])
            );
            assert_eq!(frame[0] & FRAME_END_FLAG != 0, round == 2);
            rebuilt.extend_from_slice(&frame[5..5 + usize::from(frame[4])]);
        }
        assert_eq!(frames[2][4], 2);
        assert_eq!(rebuilt, payload);
    }

    #[test]
    fn frame_bits_are_little_endian_within_each_byte() {
        let mut frame = [0_u8; FRAME_BYTES];
        frame[0] = 0b1000_0001;
        assert!(frame_bit(&frame, 0).unwrap());
        assert!(!frame_bit(&frame, 1).unwrap());
        assert!(frame_bit(&frame, 7).unwrap());
        assert!(frame_bit(&frame, FRAME_BITS).is_err());
    }

    #[test]
    fn cached_message_is_stable_across_rounds_and_retries() {
        let payload = b"stable snapshot across more than one frame".to_vec();
        let mut cache = Vec::new();
        let mut builds = 0;
        let frame_count = (payload.len() + FRAME_PAYLOAD_BYTES - 1) / FRAME_PAYLOAD_BYTES;
        let mut rebuilt = Vec::new();
        let mut first = None;
        for round in 0..frame_count {
            let frame = cached_frame_with(
                &mut cache,
                BitRequest {
                    message: 17,
                    round,
                    bit: 0,
                },
                || {
                    builds += 1;
                    Ok(payload.clone())
                },
            )
            .unwrap();
            if round == 0 {
                first = Some(frame);
            }
            rebuilt.extend_from_slice(&frame[5..5 + usize::from(frame[4])]);
        }
        let retry = cached_frame_with(
            &mut cache,
            BitRequest {
                message: 17,
                round: 0,
                bit: 127,
            },
            || {
                builds += 1;
                Ok(b"different".to_vec())
            },
        )
        .unwrap();
        assert_eq!(builds, 1);
        assert_eq!(first.unwrap(), retry);
        assert_eq!(rebuilt, payload);
    }

    #[test]
    fn bit_request_rejects_missing_or_out_of_range_fields() {
        assert_eq!(
            parse_bit_request("message=42&round=3&bit=127").unwrap(),
            BitRequest {
                message: 42,
                round: 3,
                bit: 127,
            }
        );
        assert!(parse_bit_request("round=0&bit=0").is_err());
        assert!(parse_bit_request("message=0&round=0&bit=0").is_err());
        assert!(parse_bit_request("message=1&round=256&bit=0").is_err());
        assert!(parse_bit_request("message=1&round=0&bit=128").is_err());
        assert!(parse_bit_request("message=nope&round=0&bit=0").is_err());
    }

    #[test]
    fn unknown_round_is_rejected_without_rebuilding_cached_message() {
        let mut cache = Vec::new();
        cached_frame_with(
            &mut cache,
            BitRequest {
                message: 9,
                round: 0,
                bit: 0,
            },
            || Ok(b"short".to_vec()),
        )
        .unwrap();
        assert!(cached_frame_with(
            &mut cache,
            BitRequest {
                message: 9,
                round: 1,
                bit: 0,
            },
            || panic!("cached message must remain stable"),
        )
        .is_err());
    }

    #[test]
    fn sequence_increments_monotonically() {
        let first = next_sequence();
        let second = next_sequence();
        assert!(second > first);
    }

    #[test]
    fn invalid_actions_and_slot_bounds_are_rejected() {
        assert!(parse_action("action=launch_process").is_err());
        assert!(parse_action("action=load_slot&slot=0").is_err());
        assert!(parse_action("action=save_slot&slot=9").is_err());
        assert!(parse_action("action=load_slot").is_err());
    }

    struct RecordingExecutor(Mutex<Vec<Action>>);

    impl ActionExecutor for RecordingExecutor {
        fn execute(&self, action: Action) -> Result<(), String> {
            self.0.lock().unwrap().push(action);
            Ok(())
        }
    }

    #[test]
    fn known_actions_are_accepted_and_routed() {
        let executor = RecordingExecutor(Mutex::new(Vec::new()));
        route_action(&executor, "action=load_slot&slot=8").unwrap();
        route_action(&executor, "action=save_slot&slot=1").unwrap();
        route_action(&executor, "action=undo").unwrap();
        route_action(&executor, "action=redo").unwrap();
        route_action(&executor, "action=previous_preset").unwrap();
        route_action(&executor, "action=next_preset").unwrap();
        assert_eq!(
            *executor.0.lock().unwrap(),
            vec![
                Action::LoadSlot(8),
                Action::SaveSlot(1),
                Action::Undo,
                Action::Redo,
                Action::PreviousPreset,
                Action::NextPreset,
            ]
        );
    }

    #[test]
    fn listener_is_bound_to_ipv4_localhost_only() {
        let listener = bind_listener(0).unwrap();
        assert_eq!(
            listener.local_addr().unwrap().ip(),
            IpAddr::V4(Ipv4Addr::LOCALHOST)
        );
    }

    #[test]
    fn lifecycle_start_decision_is_idempotent() {
        assert!(start_needed(false));
        assert!(!start_needed(true));
    }
}
