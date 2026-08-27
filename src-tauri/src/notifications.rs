use std::{
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Mutex,
    },
    thread::{self, JoinHandle},
};

use serde::{Deserialize, Deserializer, Serialize};

use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, POINT, RECT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, ClientToScreen, CreateFontW, CreateRoundRectRgn, CreateSolidBrush,
        DeleteObject, DrawTextW, EndPaint, FillRect, InvalidateRect, SelectObject, SetBkMode,
        SetTextColor, SetWindowRgn, UpdateWindow, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET,
        DEFAULT_PITCH, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FF_DONTCARE, FW_SEMIBOLD,
        OUT_DEFAULT_PRECIS, PAINTSTRUCT, TRANSPARENT,
    },
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
        GetMessageW, KillTimer, PostQuitMessage, PostThreadMessageW, RegisterClassW,
        SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, TranslateMessage,
        CS_HREDRAW, CS_VREDRAW, HTTRANSPARENT, HWND_TOPMOST, LWA_ALPHA, MA_NOACTIVATE, MSG,
        SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE, WM_APP, WM_CREATE, WM_DESTROY,
        WM_ERASEBKGND, WM_MOUSEACTIVATE, WM_NCCREATE, WM_NCHITTEST, WM_PAINT, WM_TIMER, WNDCLASSW,
        WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
    },
};

const COMMAND_MESSAGE: u32 = WM_APP + 41;
const HIDE_TIMER_ID: usize = 1;
const OVERLAY_WIDTH: i32 = 264;
const OVERLAY_HEIGHT: i32 = 56;
const MARGIN: i32 = 24;

enum Command {
    Show {
        text: String,
        settings: NotificationSettings,
    },
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NotificationPosition {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl<'de> Deserialize<'de> for NotificationPosition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "topLeft" => Self::TopLeft,
            "bottomLeft" => Self::BottomLeft,
            "bottomRight" => Self::BottomRight,
            _ => Self::TopRight,
        })
    }
}

impl Default for NotificationPosition {
    fn default() -> Self {
        Self::TopRight
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct NotificationSettings {
    pub enabled: bool,
    pub position: NotificationPosition,
    #[serde(deserialize_with = "deserialize_duration")]
    pub duration_ms: u32,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            position: NotificationPosition::TopRight,
            duration_ms: 1_500,
        }
    }
}

impl NotificationSettings {
    pub fn validate(&self) -> Result<(), String> {
        if valid_duration(self.duration_ms) {
            Ok(())
        } else {
            Err(format!(
                "Invalid notification duration: {} ms",
                self.duration_ms
            ))
        }
    }
}

fn deserialize_duration<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| valid_duration(*value))
        .unwrap_or(1_500))
}

const fn valid_duration(duration_ms: u32) -> bool {
    matches!(duration_ms, 1_000 | 1_500 | 2_000 | 3_000)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notification {
    Preset(u8),
    SlotSaved { slot: u8, favorite: bool },
    SlotLoaded { slot: u8, favorite: bool },
    SlotEmpty { slot: u8, favorite: bool },
    Favorites(bool),
    Undo,
    Redo,
    NothingToUndo,
    NothingToRedo,
    SaveFailed,
}

impl Notification {
    fn text(self) -> String {
        match self {
            Self::Preset(preset) => format!("SPLIT · Preset {preset}"),
            Self::SlotSaved { slot, favorite } => {
                format!("SPLIT · {} {slot} saved", slot_kind(favorite))
            }
            Self::SlotLoaded { slot, favorite } => {
                format!("SPLIT · {} {slot} loaded", slot_kind(favorite))
            }
            Self::SlotEmpty { slot, favorite } => {
                format!("SPLIT · {} {slot} empty", slot_kind(favorite))
            }
            Self::Favorites(true) => "SPLIT · Favorites enabled".to_string(),
            Self::Favorites(false) => "SPLIT · Favorites disabled".to_string(),
            Self::Undo => "SPLIT · Undo".to_string(),
            Self::Redo => "SPLIT · Redo".to_string(),
            Self::NothingToUndo => "SPLIT · Nothing to undo".to_string(),
            Self::NothingToRedo => "SPLIT · Nothing to redo".to_string(),
            Self::SaveFailed => "SPLIT · Save failed".to_string(),
        }
    }
}

fn slot_kind(favorite: bool) -> &'static str {
    if favorite {
        "Favorite"
    } else {
        "Slot"
    }
}

struct Runtime {
    sender: mpsc::Sender<Command>,
    thread_id: u32,
    join: JoinHandle<()>,
}

static RUNTIME: Mutex<Option<Runtime>> = Mutex::new(None);
static DISPLAY_TEXT: Mutex<String> = Mutex::new(String::new());
static WINDOW_READY: AtomicBool = AtomicBool::new(false);
static SETTINGS: Mutex<NotificationSettings> = Mutex::new(NotificationSettings {
    enabled: true,
    position: NotificationPosition::TopRight,
    duration_ms: 1_500,
});

pub fn start() -> Result<(), String> {
    apply_settings(crate::deadlock::get_notification_settings());
    let mut runtime = RUNTIME
        .lock()
        .map_err(|_| "Notification runtime lock poisoned".to_string())?;

    if runtime.is_some() {
        return Ok(());
    }

    let (sender, receiver) = mpsc::channel();
    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    let join = thread::Builder::new()
        .name("split-native-notifications".to_string())
        .spawn(move || notification_thread(receiver, ready_sender))
        .map_err(|error| format!("Could not start notification thread: {error}"))?;

    let thread_id = match ready_receiver.recv() {
        Ok(Ok(thread_id)) => thread_id,
        Ok(Err(error)) => {
            let _ = join.join();
            return Err(error);
        }
        Err(_) => {
            let _ = join.join();
            return Err("Notification thread stopped during startup".to_string());
        }
    };

    *runtime = Some(Runtime {
        sender,
        thread_id,
        join,
    });
    println!("[SPLIT] Overlay service started");
    Ok(())
}

pub fn show(notification: Notification) {
    let Some((text, settings)) = prepare_notification(notification) else {
        return;
    };
    let Ok(runtime) = RUNTIME.lock() else {
        return;
    };
    let Some(runtime) = runtime.as_ref() else {
        return;
    };

    if runtime
        .sender
        .send(Command::Show { text, settings })
        .is_ok()
    {
        unsafe {
            PostThreadMessageW(runtime.thread_id, COMMAND_MESSAGE, 0, 0);
        }
    }
}

pub fn apply_settings(settings: NotificationSettings) {
    if let Ok(mut current) = SETTINGS.lock() {
        *current = settings;
    }
}

fn prepare_notification(notification: Notification) -> Option<(String, NotificationSettings)> {
    let settings = SETTINGS.lock().ok()?.clone();
    settings.enabled.then(|| (notification.text(), settings))
}

pub fn stop() -> Result<(), String> {
    let runtime = RUNTIME
        .lock()
        .map_err(|_| "Notification runtime lock poisoned".to_string())?
        .take();

    if let Some(runtime) = runtime {
        runtime
            .sender
            .send(Command::Shutdown)
            .map_err(|_| "Notification thread is no longer available".to_string())?;

        if unsafe { PostThreadMessageW(runtime.thread_id, COMMAND_MESSAGE, 0, 0) } == 0 {
            return Err("Could not wake notification thread for shutdown".to_string());
        }

        runtime
            .join
            .join()
            .map_err(|_| "Notification thread panicked while stopping".to_string())?;
        println!("[SPLIT] Overlay service stopped");
    }

    Ok(())
}

fn overlay_position(client: RECT, position: NotificationPosition) -> (i32, i32) {
    match position {
        NotificationPosition::TopLeft => (client.left + MARGIN, client.top + MARGIN),
        NotificationPosition::TopRight => {
            (client.right - OVERLAY_WIDTH - MARGIN, client.top + MARGIN)
        }
        NotificationPosition::BottomLeft => (
            client.left + MARGIN,
            client.bottom - OVERLAY_HEIGHT - MARGIN,
        ),
        NotificationPosition::BottomRight => (
            client.right - OVERLAY_WIDTH - MARGIN,
            client.bottom - OVERLAY_HEIGHT - MARGIN,
        ),
    }
}

fn notification_thread(
    receiver: mpsc::Receiver<Command>,
    ready: mpsc::SyncSender<Result<u32, String>>,
) {
    let thread_id = unsafe { GetCurrentThreadId() };
    let hwnd = match create_overlay_window() {
        Ok(hwnd) => hwnd,
        Err(error) => {
            let _ = ready.send(Err(error));
            return;
        }
    };

    if ready.send(Ok(thread_id)).is_err() {
        unsafe { DestroyWindow(hwnd) };
        return;
    }

    let mut message = MSG::default();
    loop {
        let status = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
        if status <= 0 {
            break;
        }

        if message.message == COMMAND_MESSAGE {
            let mut latest = None;
            let mut shutdown = false;
            for command in receiver.try_iter() {
                match command {
                    Command::Show { text, settings } => latest = Some((text, settings)),
                    Command::Shutdown => shutdown = true,
                }
            }

            if shutdown {
                unsafe { DestroyWindow(hwnd) };
                continue;
            }

            if let Some((text, settings)) = latest {
                show_notification(hwnd, text, settings);
            }
            continue;
        }

        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn create_overlay_window() -> Result<HWND, String> {
    let class_name = wide("SPLIT.NativeNotification");
    let instance = unsafe { GetModuleHandleW(null()) };
    if instance.is_null() {
        return Err("Could not get the current module for the overlay window".to_string());
    }
    let window_class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: class_name.as_ptr(),
        ..Default::default()
    };

    if unsafe { RegisterClassW(&window_class) } == 0 {
        return Err("Could not register native notification window class".to_string());
    }

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED,
            class_name.as_ptr(),
            null(),
            WS_POPUP,
            0,
            0,
            OVERLAY_WIDTH,
            OVERLAY_HEIGHT,
            null_mut(),
            null_mut(),
            instance,
            null(),
        )
    };

    if hwnd.is_null() {
        return Err("Could not create native notification window".to_string());
    }
    WINDOW_READY.store(true, Ordering::Release);

    unsafe {
        SetLayeredWindowAttributes(hwnd, 0, 232, LWA_ALPHA);
        let region = CreateRoundRectRgn(0, 0, OVERLAY_WIDTH + 1, OVERLAY_HEIGHT + 1, 14, 14);
        if !region.is_null() {
            SetWindowRgn(hwnd, region, 0);
        }
    }

    Ok(hwnd)
}

fn show_notification(hwnd: HWND, text: String, settings: NotificationSettings) {
    let Some(deadlock) = crate::deadlock::foreground_deadlock_window() else {
        return;
    };

    let mut client = RECT::default();
    if unsafe { GetClientRect(deadlock, &mut client) } == 0 {
        return;
    }

    let mut top_left = POINT {
        x: client.left,
        y: client.top,
    };
    let mut bottom_right = POINT {
        x: client.right,
        y: client.bottom,
    };
    if unsafe {
        ClientToScreen(deadlock, &mut top_left) == 0
            || ClientToScreen(deadlock, &mut bottom_right) == 0
    } {
        return;
    }

    client = RECT {
        left: top_left.x,
        top: top_left.y,
        right: bottom_right.x,
        bottom: bottom_right.y,
    };
    let (x, y) = overlay_position(client, settings.position);

    if let Ok(mut display_text) = DISPLAY_TEXT.lock() {
        *display_text = text.clone();
    } else {
        return;
    }

    unsafe {
        KillTimer(hwnd, HIDE_TIMER_ID);
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            x,
            y,
            OVERLAY_WIDTH,
            OVERLAY_HEIGHT,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        InvalidateRect(hwnd, null(), 1);
        UpdateWindow(hwnd);
        SetTimer(hwnd, HIDE_TIMER_ID, settings.duration_ms, None);
    }

    let log_text = text.strip_prefix("SPLIT · ").unwrap_or(&text);
    println!("[SPLIT] Notification: {log_text}");
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        window_proc_inner(hwnd, message, wparam, lparam)
    })) {
        Ok(result) => result,
        Err(_) => {
            eprintln!("[SPLIT] Panic caught in overlay WndProc for message 0x{message:04X}");
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
    }
}

unsafe fn window_proc_inner(hwnd: HWND, message: u32, wparam: WPARAM, lparam: LPARAM) -> isize {
    if message == WM_NCCREATE {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    if message == WM_CREATE {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
    if !WINDOW_READY.load(Ordering::Acquire) {
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }

    match message {
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_TIMER if wparam == HIDE_TIMER_ID => {
            KillTimer(hwnd, HIDE_TIMER_ID);
            ShowWindow(hwnd, SW_HIDE);
            0
        }
        WM_NCHITTEST => HTTRANSPARENT as isize,
        WM_MOUSEACTIVATE => MA_NOACTIVATE as isize,
        WM_ERASEBKGND => 1,
        WM_DESTROY => {
            WINDOW_READY.store(false, Ordering::Release);
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, message, wparam, lparam),
    }
}

unsafe fn paint(hwnd: HWND) {
    let mut paint = PAINTSTRUCT::default();
    let device = BeginPaint(hwnd, &mut paint);
    if device.is_null() {
        return;
    }

    let mut rect = RECT::default();
    GetClientRect(hwnd, &mut rect);
    let brush = CreateSolidBrush(color(20, 24, 31));
    if !brush.is_null() {
        FillRect(device, &rect, brush);
        DeleteObject(brush);
    }

    let face = wide("Segoe UI");
    let font = CreateFontW(
        -20,
        0,
        0,
        0,
        FW_SEMIBOLD as i32,
        0,
        0,
        0,
        DEFAULT_CHARSET.into(),
        OUT_DEFAULT_PRECIS.into(),
        CLIP_DEFAULT_PRECIS.into(),
        5,
        (DEFAULT_PITCH | FF_DONTCARE).into(),
        face.as_ptr(),
    );
    let previous = if font.is_null() {
        null_mut()
    } else {
        SelectObject(device, font)
    };

    SetBkMode(device, TRANSPARENT as i32);
    SetTextColor(device, color(255, 255, 255));
    if let Ok(text) = DISPLAY_TEXT.lock() {
        if !text.is_empty() {
            let wide_text: Vec<u16> = text.encode_utf16().collect();
            DrawTextW(
                device,
                wide_text.as_ptr(),
                wide_text.len() as i32,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
        }
    }

    if !font.is_null() {
        if !previous.is_null() {
            SelectObject(device, previous);
        }
        DeleteObject(font);
    }
    EndPaint(hwnd, &paint);
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

const fn color(red: u8, green: u8, blue: u8) -> u32 {
    red as u32 | ((green as u32) << 8) | ((blue as u32) << 16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_notifications() {
        let cases = [
            (Notification::Preset(2), "SPLIT · Preset 2"),
            (
                Notification::SlotSaved {
                    slot: 3,
                    favorite: false,
                },
                "SPLIT · Slot 3 saved",
            ),
            (
                Notification::SlotSaved {
                    slot: 4,
                    favorite: true,
                },
                "SPLIT · Favorite 4 saved",
            ),
            (
                Notification::SlotLoaded {
                    slot: 2,
                    favorite: false,
                },
                "SPLIT · Slot 2 loaded",
            ),
            (
                Notification::SlotLoaded {
                    slot: 7,
                    favorite: true,
                },
                "SPLIT · Favorite 7 loaded",
            ),
            (
                Notification::SlotEmpty {
                    slot: 5,
                    favorite: false,
                },
                "SPLIT · Slot 5 empty",
            ),
            (
                Notification::SlotEmpty {
                    slot: 6,
                    favorite: true,
                },
                "SPLIT · Favorite 6 empty",
            ),
            (Notification::Favorites(true), "SPLIT · Favorites enabled"),
            (Notification::Favorites(false), "SPLIT · Favorites disabled"),
            (Notification::Undo, "SPLIT · Undo"),
            (Notification::Redo, "SPLIT · Redo"),
            (Notification::NothingToUndo, "SPLIT · Nothing to undo"),
            (Notification::NothingToRedo, "SPLIT · Nothing to redo"),
            (Notification::SaveFailed, "SPLIT · Save failed"),
        ];

        for (notification, expected) in cases {
            assert_eq!(notification.text(), expected);
        }
    }

    #[test]
    fn positions_overlay_inside_client_top_right() {
        let client = RECT {
            left: 1_920,
            top: 120,
            right: 4_480,
            bottom: 1_560,
        };

        assert_eq!(
            overlay_position(client, NotificationPosition::TopRight),
            (4_192, 144)
        );
        assert_eq!(
            overlay_position(client, NotificationPosition::TopLeft),
            (1_944, 144)
        );
        assert_eq!(
            overlay_position(client, NotificationPosition::BottomLeft),
            (1_944, 1_480)
        );
        assert_eq!(
            overlay_position(client, NotificationPosition::BottomRight),
            (4_192, 1_480)
        );
    }

    #[test]
    fn notification_settings_defaults_match_the_prototype() {
        let settings = NotificationSettings::default();
        assert!(settings.enabled);
        assert_eq!(settings.position, NotificationPosition::TopRight);
        assert_eq!(settings.duration_ms, 1_500);
    }

    #[test]
    fn all_positions_round_trip_with_stable_names() {
        let cases = [
            (NotificationPosition::TopLeft, "\"topLeft\""),
            (NotificationPosition::TopRight, "\"topRight\""),
            (NotificationPosition::BottomLeft, "\"bottomLeft\""),
            (NotificationPosition::BottomRight, "\"bottomRight\""),
        ];

        for (position, expected) in cases {
            let json = serde_json::to_string(&position).unwrap();
            assert_eq!(json, expected);
            assert_eq!(
                serde_json::from_str::<NotificationPosition>(&json).unwrap(),
                position
            );
        }
    }

    #[test]
    fn duration_validation_accepts_only_supported_values() {
        for duration_ms in [1_000, 1_500, 2_000, 3_000] {
            let settings = NotificationSettings {
                duration_ms,
                ..NotificationSettings::default()
            };
            assert!(settings.validate().is_ok());
        }

        for duration_ms in [0, 999, 3_001, u32::MAX] {
            let settings = NotificationSettings {
                duration_ms,
                ..NotificationSettings::default()
            };
            assert!(settings.validate().is_err());
        }
    }

    #[test]
    fn disabled_settings_skip_notification_preparation() {
        apply_settings(NotificationSettings {
            enabled: false,
            ..NotificationSettings::default()
        });
        assert!(prepare_notification(Notification::Preset(2)).is_none());
        apply_settings(NotificationSettings::default());
    }
}
