use std::{
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc, Mutex,
    },
    thread::{self, JoinHandle},
};

use serde::{Deserialize, Deserializer, Serialize};

use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, POINT, RECT, SIZE, WPARAM},
    Graphics::Gdi::{
        BeginPaint, ClientToScreen, CreateFontW, CreateRoundRectRgn, CreateSolidBrush,
        DeleteObject, DrawTextW, EndPaint, FillRect, GetDC, GetMonitorInfoW, GetTextExtentPoint32W,
        InvalidateRect, MonitorFromWindow, ReleaseDC, SelectObject, SetBkMode, SetTextColor,
        SetWindowRgn, UpdateWindow, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, DT_CENTER,
        DT_END_ELLIPSIS, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, FF_DONTCARE, FW_SEMIBOLD,
        MONITORINFO, MONITOR_DEFAULTTOPRIMARY, OUT_DEFAULT_PRECIS, PAINTSTRUCT, TRANSPARENT,
    },
    System::{
        LibraryLoader::GetModuleHandleW, SystemInformation::GetTickCount64,
        Threading::GetCurrentThreadId,
    },
    UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect,
        GetMessageW, KillTimer, PostQuitMessage, PostThreadMessageW, RegisterClassW,
        SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, TranslateMessage,
        CS_HREDRAW, CS_VREDRAW, HTTRANSPARENT, HWND_TOPMOST, LWA_ALPHA, MA_NOACTIVATE, MSG,
        SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_HIDE, SW_SHOWNOACTIVATE, WM_APP, WM_CREATE, WM_DESTROY,
        WM_ERASEBKGND, WM_MOUSEACTIVATE, WM_NCCREATE, WM_NCHITTEST, WM_PAINT, WM_TIMER, WNDCLASSW,
        WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
    },
};

const COMMAND_MESSAGE: u32 = WM_APP + 41;
const HIDE_TIMER_ID: usize = 1;
const FADE_TIMER_ID: usize = 2;
const FADE_MAX_MS: u32 = 200;
const FADE_INTERVAL_MS: u32 = 16;
const MIN_OVERLAY_WIDTH: i32 = 264;
const MAX_OVERLAY_WIDTH: i32 = 480;
const OVERLAY_HEIGHT: i32 = 56;
const HORIZONTAL_PADDING: i32 = 20;
const MARGIN: i32 = 24;
const BASE_BACKGROUND: (u8, u8, u8) = (20, 24, 31);
const TEST_COLOR: &str = "#4fd1c5";

struct NotificationPayload {
    text: String,
    color: Option<String>,
}

enum Command {
    Show {
        payload: NotificationPayload,
        settings: NotificationSettings,
        target: NotificationTarget,
    },
    Shutdown,
}

#[derive(Clone, Copy)]
enum NotificationTarget {
    Deadlock,
    Desktop(RECT),
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
    #[serde(default = "default_true")]
    pub use_slot_color: bool,
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            position: NotificationPosition::TopRight,
            duration_ms: 1_500,
            use_slot_color: true,
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
    matches!(duration_ms, 500 | 1_000 | 1_500 | 2_000 | 3_000)
}

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Notification {
    Preset(u8),
    SlotSaved {
        slot: u8,
        display_name: String,
        color: Option<String>,
    },
    SlotLoaded {
        slot: u8,
        display_name: String,
        color: Option<String>,
    },
    SlotEmpty {
        slot: u8,
        favorite: bool,
    },
    Favorites(bool),
    Undo,
    Redo,
    NothingToUndo,
    NothingToRedo,
    SaveFailed,
    Test,
}

impl Notification {
    fn text(&self) -> String {
        match self {
            Self::Preset(preset) => format!("SPLIT · Preset {preset}"),
            Self::SlotSaved { display_name, .. } => {
                format!("SPLIT · Saved · {display_name}")
            }
            Self::SlotLoaded { display_name, .. } => {
                format!("SPLIT · Loaded · {display_name}")
            }
            Self::SlotEmpty { slot, favorite } => {
                format!("SPLIT · {} {slot} empty", slot_kind(*favorite))
            }
            Self::Favorites(true) => "SPLIT · Favorites enabled".to_string(),
            Self::Favorites(false) => "SPLIT · Favorites disabled".to_string(),
            Self::Undo => "SPLIT · Undo".to_string(),
            Self::Redo => "SPLIT · Redo".to_string(),
            Self::NothingToUndo => "SPLIT · Nothing to undo".to_string(),
            Self::NothingToRedo => "SPLIT · Nothing to redo".to_string(),
            Self::SaveFailed => "SPLIT · Save failed".to_string(),
            Self::Test => "SPLIT · Test notification".to_string(),
        }
    }

    fn payload(&self) -> NotificationPayload {
        let color = match self {
            Self::SlotSaved { color, .. } | Self::SlotLoaded { color, .. } => color.clone(),
            Self::Test => Some(TEST_COLOR.to_string()),
            _ => None,
        };
        NotificationPayload {
            text: self.text(),
            color,
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
static DISPLAY_BACKGROUND: AtomicU32 = AtomicU32::new(color(20, 24, 31));
static DISPLAY_FADE_DURATION: AtomicU32 = AtomicU32::new(FADE_MAX_MS);
static FADE_STATE: Mutex<Option<FadeState>> = Mutex::new(None);
static SETTINGS: Mutex<NotificationSettings> = Mutex::new(NotificationSettings {
    enabled: true,
    position: NotificationPosition::TopRight,
    duration_ms: 1_500,
    use_slot_color: true,
});

struct FadeState {
    started_at: u64,
    duration_ms: u32,
}

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
    let Some((payload, settings)) = prepare_notification(notification) else {
        return;
    };
    let _ = enqueue(payload, settings, NotificationTarget::Deadlock);
}

pub fn show_test(split_hwnd: HWND) -> Result<(), String> {
    let settings = SETTINGS
        .lock()
        .map_err(|_| "Notification settings lock poisoned".to_string())?
        .clone();
    let work_area = monitor_work_area(split_hwnd)?;

    enqueue(
        Notification::Test.payload(),
        settings,
        NotificationTarget::Desktop(work_area),
    )
}

fn enqueue(
    payload: NotificationPayload,
    settings: NotificationSettings,
    target: NotificationTarget,
) -> Result<(), String> {
    let Ok(runtime) = RUNTIME.lock() else {
        return Err("Notification runtime lock poisoned".to_string());
    };
    let Some(runtime) = runtime.as_ref() else {
        return Err("Notification service is not available".to_string());
    };

    runtime
        .sender
        .send(Command::Show {
            payload,
            settings,
            target,
        })
        .map_err(|_| "Notification thread is no longer available".to_string())?;

    if unsafe { PostThreadMessageW(runtime.thread_id, COMMAND_MESSAGE, 0, 0) } == 0 {
        return Err("Could not wake notification thread".to_string());
    }

    Ok(())
}

pub fn apply_settings(settings: NotificationSettings) {
    if let Ok(mut current) = SETTINGS.lock() {
        *current = settings;
    }
}

fn prepare_notification(
    notification: Notification,
) -> Option<(NotificationPayload, NotificationSettings)> {
    let settings = SETTINGS.lock().ok()?.clone();
    settings.enabled.then(|| (notification.payload(), settings))
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

fn overlay_width(text_width: i32) -> i32 {
    (text_width + HORIZONTAL_PADDING * 2).clamp(MIN_OVERLAY_WIDTH, MAX_OVERLAY_WIDTH)
}

fn overlay_position(
    client: RECT,
    position: NotificationPosition,
    notification_width: i32,
) -> (i32, i32) {
    match position {
        NotificationPosition::TopLeft => (client.left + MARGIN, client.top + MARGIN),
        NotificationPosition::TopRight => (
            client.right - notification_width - MARGIN,
            client.top + MARGIN,
        ),
        NotificationPosition::BottomLeft => (
            client.left + MARGIN,
            client.bottom - OVERLAY_HEIGHT - MARGIN,
        ),
        NotificationPosition::BottomRight => (
            client.right - notification_width - MARGIN,
            client.bottom - OVERLAY_HEIGHT - MARGIN,
        ),
    }
}

fn monitor_work_area(hwnd: HWND) -> Result<RECT, String> {
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTOPRIMARY) };
    if monitor.is_null() {
        return Err("Could not find a monitor for SPLIT".to_string());
    }

    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return Err(format!(
            "Could not read the SPLIT monitor work area: {}",
            std::io::Error::last_os_error(),
        ));
    }

    Ok(info.rcWork)
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
                    Command::Show {
                        payload,
                        settings,
                        target,
                    } => latest = Some((payload, settings, target)),
                    Command::Shutdown => shutdown = true,
                }
            }

            if shutdown {
                unsafe { DestroyWindow(hwnd) };
                continue;
            }

            if let Some((payload, settings, target)) = latest {
                show_notification(hwnd, payload, settings, target);
            }
            continue;
        }

        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
}

fn create_notification_font() -> windows_sys::Win32::Graphics::Gdi::HFONT {
    let face = wide("Segoe UI");
    unsafe {
        CreateFontW(
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
        )
    }
}

fn measure_text_width(hwnd: HWND, text: &str) -> Option<i32> {
    unsafe {
        let device = GetDC(hwnd);
        if device.is_null() {
            return None;
        }
        let font = create_notification_font();
        if font.is_null() {
            ReleaseDC(hwnd, device);
            return None;
        }
        let previous = SelectObject(device, font);
        if previous.is_null() {
            DeleteObject(font);
            ReleaseDC(hwnd, device);
            return None;
        }

        let wide_text: Vec<u16> = text.encode_utf16().collect();
        let mut size = SIZE::default();
        let measured = GetTextExtentPoint32W(
            device,
            wide_text.as_ptr(),
            wide_text.len() as i32,
            &mut size,
        ) != 0;

        SelectObject(device, previous);
        DeleteObject(font);
        ReleaseDC(hwnd, device);
        measured.then_some(size.cx)
    }
}

unsafe fn update_window_region(hwnd: HWND, width: i32) {
    let region = CreateRoundRectRgn(0, 0, width + 1, OVERLAY_HEIGHT + 1, 14, 14);
    if !region.is_null() && SetWindowRgn(hwnd, region, 0) == 0 {
        DeleteObject(region);
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
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED | WS_EX_TOPMOST,
            class_name.as_ptr(),
            null(),
            WS_POPUP,
            0,
            0,
            MIN_OVERLAY_WIDTH,
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
        SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
        update_window_region(hwnd, MIN_OVERLAY_WIDTH);
    }

    Ok(hwnd)
}

fn show_notification(
    hwnd: HWND,
    payload: NotificationPayload,
    settings: NotificationSettings,
    target: NotificationTarget,
) {
    let client = match target {
        NotificationTarget::Deadlock => {
            let Some(client) = deadlock_client_rect() else {
                return;
            };
            client
        }
        NotificationTarget::Desktop(work_area) => work_area,
    };
    let width = measure_text_width(hwnd, &payload.text)
        .map(overlay_width)
        .unwrap_or(MIN_OVERLAY_WIDTH);
    let (x, y) = overlay_position(client, settings.position, width);

    if let Ok(mut display_text) = DISPLAY_TEXT.lock() {
        *display_text = payload.text.clone();
    } else {
        return;
    }

    unsafe {
        KillTimer(hwnd, HIDE_TIMER_ID);
        KillTimer(hwnd, FADE_TIMER_ID);
        if let Ok(mut fade) = FADE_STATE.lock() {
            *fade = None;
        }
        SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
        update_window_region(hwnd, width);
        DISPLAY_BACKGROUND.store(
            notification_background(payload.color.as_deref(), settings.use_slot_color),
            Ordering::Release,
        );
        let fade_duration = fade_duration_ms(settings.duration_ms);
        DISPLAY_FADE_DURATION.store(fade_duration, Ordering::Release);
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            x,
            y,
            width,
            OVERLAY_HEIGHT,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        InvalidateRect(hwnd, null(), 1);
        UpdateWindow(hwnd);
        SetTimer(
            hwnd,
            HIDE_TIMER_ID,
            settings.duration_ms - fade_duration,
            None,
        );
    }

    let log_text = payload
        .text
        .strip_prefix("SPLIT · ")
        .unwrap_or(&payload.text);
    println!("[SPLIT] Notification: {log_text}");
}

fn deadlock_client_rect() -> Option<RECT> {
    let Some(deadlock) = crate::deadlock::foreground_deadlock_window() else {
        return None;
    };

    let mut client = RECT::default();
    if unsafe { GetClientRect(deadlock, &mut client) } == 0 {
        return None;
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
        return None;
    }

    Some(RECT {
        left: top_left.x,
        top: top_left.y,
        right: bottom_right.x,
        bottom: bottom_right.y,
    })
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
            let duration_ms = DISPLAY_FADE_DURATION.load(Ordering::Acquire);
            if duration_ms == 0 {
                hide_and_reset(hwnd);
            } else {
                if let Ok(mut fade) = FADE_STATE.lock() {
                    *fade = Some(FadeState {
                        started_at: GetTickCount64(),
                        duration_ms,
                    });
                }
                SetTimer(hwnd, FADE_TIMER_ID, FADE_INTERVAL_MS, None);
            }
            0
        }
        WM_TIMER if wparam == FADE_TIMER_ID => {
            update_fade(hwnd);
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
    let brush = CreateSolidBrush(DISPLAY_BACKGROUND.load(Ordering::Acquire));
    if !brush.is_null() {
        FillRect(device, &rect, brush);
        DeleteObject(brush);
    }

    let font = create_notification_font();
    let previous = if font.is_null() {
        null_mut()
    } else {
        SelectObject(device, font)
    };

    SetBkMode(device, TRANSPARENT as i32);
    SetTextColor(device, color(255, 255, 255));
    if let Ok(text) = DISPLAY_TEXT.lock() {
        if !text.is_empty() {
            rect.left += HORIZONTAL_PADDING;
            rect.right -= HORIZONTAL_PADDING;
            let wide_text: Vec<u16> = text.encode_utf16().collect();
            DrawTextW(
                device,
                wide_text.as_ptr(),
                wide_text.len() as i32,
                &mut rect,
                DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
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

const fn fade_duration_ms(total_duration_ms: u32) -> u32 {
    let proportional = total_duration_ms * 2 / 5;
    if proportional < FADE_MAX_MS {
        proportional
    } else {
        FADE_MAX_MS
    }
}

fn parse_hex_color(value: &str) -> Option<(u8, u8, u8)> {
    let hex = value.strip_prefix('#')?;
    let bytes = hex.as_bytes();
    if bytes.len() != 6 || !bytes.iter().all(u8::is_ascii_hexdigit) {
        return None;
    }
    let component = |offset| {
        std::str::from_utf8(&bytes[offset..offset + 2])
            .ok()
            .and_then(|value| u8::from_str_radix(value, 16).ok())
    };
    Some((component(0)?, component(2)?, component(4)?))
}

fn notification_background(slot_color: Option<&str>, use_slot_color: bool) -> u32 {
    let Some((red, green, blue)) = use_slot_color
        .then_some(slot_color)
        .flatten()
        .and_then(parse_hex_color)
    else {
        return color(BASE_BACKGROUND.0, BASE_BACKGROUND.1, BASE_BACKGROUND.2);
    };
    let blend = |base: u8, accent: u8| ((u16::from(base) * 4 + u16::from(accent)) / 5) as u8;
    color(
        blend(BASE_BACKGROUND.0, red),
        blend(BASE_BACKGROUND.1, green),
        blend(BASE_BACKGROUND.2, blue),
    )
}

unsafe fn hide_and_reset(hwnd: HWND) {
    KillTimer(hwnd, HIDE_TIMER_ID);
    KillTimer(hwnd, FADE_TIMER_ID);
    ShowWindow(hwnd, SW_HIDE);
    SetLayeredWindowAttributes(hwnd, 0, 255, LWA_ALPHA);
    if let Ok(mut fade) = FADE_STATE.lock() {
        *fade = None;
    }
}

unsafe fn update_fade(hwnd: HWND) {
    let fade = FADE_STATE.lock().ok().and_then(|fade| {
        fade.as_ref()
            .map(|fade| (fade.started_at, fade.duration_ms))
    });
    let Some((started_at, duration_ms)) = fade else {
        hide_and_reset(hwnd);
        return;
    };
    let elapsed = GetTickCount64().saturating_sub(started_at);
    if elapsed >= u64::from(duration_ms) {
        hide_and_reset(hwnd);
        return;
    }
    let remaining = u64::from(duration_ms) - elapsed;
    let alpha = (255 * remaining / u64::from(duration_ms)) as u8;
    SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
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
                    display_name: "Rooftop".to_string(),
                    color: Some("#4fd1c5".to_string()),
                },
                "SPLIT · Saved · Rooftop",
            ),
            (
                Notification::SlotLoaded {
                    slot: 2,
                    display_name: "Slot 2".to_string(),
                    color: None,
                },
                "SPLIT · Loaded · Slot 2",
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
            (Notification::Test, "SPLIT · Test notification"),
        ];

        for (notification, expected) in cases {
            assert_eq!(notification.text(), expected);
        }
    }

    #[test]
    fn positions_overlay_inside_known_work_area() {
        let work_area = RECT {
            left: 1_920,
            top: 120,
            right: 4_480,
            bottom: 1_560,
        };

        assert_eq!(
            overlay_position(work_area, NotificationPosition::TopRight, MIN_OVERLAY_WIDTH),
            (4_192, 144)
        );
        assert_eq!(
            overlay_position(work_area, NotificationPosition::TopLeft, MIN_OVERLAY_WIDTH),
            (1_944, 144)
        );
        assert_eq!(
            overlay_position(work_area, NotificationPosition::BottomLeft, MIN_OVERLAY_WIDTH),
            (1_944, 1_480)
        );
        assert_eq!(
            overlay_position(work_area, NotificationPosition::BottomRight, MIN_OVERLAY_WIDTH),
            (4_192, 1_480)
        );
    }

    #[test]
    fn dynamic_width_uses_padding_minimum_and_maximum() {
        assert_eq!(overlay_width(80), MIN_OVERLAY_WIDTH);
        assert_eq!(overlay_width(300), 300 + HORIZONTAL_PADDING * 2);
        assert_eq!(overlay_width(1_000), MAX_OVERLAY_WIDTH);
    }

    #[test]
    fn dynamic_width_preserves_left_and_right_anchors_for_desktop_work_area() {
        let work_area = RECT {
            left: 1_920,
            top: 120,
            right: 4_480,
            bottom: 1_560,
        };
        let width = 400;

        assert_eq!(
            overlay_position(work_area, NotificationPosition::TopRight, width),
            (4_056, 144)
        );
        assert_eq!(
            overlay_position(work_area, NotificationPosition::BottomRight, width),
            (4_056, 1_480)
        );
        assert_eq!(
            overlay_position(work_area, NotificationPosition::TopLeft, width),
            (1_944, 144)
        );
        assert_eq!(
            overlay_position(work_area, NotificationPosition::BottomLeft, width),
            (1_944, 1_480)
        );
    }

    #[test]
    fn notification_settings_defaults_match_the_prototype() {
        let settings = NotificationSettings::default();
        assert!(settings.enabled);
        assert_eq!(settings.position, NotificationPosition::TopRight);
        assert_eq!(settings.duration_ms, 1_500);
        assert!(settings.use_slot_color);
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
        for duration_ms in [500, 1_000, 1_500, 2_000, 3_000] {
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

    #[test]
    fn slot_color_tints_the_dark_background_only_when_enabled() {
        let base = color(BASE_BACKGROUND.0, BASE_BACKGROUND.1, BASE_BACKGROUND.2);
        assert_eq!(notification_background(None, true), base);
        assert_eq!(notification_background(Some("#4fd1c5"), false), base);
        assert_eq!(notification_background(Some("invalid"), true), base);
        assert_eq!(
            notification_background(Some("#4fd1c5"), true),
            color(31, 61, 64)
        );
    }

    #[test]
    fn fade_is_clamped_and_keeps_total_duration() {
        assert_eq!(fade_duration_ms(500), 200);
        assert_eq!(500 - fade_duration_ms(500), 300);
        assert_eq!(fade_duration_ms(1_500), 200);
        assert_eq!(1_500 - fade_duration_ms(1_500), 1_300);
        assert_eq!(fade_duration_ms(250), 100);
    }
}
