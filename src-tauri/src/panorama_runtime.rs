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
const PROTOCOL: &str = "SPLIT_V1";

static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static RUNNING: AtomicBool = AtomicBool::new(false);
static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static SERVER: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

const BRIDGE_HTML: &str = r#"<!doctype html><meta charset="utf-8"><title>SPLIT</title><script>
const poll=()=>fetch('/state',{cache:'no-store'}).then(r=>{if(!r.ok)throw Error(r.status);return r.text()}).then(t=>{document.title=t}).catch(()=>{}).finally(()=>setTimeout(poll,100));poll();
</script>"#;

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

fn percent_encode(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(test)]
fn percent_decode(value: &str) -> Result<String, String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err("Incomplete percent escape".to_string());
            }
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3])
                .map_err(|error| error.to_string())?;
            decoded.push(u8::from_str_radix(hex, 16).map_err(|error| error.to_string())?);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|error| error.to_string())
}

fn encode_title(state: &PanoramaState) -> Result<String, String> {
    let json = serde_json::to_string(state)
        .map_err(|error| format!("Could not serialize Panorama state: {error}"))?;
    Ok(format!(
        "{PROTOCOL}:{}:{}",
        state.sequence,
        percent_encode(&json)
    ))
}

fn query_value(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|part| {
        let (candidate, value) = part.split_once('=').unwrap_or((part, ""));
        (candidate == key).then(|| value.to_string())
    })
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
        "/bridge.html" => write_response(
            &mut stream,
            "200 OK",
            "text/html; charset=utf-8",
            BRIDGE_HTML.as_bytes(),
        ),
        "/state" => match current_state().and_then(|state| encode_title(&state)) {
            Ok(body) => write_response(
                &mut stream,
                "200 OK",
                "text/plain; charset=utf-8",
                body.as_bytes(),
            ),
            Err(error) => write_response(
                &mut stream,
                "503 Service Unavailable",
                "text/plain; charset=utf-8",
                error.as_bytes(),
            ),
        },
        "/action" => {
            let executor = AppActionExecutor { app: app.clone() };
            match route_action(&executor, query) {
                Ok(()) => {
                    write_response(&mut stream, "200 OK", "application/json", br#"{"ok":true}"#)
                }
                Err(error) => write_response(
                    &mut stream,
                    "400 Bad Request",
                    "application/json",
                    serde_json::json!({ "ok": false, "error": error })
                        .to_string()
                        .as_bytes(),
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
    fn state_round_trips_through_title_encoding() {
        let state = sample_state(42);
        let encoded = encode_title(&state).unwrap();
        let prefix = format!("{PROTOCOL}:42:");
        let payload = encoded.strip_prefix(&prefix).unwrap();
        let decoded = percent_decode(payload).unwrap();
        assert_eq!(
            serde_json::from_str::<PanoramaState>(&decoded).unwrap(),
            state
        );
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

    #[test]
    fn bridge_html_polls_only_the_local_state_endpoint() {
        assert!(BRIDGE_HTML.contains("fetch('/state'"));
        assert!(!BRIDGE_HTML.contains("http://"));
        assert!(!BRIDGE_HTML.contains("https://"));
    }
}
