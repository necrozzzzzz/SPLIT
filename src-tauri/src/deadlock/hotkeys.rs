use std::{
    path::Path,
    sync::{
        atomic::{
            AtomicU16,
            Ordering,
        },
        mpsc,
        OnceLock,
    },
    thread,
    time::Duration,
};

use windows_sys::Win32::{
    Foundation::CloseHandle,

    System::Threading::{
        OpenProcess,
        QueryFullProcessImageNameW,
        PROCESS_QUERY_LIMITED_INFORMATION,
    },

    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState,
            SendInput,
            INPUT,
            INPUT_0,
            INPUT_KEYBOARD,
            KEYBDINPUT,
            KEYEVENTF_KEYUP,
            VK_F1,
            VK_F2,
            VK_F3,
            VK_F4,
            VK_F5,
            VK_F6,
            VK_F7,
            VK_F8,
            VK_H,
            VK_MENU,
        },

        WindowsAndMessaging::{
            CallNextHookEx,
            DispatchMessageW,
            GetForegroundWindow,
            GetMessageW,
            GetWindowThreadProcessId,
            KBDLLHOOKSTRUCT,
            LLKHF_ALTDOWN,
            MSG,
            SetWindowsHookExW,
            TranslateMessage,
            UnhookWindowsHookEx,
            WH_KEYBOARD_LL,
            WM_KEYDOWN,
            WM_KEYUP,
            WM_SYSKEYDOWN,
            WM_SYSKEYUP,
        },
    },
};

use super::watcher;

static SAVE_SENDER:
    OnceLock<mpsc::Sender<u8>> =
    OnceLock::new();

/*
 * Évite les répétitions Windows si F1 reste
 * appuyé quelques centaines de ms.
 *
 * 1 bit = 1 slot.
 */
static ACTIVE_SAVE_KEYS:
    AtomicU16 =
    AtomicU16::new(0);

fn slot_from_vk(
    vk: u32,
) -> Option<u8> {
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

fn alt_is_down() -> bool {
    let state = unsafe {
        GetAsyncKeyState(
            VK_MENU as i32,
        )
    };

    (state as u16 & 0x8000) != 0
}

fn wait_for_alt_release() -> bool {
    /*
     * On attend que l'utilisateur relâche Alt
     * avant d'injecter H.
     *
     * Ce petit polling n'existe QUE pendant
     * l'utilisation d'un hotkey Save.
     */
    for _ in 0..200 {
        if !alt_is_down() {
            return true;
        }

        thread::sleep(
            Duration::from_millis(10),
        );
    }

    false
}

fn is_deadlock_foreground() -> bool {
    unsafe {
        let hwnd =
            GetForegroundWindow();

        if hwnd.is_null() {
            return false;
        }

        let mut pid = 0u32;

        GetWindowThreadProcessId(
            hwnd,
            &mut pid,
        );

        if pid == 0 {
            return false;
        }

        let process =
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION,
                0,
                pid,
            );

        if process.is_null() {
            return false;
        }

        let mut buffer =
            [0u16; 1024];

        let mut size =
            buffer.len() as u32;

        let success =
            QueryFullProcessImageNameW(
                process,
                0,
                buffer.as_mut_ptr(),
                &mut size,
            );

        let _ =
            CloseHandle(process);

        if success == 0 {
            return false;
        }

        let executable =
            String::from_utf16_lossy(
                &buffer[
                    ..size as usize
                ],
            );

        Path::new(&executable)
            .file_name()
            .and_then(
                |name| name.to_str(),
            )
            .is_some_and(
                |name| {
                    name.eq_ignore_ascii_case(
                        "deadlock.exe",
                    )
                },
            )
    }
}

fn make_keyboard_input(
    vk: u16,
    flags: u32,
) -> INPUT {
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

fn send_capture_key(
) -> Result<(), String> {
    let inputs = [
        make_keyboard_input(
            VK_H,
            0,
        ),

        make_keyboard_input(
            VK_H,
            KEYEVENTF_KEYUP,
        ),
    ];

    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            std::mem::size_of::<INPUT>()
                as i32,
        )
    };

    if sent != inputs.len() as u32 {
        return Err(
            format!(
                "SendInput only sent {sent}/{} keyboard events",
                inputs.len(),
            ),
        );
    }

    Ok(())
}

unsafe extern "system"
fn keyboard_hook(
    code: i32,
    wparam: usize,
    lparam: isize,
) -> isize {
    if code < 0 {
        return CallNextHookEx(
            std::ptr::null_mut(),
            code,
            wparam,
            lparam,
        );
    }

    let keyboard =
        &*(lparam
            as *const KBDLLHOOKSTRUCT);

    let Some(slot) =
        slot_from_vk(
            keyboard.vkCode,
        )
    else {
        return CallNextHookEx(
            std::ptr::null_mut(),
            code,
            wparam,
            lparam,
        );
    };

    let key_down =
        wparam as u32
            == WM_KEYDOWN
        || wparam as u32
            == WM_SYSKEYDOWN;

    let key_up =
        wparam as u32
            == WM_KEYUP
        || wparam as u32
            == WM_SYSKEYUP;

    let bit =
        1u16 << (slot - 1);

    /*
     * Si on avait bloqué le keydown,
     * on bloque aussi son keyup.
     */
    if key_up {
        let previous =
            ACTIVE_SAVE_KEYS
                .fetch_and(
                    !bit,
                    Ordering::SeqCst,
                );

        if previous & bit != 0 {
            return 1;
        }

        return CallNextHookEx(
            std::ptr::null_mut(),
            code,
            wparam,
            lparam,
        );
    }

    if !key_down {
        return CallNextHookEx(
            std::ptr::null_mut(),
            code,
            wparam,
            lparam,
        );
    }

    let alt_down =
        keyboard.flags
            & LLKHF_ALTDOWN
            != 0;

    /*
     * IMPORTANT :
     *
     * Hors Deadlock :
     * Alt+F1..F8 restent 100 % normaux.
     */
    if !alt_down
        || !is_deadlock_foreground()
    {
        return CallNextHookEx(
            std::ptr::null_mut(),
            code,
            wparam,
            lparam,
        );
    }

    let previous =
        ACTIVE_SAVE_KEYS.fetch_or(
            bit,
            Ordering::SeqCst,
        );

    /*
     * Auto-repeat :
     * ne lancer qu'une seule sauvegarde.
     */
    if previous & bit == 0 {
        if let Some(sender) =
            SAVE_SENDER.get()
        {
            let _ =
                sender.send(slot);
        }
    }

    /*
     * Bloquer Alt+Fx dans Deadlock.
     *
     * Très important pour Alt+F4 :
     * Windows ne reçoit donc jamais
     * la commande "fermer".
     */
    1
}

pub fn start() -> Result<(), String> {
    let (tx, rx) =
        mpsc::channel::<u8>();

    SAVE_SENDER
        .set(tx)
        .map_err(|_| {
            "SPLIT hotkeys are already running"
                .to_string()
        })?;

    /*
     * Worker Save.
     */
    thread::Builder::new()
        .name(
            "split-save-hotkey-worker"
                .to_string(),
        )
        .spawn(move || {
            for slot in rx {
                println!(
                    "[SPLIT] Save hotkey: Alt+F{slot}"
                );

                /*
                 * Alt doit être relâché avant
                 * d'injecter H dans Deadlock.
                 */
                if !wait_for_alt_release() {
                    eprintln!(
                        "[SPLIT] Save {slot} cancelled: Alt was held too long"
                    );

                    continue;
                }

                match watcher::request_save_slot(
                    slot,
                ) {
                    Ok(_) => {
                        println!(
                            "[SPLIT] Capture requested for slot {slot}"
                        );
                    }

                    Err(error) => {
                        eprintln!(
                            "[SPLIT] Could not request save {slot}: {error}"
                        );

                        continue;
                    }
                }

                if let Err(error) =
                    send_capture_key()
                {
                    watcher::cancel_pending_save();

                    eprintln!(
                        "[SPLIT] Could not send H: {error}"
                    );
                }
            }
        })
        .map_err(|error| {
            format!(
                "Could not start save hotkey worker: {error}"
            )
        })?;

    /*
     * Hook clavier Windows.
     */
    thread::Builder::new()
        .name(
            "split-keyboard-hook"
                .to_string(),
        )
        .spawn(|| unsafe {
            let hook =
                SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(keyboard_hook),
                    std::ptr::null_mut(),
                    0,
                );

            if hook.is_null() {
                eprintln!(
                    "[SPLIT] Failed to install keyboard hook"
                );

                return;
            }

            println!(
                "[SPLIT] Deadlock Save hotkeys active: Alt+F1-F8"
            );

            let mut message =
                MSG::default();

            while GetMessageW(
                &mut message,
                std::ptr::null_mut(),
                0,
                0,
            ) > 0
            {
                TranslateMessage(
                    &message,
                );

                DispatchMessageW(
                    &message,
                );
            }

            let _ =
                UnhookWindowsHookEx(
                    hook,
                );
        })
        .map_err(|error| {
            format!(
                "Could not start keyboard hook: {error}"
            )
        })?;

    Ok(())
}