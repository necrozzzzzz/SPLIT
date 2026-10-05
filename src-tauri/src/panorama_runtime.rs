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
const STATE_PROTOCOL_VERSION: u8 = 1;
const FAST_STATE_BYTES: usize = 9;
const METADATA_HISTORY_SIZE: usize = 4;
const METADATA_ITEM_COUNT: u8 = SLOT_COUNT + 1;
const METADATA_ITEM_HEADER_BYTES: usize = 7;

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static RUNNING: AtomicBool = AtomicBool::new(false);
static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static SERVER: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);
static MESSAGE_CACHE: Mutex<Vec<FramedMessage>> = Mutex::new(Vec::new());
static METADATA_TRACKER: Mutex<MetadataTracker> = Mutex::new(MetadataTracker {
    revision: 0,
    current: None,
    history: Vec::new(),
});
static STATE_BIT_HITS: AtomicU64 = AtomicU64::new(0);
static STATE_BIT_ONES: AtomicU64 = AtomicU64::new(0);

const BIT_ONE_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x04, 0x00, 0x00, 0x00, 0xb5, 0x1c, 0x0c,
    0x02, 0x00, 0x00, 0x00, 0x0b, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x64, 0xf8, 0x0f, 0x00,
    0x01, 0x05, 0x01, 0x01, 0x27, 0x18, 0xe3, 0x66, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44,
    0xae, 0x42, 0x60, 0x82,
];

#[derive(Debug, Clone, PartialEq, Eq)]
struct FramedMessage {
    kind: MessageKind,
    id: u16,
    metadata_revision: Option<u16>,
    metadata_preset: Option<u8>,
    metadata_item: Option<u8>,
    frames: Vec<[u8; FRAME_BYTES]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BitRequest {
    kind: MessageKind,
    message: u16,
    round: usize,
    bit: usize,
    metadata_revision: Option<u16>,
    metadata_preset: Option<u8>,
    metadata_item: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MessageKind {
    Fast,
    Metadata,
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct StateMetadata {
    active_preset: u8,
    active_preset_name: String,
    slot_names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VersionedMetadata {
    revision: u16,
    value: StateMetadata,
}

struct MetadataTracker {
    revision: u16,
    current: Option<StateMetadata>,
    history: Vec<VersionedMetadata>,
}

impl MetadataTracker {
    fn observe(&mut self, metadata: StateMetadata) -> u16 {
        if self.current.as_ref() != Some(&metadata) {
            self.revision = next_metadata_revision(self.revision);
            self.current = Some(metadata.clone());
            self.history.push(VersionedMetadata {
                revision: self.revision,
                value: metadata,
            });
            if self.history.len() > METADATA_HISTORY_SIZE {
                self.history.remove(0);
            }
        }
        self.revision
    }
}

#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct DecodedFastState {
    protocol_version: u8,
    sequence: u16,
    visibility: Visibility,
    active_preset: u8,
    populated_mask: u8,
    can_undo: bool,
    can_redo: bool,
    metadata_revision: u16,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
struct DecodedMetadataItem {
    protocol_version: u8,
    revision: u16,
    active_preset: u8,
    item: u8,
    value: String,
}

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
        protocol_version: STATE_PROTOCOL_VERSION,
        sequence: next_sequence(),
        visibility: visibility(),
        active_preset,
        active_preset_name,
        slots,
        can_undo: history.can_undo,
        can_redo: history.can_redo,
    })
}

fn state_metadata(state: &PanoramaState) -> StateMetadata {
    let slot_names = (1..=SLOT_COUNT)
        .map(|index| {
            state
                .slots
                .iter()
                .find(|slot| slot.index == index)
                .map(|slot| slot.name.clone())
                .unwrap_or_else(|| format!("Slot {index}"))
        })
        .collect();
    StateMetadata {
        active_preset: state.active_preset,
        active_preset_name: state.active_preset_name.clone(),
        slot_names,
    }
}

fn next_metadata_revision(revision: u16) -> u16 {
    let next = revision.wrapping_add(1);
    if next == 0 {
        1
    } else {
        next
    }
}

fn observe_metadata(metadata: StateMetadata) -> Result<u16, String> {
    let mut tracker = METADATA_TRACKER
        .lock()
        .map_err(|_| "Panorama metadata tracker lock poisoned".to_string())?;
    Ok(tracker.observe(metadata))
}

fn metadata_at_revision(revision: u16, active_preset: u8) -> Result<StateMetadata, String> {
    let metadata = METADATA_TRACKER
        .lock()
        .map_err(|_| "Panorama metadata tracker lock poisoned".to_string())?
        .history
        .iter()
        .find(|metadata| metadata.revision == revision)
        .map(|metadata| metadata.value.clone())
        .ok_or_else(|| format!("Unknown metadata revision {revision}"))?;
    if metadata.active_preset != active_preset {
        return Err(format!(
            "Metadata revision {revision} belongs to preset {}, not {active_preset}",
            metadata.active_preset
        ));
    }
    Ok(metadata)
}

fn populated_mask(state: &PanoramaState) -> u8 {
    state.slots.iter().fold(0_u8, |mask, slot| {
        if slot.populated && (1..=SLOT_COUNT).contains(&slot.index) {
            mask | (1 << (slot.index - 1))
        } else {
            mask
        }
    })
}

fn encode_fast_state(state: &PanoramaState, metadata_revision: u16) -> Vec<u8> {
    let visibility = match state.visibility {
        Visibility::Hidden => 0,
        Visibility::Passive => 1,
        Visibility::Interactive => 2,
    };
    let mut flags = visibility;
    flags |= u8::from(state.can_undo) << 2;
    flags |= u8::from(state.can_redo) << 3;
    let sequence = state.sequence as u16;
    let mut payload = vec![0_u8; FAST_STATE_BYTES];
    payload[0] = state.protocol_version;
    payload[1..3].copy_from_slice(&sequence.to_le_bytes());
    payload[3] = flags;
    payload[4] = state.active_preset;
    payload[5] = populated_mask(state);
    payload[6..8].copy_from_slice(&metadata_revision.to_le_bytes());
    payload
}

#[cfg(test)]
fn decode_fast_state(payload: &[u8]) -> Result<DecodedFastState, String> {
    if payload.len() != FAST_STATE_BYTES {
        return Err(format!("Invalid fast state length {}", payload.len()));
    }
    if payload[0] != STATE_PROTOCOL_VERSION {
        return Err(format!("Unsupported fast state version {}", payload[0]));
    }
    let visibility = match payload[3] & 0x03 {
        0 => Visibility::Hidden,
        1 => Visibility::Passive,
        2 => Visibility::Interactive,
        value => return Err(format!("Invalid visibility {value}")),
    };
    if payload[3] & 0xf0 != 0 || payload[8] != 0 {
        return Err("Fast state reserved bits are not zero".to_string());
    }
    Ok(DecodedFastState {
        protocol_version: payload[0],
        sequence: u16::from_le_bytes([payload[1], payload[2]]),
        visibility,
        active_preset: payload[4],
        populated_mask: payload[5],
        can_undo: payload[3] & 0x04 != 0,
        can_redo: payload[3] & 0x08 != 0,
        metadata_revision: u16::from_le_bytes([payload[6], payload[7]]),
    })
}

fn metadata_item_value(metadata: &StateMetadata, item: u8) -> Result<&str, String> {
    match item {
        0 => Ok(&metadata.active_preset_name),
        1..=SLOT_COUNT => metadata
            .slot_names
            .get(usize::from(item - 1))
            .map(String::as_str)
            .ok_or_else(|| format!("Missing metadata item {item}")),
        _ => Err(format!("Invalid metadata item {item}")),
    }
}

fn encode_metadata_item_payload(
    revision: u16,
    metadata: &StateMetadata,
    item: u8,
) -> Result<Vec<u8>, String> {
    let value = metadata_item_value(metadata, item)?.as_bytes();
    let length = u16::try_from(value.len())
        .map_err(|_| format!("Metadata item {item} is too long: {} bytes", value.len()))?;
    let mut payload = Vec::with_capacity(METADATA_ITEM_HEADER_BYTES + value.len());
    payload.push(STATE_PROTOCOL_VERSION);
    payload.extend_from_slice(&revision.to_le_bytes());
    payload.push(metadata.active_preset);
    payload.push(item);
    payload.extend_from_slice(&length.to_le_bytes());
    payload.extend_from_slice(value);
    Ok(payload)
}

#[cfg(test)]
fn decode_metadata_item_payload(payload: &[u8]) -> Result<DecodedMetadataItem, String> {
    if payload.len() < METADATA_ITEM_HEADER_BYTES {
        return Err("Metadata item payload is truncated".to_string());
    }
    if payload[0] != STATE_PROTOCOL_VERSION {
        return Err(format!("Unsupported metadata version {}", payload[0]));
    }
    let item = payload[4];
    if item >= METADATA_ITEM_COUNT {
        return Err(format!("Invalid metadata item {item}"));
    }
    let length = usize::from(u16::from_le_bytes([payload[5], payload[6]]));
    if payload.len() != METADATA_ITEM_HEADER_BYTES + length {
        return Err("Metadata item length mismatch".to_string());
    }
    let value = std::str::from_utf8(&payload[METADATA_ITEM_HEADER_BYTES..])
        .map_err(|error| format!("Invalid metadata UTF-8: {error}"))?
        .to_string();
    Ok(DecodedMetadataItem {
        protocol_version: payload[0],
        revision: u16::from_le_bytes([payload[1], payload[2]]),
        active_preset: payload[3],
        item,
        value,
    })
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
    let kind = match query_value(query, "kind").as_deref() {
        Some("fast") => MessageKind::Fast,
        Some("metadata") => MessageKind::Metadata,
        Some(value) => return Err(format!("Invalid kind {value}")),
        None => return Err("Missing kind".to_string()),
    };
    let (metadata_revision, metadata_preset, metadata_item) = match kind {
        MessageKind::Fast => (None, None, None),
        MessageKind::Metadata => {
            let revision = parse("revision")?;
            if revision == 0 || revision > usize::from(u16::MAX) {
                return Err(format!("Invalid revision {revision}"));
            }
            let preset = parse("preset")?;
            if preset == 0 || preset > usize::from(u8::MAX) {
                return Err(format!("Invalid preset {preset}"));
            }
            let item = parse("item")?;
            if item >= usize::from(METADATA_ITEM_COUNT) {
                return Err(format!("Invalid metadata item {item}"));
            }
            (Some(revision as u16), Some(preset as u8), Some(item as u8))
        }
    };
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
        kind,
        message: message as u16,
        round,
        bit,
        metadata_revision,
        metadata_preset,
        metadata_item,
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
    let message_index = if let Some(index) = cache.iter().position(|item| {
        item.kind == request.kind
            && item.id == request.message
            && item.metadata_revision == request.metadata_revision
            && item.metadata_preset == request.metadata_preset
            && item.metadata_item == request.metadata_item
    }) {
        index
    } else {
        let frames = build_frames(request.message, &create_payload()?)?;
        if cache.len() >= MAX_CACHED_MESSAGES {
            cache.remove(0);
        }
        cache.push(FramedMessage {
            kind: request.kind,
            id: request.message,
            metadata_revision: request.metadata_revision,
            metadata_preset: request.metadata_preset,
            metadata_item: request.metadata_item,
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
    let frame = cached_frame_with(&mut cache, request, || match request.kind {
        MessageKind::Fast => {
            let state = current_state()?;
            let revision = observe_metadata(state_metadata(&state))?;
            Ok(encode_fast_state(&state, revision))
        }
        MessageKind::Metadata => {
            let revision = request
                .metadata_revision
                .ok_or_else(|| "Missing metadata revision".to_string())?;
            let active_preset = request
                .metadata_preset
                .ok_or_else(|| "Missing metadata preset".to_string())?;
            let item = request
                .metadata_item
                .ok_or_else(|| "Missing metadata item".to_string())?;
            let metadata = metadata_at_revision(revision, active_preset)?;
            encode_metadata_item_payload(revision, &metadata, item)
        }
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
        "/ipc/state-bit" => {
            STATE_BIT_HITS.fetch_add(1, Ordering::SeqCst);

            match parse_bit_request(query)
                .and_then(|request| state_bit(request).map(|is_one| (request, is_one)))
            {
                Ok((_request, true)) => {
                    STATE_BIT_ONES.fetch_add(1, Ordering::SeqCst);
                    write_response(&mut stream, "200 OK", "image/png", BIT_ONE_PNG)
                }
                Ok((_request, false)) => {
                    write_response(&mut stream, "404 Not Found", "image/png", &[])
                }
                Err(error) => write_response(
                    &mut stream,
                    "400 Bad Request",
                    "text/plain; charset=utf-8",
                    error.as_bytes(),
                ),
            }
        }

        "/debug-ipc" => {
            let body = format!(
                "hits={} ones={}",
                STATE_BIT_HITS.load(Ordering::SeqCst),
                STATE_BIT_ONES.load(Ordering::SeqCst)
            );

            write_response(
                &mut stream,
                "200 OK",
                "text/plain; charset=utf-8",
                body.as_bytes(),
            )
        }

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
    *METADATA_TRACKER
        .lock()
        .map_err(|_| "Panorama metadata tracker lock poisoned".to_string())? = MetadataTracker {
        revision: 0,
        current: None,
        history: Vec::new(),
    };

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
    *METADATA_TRACKER
        .lock()
        .map_err(|_| "Panorama metadata tracker lock poisoned".to_string())? = MetadataTracker {
        revision: 0,
        current: None,
        history: Vec::new(),
    };
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
    fn fast_state_round_trip_preserves_fields_in_one_frame() {
        let state = sample_state(42);
        let payload = encode_fast_state(&state, 17);
        let decoded = decode_fast_state(&payload).unwrap();
        assert_eq!(payload.len(), FAST_STATE_BYTES);
        assert_eq!(decoded.protocol_version, STATE_PROTOCOL_VERSION);
        assert_eq!(decoded.sequence, 42);
        assert_eq!(decoded.visibility, Visibility::Interactive);
        assert_eq!(decoded.active_preset, 2);
        assert_eq!(decoded.populated_mask, 0x01);
        assert!(decoded.can_undo);
        assert!(!decoded.can_redo);
        assert_eq!(decoded.metadata_revision, 17);
        assert_eq!(build_frames(7, &payload).unwrap().len(), 1);
    }

    #[test]
    fn fast_state_sequence_wrap_is_intentional() {
        let state = sample_state(u64::from(u16::MAX) + 3);
        let decoded = decode_fast_state(&encode_fast_state(&state, 1)).unwrap();
        assert_eq!(decoded.sequence, 2);
    }

    #[test]
    fn metadata_item_binary_payload_preserves_unicode_and_identity() {
        let metadata = StateMetadata {
            active_preset: 2,
            active_preset_name: "Routes: α & β".to_string(),
            slot_names: vec![
                "Mid / Bridge #1".to_string(),
                String::new(),
                "東京".to_string(),
                "Slot 4".to_string(),
                "Slot 5".to_string(),
                "Slot 6".to_string(),
                "Slot 7".to_string(),
                "Slot 8".to_string(),
            ],
        };
        let decoded =
            decode_metadata_item_payload(&encode_metadata_item_payload(9, &metadata, 0).unwrap())
                .unwrap();
        assert_eq!(decoded.protocol_version, STATE_PROTOCOL_VERSION);
        assert_eq!(decoded.revision, 9);
        assert_eq!(decoded.active_preset, 2);
        assert_eq!(decoded.item, 0);
        assert_eq!(decoded.value, "Routes: α & β");

        let unicode =
            decode_metadata_item_payload(&encode_metadata_item_payload(9, &metadata, 3).unwrap())
                .unwrap();
        assert_eq!(unicode.value, "東京");
    }

    #[test]
    fn metadata_item_binary_payload_preserves_empty_name() {
        let mut metadata = state_metadata(&sample_state(1));
        metadata.slot_names[1] = String::new();
        let decoded =
            decode_metadata_item_payload(&encode_metadata_item_payload(4, &metadata, 2).unwrap())
                .unwrap();
        assert_eq!(decoded.item, 2);
        assert_eq!(decoded.value, "");
    }

    #[test]
    fn long_metadata_item_uses_multiple_frames() {
        let mut metadata = state_metadata(&sample_state(1));
        metadata.slot_names[0] = "long metadata value ".repeat(8);
        let encoded = encode_metadata_item_payload(5, &metadata, 1).unwrap();
        assert_eq!(
            encoded.len(),
            METADATA_ITEM_HEADER_BYTES + metadata.slot_names[0].len()
        );
        assert!(build_frames(8, &encoded).unwrap().len() > 1);
    }

    #[test]
    fn metadata_revision_changes_only_when_metadata_changes() {
        let mut tracker = MetadataTracker {
            revision: 0,
            current: None,
            history: Vec::new(),
        };
        let metadata = state_metadata(&sample_state(1));
        let first = tracker.observe(metadata.clone());
        assert_eq!(tracker.observe(metadata.clone()), first);

        let mut changed = metadata;
        changed.slot_names[0] = "Changed".to_string();
        let second = tracker.observe(changed);
        assert_eq!(second, next_metadata_revision(first));
        assert_eq!(tracker.history.len(), 2);
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
                    kind: MessageKind::Fast,
                    message: 17,
                    round,
                    bit: 0,
                    metadata_revision: None,
                    metadata_preset: None,
                    metadata_item: None,
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
                kind: MessageKind::Fast,
                message: 17,
                round: 0,
                bit: 127,
                metadata_revision: None,
                metadata_preset: None,
                metadata_item: None,
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
    fn metadata_items_are_cached_independently() {
        let mut cache = Vec::new();
        let request = |item| BitRequest {
            kind: MessageKind::Metadata,
            message: 23,
            round: 0,
            bit: 0,
            metadata_revision: Some(7),
            metadata_preset: Some(2),
            metadata_item: Some(item),
        };
        let first = cached_frame_with(&mut cache, request(1), || Ok(b"first".to_vec())).unwrap();
        let second = cached_frame_with(&mut cache, request(2), || Ok(b"second".to_vec())).unwrap();
        let first_retry = cached_frame_with(&mut cache, request(1), || {
            panic!("the completed item snapshot must remain cached")
        })
        .unwrap();
        assert_ne!(first, second);
        assert_eq!(first, first_retry);
        assert_eq!(cache.len(), 2);
    }

    #[test]
    fn bit_request_rejects_missing_or_out_of_range_fields() {
        assert_eq!(
            parse_bit_request(
                "kind=metadata&revision=9&preset=2&item=6&message=42&round=3&bit=127",
            )
            .unwrap(),
            BitRequest {
                kind: MessageKind::Metadata,
                message: 42,
                round: 3,
                bit: 127,
                metadata_revision: Some(9),
                metadata_preset: Some(2),
                metadata_item: Some(6),
            }
        );
        assert!(parse_bit_request("round=0&bit=0").is_err());
        assert!(parse_bit_request("message=0&round=0&bit=0").is_err());
        assert!(parse_bit_request("message=1&round=256&bit=0").is_err());
        assert!(parse_bit_request("message=1&round=0&bit=128").is_err());
        assert!(parse_bit_request("message=nope&round=0&bit=0").is_err());
        assert!(
            parse_bit_request("kind=metadata&revision=9&item=1&message=1&round=0&bit=0").is_err()
        );
        assert!(parse_bit_request(
            "kind=metadata&revision=9&preset=2&item=9&message=1&round=0&bit=0"
        )
        .is_err());
    }

    #[test]
    fn unknown_round_is_rejected_without_rebuilding_cached_message() {
        let mut cache = Vec::new();
        cached_frame_with(
            &mut cache,
            BitRequest {
                kind: MessageKind::Fast,
                message: 9,
                round: 0,
                bit: 0,
                metadata_revision: None,
                metadata_preset: None,
                metadata_item: None,
            },
            || Ok(b"short".to_vec()),
        )
        .unwrap();
        assert!(cached_frame_with(
            &mut cache,
            BitRequest {
                kind: MessageKind::Fast,
                message: 9,
                round: 1,
                bit: 0,
                metadata_revision: None,
                metadata_preset: None,
                metadata_item: None,
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
