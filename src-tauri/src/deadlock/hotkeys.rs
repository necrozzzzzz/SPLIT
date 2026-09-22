use std::{
    collections::HashSet,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc, LazyLock, Mutex, OnceLock, RwLock,
    },
    thread,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use windows_sys::{
    core::BOOL,
    Win32::{
        Foundation::{
            CloseHandle, GetLastError, SetLastError, HWND, LPARAM, RECT, ERROR_SUCCESS,
        },
        System::Threading::{
            AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::{
            Accessibility::{
                SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK,
            },
            Input::KeyboardAndMouse::{
                GetAsyncKeyState, GetKeyState, RegisterHotKey, SendInput, SetActiveWindow,
                SetFocus, UnregisterHotKey, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
                KEYEVENTF_KEYUP, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F10, VK_F11,
                VK_F12, VK_F13, VK_F14, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6, VK_F7, VK_F8, VK_F9,
                VK_HOME, VK_INSERT, VK_LCONTROL, VK_LEFT, VK_LMENU, VK_LSHIFT, VK_MENU, VK_NEXT,
                VK_PRIOR, VK_RCONTROL, VK_RIGHT, VK_RMENU, VK_RSHIFT, VK_SHIFT, VK_SPACE,
                VK_UP, VK_CAPITAL, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT,
            },
            WindowsAndMessaging::{
                BringWindowToTop, CallNextHookEx, DispatchMessageW, EnumWindows,
                GetForegroundWindow, GetMessageW, GetWindowRect, GetWindowThreadProcessId, IsIconic,
                IsWindowVisible, PostThreadMessageW, SetForegroundWindow, SetWindowsHookExW,
                ShowWindow, TranslateMessage, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG,
                EVENT_SYSTEM_FOREGROUND, SW_RESTORE, WH_KEYBOARD_LL, WINEVENT_OUTOFCONTEXT,
                WM_APP, WM_HOTKEY, WM_KEYDOWN, WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN,
                WM_SYSKEYUP,
            },
        },
    },
};

use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use super::watcher;

/*
 * Action envoyée par le hook clavier
 * au worker SPLIT.
 */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UserHotkeyAction {
    Save(u8),
    Load(u8),
    CyclePreset,
    Undo,
    Redo,
    ToggleFavorites,
}

#[derive(Debug, Clone)]
enum HotkeyAction {
    User {
        action: UserHotkeyAction,
        hotkey: Hotkey,
    },
    Prime(u8),
    Shutdown,
}

const QUICK_ACCESS_HOLD_DURATION: Duration = Duration::from_millis(250);
const QUICK_ACCESS_CAPSLOCK_ID: i32 = 0x5350;
const QUICK_ACCESS_ESCAPE_ID: i32 = 0x5351;
const QUICK_ACCESS_HIDDEN_MESSAGE: u32 = WM_APP + 1;
const QUICK_ACCESS_CONTROL_MESSAGE: u32 = WM_APP + 2;
const QUICK_ACCESS_FOREGROUND_MESSAGE: u32 = WM_APP + 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuickAccessMode {
    Hidden,
    Passive,
    Interactive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuickAccessForeground {
    Deadlock,
    QuickAccess,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct QuickAccessRegistrationPlan {
    caps_lock: bool,
    escape: bool,
}

fn quick_access_auto_hide_mode(
    foreground: QuickAccessForeground,
    visible: bool,
) -> Option<QuickAccessMode> {
    (foreground == QuickAccessForeground::Other && visible)
        .then_some(QuickAccessMode::Hidden)
}

fn quick_access_registration_plan(
    foreground: QuickAccessForeground,
    text_input_active: bool,
    quick_access_visible: bool,
    quick_access_enabled: bool,
) -> QuickAccessRegistrationPlan {
    let context_active = matches!(
        foreground,
        QuickAccessForeground::Deadlock |
            QuickAccessForeground::QuickAccess
    );

    QuickAccessRegistrationPlan {
        caps_lock:
            quick_access_enabled &&
                context_active &&
                !text_input_active,
        escape:
            quick_access_enabled &&
                context_active &&
                !text_input_active &&
                quick_access_visible,
    }
}

enum QuickAccessControlAction {
    SetTextInputActive {
        active: bool,
        response: mpsc::SyncSender<Result<(), String>>,
    },
    Reconfigure {
        registration: SystemHotkeyRegistration,
        response: mpsc::SyncSender<Result<(), String>>,
    },
    Refresh {
        response: mpsc::SyncSender<Result<(), String>>,
    },
}

static HOTKEY_SENDER: OnceLock<mpsc::Sender<HotkeyAction>> = OnceLock::new();

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hotkey {
    pub key: String,
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
}

impl Hotkey {
    fn new(key: impl Into<String>, ctrl: bool, alt: bool, shift: bool) -> Self {
        Self {
            key: key.into(),
            ctrl,
            alt,
            shift,
        }
    }

    fn display(&self) -> String {
        let mut parts = Vec::new();
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        parts.push(&self.key);
        parts.join("+")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    rename_all = "camelCase",
    default
)]
pub struct HotkeySettings {
    pub load_slots: [Hotkey; 8],
    pub save_slots: [Hotkey; 8],
    pub undo: Hotkey,
    pub redo: Hotkey,
    pub cycle_preset: Hotkey,
    pub favorite_mode: Hotkey,
    pub quick_access: Hotkey,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        Self {
            load_slots: std::array::from_fn(|index| {
                Hotkey::new(format!("F{}", index + 1), false, false, false)
            }),
            save_slots: std::array::from_fn(|index| {
                Hotkey::new(format!("F{}", index + 1), false, true, false)
            }),
            undo: Hotkey::new("F9", false, false, false),
            redo: Hotkey::new("F10", false, false, false),
            cycle_preset: Hotkey::new("V", false, false, false),
            favorite_mode: Hotkey::new("F11", false, false, false),
            quick_access: Hotkey::new(
                "CapsLock",
                false,
                false,
                false,
            ),
        }
    }
}

const LOAD_LABELS: [&str; 8] = [
    "Load Slot 1",
    "Load Slot 2",
    "Load Slot 3",
    "Load Slot 4",
    "Load Slot 5",
    "Load Slot 6",
    "Load Slot 7",
    "Load Slot 8",
];
const SAVE_LABELS: [&str; 8] = [
    "Save Slot 1",
    "Save Slot 2",
    "Save Slot 3",
    "Save Slot 4",
    "Save Slot 5",
    "Save Slot 6",
    "Save Slot 7",
    "Save Slot 8",
];

impl HotkeySettings {
    pub fn normalized(mut self) -> Result<Self, String> {
        for hotkey in self.all_mut() {
            hotkey.key = normalize_key(&hotkey.key)?;
        }
        for hotkey in &self.load_slots {
            validate_hotkey(hotkey, false)?;
        }
        for (index, hotkey) in self.save_slots.iter().enumerate() {
            // Alt+F4 is the historical Save Slot 4 default. It remains valid
            // only in that exact default position; new assignments reject it.
            validate_hotkey(hotkey, index == 3)?;
        }
        for hotkey in [
            &self.undo,
            &self.redo,
            &self.cycle_preset,
            &self.favorite_mode,
            &self.quick_access,
        ] {
            validate_hotkey(hotkey, false)?;
        }
        self.validate_conflicts()?;
        Ok(self)
    }

    pub fn validate_update_from(&self, previous: &Self) -> Result<(), String> {
        let historical = Hotkey::new("F4", false, true, false);
        if self.save_slots[3] == historical && previous.save_slots[3] != historical {
            return Err("Alt+F4 is reserved by Windows.".to_string());
        }
        Ok(())
    }

    fn all(&self) -> Vec<(&'static str, &Hotkey)> {
        let mut bindings = Vec::with_capacity(20);
        for (index, hotkey) in self.load_slots.iter().enumerate() {
            bindings.push((LOAD_LABELS[index], hotkey));
        }
        for (index, hotkey) in self.save_slots.iter().enumerate() {
            bindings.push((SAVE_LABELS[index], hotkey));
        }
        bindings.extend([
            ("Undo", &self.undo),
            ("Redo", &self.redo),
            ("Cycle Preset", &self.cycle_preset),
            ("Favorite Mode", &self.favorite_mode),
            ("Quick Access", &self.quick_access),
        ]);
        bindings
    }

    fn all_mut(&mut self) -> Vec<&mut Hotkey> {
        let mut bindings = Vec::with_capacity(20);
        bindings.extend(self.load_slots.iter_mut());
        bindings.extend(self.save_slots.iter_mut());
        bindings.extend([
            &mut self.undo,
            &mut self.redo,
            &mut self.cycle_preset,
            &mut self.favorite_mode,
            &mut self.quick_access,
        ]);
        bindings
    }

    fn validate_conflicts(&self) -> Result<(), String> {
        let bindings = self.all();
        for (index, (_, hotkey)) in bindings.iter().enumerate() {
            if let Some((other_label, _)) = bindings[..index]
                .iter()
                .find(|(_, other)| *other == *hotkey)
            {
                return Err(format!(
                    "{} is already assigned to {other_label}.",
                    hotkey.display()
                ));
            }
        }
        Ok(())
    }

    fn action_for(&self, hotkey: &Hotkey) -> Option<UserHotkeyAction> {
        self.load_slots
            .iter()
            .position(|item| item == hotkey)
            .map(|index| UserHotkeyAction::Load(index as u8 + 1))
            .or_else(|| {
                self.save_slots
                    .iter()
                    .position(|item| item == hotkey)
                    .map(|index| UserHotkeyAction::Save(index as u8 + 1))
            })
            .or_else(|| (self.undo == *hotkey).then_some(UserHotkeyAction::Undo))
            .or_else(|| (self.redo == *hotkey).then_some(UserHotkeyAction::Redo))
            .or_else(|| (self.cycle_preset == *hotkey).then_some(UserHotkeyAction::CyclePreset))
            .or_else(|| {
                (self.favorite_mode == *hotkey).then_some(UserHotkeyAction::ToggleFavorites)
            })
    }
}

fn normalize_key(key: &str) -> Result<String, String> {
    let compact = key.trim().replace([' ', '_', '-'], "").to_ascii_uppercase();
    let canonical = match compact.as_str() {
        "ARROWUP" | "UP" => "ArrowUp",
        "ARROWDOWN" | "DOWN" => "ArrowDown",
        "ARROWLEFT" | "LEFT" => "ArrowLeft",
        "ARROWRIGHT" | "RIGHT" => "ArrowRight",
        "PAGEUP" => "PageUp",
        "PAGEDOWN" => "PageDown",
        "HOME" => "Home",
        "END" => "End",
        "INSERT" => "Insert",
        "DELETE" | "DEL" => "Delete",
        "CAPSLOCK" | "CAPS" => "CapsLock",
        "SPACE" | "SPACEBAR" => "Space",
        value if value.len() == 1 && value.as_bytes()[0].is_ascii_alphanumeric() => value,
        value
            if value
                .strip_prefix('F')
                .and_then(|number| number.parse::<u8>().ok())
                .is_some_and(|number| (1..=12).contains(&number)) =>
        {
            value
        }
        "CONTROL" | "CTRL" | "ALT" | "SHIFT" => {
            return Err("A modifier alone cannot be assigned.".to_string())
        }
        "META" | "WIN" | "WINDOWS" => return Err("Win/Meta hotkeys are not supported.".to_string()),
        _ => return Err(format!("{key} is not a supported hotkey.")),
    };
    Ok(canonical.to_string())
}

fn validate_hotkey(hotkey: &Hotkey, allow_historical_alt_f4: bool) -> Result<(), String> {
    let display = hotkey.display();

    if (hotkey.alt
        && !hotkey.ctrl
        && !hotkey.shift
        && (matches!(hotkey.key.as_str(), "Tab" | "Escape")
            || (hotkey.key == "F4" && !allow_historical_alt_f4)))
        || (hotkey.ctrl && !hotkey.alt && !hotkey.shift && hotkey.key == "Escape")
        || (hotkey.ctrl && hotkey.alt && hotkey.key == "Delete")
    {
        return Err(format!("{display} is reserved by Windows."));
    }

    if matches!(
        hotkey.key.as_str(),
        "H" | "U" | "I" | "O" | "J" | "K" | "L" | "N" | "M"
    ) {
        return Err(format!("{} is reserved by SPLIT.", hotkey.key));
    }

    Ok(())
}

static HOTKEY_SETTINGS: OnceLock<RwLock<HotkeySettings>> = OnceLock::new();

fn runtime_settings() -> &'static RwLock<HotkeySettings> {
    HOTKEY_SETTINGS.get_or_init(|| RwLock::new(super::paths::load_hotkey_settings()))
}

pub fn current_settings() -> HotkeySettings {
    runtime_settings()
        .read()
        .unwrap_or_else(|error| error.into_inner())
        .clone()
}

pub fn apply_settings(settings: HotkeySettings) {
    *runtime_settings()
        .write()
        .unwrap_or_else(|error| error.into_inner()) = settings;
}

/*
 * Signature placée dans dwExtraInfo pour
 * reconnaître les inputs créés par SPLIT.
 */
const SPLIT_INJECT_TAG: usize = 0x5350_4C54;

/*
 * Vrai pendant le masque d'affichage d'un Load.
 *
 * Si quelque chose échoue pendant cette période,
 * F10 physique est laissé passer à Deadlock et
 * exécute "r_force_no_present 0".
 */
static PRESENTATION_MASK_ACTIVE: AtomicBool = AtomicBool::new(false);

/*
 * Vrai uniquement lorsque le hook clavier
 * Windows est réellement installé et actif.
 */
static HOTKEYS_RUNNING: AtomicBool = AtomicBool::new(false);

/*
 * Dernière erreur fatale du système de hotkeys.
 *
 * Principalement utile si Windows refuse
 * l'installation du WH_KEYBOARD_LL hook.
 */
static HOTKEY_LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

pub fn is_running() -> bool {
    HOTKEYS_RUNNING.load(Ordering::SeqCst)
}

fn set_hotkey_error(error: impl Into<String>) {
    if let Ok(mut last_error) = HOTKEY_LAST_ERROR.lock() {
        *last_error = Some(error.into());
    }
}

fn clear_hotkey_error() {
    if let Ok(mut last_error) = HOTKEY_LAST_ERROR.lock() {
        *last_error = None;
    }
}

pub fn runtime_status() -> (bool, Option<String>) {
    let error = HOTKEY_LAST_ERROR
        .lock()
        .ok()
        .and_then(|last_error| last_error.clone());

    (is_running(), error)
}

pub fn presentation_mask_active() -> bool {
    PRESENTATION_MASK_ACTIVE.load(Ordering::SeqCst)
}

static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);
static QUICK_ACCESS_HOTKEY_THREAD_ID: AtomicU32 = AtomicU32::new(0);
static QUICK_ACCESS_HOTKEY_SHUTDOWN: AtomicBool = AtomicBool::new(false);
static QUICK_ACCESS_CONTROL_SENDER: Mutex<
    Option<mpsc::Sender<QuickAccessControlAction>>,
> = Mutex::new(None);

unsafe extern "system" fn quick_access_foreground_event(
    _hook: HWINEVENTHOOK,
    _event: u32,
    _hwnd: HWND,
    _object_id: i32,
    _child_id: i32,
    _event_thread: u32,
    _event_time: u32,
) {
    let thread_id =
        QUICK_ACCESS_HOTKEY_THREAD_ID.load(Ordering::SeqCst);

    if thread_id != 0 {
        PostThreadMessageW(
            thread_id,
            QUICK_ACCESS_FOREGROUND_MESSAGE,
            0,
            0,
        );
    }
}

struct HotkeyRuntime {
    worker: JoinHandle<()>,
    hook: JoinHandle<()>,
    quick_access: JoinHandle<()>,
}

static HOTKEY_RUNTIME: Mutex<Option<HotkeyRuntime>> = Mutex::new(None);

pub fn stop() -> Result<(), String> {
    let runtime = HOTKEY_RUNTIME
        .lock()
        .map_err(|_| "Hotkey runtime lock poisoned".to_string())?
        .take();

    if let Some(runtime) = runtime {
        QUICK_ACCESS_HOTKEY_SHUTDOWN.store(true, Ordering::SeqCst);

        if let Some(sender) = HOTKEY_SENDER.get() {
            let _ = sender.send(HotkeyAction::Shutdown);
        }

        let thread_id = HOOK_THREAD_ID.swap(0, Ordering::SeqCst);
        let hook_stop_posted = if thread_id != 0 {
            unsafe { PostThreadMessageW(thread_id, WM_QUIT, 0, 0) != 0 }
        } else {
            true
        };

        let quick_access_thread_id =
            QUICK_ACCESS_HOTKEY_THREAD_ID.swap(0, Ordering::SeqCst);
        let quick_access_stop_posted = if quick_access_thread_id != 0 {
            unsafe {
                PostThreadMessageW(
                    quick_access_thread_id,
                    WM_QUIT,
                    0,
                    0,
                ) != 0
            }
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

        if quick_access_stop_posted {
            runtime
                .quick_access
                .join()
                .map_err(|_| {
                    "Quick Access hotkey service panicked while stopping".to_string()
                })?;
        } else {
            return Err(
                "Could not post WM_QUIT to Quick Access hotkey thread".to_string()
            );
        }
    }

    Ok(())
}

fn key_to_vk(key: &str) -> Option<u16> {
    match key {
        value if value.len() == 1 && value.as_bytes()[0].is_ascii_alphanumeric() => {
            Some(value.as_bytes()[0] as u16)
        }
        "F1" => Some(VK_F1),
        "F2" => Some(VK_F2),
        "F3" => Some(VK_F3),
        "F4" => Some(VK_F4),
        "F5" => Some(VK_F5),
        "F6" => Some(VK_F6),
        "F7" => Some(VK_F7),
        "F8" => Some(VK_F8),
        "F9" => Some(VK_F9),
        "F10" => Some(VK_F10),
        "F11" => Some(VK_F11),
        "F12" => Some(VK_F12),
        "ArrowUp" => Some(VK_UP),
        "ArrowDown" => Some(VK_DOWN),
        "ArrowLeft" => Some(VK_LEFT),
        "ArrowRight" => Some(VK_RIGHT),
        "Home" => Some(VK_HOME),
        "End" => Some(VK_END),
        "Insert" => Some(VK_INSERT),
        "Delete" => Some(VK_DELETE),
        "PageUp" => Some(VK_PRIOR),
        "PageDown" => Some(VK_NEXT),
        "Space" => Some(VK_SPACE),
        "CapsLock" => Some(VK_CAPITAL),
        _ => None,
    }
}

fn vk_to_key(vk: u16) -> Option<String> {
    if (b'A' as u16..=b'Z' as u16).contains(&vk) || (b'0' as u16..=b'9' as u16).contains(&vk) {
        return char::from_u32(vk as u32).map(|value| value.to_string());
    }
    let key = match vk {
        VK_F1 => "F1",
        VK_F2 => "F2",
        VK_F3 => "F3",
        VK_F4 => "F4",
        VK_F5 => "F5",
        VK_F6 => "F6",
        VK_F7 => "F7",
        VK_F8 => "F8",
        VK_F9 => "F9",
        VK_F10 => "F10",
        VK_F11 => "F11",
        VK_F12 => "F12",
        VK_UP => "ArrowUp",
        VK_DOWN => "ArrowDown",
        VK_LEFT => "ArrowLeft",
        VK_RIGHT => "ArrowRight",
        VK_HOME => "Home",
        VK_END => "End",
        VK_INSERT => "Insert",
        VK_DELETE => "Delete",
        VK_CAPITAL => "CapsLock",
        VK_PRIOR => "PageUp",
        VK_NEXT => "PageDown",
        VK_SPACE => "Space",
        _ => return None,
    };
    Some(key.to_string())
}

fn modifier_for_vk(vk: u16) -> Option<Modifier> {
    match vk {
        VK_CONTROL | VK_LCONTROL | VK_RCONTROL => Some(Modifier::Ctrl),
        VK_MENU | VK_LMENU | VK_RMENU => Some(Modifier::Alt),
        VK_SHIFT | VK_LSHIFT | VK_RSHIFT => Some(Modifier::Shift),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy)]
enum Modifier {
    Ctrl,
    Alt,
    Shift,
}

#[derive(Debug, Default)]
struct HookEngine {
    ctrl_keys: HashSet<u16>,
    alt_keys: HashSet<u16>,
    shift_keys: HashSet<u16>,
    down_keys: HashSet<u16>,
    consumed_keys: HashSet<u16>,
    emergency_f10: bool,
}

#[derive(Debug, PartialEq, Eq)]
enum HookDecision {
    Pass,
    Consume,
    Trigger(UserHotkeyAction, Hotkey),
    EmergencyF10,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ModifierState {
    ctrl: bool,
    alt: bool,
    shift: bool,
}

impl HookEngine {
    fn modifier_state(&self) -> ModifierState {
        ModifierState {
            ctrl: !self.ctrl_keys.is_empty(),
            alt: !self.alt_keys.is_empty(),
            shift: !self.shift_keys.is_empty(),
        }
    }

    fn reconcile_modifiers(
        &mut self,
        ctrl_down: bool,
        alt_down: bool,
        shift_down: bool,
    ) -> Option<(ModifierState, ModifierState)> {
        let tracked = self.modifier_state();
        let physical = ModifierState {
            ctrl: ctrl_down,
            alt: alt_down,
            shift: shift_down,
        };

        if tracked == physical {
            return None;
        }

        Self::reconcile_modifier(&mut self.ctrl_keys, ctrl_down, VK_CONTROL);
        Self::reconcile_modifier(&mut self.alt_keys, alt_down, VK_MENU);
        Self::reconcile_modifier(&mut self.shift_keys, shift_down, VK_SHIFT);

        Some((tracked, physical))
    }

    fn reconcile_modifier(keys: &mut HashSet<u16>, is_down: bool, fallback_vk: u16) {
        if is_down {
            if keys.is_empty() {
                keys.insert(fallback_vk);
            }
        } else {
            keys.clear();
        }
    }

    fn classify(
        &mut self,
        vk: u16,
        key_down: bool,
        key_up: bool,
        injected_by_split: bool,
        deadlock_foreground: bool,
        presentation_mask_active: bool,
        settings: &HotkeySettings,
    ) -> HookDecision {
        if injected_by_split {
            return HookDecision::Pass;
        }

        if matches!(vk, VK_CAPITAL | VK_ESCAPE) {
            return HookDecision::Pass;
        }

        if let Some(modifier) = modifier_for_vk(vk) {
            if key_down {
                self.modifier_keys_mut(modifier).insert(vk);
            } else if key_up {
                self.modifier_keys_mut(modifier).remove(&vk);
            }
            return HookDecision::Pass;
        }

        if key_up {
            self.down_keys.remove(&vk);
            if vk == VK_F10 && self.emergency_f10 {
                self.emergency_f10 = false;
                return HookDecision::Pass;
            }
            return if self.consumed_keys.remove(&vk) {
                HookDecision::Consume
            } else {
                HookDecision::Pass
            };
        }

        if !key_down {
            return HookDecision::Pass;
        }

        let newly_pressed = self.down_keys.insert(vk);
        if !newly_pressed {
            if vk == VK_F10 && self.emergency_f10 {
                return HookDecision::Pass;
            }
            return if self.consumed_keys.contains(&vk) {
                HookDecision::Consume
            } else {
                HookDecision::Pass
            };
        }

        if vk == VK_F10 && presentation_mask_active {
            self.emergency_f10 = true;
            return HookDecision::EmergencyF10;
        }

        if !deadlock_foreground {
            return HookDecision::Pass;
        }

        let Some(key) = vk_to_key(vk) else {
            return HookDecision::Pass;
        };
        let hotkey = Hotkey::new(
            key,
            !self.ctrl_keys.is_empty(),
            !self.alt_keys.is_empty(),
            !self.shift_keys.is_empty(),
        );
        let Some(action) = settings.action_for(&hotkey) else {
            return HookDecision::Pass;
        };
        self.consumed_keys.insert(vk);
        HookDecision::Trigger(action, hotkey)
    }

    fn modifier_keys_mut(&mut self, modifier: Modifier) -> &mut HashSet<u16> {
        match modifier {
            Modifier::Ctrl => &mut self.ctrl_keys,
            Modifier::Alt => &mut self.alt_keys,
            Modifier::Shift => &mut self.shift_keys,
        }
    }
}

static HOOK_ENGINE: LazyLock<Mutex<HookEngine>> =
    LazyLock::new(|| Mutex::new(HookEngine::default()));

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

fn physical_key_is_down(vk: u16) -> bool {
    let state = unsafe { GetAsyncKeyState(vk as i32) };
    (state as u16 & 0x8000) != 0
}

fn caps_lock_toggle_enabled() -> bool {
    let state = unsafe { GetKeyState(VK_CAPITAL as i32) };
    (state as u16 & 1) != 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SystemHotkeyRegistration {
    modifiers: u32,
    vk: u32,
}

fn restore_caps_lock_toggle(
    expected: bool,
    registration: SystemHotkeyRegistration,
) -> Result<(), String> {
    if caps_lock_toggle_enabled() == expected {
        return Ok(());
    }

    if unsafe {
        UnregisterHotKey(
            std::ptr::null_mut(),
            QUICK_ACCESS_CAPSLOCK_ID,
        )
    } == 0
    {
        return Err(format!(
            "Could not pause CapsLock system hotkey for toggle restoration: {}",
            std::io::Error::last_os_error(),
        ));
    }

    let restore_result = send_virtual_key(VK_CAPITAL);
    thread::sleep(Duration::from_millis(10));
    let register_result = if unsafe {
        RegisterHotKey(
            std::ptr::null_mut(),
            QUICK_ACCESS_CAPSLOCK_ID,
            registration.modifiers,
            registration.vk,
        )
    } == 0
    {
        Err(format!(
            "Could not re-register CapsLock system hotkey after toggle restoration: {}",
            std::io::Error::last_os_error(),
        ))
    } else {
        Ok(())
    };

    match (restore_result, register_result) {
        (Err(restore), Err(register)) => Err(format!("{restore}; {register}")),
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn quick_access_registration(
    hotkey: &Hotkey,
) -> Result<SystemHotkeyRegistration, String> {
    let vk = key_to_vk(&hotkey.key)
        .ok_or_else(|| format!("Unsupported Quick Access hotkey: {}", hotkey.key))?;
    let mut modifiers = MOD_NOREPEAT;

    if hotkey.ctrl {
        modifiers |= MOD_CONTROL;
    }
    if hotkey.alt {
        modifiers |= MOD_ALT;
    }
    if hotkey.shift {
        modifiers |= MOD_SHIFT;
    }

    Ok(SystemHotkeyRegistration {
        modifiers,
        vk: vk as u32,
    })
}

fn caps_lock_release_transition(
    mode: QuickAccessMode,
    held_for: Duration,
) -> QuickAccessMode {
    match mode {
        QuickAccessMode::Hidden => QuickAccessMode::Passive,
        QuickAccessMode::Passive
            if held_for >= QUICK_ACCESS_HOLD_DURATION =>
        {
            QuickAccessMode::Interactive
        }
        QuickAccessMode::Passive => QuickAccessMode::Hidden,
        QuickAccessMode::Interactive => QuickAccessMode::Interactive,
    }
}

fn caps_lock_press_transition(mode: QuickAccessMode) -> QuickAccessMode {
    match mode {
        QuickAccessMode::Interactive => QuickAccessMode::Passive,
        other => other,
    }
}

fn escape_transition(mode: QuickAccessMode) -> QuickAccessMode {
    match mode {
        QuickAccessMode::Interactive => QuickAccessMode::Passive,
        QuickAccessMode::Passive => QuickAccessMode::Hidden,
        QuickAccessMode::Hidden => QuickAccessMode::Hidden,
    }
}

fn unregister_escape_hotkey(registered: &mut bool) {
    if !*registered {
        return;
    }

    if unsafe { UnregisterHotKey(std::ptr::null_mut(), QUICK_ACCESS_ESCAPE_ID) } == 0 {
        eprintln!(
            "[SPLIT][QA] Could not unregister Escape system hotkey: {}",
            std::io::Error::last_os_error(),
        );
    } else {
        *registered = false;
    }
}

fn register_escape_hotkey(registered: &mut bool) {
    if *registered {
        return;
    }

    if unsafe {
        RegisterHotKey(
            std::ptr::null_mut(),
            QUICK_ACCESS_ESCAPE_ID,
            MOD_NOREPEAT,
            VK_ESCAPE as u32,
        )
    } == 0
    {
        eprintln!(
            "[SPLIT][QA] Could not register Escape system hotkey: {}",
            std::io::Error::last_os_error(),
        );
    } else {
        *registered = true;
    }
}

fn unregister_caps_lock_hotkey(
    registered: &mut bool,
) -> Result<(), String> {
    if !*registered {
        return Ok(());
    }

    if unsafe {
        UnregisterHotKey(
            std::ptr::null_mut(),
            QUICK_ACCESS_CAPSLOCK_ID,
        )
    } == 0
    {
        return Err(format!(
            "UnregisterHotKey failed for Quick Access shortcut: {}",
            std::io::Error::last_os_error(),
        ));
    }

    *registered = false;
    println!(
        "[SPLIT][QA] Quick Access shortcut unregistered"
    );
    Ok(())
}

fn register_caps_lock_hotkey(
    registered: &mut bool,
    registration: SystemHotkeyRegistration,
) -> Result<(), String> {
    if *registered {
        return Ok(());
    }

    if unsafe {
        RegisterHotKey(
            std::ptr::null_mut(),
            QUICK_ACCESS_CAPSLOCK_ID,
            registration.modifiers,
            registration.vk,
        )
    } == 0
    {
        return Err(format!(
            "RegisterHotKey failed for Quick Access shortcut: {}",
            std::io::Error::last_os_error(),
        ));
    }

    *registered = true;
    println!(
        "[SPLIT][QA] Quick Access shortcut registered"
    );
    Ok(())
}

fn quick_access_foreground(
    app: &AppHandle,
    hwnd: HWND,
) -> QuickAccessForeground {
    if hwnd.is_null() {
        return QuickAccessForeground::Other;
    }

    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut pid);
    }

    if is_deadlock_process(pid) {
        QuickAccessForeground::Deadlock
    } else if crate::quick_access::owns_window_handle(
        app,
        hwnd,
    ) {
        QuickAccessForeground::QuickAccess
    } else {
        QuickAccessForeground::Other
    }
}

fn apply_quick_access_registration_plan(
    plan: QuickAccessRegistrationPlan,
    caps_lock_registered: &mut bool,
    escape_registered: &mut bool,
    registration: SystemHotkeyRegistration,
) -> Result<(), String> {
    if plan.caps_lock {
        register_caps_lock_hotkey(
            caps_lock_registered,
            registration,
        )?;
    } else {
        unregister_caps_lock_hotkey(
            caps_lock_registered,
        )?;
    }

    if plan.escape {
        register_escape_hotkey(escape_registered);
    } else {
        unregister_escape_hotkey(escape_registered);
    }

    if *escape_registered != plan.escape {
        return Err(format!(
            "Could not {} Escape system hotkey",
            if plan.escape {
                "register"
            } else {
                "unregister"
            },
        ));
    }

    Ok(())
}

fn reconcile_quick_access_hotkeys(
    app: &AppHandle,
    text_input_active: bool,
    caps_lock_registered: &mut bool,
    escape_registered: &mut bool,
    registration: SystemHotkeyRegistration,
) -> Result<QuickAccessForeground, String> {
    let hwnd = unsafe { GetForegroundWindow() };
    let foreground =
        quick_access_foreground(app, hwnd);
    let plan = quick_access_registration_plan(
        foreground,
        text_input_active,
        crate::quick_access::is_visible(),
        crate::quick_access::is_enabled(),
    );

    apply_quick_access_registration_plan(
        plan,
        caps_lock_registered,
        escape_registered,
        registration,
    )?;

    Ok(foreground)
}

fn log_quick_access_foreground(
    foreground: QuickAccessForeground,
) {
    println!(
        "[SPLIT][QA] foreground reconcile -> {}",
        quick_access_foreground_name(foreground),
    );
}

fn quick_access_foreground_name(
    foreground: QuickAccessForeground,
) -> &'static str {
    match foreground {
        QuickAccessForeground::Deadlock => "Deadlock",
        QuickAccessForeground::QuickAccess => "QuickAccess",
        QuickAccessForeground::Other => "Other",
    }
}

fn set_text_input_active_on_service(
    app: &AppHandle,
    active: bool,
    text_input_active: &mut bool,
    caps_lock_registered: &mut bool,
    escape_registered: &mut bool,
    registration: SystemHotkeyRegistration,
) -> Result<(), String> {
    if active == *text_input_active {
        return Ok(());
    }

    let previous = *text_input_active;
    *text_input_active = active;
    let result = reconcile_quick_access_hotkeys(
        app,
        *text_input_active,
        caps_lock_registered,
        escape_registered,
        registration,
    );

    if let Err(error) = result {
        *text_input_active = previous;
        let _ = reconcile_quick_access_hotkeys(
            app,
            previous,
            caps_lock_registered,
            escape_registered,
            registration,
        );
        return Err(error);
    }

    Ok(())
}

fn wait_for_physical_release(vk: u16) -> Option<Duration> {
    let started_at = Instant::now();

    while physical_key_is_down(vk) {
        if QUICK_ACCESS_HOTKEY_SHUTDOWN.load(Ordering::SeqCst) {
            return None;
        }
        thread::sleep(Duration::from_millis(10));
    }

    Some(started_at.elapsed())
}

fn handle_caps_lock_system_hotkey(
    app: &AppHandle,
    mode: &mut QuickAccessMode,
    escape_registered: &mut bool,
    registration: SystemHotkeyRegistration,
) {
    println!("[SPLIT][QA] CapsLock hotkey");

    let previous_toggle = (registration.vk == VK_CAPITAL as u32)
        .then(caps_lock_toggle_enabled);
    let vk = registration.vk as u16;

    match *mode {
        QuickAccessMode::Hidden => {
            let Some(held_for) = wait_for_physical_release(vk) else {
                return;
            };

            if let Some(expected) = previous_toggle {
                if let Err(error) = restore_caps_lock_toggle(expected, registration) {
                    eprintln!(
                        "[SPLIT][QA] Could not restore CapsLock system toggle: {error}"
                    );
                }
            }

            let next = caps_lock_release_transition(*mode, held_for);
            if let Err(error) = crate::quick_access::show(app) {
                eprintln!("[SPLIT][QA] Could not show Quick Access: {error}");
                return;
            }

            register_escape_hotkey(escape_registered);
            *mode = next;
            println!("[SPLIT][QA] Hidden -> Passive");
        }

        QuickAccessMode::Passive => {
            let started_at = Instant::now();

            while physical_key_is_down(vk)
                && started_at.elapsed() < QUICK_ACCESS_HOLD_DURATION
            {
                if QUICK_ACCESS_HOTKEY_SHUTDOWN.load(Ordering::SeqCst) {
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }

            let held_long_enough = physical_key_is_down(vk);
            let next = caps_lock_release_transition(
                *mode,
                if held_long_enough {
                    QUICK_ACCESS_HOLD_DURATION
                } else {
                    started_at.elapsed()
                },
            );

            if held_long_enough {
                if let Err(error) = crate::quick_access::enter_interaction_mode(app) {
                    eprintln!(
                        "[SPLIT][QA] Could not enter Quick Access interaction mode: {error}"
                    );
                } else {
                    *mode = next;
                    println!("[SPLIT][QA] Passive -> Interactive");
                }

                if wait_for_physical_release(vk).is_none() {
                    return;
                }
            } else if let Err(error) = crate::quick_access::hide(app) {
                eprintln!("[SPLIT][QA] Could not hide Quick Access: {error}");
            } else {
                unregister_escape_hotkey(escape_registered);
                *mode = next;
                println!("[SPLIT][QA] Passive -> Hidden");
            }

            if let Some(expected) = previous_toggle {
                if let Err(error) = restore_caps_lock_toggle(expected, registration) {
                    eprintln!(
                        "[SPLIT][QA] Could not restore CapsLock system toggle: {error}"
                    );
                }
            }
        }

        QuickAccessMode::Interactive => {
            let next = caps_lock_press_transition(*mode);

            if let Err(error) = crate::quick_access::exit_interaction_mode(app) {
                eprintln!(
                    "[SPLIT][QA] Could not leave Quick Access interaction mode: {error}"
                );
            } else {
                *mode = next;
                println!("[SPLIT][QA] Interactive -> Passive");
            }

            if wait_for_physical_release(vk).is_none() {
                return;
            }

            if let Some(expected) = previous_toggle {
                if let Err(error) = restore_caps_lock_toggle(expected, registration) {
                    eprintln!(
                        "[SPLIT][QA] Could not restore CapsLock system toggle: {error}"
                    );
                }
            }
        }
    }
}

fn handle_escape_system_hotkey(
    app: &AppHandle,
    mode: &mut QuickAccessMode,
    escape_registered: &mut bool,
) {
    println!("[SPLIT][QA] Escape hotkey");
    let next = escape_transition(*mode);

    match *mode {
        QuickAccessMode::Interactive => {
            if let Err(error) = crate::quick_access::exit_interaction_mode(app) {
                eprintln!(
                    "[SPLIT][QA] Could not leave Quick Access interaction mode: {error}"
                );
            } else {
                *mode = next;
                println!("[SPLIT][QA] Interactive -> Passive");
            }
        }

        QuickAccessMode::Passive => {
            if let Err(error) = crate::quick_access::hide(app) {
                eprintln!("[SPLIT][QA] Could not hide Quick Access: {error}");
            } else {
                unregister_escape_hotkey(escape_registered);
                *mode = next;
                println!("[SPLIT][QA] Passive -> Hidden");
            }
        }

        QuickAccessMode::Hidden => {}
    }
}

fn spawn_quick_access_hotkey_service(
    app: AppHandle,
) -> Result<JoinHandle<()>, String> {
    QUICK_ACCESS_HOTKEY_SHUTDOWN.store(false, Ordering::SeqCst);
    let initial_registration = quick_access_registration(
        &current_settings().quick_access,
    )?;
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let (control_tx, control_rx) = mpsc::channel();
    *QUICK_ACCESS_CONTROL_SENDER
        .lock()
        .map_err(|_| {
            "Quick Access control lock poisoned"
                .to_string()
        })? = Some(control_tx);
    let service = thread::Builder::new()
        .name("split-quick-access-hotkeys".to_string())
        .spawn(move || {
            let mut registration = initial_registration;
            QUICK_ACCESS_HOTKEY_THREAD_ID.store(
                unsafe { GetCurrentThreadId() },
                Ordering::SeqCst,
            );

            let foreground_hook = unsafe {
                SetWinEventHook(
                    EVENT_SYSTEM_FOREGROUND,
                    EVENT_SYSTEM_FOREGROUND,
                    std::ptr::null_mut(),
                    Some(quick_access_foreground_event),
                    0,
                    0,
                    WINEVENT_OUTOFCONTEXT,
                )
            };

            if foreground_hook.is_null() {
                let error = format!(
                    "Could not install foreground event hook: {}",
                    std::io::Error::last_os_error(),
                );
                eprintln!("[SPLIT][QA] {error}");
                QUICK_ACCESS_HOTKEY_THREAD_ID.store(0, Ordering::SeqCst);
                if let Ok(mut sender) = QUICK_ACCESS_CONTROL_SENDER.lock() {
                    *sender = None;
                }
                let _ = ready_tx.send(Err(error));
                return;
            }

            let mut mode = if crate::quick_access::is_interactive() {
                QuickAccessMode::Interactive
            } else if crate::quick_access::is_visible() {
                QuickAccessMode::Passive
            } else {
                QuickAccessMode::Hidden
            };
            let mut escape_registered = false;
            let mut caps_lock_registered = false;
            let mut text_input_active = false;
            let initial_hwnd = unsafe { GetForegroundWindow() };
            let mut last_foreground =
                quick_access_foreground(&app, initial_hwnd);

            if let Err(error) = reconcile_quick_access_hotkeys(
                &app,
                text_input_active,
                &mut caps_lock_registered,
                &mut escape_registered,
                registration,
            ) {
                unsafe {
                    UnhookWinEvent(foreground_hook);
                }
                eprintln!("[SPLIT][QA] {error}");
                QUICK_ACCESS_HOTKEY_THREAD_ID.store(0, Ordering::SeqCst);
                if let Ok(mut sender) = QUICK_ACCESS_CONTROL_SENDER.lock() {
                    *sender = None;
                }
                let _ = ready_tx.send(Err(error));
                return;
            }

            log_quick_access_foreground(last_foreground);
            println!("[SPLIT][QA] system hotkey service started");
            let _ = ready_tx.send(Ok(()));

            let mut message = MSG::default();

            loop {
                let result = unsafe {
                    GetMessageW(&mut message, std::ptr::null_mut(), 0, 0)
                };

                if result == 0 {
                    break;
                }
                if result == -1 {
                    eprintln!(
                        "[SPLIT][QA] System hotkey message loop failed: {}",
                        std::io::Error::last_os_error(),
                    );
                    break;
                }

                if message.message == WM_HOTKEY {
                    if text_input_active {
                        continue;
                    }

                    match message.wParam as i32 {
                        QUICK_ACCESS_CAPSLOCK_ID => {
                            handle_caps_lock_system_hotkey(
                                &app,
                                &mut mode,
                                &mut escape_registered,
                                registration,
                            );
                        }
                        QUICK_ACCESS_ESCAPE_ID => {
                            handle_escape_system_hotkey(
                                &app,
                                &mut mode,
                                &mut escape_registered,
                            );
                        }
                        _ => {}
                    }
                } else if message.message == QUICK_ACCESS_HIDDEN_MESSAGE {
                    mode = QuickAccessMode::Hidden;
                    if let Err(error) = set_text_input_active_on_service(
                        &app,
                        false,
                        &mut text_input_active,
                        &mut caps_lock_registered,
                        &mut escape_registered,
                        registration,
                    ) {
                        eprintln!(
                            "[SPLIT][QA] Could not reset text input hotkeys after hide: {error}"
                        );
                    }
                    unregister_escape_hotkey(&mut escape_registered);
                } else if message.message == QUICK_ACCESS_CONTROL_MESSAGE {
                    while let Ok(action) = control_rx.try_recv() {
                        match action {
                            QuickAccessControlAction::SetTextInputActive {
                                active,
                                response,
                            } => {
                                println!(
                                    "[SPLIT][QA] text input request received active={active}"
                                );
                                let result = set_text_input_active_on_service(
                                    &app,
                                    active,
                                    &mut text_input_active,
                                    &mut caps_lock_registered,
                                    &mut escape_registered,
                                    registration,
                                );
                                if active && result.is_ok() {
                                    println!(
                                        "[SPLIT][QA] CapsLock/Escape unregistered for text input"
                                    );
                                }
                                if response.send(result).is_ok() {
                                    println!(
                                        "[SPLIT][QA] text input acknowledgement sent"
                                    );
                                }
                            }
                            QuickAccessControlAction::Reconfigure {
                                registration: next_registration,
                                response,
                            } => {
                                let previous_registration = registration;
                                let previous_caps = caps_lock_registered;
                                let previous_escape = escape_registered;

                                unregister_escape_hotkey(
                                    &mut escape_registered,
                                );
                                let result = unregister_caps_lock_hotkey(
                                    &mut caps_lock_registered,
                                )
                                .and_then(|()| {
                                    register_caps_lock_hotkey(
                                        &mut caps_lock_registered,
                                        next_registration,
                                    )
                                })
                                .and_then(|()| {
                                    unregister_caps_lock_hotkey(
                                        &mut caps_lock_registered,
                                    )
                                })
                                .and_then(|()| {
                                    reconcile_quick_access_hotkeys(
                                        &app,
                                        text_input_active,
                                        &mut caps_lock_registered,
                                        &mut escape_registered,
                                        next_registration,
                                    )
                                    .map(|_| ())
                                });

                                if result.is_ok() {
                                    registration = next_registration;
                                } else {
                                    unregister_escape_hotkey(
                                        &mut escape_registered,
                                    );
                                    let _ = unregister_caps_lock_hotkey(
                                        &mut caps_lock_registered,
                                    );
                                    if previous_caps || previous_escape {
                                        let _ = reconcile_quick_access_hotkeys(
                                            &app,
                                            text_input_active,
                                            &mut caps_lock_registered,
                                            &mut escape_registered,
                                            previous_registration,
                                        );
                                    }
                                }

                                let _ = response.send(result);
                            }
                            QuickAccessControlAction::Refresh {
                                response,
                            } => {
                                let result = reconcile_quick_access_hotkeys(
                                    &app,
                                    text_input_active,
                                    &mut caps_lock_registered,
                                    &mut escape_registered,
                                    registration,
                                )
                                .map(|_| ());
                                let _ = response.send(result);
                            }
                        }
                    }
                } else if message.message == QUICK_ACCESS_FOREGROUND_MESSAGE {
                    match reconcile_quick_access_hotkeys(
                        &app,
                        text_input_active,
                        &mut caps_lock_registered,
                        &mut escape_registered,
                        registration,
                    ) {
                        Ok(foreground) => {
                            if foreground != last_foreground {
                                log_quick_access_foreground(foreground);
                                last_foreground = foreground;
                            }

                            if let Some(next_mode) =
                                quick_access_auto_hide_mode(
                                    foreground,
                                    crate::quick_access::is_visible(),
                                )
                            {
                                match crate::quick_access::hide_without_focus(&app) {
                                    Ok(()) => {
                                        mode = next_mode;
                                        text_input_active = false;
                                        unregister_escape_hotkey(
                                            &mut escape_registered,
                                        );
                                        println!(
                                            "[SPLIT][QA] Quick Access auto-hidden without focus"
                                        );

                                        match reconcile_quick_access_hotkeys(
                                            &app,
                                            text_input_active,
                                            &mut caps_lock_registered,
                                            &mut escape_registered,
                                            registration,
                                        ) {
                                            Ok(final_foreground) => {
                                                if final_foreground != foreground {
                                                    println!(
                                                        "[SPLIT][QA] foreground changed during auto-hide -> {}",
                                                        quick_access_foreground_name(
                                                            final_foreground,
                                                        ),
                                                    );
                                                }
                                                if final_foreground != last_foreground {
                                                    log_quick_access_foreground(
                                                        final_foreground,
                                                    );
                                                    last_foreground = final_foreground;
                                                }
                                            }
                                            Err(error) => eprintln!(
                                                "[SPLIT][QA] Could not reconcile hotkeys after foreground dismissal: {error}"
                                            ),
                                        }
                                    }
                                    Err(error) => eprintln!(
                                        "[SPLIT][QA] Could not hide Quick Access after foreground change: {error}"
                                    ),
                                }
                            }
                        }
                        Err(error) => eprintln!(
                            "[SPLIT][QA] Could not update hotkeys for foreground change: {error}"
                        ),
                    }
                }
            }

            if mode == QuickAccessMode::Interactive {
                let _ = crate::quick_access::exit_interaction_mode(&app);
            }
            if unsafe { UnhookWinEvent(foreground_hook) } == 0 {
                eprintln!(
                    "[SPLIT][QA] Could not remove foreground event hook: {}",
                    std::io::Error::last_os_error(),
                );
            }
            unregister_escape_hotkey(&mut escape_registered);
            if let Err(error) =
                unregister_caps_lock_hotkey(&mut caps_lock_registered)
            {
                eprintln!(
                    "[SPLIT][QA] {error}",
                );
            }
            if let Ok(mut sender) = QUICK_ACCESS_CONTROL_SENDER.lock() {
                *sender = None;
            }
            QUICK_ACCESS_HOTKEY_THREAD_ID.store(0, Ordering::SeqCst);
        })
        .map_err(|error| {
            if let Ok(mut sender) = QUICK_ACCESS_CONTROL_SENDER.lock() {
                *sender = None;
            }
            format!("Could not start Quick Access hotkey service: {error}")
        })?;

    match ready_rx.recv() {
        Ok(Ok(())) => Ok(service),
        Ok(Err(error)) => {
            let _ = service.join();
            Err(error)
        }
        Err(error) => {
            let _ = service.join();
            Err(format!(
                "Quick Access hotkey service did not initialize: {error}"
            ))
        }
    }
}

pub fn set_quick_access_text_input_active(
    active: bool,
) -> Result<(), String> {
    let sender = {
        QUICK_ACCESS_CONTROL_SENDER
            .lock()
            .map_err(|_| {
                "Quick Access control lock poisoned"
                    .to_string()
            })?
            .clone()
            .ok_or_else(|| {
                "Quick Access hotkey service is not running"
                    .to_string()
            })?
    };
    let thread_id =
        QUICK_ACCESS_HOTKEY_THREAD_ID.load(Ordering::SeqCst);

    if thread_id == 0 {
        return Err(
            "Quick Access hotkey service is not running"
                .to_string(),
        );
    }

    let (response_tx, response_rx) = mpsc::sync_channel(1);
    sender
        .send(QuickAccessControlAction::SetTextInputActive {
            active,
            response: response_tx,
        })
        .map_err(|error| {
            format!(
                "Could not send Quick Access hotkey control: {error}"
            )
        })?;

    println!(
        "[SPLIT][QA] text input request queued"
    );

    if unsafe {
        PostThreadMessageW(
            thread_id,
            QUICK_ACCESS_CONTROL_MESSAGE,
            0,
            0,
        )
    } == 0
    {
        return Err(format!(
            "Could not wake Quick Access hotkey service: {}",
            std::io::Error::last_os_error(),
        ));
    }

    match response_rx.recv_timeout(Duration::from_secs(1)) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            if active {
                let (rollback_tx, _rollback_rx) =
                    mpsc::sync_channel(1);
                if sender
                    .send(
                        QuickAccessControlAction::SetTextInputActive {
                            active: false,
                            response: rollback_tx,
                        },
                    )
                    .is_ok()
                {
                    unsafe {
                        PostThreadMessageW(
                            thread_id,
                            QUICK_ACCESS_CONTROL_MESSAGE,
                            0,
                            0,
                        );
                    }
                }
            }

            Err(
                "Quick Access hotkey service did not acknowledge text input mode"
                    .to_string(),
            )
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => Err(
            "Quick Access hotkey service disconnected before acknowledging text input mode"
                .to_string(),
        ),
    }
}

pub fn reconfigure_quick_access_hotkey(
    hotkey: &Hotkey,
) -> Result<(), String> {
    let registration = quick_access_registration(hotkey)?;
    let sender = {
        QUICK_ACCESS_CONTROL_SENDER
            .lock()
            .map_err(|_| {
                "Quick Access control lock poisoned"
                    .to_string()
            })?
            .clone()
    };
    let thread_id =
        QUICK_ACCESS_HOTKEY_THREAD_ID.load(Ordering::SeqCst);

    let Some(sender) = sender else {
        return Ok(());
    };
    if thread_id == 0 {
        return Ok(());
    }

    let (response_tx, response_rx) = mpsc::sync_channel(1);
    sender
        .send(QuickAccessControlAction::Reconfigure {
            registration,
            response: response_tx,
        })
        .map_err(|error| {
            format!(
                "Could not queue Quick Access shortcut update: {error}"
            )
        })?;

    if unsafe {
        PostThreadMessageW(
            thread_id,
            QUICK_ACCESS_CONTROL_MESSAGE,
            0,
            0,
        )
    } == 0
    {
        return Err(format!(
            "Could not wake Quick Access hotkey service: {}",
            std::io::Error::last_os_error(),
        ));
    }

    response_rx
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| {
            "Quick Access hotkey service did not acknowledge shortcut update"
                .to_string()
        })?
}

pub fn refresh_quick_access_hotkeys() -> Result<(), String> {
    let sender = {
        QUICK_ACCESS_CONTROL_SENDER
            .lock()
            .map_err(|_| {
                "Quick Access control lock poisoned"
                    .to_string()
            })?
            .clone()
    };
    let thread_id =
        QUICK_ACCESS_HOTKEY_THREAD_ID.load(Ordering::SeqCst);

    let Some(sender) = sender else {
        return Ok(());
    };
    if thread_id == 0 {
        return Ok(());
    }

    let (response_tx, response_rx) = mpsc::sync_channel(1);
    sender
        .send(QuickAccessControlAction::Refresh {
            response: response_tx,
        })
        .map_err(|error| {
            format!(
                "Could not queue Quick Access state refresh: {error}"
            )
        })?;

    if unsafe {
        PostThreadMessageW(
            thread_id,
            QUICK_ACCESS_CONTROL_MESSAGE,
            0,
            0,
        )
    } == 0
    {
        return Err(format!(
            "Could not wake Quick Access hotkey service: {}",
            std::io::Error::last_os_error(),
        ));
    }

    response_rx
        .recv_timeout(Duration::from_secs(1))
        .map_err(|_| {
            "Quick Access hotkey service did not acknowledge settings update"
                .to_string()
        })?
}

pub fn quick_access_hidden() {
    let thread_id = QUICK_ACCESS_HOTKEY_THREAD_ID.load(Ordering::SeqCst);

    if thread_id != 0 {
        unsafe {
            PostThreadMessageW(
                thread_id,
                QUICK_ACCESS_HIDDEN_MESSAGE,
                0,
                0,
            );
        }
    }
}

fn wait_for_hotkey_release(hotkey: &Hotkey) -> bool {
    let Some(main_key) = key_to_vk(&hotkey.key) else {
        return false;
    };
    for _ in 0..200 {
        let released = !physical_key_is_down(main_key)
            && !physical_key_is_down(VK_CONTROL)
            && !physical_key_is_down(VK_MENU)
            && !physical_key_is_down(VK_SHIFT);
        if released {
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

pub(crate) fn deadlock_window_rect() -> Result<RECT, String> {
    let hwnd = find_deadlock_window()
        .ok_or_else(|| {
            "Could not find the Deadlock window"
                .to_string()
        })?;
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };

    if unsafe { GetWindowRect(hwnd, &mut rect) } == 0 {
        return Err(format!(
            "Could not read Deadlock window bounds: {}",
            std::io::Error::last_os_error(),
        ));
    }

    Ok(rect)
}

struct ThreadInputAttachments {
    pairs: Vec<(u32, u32)>,
}

impl ThreadInputAttachments {
    fn new() -> Self {
        Self { pairs: Vec::new() }
    }

    fn attach(&mut self, from: u32, to: u32) -> Result<(), String> {
        if from == to || self.pairs.contains(&(from, to)) {
            return Ok(());
        }

        if unsafe { AttachThreadInput(from, to, 1) } == 0 {
            return Err(format!(
                "AttachThreadInput({from}, {to}) failed: {}",
                std::io::Error::last_os_error(),
            ));
        }

        self.pairs.push((from, to));
        Ok(())
    }

    fn detach_all(&mut self) -> Result<(), String> {
        let mut failed = Vec::new();
        let mut first_error = None;

        while let Some((from, to)) = self.pairs.pop() {
            if unsafe { AttachThreadInput(from, to, 0) } == 0 {
                if first_error.is_none() {
                    first_error = Some(format!(
                        "AttachThreadInput({from}, {to}, detach) failed: {}",
                        std::io::Error::last_os_error(),
                    ));
                }
                failed.push((from, to));
            }
        }

        self.pairs = failed;

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for ThreadInputAttachments {
    fn drop(&mut self) {
        if let Err(error) = self.detach_all() {
            eprintln!("[SPLIT][QA] {error}");
        }
    }
}

fn hwnd_call_succeeded(name: &str, previous: HWND) -> Result<(), String> {
    let error = unsafe { GetLastError() };

    if previous.is_null() && error != ERROR_SUCCESS {
        Err(format!(
            "{name} failed: {}",
            std::io::Error::from_raw_os_error(error as i32),
        ))
    } else {
        Ok(())
    }
}

fn focus_deadlock_once(hwnd: HWND) -> Result<(), String> {
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);

            for _ in 0..20 {
                if IsIconic(hwnd) == 0 {
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }

            if IsIconic(hwnd) != 0 {
                return Err("Deadlock window remained minimized after SW_RESTORE".to_string());
            }
        }
    }

    let current_thread = unsafe { GetCurrentThreadId() };
    let foreground = unsafe { GetForegroundWindow() };
    let foreground_thread = if foreground.is_null() {
        0
    } else {
        unsafe { GetWindowThreadProcessId(foreground, std::ptr::null_mut()) }
    };
    let deadlock_thread =
        unsafe { GetWindowThreadProcessId(hwnd, std::ptr::null_mut()) };

    if deadlock_thread == 0 {
        return Err(format!(
            "Could not get the Deadlock window thread: {}",
            std::io::Error::last_os_error(),
        ));
    }

    let mut attachments = ThreadInputAttachments::new();
    if foreground_thread != 0 {
        attachments.attach(current_thread, foreground_thread)?;
    }
    attachments.attach(current_thread, deadlock_thread)?;

    let activation_result = (|| unsafe {
        if BringWindowToTop(hwnd) == 0 {
            return Err(format!(
                "BringWindowToTop failed: {}",
                std::io::Error::last_os_error(),
            ));
        }

        if SetForegroundWindow(hwnd) == 0 {
            return Err(format!(
                "SetForegroundWindow failed: {}",
                std::io::Error::last_os_error(),
            ));
        }

        SetLastError(ERROR_SUCCESS);
        let previous_active = SetActiveWindow(hwnd);
        hwnd_call_succeeded("SetActiveWindow", previous_active)?;

        SetLastError(ERROR_SUCCESS);
        let previous_focus = SetFocus(hwnd);
        hwnd_call_succeeded("SetFocus", previous_focus)?;

        Ok(())
    })();

    let detach_result = attachments.detach_all();

    match (activation_result, detach_result) {
        (Err(activation), Err(detach)) => Err(format!("{activation}; {detach}")),
        (Err(error), _) | (_, Err(error)) => Err(error),
        (Ok(()), Ok(())) => Ok(()),
    }
}

pub(crate) fn focus_deadlock_window() -> Result<(), String> {
    let hwnd =
        find_deadlock_window().ok_or_else(|| "Could not find the Deadlock window".to_string())?;

    let mut last_error = None;

    for attempt in 0..2 {
        match focus_deadlock_once(hwnd) {
            Ok(()) => {
                let mut acquired = false;

                for _ in 0..20 {
                    if unsafe { GetForegroundWindow() } == hwnd {
                        acquired = true;
                        break;
                    }
                    thread::sleep(Duration::from_millis(10));
                }

                if acquired {
                    thread::sleep(Duration::from_millis(50));

                    if unsafe { GetForegroundWindow() } == hwnd {
                        println!("[SPLIT][QA] Deadlock focus stable");
                        return Ok(());
                    }

                    last_error = Some(
                        "Deadlock lost foreground focus after activation".to_string(),
                    );

                    if attempt == 0 {
                        println!(
                            "[SPLIT][QA] Deadlock focus lost after activation, retrying"
                        );
                    }
                } else {
                    last_error = Some(
                        "Deadlock did not receive foreground focus within 200 ms".to_string(),
                    );

                    if attempt == 0 {
                        eprintln!(
                            "[SPLIT][QA] Deadlock foreground not acquired, retrying"
                        );
                    }
                }
            }
            Err(error) => {
                last_error = Some(error);

                if attempt == 0 {
                    eprintln!("[SPLIT][QA] Deadlock activation failed, retrying");
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| {
        "Deadlock focus activation failed for an unknown reason".to_string()
    }))
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
                dwExtraInfo: SPLIT_INJECT_TAG,
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

fn send_prepare_key() -> Result<(), String> {
    send_virtual_key(VK_F13)
}

fn send_present_resume_key() -> Result<(), String> {
    send_virtual_key(VK_F10)
}

fn send_momentum_reset_key() -> Result<(), String> {
    send_virtual_key(VK_F14)
}

pub(crate) fn prepare_teleports_after_cfg_update() {
    if !super::cfg::teleports_dirty() {
        return;
    }

    /*
     * Ne JAMAIS envoyer F12 à une autre
     * application que Deadlock.
     */
    if !is_deadlock_foreground() {
        println!("[SPLIT] Teleport preparation deferred: Deadlock is not foreground");

        return;
    }

    match send_prepare_key() {
        Ok(()) => {
            super::cfg::mark_teleports_prepared();

            println!("[SPLIT] Teleport points prepared");
        }

        Err(error) => {
            eprintln!("[SPLIT] Could not prepare teleport points: {error}");
        }
    }
}

fn ensure_teleports_prepared() -> Result<(), String> {
    if !super::cfg::teleports_dirty() {
        return Ok(());
    }

    println!("[SPLIT] Preparing teleport points before load...");

    send_prepare_key()?;

    /*
     * Seulement pour le fallback :
     * laisser Source 2 enregistrer les
     * point_teleport fraîchement créés.
     *
     * En usage normal ce délai ne se produit
     * PAS car les points sont préparés
     * immédiatement après Save/ changements.
     */
    thread::sleep(Duration::from_millis(50));

    super::cfg::mark_teleports_prepared();

    Ok(())
}

pub fn prepare_teleports_from_ui() -> Result<(), String> {
    /*
     * Rien à préparer :
     * l'action est déjà terminée.
     */
    if !super::cfg::teleports_dirty() {
        return Ok(());
    }

    /*
     * Le clic vient de SPLIT.
     * F13 doit impérativement être reçu
     * par Deadlock et non par l'UI.
     */
    focus_deadlock_window()?;

    /*
     * Petit délai pour laisser Windows
     * terminer le changement de focus.
     */
    thread::sleep(Duration::from_millis(75));

    ensure_teleports_prepared()?;

    println!("[SPLIT] Teleport preparation requested from UI");

    Ok(())
}

pub fn resume_presentation_from_ui() -> Result<(), String> {
    /*
     * Le bouton n'est normalement visible
     * que lorsque le masque est actif.
     *
     * Si l'état a déjà été corrigé entre-temps,
     * inutile d'envoyer quoi que ce soit.
     */
    if !PRESENTATION_MASK_ACTIVE.load(Ordering::SeqCst) {
        return Ok(());
    }

    /*
     * Le F10 interne doit arriver dans Deadlock.
     */
    focus_deadlock_window()?;

    thread::sleep(Duration::from_millis(75));

    /*
     * F10 est bindé dans savestate.cfg à :
     *
     *     r_force_no_present 0
     */
    send_present_resume_key()?;

    PRESENTATION_MASK_ACTIVE.store(false, Ordering::SeqCst);

    println!("[SPLIT] Presentation resumed from diagnostic UI");

    Ok(())
}

fn send_load_key(slot: u8) -> Result<(), String> {
    let vk = load_transport_vk(slot).ok_or_else(|| format!("Invalid load slot {slot}"))?;

    send_virtual_key(vk)
}

pub(crate) fn queue_prime_after_save(slot: u8) {
    let Some(sender) = HOTKEY_SENDER.get() else {
        eprintln!("[SPLIT] Could not queue prime for slot {slot}: hotkey worker unavailable");
        return;
    };

    if let Err(error) = sender.send(HotkeyAction::Prime(slot)) {
        eprintln!("[SPLIT] Could not queue prime for slot {slot}: {error}");
    }
}

fn prime_active_slot(slot: u8) -> Result<bool, String> {
    let (_favorite, snapshot) = super::active_slot_state(slot)?;

    let Some(_snapshot) = snapshot else {
        return Ok(false);
    };

    /*
     * Le point_teleport doit être créé
     * avant le Prime exactement comme pour
     * un Load normal.
     */
    ensure_teleports_prepared()?;

    /*
     * Le CFG du slot commence toujours par :
     *
     *     r_force_no_present 1
     *
     * puis :
     *
     *     TeleportEntity
     *     setang_exact
     *
     * On conserve volontairement ce CFG :
     * les vrais Loads ne changent donc PAS.
     */
    PRESENTATION_MASK_ACTIVE.store(true, Ordering::SeqCst);

    if let Err(error) = send_load_key(slot) {
        return Err(format!(
            "Could not prime slot {slot}: {error}. Press F10 if Deadlock is frozen."
        ));
    }

    /*
     * Contrairement à un vrai Load,
     * on ne garde PAS l'écran figé pendant 35 ms.
     *
     * Les inputs sont envoyés dans cet ordre :
     *
     *     U/I/O/J...
     *     puis F10
     *
     * Deadlock exécute donc le TP puis reçoit
     * immédiatement r_force_no_present 0.
     */
    if let Err(error) = send_present_resume_key() {
        return Err(format!(
            "Could not resume Deadlock presentation after prime: {error}. Press F10 manually."
        ));
    }

    PRESENTATION_MASK_ACTIVE.store(false, Ordering::SeqCst);

    /*
     * Le modèle a toujours besoin d'environ
     * deux frames pour terminer son état interne.
     *
     * Cette fois les 35 ms passent écran visible,
     * puisque le joueur vient juste de sauvegarder
     * exactement cette position/orientation.
     */
    thread::sleep(Duration::from_millis(35));

    Ok(true)
}

fn load_active_slot(slot: u8, show_notification: bool) -> Result<bool, String> {
    let (favorite, snapshot) = super::active_slot_state(slot)?;

    let Some(snapshot) = snapshot else {
        if show_notification {
            crate::notifications::show(crate::notifications::Notification::SlotEmpty {
                slot,
                favorite,
            });
        }

        return Ok(false);
    };

    /*
     * Le point_teleport doit déjà exister
     * AVANT d'exécuter le CFG du slot.
     */
    ensure_teleports_prepared()?;

    /*
     * IMPORTANT :
     *
     * Ne PAS restaurer la caméra avant le Load.
     *
     * Une écriture caméra avant que Deadlock exécute
     * r_force_no_present 1 peut laisser passer quelques
     * frames avec :
     *
     *     ancienne position + caméra sauvegardée
     *
     * ce qui provoque un flash visuel aléatoire.
     *
     * La caméra sera restaurée plus bas, pendant que
     * la présentation est déjà masquée.
     */
    PRESENTATION_MASK_ACTIVE.store(true, Ordering::SeqCst);

    /*
     * Tout ce qui se déroule pendant
     * r_force_no_present est contenu ici.
     *
     * Si une opération Rust devient fatale,
     * on sort du bloc puis on tente
     * automatiquement de réactiver l'affichage.
     */
    let masked_result = (|| -> Result<(), String> {
        /*
         * savestate_slot_X.cfg fait :
         *
         * r_force_no_present 1
         * ent_fire <point> TeleportEntity !player
         * setang_exact ...
         */
        send_load_key(slot).map_err(|error| format!("Could not load slot {slot}: {error}"))?;

        /*
         * point_teleport conserve le momentum.
         *
         * Deadlock applique brièvement son modifier Root :
         *
         *     modifier_citadel_root
         *
         * Le Root remet la vélocité à zéro immédiatement,
         * puis il est retiré dans la même commande.
         *
         * IMPORTANT :
         * prime_active_slot() n'exécute pas ce transport.
         * Un Save en mouvement ne stoppe donc pas le joueur.
         */
        if let Err(error) = send_momentum_reset_key() {
            eprintln!("[SPLIT] Could not reset momentum after Load: {error}");
        }

        thread::sleep(Duration::from_millis(35));

        /*
         * Deuxième restore caméra juste avant
         * de rendre le jeu visible.
         */
        if let Some(camera) = snapshot.camera {
            match super::camera::restore(camera) {
                Ok(()) => {
                    println!(
                        "[SPLIT] Camera final restore -> P={:.3} Y={:.3} R={:.3}",
                        camera.pitch, camera.yaw, camera.roll,
                    );
                }

                Err(error) => {
                    eprintln!("[SPLIT] Camera final restore unavailable: {error}");
                }
            }
        }

        send_present_resume_key()
            .map_err(|error| format!("Could not resume Deadlock presentation: {error}"))?;

        Ok(())
    })();

    /*
     * IMPORTANT :
     *
     * Si le bloc masqué a échoué avant son F10 normal,
     * SPLIT tente immédiatement un F10 de récupération.
     *
     * Si ce deuxième envoi échoue lui aussi,
     * PRESENTATION_MASK_ACTIVE reste à true :
     * F10 physique conserve donc son rôle
     * de touche de secours.
     */
    if let Err(error) = masked_result {
        eprintln!("[SPLIT] Load failed while presentation was masked: {error}");

        match send_present_resume_key() {
            Ok(()) => {
                PRESENTATION_MASK_ACTIVE.store(false, Ordering::SeqCst);

                eprintln!("[SPLIT] Presentation automatically recovered after Load failure");

                return Err(format!(
                    "{error}. Deadlock presentation was automatically restored."
                ));
            }

            Err(recovery_error) => {
                return Err(format!(
                    "{error}. Automatic presentation recovery failed: \
                     {recovery_error}. Press F10 manually."
                ));
            }
        }
    }

    PRESENTATION_MASK_ACTIVE.store(false, Ordering::SeqCst);

    /*
     * Le Prime automatique après un Save
     * ne doit afficher aucune notification "Loaded".
     */
    if show_notification {
        crate::notifications::show(crate::notifications::Notification::SlotLoaded {
            slot,
            favorite,
        });
    }

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

    if load_active_slot(slot, true)? {
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

    /* Keep this classification before every user-hotkey dispatch. */
    let injected_by_split = keyboard.dwExtraInfo == SPLIT_INJECT_TAG;
    if injected_by_split {
        return CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam);
    }

    let should_check_foreground = key_down
        && modifier_for_vk(keyboard.vkCode as u16).is_none()
        && vk_to_key(keyboard.vkCode as u16).is_some();
    let deadlock_foreground = should_check_foreground && is_deadlock_foreground();
    let settings = runtime_settings()
        .read()
        .unwrap_or_else(|error| error.into_inner());
    let mut hook_engine = HOOK_ENGINE
        .lock()
        .unwrap_or_else(|error| error.into_inner());

    if key_down && modifier_for_vk(keyboard.vkCode as u16).is_none() {
        let ctrl_down = physical_key_is_down(VK_CONTROL);
        let alt_down = physical_key_is_down(VK_MENU);
        let shift_down = physical_key_is_down(VK_SHIFT);

        if let Some((tracked, physical)) =
            hook_engine.reconcile_modifiers(ctrl_down, alt_down, shift_down)
        {
            println!(
                "[SPLIT][HOTKEY] Modifier state resynced: \
tracked ctrl={} alt={} shift={} \
physical ctrl={} alt={} shift={}",
                tracked.ctrl,
                tracked.alt,
                tracked.shift,
                physical.ctrl,
                physical.alt,
                physical.shift,
            );
        }
    }

    let decision = hook_engine.classify(
            keyboard.vkCode as u16,
            key_down,
            key_up,
            false,
            deadlock_foreground,
            PRESENTATION_MASK_ACTIVE.load(Ordering::SeqCst),
            &settings,
        );

    drop(hook_engine);

    match decision {
        HookDecision::Pass => CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam),
        HookDecision::Consume => 1,
        HookDecision::EmergencyF10 => {
            PRESENTATION_MASK_ACTIVE.store(false, Ordering::SeqCst);
            println!("[SPLIT] Emergency presentation resume via F10");
            CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
        }
        HookDecision::Trigger(action, hotkey) => {
            if let Some(sender) = HOTKEY_SENDER.get() {
                let _ = sender.send(HotkeyAction::User { action, hotkey });
            }
            1
        }
    }
}

fn start_inner(app: AppHandle) -> Result<(), String> {
    let _ = runtime_settings();
    let (tx, rx) = mpsc::channel::<HotkeyAction>();
    let quick_access_app = app.clone();

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
            loop {
                let action = match rx.recv() {
                    Ok(action) => action,
                    Err(_) => break,
                };

                if let HotkeyAction::User {
                    action: _,
                    hotkey,
                } = &action
                {
                    if !wait_for_hotkey_release(hotkey) {
                        eprintln!(
                            "[SPLIT] {} cancelled: physical shortcut was held too long",
                            hotkey.display()
                        );

                        continue;
                    }

                    if !is_deadlock_foreground() {
                        println!(
                            "[SPLIT] {} cancelled: Deadlock lost focus",
                            hotkey.display()
                        );

                        continue;
                    }
                }
                match action {
                    /*
                     * ALT + F1-F8
                     *
                     * Capture la position,
                     * puis watcher.rs la sauvegarde.
                     */
                    HotkeyAction::User {
                        action: UserHotkeyAction::Save(slot),
                        hotkey,
                    } => {
                        println!("[SPLIT] Save hotkey: {}", hotkey.display());

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
                    HotkeyAction::User {
                        action: UserHotkeyAction::Load(slot),
                        hotkey,
                    } => {
                        println!("[SPLIT] Load hotkey: {}", hotkey.display());

                        if !is_deadlock_foreground() {
                            println!("[SPLIT] Load {slot} cancelled: Deadlock lost focus");

                            continue;
                        }

                        if let Err(error) = load_active_slot(slot, true) {
                            eprintln!("[SPLIT] Could not load slot {slot}: {error}");
                        }
                    }

                    HotkeyAction::Prime(slot) => {
                        println!("[SPLIT] Priming freshly saved teleport: slot {slot}");

                        if !is_deadlock_foreground() {
                            println!("[SPLIT] Prime {slot} skipped: Deadlock lost focus");

                            continue;
                        }

                        /*
                         * Laisse savestate_prepare créer/enregistrer
                         * correctement le nouveau point_teleport.
                         *
                         * 20 ms suffit normalement à laisser passer
                         * au moins une frame sans retarder le Prime
                         * assez pour provoquer un snap perceptible.
                         */
                        thread::sleep(Duration::from_millis(20));

                        /*
                         * Prime spécial :
                         *
                         * contrairement à un vrai Load,
                         * l'affichage est réactivé immédiatement.
                         */
                        match prime_active_slot(slot) {
                            Ok(true) => {
                                println!("[SPLIT] Fresh teleport primed: slot {slot}");
                            }

                            Ok(false) => {
                                println!("[SPLIT] Prime {slot} skipped: slot unexpectedly empty");
                            }

                            Err(error) => {
                                eprintln!("[SPLIT] Could not prime slot {slot}: {error}");
                            }
                        }
                    }

                    HotkeyAction::User {
                        action: UserHotkeyAction::CyclePreset,
                        hotkey,
                    } => {
                        println!("[SPLIT] Preset hotkey: {}", hotkey.display());

                        match super::cycle_active_preset() {
                            Ok(Some((preset, saved_slots))) => {
                                println!("[SPLIT] Preset switched to {preset}");

                                prepare_teleports_after_cfg_update();

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

                    HotkeyAction::User {
                        action: UserHotkeyAction::Undo,
                        hotkey,
                    } => {
                        println!("[SPLIT] Undo hotkey: {}", hotkey.display());

                        match super::undo_last_action() {
                            Ok(result) => {
                                super::emit_history_operation(&app, &result);

                                prepare_teleports_after_cfg_update();
                            }

                            Err(error) => {
                                eprintln!("[SPLIT] Could not undo: {error}");
                            }
                        }
                    }

                    HotkeyAction::User {
                        action: UserHotkeyAction::Redo,
                        hotkey,
                    } => {
                        println!("[SPLIT] Redo hotkey: {}", hotkey.display());

                        match super::redo_last_action() {
                            Ok(result) => {
                                super::emit_history_operation(&app, &result);

                                prepare_teleports_after_cfg_update();
                            }

                            Err(error) => {
                                eprintln!("[SPLIT] Could not redo: {error}");
                            }
                        }
                    }

                    HotkeyAction::User {
                        action: UserHotkeyAction::ToggleFavorites,
                        hotkey,
                    } => {
                        println!(
                            "[SPLIT] Favorite Mode hotkey: {}",
                            hotkey.display()
                        );

                        match super::toggle_favorite_mode() {
                            Ok(result) => {
                                super::emit_active_bank(&app, &result);

                                prepare_teleports_after_cfg_update();
                            }

                            Err(error) => {
                                eprintln!(
                                    "[SPLIT] Could not toggle Favorite Mode: {error}"
                                );
                            }
                        }
                    }


                    HotkeyAction::Shutdown => {
                        break;
                    }
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
                let error = format!(
                    "Windows failed to install the WH_KEYBOARD_LL hook: {}",
                    std::io::Error::last_os_error(),
                );

                eprintln!("[SPLIT] {error}");

                set_hotkey_error(error);

                HOTKEYS_RUNNING.store(false, Ordering::SeqCst);
                HOOK_THREAD_ID.store(0, Ordering::SeqCst);

                return;
            }

            /*
             * Le hook est réellement installé :
             * l'ancienne erreur n'est plus pertinente.
             */
            clear_hotkey_error();
            HOTKEYS_RUNNING.store(true, Ordering::SeqCst);

            println!("[SPLIT] Deadlock hotkeys active:");

            println!("[SPLIT]   Save = Alt+F1-F8");

            println!("[SPLIT]   Load = F1-F8");

            let mut message = MSG::default();

            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);

                DispatchMessageW(&message);
            }

            let _ = UnhookWindowsHookEx(hook);

            HOTKEYS_RUNNING.store(false, Ordering::SeqCst);
            HOOK_THREAD_ID.store(0, Ordering::SeqCst);
        })
        .map_err(|error| format!("Could not start keyboard hook: {error}"))?;

    let quick_access = match spawn_quick_access_hotkey_service(
        quick_access_app,
    ) {
        Ok(service) => service,
        Err(error) => {
            if let Some(sender) = HOTKEY_SENDER.get() {
                let _ = sender.send(HotkeyAction::Shutdown);
            }

            while HOOK_THREAD_ID.load(Ordering::SeqCst) == 0
                && !hook.is_finished()
            {
                thread::sleep(Duration::from_millis(1));
            }

            let hook_thread_id = HOOK_THREAD_ID.load(Ordering::SeqCst);
            if hook_thread_id != 0 {
                unsafe {
                    PostThreadMessageW(hook_thread_id, WM_QUIT, 0, 0);
                }
            }

            let _ = worker.join();
            let _ = hook.join();
            return Err(error);
        }
    };

    *HOTKEY_RUNTIME
        .lock()
        .map_err(|_| "Hotkey runtime lock poisoned".to_string())? =
        Some(HotkeyRuntime {
            worker,
            hook,
            quick_access,
        });

    Ok(())
}

pub fn start(app: AppHandle) -> Result<(), String> {
    /*
     * Nouvelle tentative de démarrage :
     * on efface une éventuelle erreur ancienne.
     */
    clear_hotkey_error();

    match start_inner(app) {
        Ok(()) => Ok(()),

        Err(error) => {
            set_hotkey_error(error.clone());

            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classify_down(engine: &mut HookEngine, vk: u16, settings: &HotkeySettings) -> HookDecision {
        engine.classify(vk, true, false, false, true, false, settings)
    }

    #[test]
    fn defaults_are_exact_and_match_load_save_actions() {
        let settings = HotkeySettings::default();
        for index in 0..8 {
            assert_eq!(
                settings.load_slots[index],
                Hotkey::new(format!("F{}", index + 1), false, false, false)
            );
            assert_eq!(
                settings.save_slots[index],
                Hotkey::new(format!("F{}", index + 1), false, true, false)
            );
        }
        assert_eq!(settings.undo, Hotkey::new("F9", false, false, false));
        assert_eq!(settings.redo, Hotkey::new("F10", false, false, false));
        assert_eq!(settings.cycle_preset, Hotkey::new("V", false, false, false));
        assert_eq!(
            settings.favorite_mode,
            Hotkey::new("F11", false, false, false)
        );

        let mut load = HookEngine::default();
        assert!(matches!(
            classify_down(&mut load, VK_F1, &settings),
            HookDecision::Trigger(UserHotkeyAction::Load(1), _)
        ));

        let mut save = HookEngine::default();
        assert_eq!(
            classify_down(&mut save, VK_LMENU, &settings),
            HookDecision::Pass
        );
        assert!(matches!(
            classify_down(&mut save, VK_F1, &settings),
            HookDecision::Trigger(UserHotkeyAction::Save(1), _)
        ));
    }

    #[test]
    fn matching_requires_the_exact_modifiers() {
        let settings = HotkeySettings::default();
        let mut alt = HookEngine::default();
        classify_down(&mut alt, VK_LMENU, &settings);
        assert!(!matches!(
            classify_down(&mut alt, VK_F1, &settings),
            HookDecision::Trigger(UserHotkeyAction::Load(1), _)
        ));

        let mut ctrl = HookEngine::default();
        classify_down(&mut ctrl, VK_LCONTROL, &settings);
        assert_eq!(
            classify_down(&mut ctrl, VK_F1, &settings),
            HookDecision::Pass
        );
    }

    #[test]
    fn physical_modifier_reconciliation_recovers_stale_hotkeys() {
        let settings = HotkeySettings::default();
        let mut engine = HookEngine::default();

        classify_down(&mut engine, VK_LMENU, &settings);
        assert!(engine.modifier_state().alt);
        assert!(engine
            .reconcile_modifiers(false, false, false)
            .is_some());

        assert!(matches!(
            classify_down(&mut engine, VK_F1, &settings),
            HookDecision::Trigger(UserHotkeyAction::Load(1), _)
        ));
        assert!(matches!(
            classify_down(&mut engine, b'V' as u16, &settings),
            HookDecision::Trigger(UserHotkeyAction::CyclePreset, _)
        ));
        assert!(matches!(
            classify_down(&mut engine, VK_F9, &settings),
            HookDecision::Trigger(UserHotkeyAction::Undo, _)
        ));
    }

    #[test]
    fn physical_modifier_reconciliation_preserves_real_alt_shortcuts() {
        let settings = HotkeySettings::default();
        let mut engine = HookEngine::default();

        assert!(engine
            .reconcile_modifiers(false, true, false)
            .is_some());
        assert!(matches!(
            classify_down(&mut engine, VK_F1, &settings),
            HookDecision::Trigger(UserHotkeyAction::Save(1), _)
        ));
    }

    #[test]
    fn hotkeys_pass_through_outside_deadlock() {
        let settings = HotkeySettings::default();
        let mut engine = HookEngine::default();
        assert_eq!(
            engine.classify(VK_F1, true, false, false, false, false, &settings),
            HookDecision::Pass
        );
        assert_eq!(
            engine.classify(VK_F1, false, true, false, false, false, &settings),
            HookDecision::Pass
        );
    }

    #[test]
    fn duplicate_bindings_are_rejected() {
        let mut settings = HotkeySettings::default();
        settings.undo = Hotkey::new("Z", true, false, false);
        settings.favorite_mode = Hotkey::new("Z", true, false, false);
        assert_eq!(
            settings.normalized().unwrap_err(),
            "Ctrl+Z is already assigned to Undo."
        );
    }

    #[test]
    fn consumed_keyup_is_tied_to_its_keydown_after_modifier_release() {
        let mut settings = HotkeySettings::default();
        settings.undo = Hotkey::new("Z", true, false, false);
        let settings = settings.normalized().unwrap();
        let mut engine = HookEngine::default();
        classify_down(&mut engine, VK_LCONTROL, &settings);
        assert!(matches!(
            classify_down(&mut engine, b'Z' as u16, &settings),
            HookDecision::Trigger(UserHotkeyAction::Undo, _)
        ));
        assert_eq!(
            engine.classify(VK_LCONTROL, false, true, false, true, false, &settings),
            HookDecision::Pass
        );
        assert_eq!(
            engine.classify(b'Z' as u16, false, true, false, true, false, &settings),
            HookDecision::Consume
        );
    }

    #[test]
    fn function_keys_f9_through_f12_are_accepted() {
        for key in ["F9", "F10", "F11", "F12"] {
            assert!(validate_hotkey(&Hotkey::new(key, false, false, false), false).is_ok());
            assert!(key_to_vk(key).is_some());
        }
    }

    #[test]
    fn historical_alt_f4_default_is_preserved_but_cannot_be_newly_assigned() {
        let defaults = HotkeySettings::default().normalized().unwrap();
        let mut previous = defaults.clone();
        previous.save_slots[3] = Hotkey::new("Z", false, true, false);
        assert!(defaults.validate_update_from(&previous).is_err());
    }

    #[test]
    fn internal_transport_keys_are_reserved_with_or_without_modifiers() {
        for key in ["H", "U", "I", "O", "J", "K", "L", "N", "M"] {
            assert!(
                validate_hotkey(&Hotkey::new(key, false, false, false), false).is_err(),
                "{key} should be reserved"
            );

            assert!(
                validate_hotkey(&Hotkey::new(key, true, false, false), false).is_err(),
                "Ctrl+{key} should be reserved"
            );

            assert!(
                validate_hotkey(&Hotkey::new(key, false, true, false), false).is_err(),
                "Alt+{key} should be reserved"
            );

            assert!(
                validate_hotkey(&Hotkey::new(key, false, false, true), false).is_err(),
                "Shift+{key} should be reserved"
            );

            assert!(
                validate_hotkey(&Hotkey::new(key, true, true, true), false).is_err(),
                "Ctrl+Alt+Shift+{key} should be reserved"
            );
        }
    }

    #[test]
    fn modifiers_meta_and_internal_function_keys_are_rejected() {
        assert!(normalize_key("Control").is_err());
        assert!(normalize_key("Alt").is_err());
        assert!(normalize_key("Shift").is_err());
        assert!(normalize_key("Meta").is_err());
        assert!(normalize_key("F13").is_err());
        assert!(normalize_key("F14").is_err());
    }

    #[test]
    fn consumed_f10_cannot_turn_into_emergency_until_released() {
        let mut settings = HotkeySettings::default();
        settings.load_slots[0] = Hotkey::new("F10", false, false, false);
        settings.redo = Hotkey::new("Z", true, false, false);
        let settings = settings.normalized().unwrap();
        let mut engine = HookEngine::default();

        assert!(matches!(
            engine.classify(VK_F10, true, false, false, true, false, &settings),
            HookDecision::Trigger(UserHotkeyAction::Load(1), _)
        ));
        assert_eq!(
            engine.classify(VK_F10, true, false, false, true, true, &settings),
            HookDecision::Consume
        );
        assert_eq!(
            engine.classify(VK_F10, false, true, false, true, true, &settings),
            HookDecision::Consume
        );
        assert_eq!(
            engine.classify(VK_F10, true, false, false, true, true, &settings),
            HookDecision::EmergencyF10
        );
        assert_eq!(
            engine.classify(VK_F10, false, true, false, true, true, &settings),
            HookDecision::Pass
        );
    }

    #[test]
    fn split_injected_input_never_dispatches() {
        let settings = HotkeySettings::default();
        let mut engine = HookEngine::default();
        assert_eq!(
            engine.classify(VK_F9, true, false, true, true, false, &settings),
            HookDecision::Pass
        );
    }

    #[test]
    fn quick_access_keys_bypass_the_low_level_hook() {
        let settings = HotkeySettings::default();
        let mut engine = HookEngine::default();

        for vk in [VK_CAPITAL, VK_ESCAPE] {
            assert_eq!(
                engine.classify(vk, true, false, false, true, false, &settings),
                HookDecision::Pass
            );
            assert_eq!(
                engine.classify(vk, false, true, false, true, false, &settings),
                HookDecision::Pass
            );
            assert!(!engine.down_keys.contains(&vk));
            assert!(!engine.consumed_keys.contains(&vk));
        }
    }

    #[test]
    fn default_quick_access_setting_maps_to_caps_lock_system_hotkey() {
        let registration = quick_access_registration(
            &HotkeySettings::default().quick_access,
        )
        .unwrap();

        assert_eq!(registration.vk, VK_CAPITAL as u32);
        assert_eq!(registration.modifiers, MOD_NOREPEAT);
    }

    #[test]
    fn runtime_quick_access_shortcut_maps_modifiers_and_main_key() {
        let registration = quick_access_registration(
            &Hotkey::new("Q", true, true, true),
        )
        .unwrap();

        assert_eq!(registration.vk, b'Q' as u32);
        assert_eq!(
            registration.modifiers,
            MOD_CONTROL |
                MOD_ALT |
                MOD_SHIFT |
                MOD_NOREPEAT,
        );
    }

    #[test]
    fn quick_access_hotkeys_follow_foreground_and_visibility() {
        assert_eq!(
            quick_access_registration_plan(
                QuickAccessForeground::Deadlock,
                false,
                false,
                true,
            ),
            QuickAccessRegistrationPlan {
                caps_lock: true,
                escape: false,
            },
        );
        assert_eq!(
            quick_access_registration_plan(
                QuickAccessForeground::Deadlock,
                false,
                true,
                true,
            ),
            QuickAccessRegistrationPlan {
                caps_lock: true,
                escape: true,
            },
        );
        assert_eq!(
            quick_access_registration_plan(
                QuickAccessForeground::QuickAccess,
                false,
                true,
                true,
            ),
            QuickAccessRegistrationPlan {
                caps_lock: true,
                escape: true,
            },
        );
        assert_eq!(
            quick_access_registration_plan(
                QuickAccessForeground::Other,
                false,
                true,
                true,
            ),
            QuickAccessRegistrationPlan {
                caps_lock: false,
                escape: false,
            },
        );
    }

    #[test]
    fn quick_access_auto_hide_only_applies_to_visible_other_foreground() {
        assert_eq!(
            quick_access_auto_hide_mode(
                QuickAccessForeground::Deadlock,
                true,
            ),
            None,
        );
        assert_eq!(
            quick_access_auto_hide_mode(
                QuickAccessForeground::QuickAccess,
                true,
            ),
            None,
        );
        assert_eq!(
            quick_access_auto_hide_mode(
                QuickAccessForeground::Other,
                false,
            ),
            None,
        );
        assert_eq!(
            quick_access_auto_hide_mode(
                QuickAccessForeground::Other,
                true,
            ),
            Some(QuickAccessMode::Hidden),
        );
    }

    #[test]
    fn current_foreground_wins_over_a_stale_event_hint() {
        let _stale_other_hint = QuickAccessForeground::Other;
        let current_deadlock = quick_access_registration_plan(
            QuickAccessForeground::Deadlock,
            false,
            false,
            true,
        );
        assert_eq!(
            current_deadlock,
            QuickAccessRegistrationPlan {
                caps_lock: true,
                escape: false,
            },
        );

        let _stale_deadlock_hint = QuickAccessForeground::Deadlock;
        let current_other = quick_access_registration_plan(
            QuickAccessForeground::Other,
            false,
            true,
            true,
        );
        assert_eq!(
            current_other,
            QuickAccessRegistrationPlan {
                caps_lock: false,
                escape: false,
            },
        );
    }

    #[test]
    fn post_auto_hide_plan_uses_the_second_foreground_read() {
        assert_eq!(
            quick_access_auto_hide_mode(
                QuickAccessForeground::Other,
                true,
            ),
            Some(QuickAccessMode::Hidden),
        );

        let returned_to_deadlock = quick_access_registration_plan(
            QuickAccessForeground::Deadlock,
            false,
            false,
            true,
        );
        assert_eq!(
            returned_to_deadlock,
            QuickAccessRegistrationPlan {
                caps_lock: true,
                escape: false,
            },
        );

        let remained_elsewhere = quick_access_registration_plan(
            QuickAccessForeground::Other,
            false,
            false,
            true,
        );
        assert_eq!(
            remained_elsewhere,
            QuickAccessRegistrationPlan {
                caps_lock: false,
                escape: false,
            },
        );
    }

    #[test]
    fn text_input_suspends_quick_access_hotkeys() {
        assert_eq!(
            quick_access_registration_plan(
                QuickAccessForeground::QuickAccess,
                true,
                true,
                true,
            ),
            QuickAccessRegistrationPlan {
                caps_lock: false,
                escape: false,
            },
        );
    }

    #[test]
    fn disabled_quick_access_registers_no_hotkeys() {
        assert_eq!(
            quick_access_registration_plan(
                QuickAccessForeground::Deadlock,
                false,
                true,
                false,
            ),
            QuickAccessRegistrationPlan {
                caps_lock: false,
                escape: false,
            },
        );
    }

    #[test]
    fn hidden_caps_lock_release_becomes_passive() {
        assert_eq!(
            caps_lock_release_transition(QuickAccessMode::Hidden, Duration::ZERO),
            QuickAccessMode::Passive
        );
    }

    #[test]
    fn passive_short_caps_lock_becomes_hidden() {
        assert_eq!(
            caps_lock_release_transition(
                QuickAccessMode::Passive,
                QUICK_ACCESS_HOLD_DURATION - Duration::from_millis(1),
            ),
            QuickAccessMode::Hidden
        );
    }

    #[test]
    fn passive_long_caps_lock_becomes_interactive() {
        assert_eq!(
            caps_lock_release_transition(
                QuickAccessMode::Passive,
                QUICK_ACCESS_HOLD_DURATION,
            ),
            QuickAccessMode::Interactive
        );
    }

    #[test]
    fn interactive_caps_lock_becomes_passive() {
        assert_eq!(
            caps_lock_press_transition(QuickAccessMode::Interactive),
            QuickAccessMode::Passive
        );
    }

    #[test]
    fn interactive_escape_becomes_passive() {
        assert_eq!(
            escape_transition(QuickAccessMode::Interactive),
            QuickAccessMode::Passive
        );
    }

    #[test]
    fn passive_escape_becomes_hidden() {
        assert_eq!(
            escape_transition(QuickAccessMode::Passive),
            QuickAccessMode::Hidden
        );
    }

}
