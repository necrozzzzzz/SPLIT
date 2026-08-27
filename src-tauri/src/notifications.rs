use std::{
    ptr::{null, null_mut},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Mutex,
    },
    thread::{self, JoinHandle},
};

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
const DISPLAY_MS: u32 = 1_500;
const OVERLAY_WIDTH: i32 = 264;
const OVERLAY_HEIGHT: i32 = 56;
const MARGIN: i32 = 24;

enum Command {
    Show(String),
    Shutdown,
}

struct Runtime {
    sender: mpsc::Sender<Command>,
    thread_id: u32,
    join: JoinHandle<()>,
}

static RUNTIME: Mutex<Option<Runtime>> = Mutex::new(None);
static DISPLAY_TEXT: Mutex<String> = Mutex::new(String::new());
static WINDOW_READY: AtomicBool = AtomicBool::new(false);

pub fn start() -> Result<(), String> {
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

pub fn show_preset(preset: u8) {
    let text = preset_notification_text(preset);
    let Ok(runtime) = RUNTIME.lock() else {
        return;
    };
    let Some(runtime) = runtime.as_ref() else {
        return;
    };

    if runtime.sender.send(Command::Show(text)).is_ok() {
        unsafe {
            PostThreadMessageW(runtime.thread_id, COMMAND_MESSAGE, 0, 0);
        }
    }
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

fn preset_notification_text(preset: u8) -> String {
    format!("SPLIT · Preset {preset}")
}

fn overlay_position(client: RECT) -> (i32, i32) {
    (client.right - OVERLAY_WIDTH - MARGIN, client.top + MARGIN)
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
                    Command::Show(text) => latest = Some(text),
                    Command::Shutdown => shutdown = true,
                }
            }

            if shutdown {
                unsafe { DestroyWindow(hwnd) };
                continue;
            }

            if let Some(text) = latest {
                show_notification(hwnd, text);
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

fn show_notification(hwnd: HWND, text: String) {
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
    let (x, y) = overlay_position(client);

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
        SetTimer(hwnd, HIDE_TIMER_ID, DISPLAY_MS, None);
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
    fn formats_preset_notification() {
        assert_eq!(preset_notification_text(3), "SPLIT · Preset 3");
    }

    #[test]
    fn positions_overlay_inside_client_top_right() {
        let client = RECT {
            left: 1_920,
            top: 120,
            right: 4_480,
            bottom: 1_560,
        };

        assert_eq!(overlay_position(client), (4_192, 144));
    }
}
