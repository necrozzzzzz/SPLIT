use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU16, AtomicU32, AtomicU8, Ordering},
        mpsc, Mutex, OnceLock,
    },
    thread,
    thread::JoinHandle,
    time::Duration,
};

use windows_sys::{
    core::BOOL,
    Win32::{
        Foundation::{CloseHandle, HWND, LPARAM},
        System::Threading::{
            GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::{
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
                KEYEVENTF_KEYUP, VK_F1, VK_F10, VK_F11, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7,
                VK_F8, VK_F9, VK_MENU,
            },
            WindowsAndMessaging::{
                CallNextHookEx, DispatchMessageW, EnumWindows, GetForegroundWindow, GetMessageW,
                GetWindowThreadProcessId, IsIconic, IsWindowVisible, PostThreadMessageW,
                SetForegroundWindow, SetWindowsHookExW, ShowWindow, TranslateMessage,
                UnhookWindowsHookEx, KBDLLHOOKSTRUCT, LLKHF_ALTDOWN, MSG, SW_RESTORE,
                WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP,
            },
        },
    },
};

use tauri::AppHandle;

use super::watcher;

/*
 * Action envoyée par le hook clavier
 * au worker SPLIT.
 */
#[derive(Debug, Clone, Copy)]
enum HotkeyAction {
    Save(u8),
    Load(u8),
    CyclePreset,
    Undo,
    Redo,
    ToggleFavorites,
    Shutdown,
}

static HOTKEY_SENDER: OnceLock<mpsc::Sender<HotkeyAction>> = OnceLock::new();

/*
 * Empêche l'auto-repeat Windows.
 *
 * Un bit par touche F1-F8.
 */
static ACTIVE_HOTKEY_KEYS: AtomicU16 = AtomicU16::new(0);

/*
 * Empêche le repeat de V si
 * la touche reste appuyée.
 */
static ACTIVE_PRESET_KEY: AtomicBool = AtomicBool::new(false);
static ACTIVE_HISTORY_KEYS: AtomicU8 = AtomicU8::new(0);
static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);

struct HotkeyRuntime {
    worker: JoinHandle<()>,
    hook: JoinHandle<()>,
}

static HOTKEY_RUNTIME: Mutex<Option<HotkeyRuntime>> = Mutex::new(None);

pub fn stop() -> Result<(), String> {
    let runtime = HOTKEY_RUNTIME
        .lock()
        .map_err(|_| "Hotkey runtime lock poisoned".to_string())?
        .take();

    if let Some(runtime) = runtime {
        if let Some(sender) = HOTKEY_SENDER.get() {
            let _ = sender.send(HotkeyAction::Shutdown);
        }

        let thread_id = HOOK_THREAD_ID.swap(0, Ordering::SeqCst);
        let hook_stop_posted = if thread_id != 0 {
            unsafe { PostThreadMessageW(thread_id, WM_QUIT, 0, 0) != 0 }
        } else {
            true
        };

        runtime
            .worker
            .join()
            .map_err(|_| "Hotkey worker panicked while stopping".to_string())?;
        if hook_stop_posted {
            runtime
                .hook
                .join()
                .map_err(|_| "Keyboard hook panicked while stopping".to_string())?;
        } else {
            return Err("Could not post WM_QUIT to keyboard hook thread".to_string());
        }
    }

    Ok(())
}

fn slot_from_vk(vk: u32) -> Option<u8> {
    match vk as u16 {
        VK_F1 => Some(1),
        VK_F2 => Some(2),
        VK_F3 => Some(3),
        VK_F4 => Some(4),
        VK_F5 => Some(5),
        VK_F6 => Some(6),
        VK_F7 => Some(7),
        VK_F8 => Some(8),

        _ => None,
    }
}

/*
 * Touches internes utilisées par
 * savestate.cfg pour charger les slots.
 *
 * Slot 1 -> U
 * Slot 2 -> I
 * Slot 3 -> O
 * Slot 4 -> J
 * Slot 5 -> K
 * Slot 6 -> L
 * Slot 7 -> N
 * Slot 8 -> M
 */
fn load_transport_vk(slot: u8) -> Option<u16> {
    match slot {
        1 => Some(b'U' as u16),

        2 => Some(b'I' as u16),

        3 => Some(b'O' as u16),

        4 => Some(b'J' as u16),

        5 => Some(b'K' as u16),

        6 => Some(b'L' as u16),

        7 => Some(b'N' as u16),

        8 => Some(b'M' as u16),

        _ => None,
    }
}

fn alt_is_down() -> bool {
    let state = unsafe { GetAsyncKeyState(VK_MENU as i32) };

    (state as u16 & 0x8000) != 0
}

fn wait_for_alt_release() -> bool {
    /*
     * Pour le Save :
     * ne pas injecter H pendant
     * qu'Alt est encore enfoncé.
     */
    for _ in 0..200 {
        if !alt_is_down() {
            return true;
        }

        thread::sleep(Duration::from_millis(10));
    }

    false
}

fn is_deadlock_process(pid: u32) -> bool {
    unsafe {
        if pid == 0 {
            return false;
        }

        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);

        if process.is_null() {
            return false;
        }

        let mut buffer = [0u16; 1024];

        let mut size = buffer.len() as u32;

        let success = QueryFullProcessImageNameW(process, 0, buffer.as_mut_ptr(), &mut size);

        let _ = CloseHandle(process);

        if success == 0 {
            return false;
        }

        let executable = String::from_utf16_lossy(&buffer[..size as usize]);

        Path::new(&executable)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("deadlock.exe"))
    }
}

fn is_deadlock_foreground() -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();

        if hwnd.is_null() {
            return false;
        }

        let mut pid = 0u32;

        GetWindowThreadProcessId(hwnd, &mut pid);

        is_deadlock_process(pid)
    }
}

pub(crate) fn foreground_deadlock_window() -> Option<HWND> {
    unsafe {
        let hwnd = GetForegroundWindow();

        if hwnd.is_null() || IsWindowVisible(hwnd) == 0 {
            return None;
        }

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);

        is_deadlock_process(pid).then_some(hwnd)
    }
}

unsafe extern "system" fn enum_deadlock_window(hwnd: HWND, lparam: LPARAM) -> BOOL {
    /*
     * On ignore les fenêtres invisibles
     * appartenant éventuellement au process.
     */
    if IsWindowVisible(hwnd) == 0 {
        return 1;
    }

    let mut pid = 0u32;

    GetWindowThreadProcessId(hwnd, &mut pid);

    if !is_deadlock_process(pid) {
        return 1;
    }

    let target = lparam as *mut HWND;

    if target.is_null() {
        return 0;
    }

    *target = hwnd;

    /*
     * 0 = on arrête EnumWindows :
     * on a trouvé Deadlock.
     */
    0
}

fn find_deadlock_window() -> Option<HWND> {
    let mut hwnd: HWND = std::ptr::null_mut();

    unsafe {
        EnumWindows(Some(enum_deadlock_window), &mut hwnd as *mut HWND as LPARAM);
    }

    if hwnd.is_null() {
        None
    } else {
        Some(hwnd)
    }
}

fn focus_deadlock_window() -> Result<(), String> {
    let hwnd =
        find_deadlock_window().ok_or_else(|| "Could not find the Deadlock window".to_string())?;

    unsafe {
        /*
         * Si Deadlock est minimisé,
         * on le restaure d'abord.
         */
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }

        SetForegroundWindow(hwnd);
    }

    /*
     * On laisse un très court délai
     * à Windows pour réellement transférer
     * le focus.
     */
    for _ in 0..30 {
        if unsafe { GetForegroundWindow() } == hwnd {
            return Ok(());
        }

        thread::sleep(Duration::from_millis(10));
    }

    Err("Deadlock did not receive foreground focus".to_string())
}

fn make_keyboard_input(vk: u16, flags: u32) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,

        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send_virtual_key(vk: u16) -> Result<(), String> {
    let inputs = [
        make_keyboard_input(vk, 0),
        make_keyboard_input(vk, KEYEVENTF_KEYUP),
    ];

    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>() as i32,
        )
    };

    if sent != inputs.len() as u32 {
        return Err(format!(
            "SendInput only sent {sent}/{} keyboard events",
            inputs.len(),
        ));
    }

    Ok(())
}

fn send_capture_key() -> Result<(), String> {
    send_virtual_key(b'H' as u16)
}

fn send_load_key(slot: u8) -> Result<(), String> {
    let vk = load_transport_vk(slot).ok_or_else(|| format!("Invalid load slot {slot}"))?;

    send_virtual_key(vk)
}

fn load_active_slot(slot: u8) -> Result<bool, String> {
    let (favorite, snapshot) = super::active_slot_state(slot)?;

    let Some(snapshot) = snapshot else {
        crate::notifications::show(crate::notifications::Notification::SlotEmpty {
            slot,
            favorite,
        });

        return Ok(false);
    };

    let camera_before_load = match super::camera::capture() {
        Ok(camera) => Some(camera),

        Err(error) => {
            eprintln!("[SPLIT] Camera freeze capture unavailable: {error}");

            None
        }
    };

    let camera_hold = camera_before_load.map(|camera| {
        thread::spawn(move || super::camera::hold(camera, Duration::from_millis(40)))
    });

    send_load_key(slot)?;

    if let Some(worker) = camera_hold {
        match worker.join() {
            Ok(Ok(())) => {
                println!("[SPLIT] Camera held for 40 ms");
            }

            Ok(Err(error)) => {
                eprintln!("[SPLIT] Camera hold failed: {error}");
            }

            Err(_) => {
                eprintln!("[SPLIT] Camera hold thread panicked");
            }
        }
    }

    /*
     * Maintenant seulement, on remet
     * l'orientation caméra du slot.
     */
    if let Some(camera) = snapshot.camera {
        match super::camera::restore(camera) {
            Ok(()) => {
                println!(
                    "[SPLIT] Camera post-restored -> P={:.3} Y={:.3} R={:.3}",
                    camera.pitch, camera.yaw, camera.roll,
                );
            }

            Err(error) => {
                eprintln!("[SPLIT] Camera restore unavailable: {error}");
            }
        }
    }

    crate::notifications::show(crate::notifications::Notification::SlotLoaded { slot, favorite });

    Ok(true)
}

pub fn load_slot_from_ui(slot: u8) -> Result<(), String> {
    if !(1..=8).contains(&slot) {
        return Err(format!("Invalid load slot {slot}"));
    }

    println!("[SPLIT] UI load requested: slot {slot}");

    /*
     * Au moment du clic, SPLIT est
     * forcément la fenêtre au premier plan.
     *
     * On transfère donc le focus à Deadlock.
     */
    focus_deadlock_window()?;

    /*
     * Petit délai supplémentaire :
     * on veut que le jeu soit complètement
     * prêt à recevoir l'input.
     */
    thread::sleep(Duration::from_millis(75));

    if load_active_slot(slot)? {
        println!("[SPLIT] UI load injected: slot {slot}");
    } else {
        println!("[SPLIT] UI load skipped: slot {slot} is empty");
    }

    Ok(())
}

pub fn save_slot_from_ui(app: AppHandle, slot: u8) -> Result<(), String> {
    if !(1..=8).contains(&slot) {
        return Err(format!("Invalid save slot {slot}"));
    }

    println!("[SPLIT] UI save requested: slot {slot}");

    /*
     * Le clic vient de SPLIT.
     * On remet d'abord Deadlock
     * au premier plan.
     */
    focus_deadlock_window()?;

    /*
     * Laisser Windows terminer
     * le changement de focus.
     */
    thread::sleep(Duration::from_millis(75));

    /*
     * IMPORTANT :
     * on marque le slot AVANT d'envoyer H.
     *
     * Ainsi, lorsque watcher.rs reçoit
     * le nouveau getpos_exact, il sait
     * dans quel slot sauvegarder.
     */
    let generation = watcher::request_save_slot(app.clone(), slot)?;

    if let Err(error) = send_capture_key() {
        watcher::cancel_pending_save(generation);

        watcher::report_save_failed(&app, slot, &error);

        return Err(format!("Could not capture slot {slot}: {error}"));
    }

    println!("[SPLIT] UI capture injected: slot {slot}");

    Ok(())
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: usize, lparam: isize) -> isize {
    if code < 0 {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }

    let keyboard = &*(lparam as *const KBDLLHOOKSTRUCT);

    let key_down = wparam as u32 == WM_KEYDOWN || wparam as u32 == WM_SYSKEYDOWN;

    let key_up = wparam as u32 == WM_KEYUP || wparam as u32 == WM_SYSKEYUP;

    /*
     * V = preset suivant.
     *
     * On le traite séparément des
     * touches F1-F8.
     */
    if keyboard.vkCode == b'V' as u32 {
        /*
         * Si SPLIT avait intercepté
         * le keydown, il intercepte
         * aussi le keyup.
         */
        if key_up {
            let was_active = ACTIVE_PRESET_KEY.swap(false, Ordering::SeqCst);

            if was_active {
                return 1;
            }

            return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
        }

        if !key_down {
            return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
        }

        /*
         * Dans Brave, VS Code, etc.,
         * V reste une touche normale.
         */
        if !is_deadlock_foreground() {
            return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
        }

        let was_active = ACTIVE_PRESET_KEY.swap(true, Ordering::SeqCst);

        /*
         * Ignorer l'auto-repeat.
         */
        if !was_active {
            if let Some(sender) = HOTKEY_SENDER.get() {
                let _ = sender.send(HotkeyAction::CyclePreset);
            }
        }

        /*
         * Deadlock ne reçoit pas
         * directement le V physique.
         */
        return 1;
    }

    let history_action = match keyboard.vkCode as u16 {
        VK_F9 => Some((1u8, HotkeyAction::Undo)),
        VK_F10 => Some((2u8, HotkeyAction::Redo)),
        VK_F11 => Some((4u8, HotkeyAction::ToggleFavorites)),
        _ => None,
    };

    if let Some((bit, action)) = history_action {
        if key_up {
            let previous = ACTIVE_HISTORY_KEYS.fetch_and(!bit, Ordering::SeqCst);
            if previous & bit != 0 {
                return 1;
            }
            return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
        }

        if !key_down {
            return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
        }

        if !is_deadlock_foreground() {
            return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
        }

        let previous = ACTIVE_HISTORY_KEYS.fetch_or(bit, Ordering::SeqCst);
        if previous & bit == 0 {
            if let Some(sender) = HOTKEY_SENDER.get() {
                let _ = sender.send(action);
            }
        }

        return 1;
    }

    /*
     * À partir d'ici :
     * uniquement F1-F8.
     */
    let Some(slot) = slot_from_vk(keyboard.vkCode) else {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    };

    let bit = 1u16 << (slot - 1);

    /*
     * Si SPLIT a intercepté le keydown,
     * il doit aussi intercepter le keyup.
     */
    if key_up {
        let previous = ACTIVE_HOTKEY_KEYS.fetch_and(!bit, Ordering::SeqCst);

        if previous & bit != 0 {
            return 1;
        }

        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }

    if !key_down {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }

    /*
     * Ne toucher à F1-F8 QUE
     * lorsque Deadlock est réellement
     * la fenêtre au premier plan.
     */
    if !is_deadlock_foreground() {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }

    let previous = ACTIVE_HOTKEY_KEYS.fetch_or(bit, Ordering::SeqCst);

    /*
     * Ignorer l'auto-repeat.
     */
    if previous & bit == 0 {
        let alt_down = keyboard.flags & LLKHF_ALTDOWN != 0;

        let action = if alt_down {
            HotkeyAction::Save(slot)
        } else {
            HotkeyAction::Load(slot)
        };

        if let Some(sender) = HOTKEY_SENDER.get() {
            let _ = sender.send(action);
        }
    }

    /*
     * Deadlock ne reçoit jamais directement
     * le F1-F8 physique.
     *
     * SPLIT décide ensuite quoi envoyer :
     *
     * F1      -> U
     * Alt+F1  -> H
     */
    1
}

pub fn start(app: AppHandle) -> Result<(), String> {
    let (tx, rx) = mpsc::channel::<HotkeyAction>();

    HOTKEY_SENDER
        .set(tx)
        .map_err(|_| "SPLIT hotkeys are already running".to_string())?;

    /*
     * Worker.
     *
     * Aucune logique lourde n'est exécutée
     * directement dans le hook Windows.
     */
    let worker = thread::Builder::new()
        .name("split-hotkey-worker".to_string())
        .spawn(move || {
            for action in rx {
                match action {
                    /*
                     * ALT + F1-F8
                     *
                     * Capture la position,
                     * puis watcher.rs la sauvegarde.
                     */
                    HotkeyAction::Save(slot) => {
                        println!("[SPLIT] Save hotkey: Alt+F{slot}");

                        if !wait_for_alt_release() {
                            eprintln!("[SPLIT] Save {slot} cancelled: Alt was held too long");

                            continue;
                        }

                        /*
                         * Important :
                         * l'utilisateur peut avoir
                         * Alt-Tab pendant l'attente.
                         */
                        if !is_deadlock_foreground() {
                            println!("[SPLIT] Save {slot} cancelled: Deadlock lost focus");

                            continue;
                        }

                        let generation = match watcher::request_save_slot(app.clone(), slot) {
                            Ok(generation) => {
                                println!("[SPLIT] Capture requested for slot {slot}");
                                generation
                            }

                            Err(error) => {
                                eprintln!("[SPLIT] Could not request save {slot}: {error}");

                                continue;
                            }
                        };

                        if let Err(error) = send_capture_key() {
                            watcher::cancel_pending_save(generation);

                            watcher::report_save_failed(&app, slot, &error);

                            eprintln!("[SPLIT] Could not send H: {error}");
                        }
                    }

                    /*
                     * F1-F8
                     *
                     * On NE dépend PAS des binds
                     * F1-F8 de Deadlock.
                     *
                     * SPLIT transforme :
                     *
                     * F1 -> U
                     * F2 -> I
                     * etc.
                     */
                    HotkeyAction::Load(slot) => {
                        println!("[SPLIT] Load hotkey: F{slot}");

                        if !is_deadlock_foreground() {
                            println!("[SPLIT] Load {slot} cancelled: Deadlock lost focus");

                            continue;
                        }

                        if let Err(error) = load_active_slot(slot) {
                            eprintln!("[SPLIT] Could not load slot {slot}: {error}");
                        }
                    }

                    HotkeyAction::CyclePreset => {
                        println!("[SPLIT] Preset hotkey: V");

                        match super::cycle_active_preset() {
                            Ok(Some((preset, saved_slots))) => {
                                println!("[SPLIT] Preset switched to {preset}");

                                crate::notifications::show(
                                    crate::notifications::Notification::Preset(preset),
                                );

                                /*
                                 * Mettre à jour les 8 cartes
                                 * dans React.
                                 */
                                crate::ui::emit_to_main_if_present(
                                    &app,
                                    "deadlock-slots",
                                    saved_slots,
                                );

                                /*
                                 * Mettre à jour le bouton
                                 * Preset actif dans React.
                                 */
                                crate::ui::emit_to_main_if_present(&app, "deadlock-preset", preset);
                            }

                            Ok(None) => {}

                            Err(error) => {
                                eprintln!("[SPLIT] Could not cycle preset: {error}");
                            }
                        }
                    }

                    HotkeyAction::Undo => {
                        println!("[SPLIT] Undo hotkey: F9");
                        match super::undo_last_action() {
                            Ok(result) => super::emit_history_operation(&app, &result),
                            Err(error) => eprintln!("[SPLIT] Could not undo: {error}"),
                        }
                    }

                    HotkeyAction::Redo => {
                        println!("[SPLIT] Redo hotkey: F10");
                        match super::redo_last_action() {
                            Ok(result) => super::emit_history_operation(&app, &result),
                            Err(error) => eprintln!("[SPLIT] Could not redo: {error}"),
                        }
                    }

                    HotkeyAction::ToggleFavorites => {
                        println!("[SPLIT] Favorite Mode hotkey: F11");
                        match super::toggle_favorite_mode() {
                            Ok(result) => super::emit_active_bank(&app, &result),
                            Err(error) => {
                                eprintln!("[SPLIT] Could not toggle Favorite Mode: {error}")
                            }
                        }
                    }
                    HotkeyAction::Shutdown => break,
                }
            }
        })
        .map_err(|error| format!("Could not start hotkey worker: {error}"))?;

    /*
     * Hook clavier Windows.
     */
    let hook = thread::Builder::new()
        .name("split-keyboard-hook".to_string())
        .spawn(|| unsafe {
            HOOK_THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);
            let hook =
                SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), std::ptr::null_mut(), 0);

            if hook.is_null() {
                eprintln!("[SPLIT] Failed to install keyboard hook");
                HOOK_THREAD_ID.store(0, Ordering::SeqCst);

                return;
            }

            println!("[SPLIT] Deadlock hotkeys active:");

            println!("[SPLIT]   Save = Alt+F1-F8");

            println!("[SPLIT]   Load = F1-F8");

            let mut message = MSG::default();

            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);

                DispatchMessageW(&message);
            }

            let _ = UnhookWindowsHookEx(hook);
            HOOK_THREAD_ID.store(0, Ordering::SeqCst);
        })
        .map_err(|error| format!("Could not start keyboard hook: {error}"))?;

    *HOTKEY_RUNTIME
        .lock()
        .map_err(|_| "Hotkey runtime lock poisoned".to_string())? =
        Some(HotkeyRuntime { worker, hook });

    Ok(())
}
