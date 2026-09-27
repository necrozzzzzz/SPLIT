use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use tauri::AppHandle;

const HOST: &str = "127.0.0.1";
const PORT: u16 = 32145;
const PANORAMA_ACTIVE_WINDOW: Duration = Duration::from_secs(15);

static STARTED_AT: OnceLock<Instant> = OnceLock::new();
static LAST_PANORAMA_REQUEST: AtomicU64 = AtomicU64::new(0);
static PANORAMA_RENDERER_SELECTED: AtomicBool = AtomicBool::new(false);
static RUNNING: AtomicBool = AtomicBool::new(false);
static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static SERVER_THREAD: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

const PING_PNG: &[u8] = include_bytes!("../assets/panorama_bridge/ping_37x23.png");
const ACTION_PNG: &[u8] = include_bytes!("../assets/panorama_bridge/ack_17x13.png");
const HIDDEN_PNG: &[u8] = include_bytes!("../assets/panorama_bridge/hidden_11x11.png");
const PASSIVE_PNG: &[u8] = include_bytes!("../assets/panorama_bridge/passive_22x11.png");
const INTERACTIVE_PNG: &[u8] = include_bytes!("../assets/panorama_bridge/interactive_33x11.png");

fn elapsed_millis() -> u64 {
    STARTED_AT
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

fn request_age_at(now_millis: u64, request_stamp: u64) -> Option<u64> {
    (request_stamp != 0).then(|| now_millis.saturating_add(1).saturating_sub(request_stamp))
}

fn active_from_timestamp(now_millis: u64, request_stamp: u64) -> bool {
    request_age_at(now_millis, request_stamp)
        .is_some_and(|age| age <= PANORAMA_ACTIVE_WINDOW.as_millis() as u64)
}

fn mark_panorama_request(app: &AppHandle) {
    let now_millis = elapsed_millis();
    let previous_stamp = LAST_PANORAMA_REQUEST.load(Ordering::SeqCst);
    let was_active = active_from_timestamp(now_millis, previous_stamp);
    LAST_PANORAMA_REQUEST.store(now_millis.saturating_add(1), Ordering::SeqCst);
    let renderer_was_selected = PANORAMA_RENDERER_SELECTED.swap(true, Ordering::SeqCst);

    if !was_active || !renderer_was_selected {
        println!("[QuickAccess] renderer=panorama");
        if let Err(error) = crate::quick_access::suppress_external_window_for_panorama(app) {
            eprintln!("[QuickAccess] Could not activate Panorama renderer: {error}");
        }
    }
}

pub(crate) fn panorama_request_age_ms() -> Option<u64> {
    request_age_at(
        elapsed_millis(),
        LAST_PANORAMA_REQUEST.load(Ordering::SeqCst),
    )
}

pub(crate) fn is_panorama_active() -> bool {
    let age = panorama_request_age_ms();
    let active = age.is_some_and(|value| value <= PANORAMA_ACTIVE_WINDOW.as_millis() as u64);
    if !active && PANORAMA_RENDERER_SELECTED.swap(false, Ordering::SeqCst) {
        println!(
            "[QuickAccess] renderer=windows panorama_age_ms={}",
            age.map_or_else(|| "none".to_string(), |value| value.to_string())
        );
    }
    active
}

fn state_dimensions(visible: bool, interactive: bool) -> (u32, u32) {
    if interactive {
        (33, 11)
    } else if visible {
        (22, 11)
    } else {
        (11, 11)
    }
}

fn state_png() -> &'static [u8] {
    let (width, _) = state_dimensions(
        crate::quick_access::is_visible(),
        crate::quick_access::is_interactive(),
    );
    match width {
        33 => INTERACTIVE_PNG,
        22 => PASSIVE_PNG,
        _ => HIDDEN_PNG,
    }
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decode_query_component(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                decoded.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let high = hex_value(bytes[index + 1])?;
                let low = hex_value(bytes[index + 2])?;
                decoded.push((high << 4) | low);
                index += 3;
            }
            b'%' => return None,
            byte => {
                decoded.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded).ok()
}

fn query_value(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let decoded_key = decode_query_component(raw_key)?;
        (decoded_key == key)
            .then(|| decode_query_component(raw_value))
            .flatten()
    })
}

fn write_response(stream: &mut TcpStream, status: &str, content_type: &str, body: &[u8]) {
    let headers = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        body.len(),
    );
    let _ = stream.write_all(headers.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn handle_connection(mut stream: TcpStream, app: &AppHandle) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));

    let mut buffer = [0u8; 4096];
    let Ok(count) = stream.read(&mut buffer) else {
        return;
    };
    if count == 0 {
        return;
    }

    let Ok(request) = std::str::from_utf8(&buffer[..count]) else {
        write_response(&mut stream, "400 Bad Request", "text/plain", b"Bad Request");
        return;
    };
    let Some(request_line) = request.lines().next() else {
        return;
    };
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    if method != "GET" || target.len() > 2048 {
        write_response(&mut stream, "400 Bad Request", "text/plain", b"Bad Request");
        return;
    }

    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    match path {
        "/ping.png" => {
            mark_panorama_request(app);
            println!("[PanoramaBridge] ping request received");
            write_response(&mut stream, "200 OK", "image/png", PING_PNG);
            println!(
                "[PanoramaBridge] ping response bytes={} logical=37x23",
                PING_PNG.len()
            );
        }
        "/state.png" => {
            mark_panorama_request(app);
            write_response(&mut stream, "200 OK", "image/png", state_png());
        }
        "/action.png" => {
            mark_panorama_request(app);
            let action = query_value(query, "action").unwrap_or_default();
            println!("[PanoramaBridge] action={action}");
            write_response(&mut stream, "200 OK", "image/png", ACTION_PNG);
        }
        _ => write_response(&mut stream, "404 Not Found", "text/plain", b"Not Found"),
    }
}

pub(crate) fn start(app: AppHandle) -> Result<(), String> {
    let mut thread_slot = SERVER_THREAD
        .lock()
        .map_err(|_| "Panorama bridge thread lock poisoned".to_string())?;
    if thread_slot.is_some() {
        return Ok(());
    }

    let listener = TcpListener::bind((HOST, PORT))
        .map_err(|error| format!("Could not bind Panorama bridge to {HOST}:{PORT}: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("Could not configure Panorama bridge listener: {error}"))?;

    SHUTDOWN.store(false, Ordering::SeqCst);
    RUNNING.store(true, Ordering::SeqCst);
    let handle = thread::Builder::new()
        .name("split-panorama-bridge".to_string())
        .spawn(move || {
            println!("[PanoramaBridge] listening on {HOST}:{PORT}");
            while !SHUTDOWN.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => handle_connection(stream, &app),
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20));
                    }
                    Err(error) => {
                        eprintln!("[PanoramaBridge] accept failed: {error}");
                        thread::sleep(Duration::from_millis(50));
                    }
                }
            }
            RUNNING.store(false, Ordering::SeqCst);
        })
        .map_err(|error| format!("Could not start Panorama bridge thread: {error}"))?;

    *thread_slot = Some(handle);
    Ok(())
}

pub(crate) fn stop() -> Result<(), String> {
    SHUTDOWN.store(true, Ordering::SeqCst);
    LAST_PANORAMA_REQUEST.store(0, Ordering::SeqCst);
    PANORAMA_RENDERER_SELECTED.store(false, Ordering::SeqCst);
    let handle = SERVER_THREAD
        .lock()
        .map_err(|_| "Panorama bridge thread lock poisoned".to_string())?
        .take();
    if let Some(handle) = handle {
        handle
            .join()
            .map_err(|_| "Panorama bridge thread panicked".to_string())?;
    }
    RUNNING.store(false, Ordering::SeqCst);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dimensions(png: &[u8]) -> (u32, u32) {
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        (
            u32::from_be_bytes(png[16..20].try_into().unwrap()),
            u32::from_be_bytes(png[20..24].try_into().unwrap()),
        )
    }

    #[test]
    fn embedded_png_dimensions_match_protocol() {
        assert_eq!(dimensions(PING_PNG), (37, 23));
        assert_eq!(dimensions(ACTION_PNG), (17, 13));
        assert_eq!(dimensions(HIDDEN_PNG), (11, 11));
        assert_eq!(dimensions(PASSIVE_PNG), (22, 11));
        assert_eq!(dimensions(INTERACTIVE_PNG), (33, 11));
    }

    #[test]
    fn state_dimensions_follow_existing_quick_access_state() {
        assert_eq!(state_dimensions(false, false), (11, 11));
        assert_eq!(state_dimensions(true, false), (22, 11));
        assert_eq!(state_dimensions(true, true), (33, 11));
    }

    #[test]
    fn action_query_is_percent_decoded() {
        assert_eq!(
            query_value("nonce=1&action=slot%31", "action").as_deref(),
            Some("slot1"),
        );
    }

    #[test]
    fn panorama_activity_expires_after_fifteen_seconds() {
        assert!(active_from_timestamp(14_999, 1));
        assert!(active_from_timestamp(15_000, 1));
        assert!(!active_from_timestamp(15_001, 1));
        assert!(!active_from_timestamp(0, 0));
    }
}
