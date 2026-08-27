use std::{
    mem::size_of,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{
        Input::{
            GetRawInputData,
            KeyboardAndMouse::{
                SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_MOVE,
                MOUSEEVENTF_MOVE_NOCOALESCE, MOUSEINPUT,
            },
            RegisterRawInputDevices, MOUSE_MOVE_ABSOLUTE, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER,
            RIDEV_INPUTSINK, RID_INPUT, RIM_TYPEMOUSE,
        },
        WindowsAndMessaging::{
            CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetForegroundWindow,
            GetMessageW, PostThreadMessageW, RegisterClassW, TranslateMessage, MSG, WM_INPUT,
            WM_QUIT, WNDCLASSW,
        },
    },
};

/*
 * On log seulement après un certain
 * nombre de counts pour ne pas spammer
 * le terminal.
 */
const REPORT_DISTANCE: i64 = 200;

/*
 * Usage Page 0x01 = Generic Desktop
 * Usage      0x02 = Mouse
 */
const HID_USAGE_PAGE_GENERIC: u16 = 0x01;
const HID_USAGE_GENERIC_MOUSE: u16 = 0x02;

/*
 * Classe Win32 de notre petite fenêtre
 * invisible dédiée au Raw Input.
 *
 * "SPLITRawInput\0"
 */
static WINDOW_CLASS_NAME: [u16; 14] = [
    b'S' as u16,
    b'P' as u16,
    b'L' as u16,
    b'I' as u16,
    b'T' as u16,
    b'R' as u16,
    b'a' as u16,
    b'w' as u16,
    b'I' as u16,
    b'n' as u16,
    b'p' as u16,
    b'u' as u16,
    b't' as u16,
    0,
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MouseSnapshot {
    /*
     * Important :
     * les compteurs Raw Input n'ont de sens
     * que dans la session SPLIT qui les
     * a enregistrés.
     */
    pub session_id: u64,

    pub x: i64,
    pub y: i64,
}

struct MouseState {
    /*
     * Compteurs RAW cumulés.
     */
    total_x: i64,
    total_y: i64,

    /*
     * Dernière position imprimée.
     */
    report_x: i64,
    report_y: i64,

    /*
     * Cache de la fenêtre foreground
     * pour ne pas refaire la détection
     * deadlock.exe pour chaque packet.
     */
    foreground_hwnd: usize,
    deadlock_foreground: bool,
}

impl MouseState {
    const fn new() -> Self {
        Self {
            total_x: 0,
            total_y: 0,

            report_x: 0,
            report_y: 0,

            foreground_hwnd: 0,
            deadlock_foreground: false,
        }
    }
}

static STATE: Mutex<MouseState> = Mutex::new(MouseState::new());

static THREAD_ID: AtomicU32 = AtomicU32::new(0);

static SESSION_ID: AtomicU64 = AtomicU64::new(0);

fn current_session_id() -> u64 {
    let existing = SESSION_ID.load(Ordering::SeqCst);

    if existing != 0 {
        return existing;
    }

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;

    let generated = (timestamp ^ ((std::process::id() as u64) << 32)).max(1);

    match SESSION_ID.compare_exchange(0, generated, Ordering::SeqCst, Ordering::SeqCst) {
        Ok(_) => generated,
        Err(existing) => existing,
    }
}

/*
 * Pendant que SPLIT restaure la caméra,
 * on ne veut pas que les mouvements
 * synthétiques contaminent notre compteur.
 */
static INJECTING: AtomicBool = AtomicBool::new(false);

/*
 * Utilisé uniquement pour notre test F12.
 *
 * Premier F12  -> mémorise.
 * Deuxième F12 -> restaure.
 */
static TEST_ANCHOR: Mutex<Option<MouseSnapshot>> = Mutex::new(None);

static RUNTIME: Mutex<Option<JoinHandle<()>>> = Mutex::new(None);

pub fn snapshot() -> MouseSnapshot {
    let session_id = current_session_id();

    let Ok(state) = STATE.lock() else {
        return MouseSnapshot {
            session_id,
            ..MouseSnapshot::default()
        };
    };

    MouseSnapshot {
        session_id,
        x: state.total_x,
        y: state.total_y,
    }
}

fn set_snapshot(snapshot: MouseSnapshot) {
    if let Ok(mut state) = STATE.lock() {
        state.total_x = snapshot.x;

        state.total_y = snapshot.y;

        /*
         * Évite qu'un énorme rapport
         * diagnostic soit imprimé
         * immédiatement après restauration.
         */
        state.report_x = snapshot.x;

        state.report_y = snapshot.y;
    }
}

fn make_mouse_input(dx: i32, dy: i32) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,

        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,

                mouseData: 0,

                /*
                 * NOCOALESCE évite que Windows
                 * fusionne nos petits mouvements
                 * en un seul énorme mouvement.
                 */
                dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_MOVE_NOCOALESCE,

                time: 0,

                dwExtraInfo: 0,
            },
        },
    }
}

fn send_mouse_batch(inputs: &[INPUT]) -> Result<(), String> {
    if inputs.is_empty() {
        return Ok(());
    }

    let sent = unsafe {
        SendInput(
            inputs.len() as u32,
            inputs.as_ptr(),
            size_of::<INPUT>() as i32,
        )
    };

    if sent != inputs.len() as u32 {
        return Err(format!(
            "SendInput mouse correction failed ({sent}/{})",
            inputs.len(),
        ));
    }

    Ok(())
}

pub(super) fn restore_to_snapshot(target: MouseSnapshot) -> Result<(), String> {
    let current = snapshot();

    if target.session_id != current.session_id {
        return Err("Camera snapshot belongs to another SPLIT session".to_string());
    }

    let mut remaining_x = target.x - current.x;

    let mut remaining_y = target.y - current.y;

    println!(
        "[SPLIT] Camera restore: current=({}, {}) target=({}, {}) delta=({}, {})",
        current.x, current.y, target.x, target.y, remaining_x, remaining_y,
    );

    if remaining_x == 0 && remaining_y == 0 {
        println!("[SPLIT] Camera restore: already at target");

        return Ok(());
    }

    /*
     * Notre Python a montré qu'un énorme
     * SendInput unique n'est pas fiable
     * dans Deadlock.
     *
     * On rejoue donc le déplacement en
     * petits morceaux de 25 counts.
     */
    INJECTING.store(true, Ordering::SeqCst);

    let result = (|| -> Result<(), String> {
        let mut inputs = Vec::<INPUT>::new();

        /*
         * Même principe que le test Python
         * qui fonctionnait :
         *
         * PAS un énorme mouvement unique.
         *
         * On construit plein de petits
         * mouvements de maximum 25 counts.
         *
         * Différence :
         * ils seront envoyés tous ensemble
         * à Windows, donc aucune animation
         * lente visible.
         */
        while remaining_x != 0 || remaining_y != 0 {
            let step_x = remaining_x.clamp(-25, 25) as i32;

            let step_y = remaining_y.clamp(-25, 25) as i32;

            inputs.push(make_mouse_input(step_x, step_y));

            remaining_x -= i64::from(step_x);

            remaining_y -= i64::from(step_y);
        }

        println!(
            "[SPLIT] Camera restore batch: {} mouse events",
            inputs.len(),
        );

        send_mouse_batch(&inputs)
    })();

    /*
     * Laisse quelques millisecondes aux
     * derniers WM_INPUT éventuels.
     */
    thread::sleep(Duration::from_millis(5));

    INJECTING.store(false, Ordering::SeqCst);

    if result.is_ok() {
        /*
         * Notre compteur logique représente
         * maintenant à nouveau la visée cible.
         */
        set_snapshot(target);

        println!("[SPLIT] Camera restore completed");
    }

    result
}

/*
 * Diagnostic temporaire.
 *
 * F12 #1 = ancre la caméra.
 * F12 #2 = revient sur cette ancre.
 */
pub(super) fn toggle_test_anchor() -> Result<(), String> {
    let mut anchor = TEST_ANCHOR
        .lock()
        .map_err(|_| "Camera test anchor lock poisoned".to_string())?;

    if let Some(target) = anchor.take() {
        drop(anchor);

        println!("[SPLIT] Camera test: restoring F12 anchor");

        restore_to_snapshot(target)
    } else {
        let current = snapshot();

        *anchor = Some(current);

        println!(
            "[SPLIT] Camera test anchor saved: X={} Y={}",
            current.x, current.y,
        );

        Ok(())
    }
}

unsafe fn process_raw_input(lparam: LPARAM) {
    let mut raw = RAWINPUT::default();

    let mut size = size_of::<RAWINPUT>() as u32;

    let result = GetRawInputData(
        lparam as _,
        RID_INPUT,
        &mut raw as *mut RAWINPUT as *mut _,
        &mut size,
        size_of::<RAWINPUTHEADER>() as u32,
    );

    /*
     * UINT(-1) = erreur.
     * 0 = aucune donnée.
     */
    if result == u32::MAX || result == 0 {
        return;
    }

    if raw.header.dwType != RIM_TYPEMOUSE {
        return;
    }

    let mouse = raw.data.mouse;

    /*
     * Une vraie souris PC nous fournit
     * normalement des deltas relatifs.
     *
     * Les événements absolus ne nous
     * intéressent pas pour la caméra.
     */
    if mouse.usFlags & MOUSE_MOVE_ABSOLUTE != 0 {
        return;
    }

    let dx = mouse.lLastX;

    let dy = mouse.lLastY;

    if dx == 0 && dy == 0 {
        return;
    }

    /*
     * SendInput pourra lui aussi produire
     * des événements souris.
     *
     * Pendant notre correction caméra,
     * on ne les considère pas comme
     * des mouvements physiques utilisateur.
     */
    if INJECTING.load(Ordering::SeqCst) {
        return;
    }

    let foreground = GetForegroundWindow() as usize;

    let Ok(mut state) = STATE.lock() else {
        return;
    };

    /*
     * Détection Deadlock uniquement si
     * la fenêtre foreground a changé.
     */
    if state.foreground_hwnd != foreground {
        let was_deadlock = state.deadlock_foreground;

        state.foreground_hwnd = foreground;

        state.deadlock_foreground = super::foreground_deadlock_window().is_some();

        if state.deadlock_foreground && !was_deadlock {
            println!("[SPLIT] Raw mouse tracker: Deadlock foreground");
        }
    }

    if !state.deadlock_foreground {
        return;
    }

    state.total_x += i64::from(dx);

    state.total_y += i64::from(dy);

    let distance = (state.total_x - state.report_x).abs() + (state.total_y - state.report_y).abs();

    if distance >= REPORT_DISTANCE {
        println!(
            "[SPLIT] Raw mouse -> X={} Y={} (dx={} dy={})",
            state.total_x, state.total_y, dx, dy,
        );

        state.report_x = state.total_x;

        state.report_y = state.total_y;
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    if message == WM_INPUT {
        process_raw_input(lparam);
    }

    DefWindowProcW(hwnd, message, wparam, lparam)
}

unsafe fn create_raw_input_window() -> Result<HWND, String> {
    let instance = GetModuleHandleW(std::ptr::null());

    if instance.is_null() {
        return Err("GetModuleHandleW failed for mouse tracker".to_string());
    }

    let window_class = WNDCLASSW {
        lpfnWndProc: Some(window_proc),

        hInstance: instance,

        lpszClassName: WINDOW_CLASS_NAME.as_ptr(),

        ..Default::default()
    };

    if RegisterClassW(&window_class) == 0 {
        return Err("RegisterClassW failed for mouse tracker".to_string());
    }

    /*
     * Fenêtre totalement invisible.
     *
     * Elle sert uniquement à recevoir
     * les WM_INPUT.
     */
    let hwnd = CreateWindowExW(
        0,
        WINDOW_CLASS_NAME.as_ptr(),
        WINDOW_CLASS_NAME.as_ptr(),
        0,
        0,
        0,
        0,
        0,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        instance,
        std::ptr::null(),
    );

    if hwnd.is_null() {
        return Err("CreateWindowExW failed for mouse tracker".to_string());
    }

    let device = RAWINPUTDEVICE {
        usUsagePage: HID_USAGE_PAGE_GENERIC,

        usUsage: HID_USAGE_GENERIC_MOUSE,

        /*
         * On veut recevoir les packets
         * même si notre fenêtre SPLIT
         * n'est pas foreground.
         *
         * On filtrera ensuite pour ne
         * compter que lorsque Deadlock
         * est foreground.
         */
        dwFlags: RIDEV_INPUTSINK,

        hwndTarget: hwnd,
    };

    let registered = RegisterRawInputDevices(&device, 1, size_of::<RAWINPUTDEVICE>() as u32);

    if registered == 0 {
        let _ = DestroyWindow(hwnd);

        return Err("RegisterRawInputDevices failed for mouse tracker".to_string());
    }

    Ok(hwnd)
}

pub fn start() -> Result<(), String> {
    let mut runtime = RUNTIME
        .lock()
        .map_err(|_| "Mouse tracker runtime lock poisoned".to_string())?;

    if runtime.is_some() {
        return Ok(());
    }

    let thread = thread::Builder::new()
        .name("split-raw-mouse".to_string())
        .spawn(|| unsafe {
            THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);

            let hwnd = match create_raw_input_window() {
                Ok(hwnd) => hwnd,

                Err(error) => {
                    eprintln!("[SPLIT] Raw mouse tracker unavailable: {error}");

                    THREAD_ID.store(0, Ordering::SeqCst);

                    return;
                }
            };

            println!("[SPLIT] Raw mouse tracker active (diagnostic)");

            let mut message = MSG::default();

            while GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) > 0 {
                TranslateMessage(&message);

                DispatchMessageW(&message);
            }

            let _ = DestroyWindow(hwnd);

            THREAD_ID.store(0, Ordering::SeqCst);

            println!("[SPLIT] Raw mouse tracker stopped");
        })
        .map_err(|error| format!("Could not start Raw Input mouse thread: {error}"))?;

    *runtime = Some(thread);

    Ok(())
}

pub fn stop() -> Result<(), String> {
    let runtime = RUNTIME
        .lock()
        .map_err(|_| "Mouse tracker runtime lock poisoned".to_string())?
        .take();

    let Some(runtime) = runtime else {
        return Ok(());
    };

    let thread_id = THREAD_ID.swap(0, Ordering::SeqCst);

    if thread_id != 0 {
        let posted = unsafe { PostThreadMessageW(thread_id, WM_QUIT, 0, 0) };

        if posted == 0 {
            return Err("Could not post WM_QUIT to Raw Input mouse tracker".to_string());
        }
    }

    runtime
        .join()
        .map_err(|_| "Raw Input mouse tracker thread panicked".to_string())?;

    Ok(())
}
