use tauri::{
    AppHandle,
    Emitter,
    Manager,
    PhysicalPosition,
    PhysicalSize,
    WebviewUrl,
    WebviewWindow,
    WebviewWindowBuilder,
};

use serde::{Deserialize, Serialize};

use std::sync::atomic::{
    AtomicBool,
    Ordering,
};
use std::sync::{OnceLock, RwLock};
use std::{
    thread,
    time::Duration,
};

use windows_sys::Win32::{
    Foundation::{
        GetLastError,
        SetLastError,
        ERROR_SUCCESS,
        HWND,
        RECT,
    },
    UI::WindowsAndMessaging::{
        BringWindowToTop,
        GetWindowLongPtrW,
        GetForegroundWindow,
        SetForegroundWindow,
        SetWindowLongPtrW,
        SetWindowPos,
        GWL_EXSTYLE,
        GWLP_HWNDPARENT,
        SWP_FRAMECHANGED,
        SWP_NOACTIVATE,
        SWP_NOMOVE,
        SWP_NOSIZE,
        SWP_NOZORDER,
        SWP_SHOWWINDOW,
        WS_EX_APPWINDOW,
        WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW,
    },
};

const QUICK_ACCESS_LABEL: &str =
    "quick-access";

const QUICK_ACCESS_WIDTH: u32 = 390;
const QUICK_ACCESS_VIEWER_MIN_WIDTH: u32 = 760;
const QUICK_ACCESS_VIEWER_MAX_WIDTH: u32 = 1100;
const QUICK_ACCESS_MARGIN: i32 = 14;

static QUICK_ACCESS_VISIBLE: AtomicBool =
    AtomicBool::new(false);
static QUICK_ACCESS_INTERACTIVE: AtomicBool =
    AtomicBool::new(false);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QuickAccessWindowMode {
    Passive,
    Interactive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum QuickAccessPosition {
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QuickAccessSettings {
    pub enabled: bool,
    pub position: QuickAccessPosition,
}

impl Default for QuickAccessSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            position: QuickAccessPosition::Left,
        }
    }
}

static QUICK_ACCESS_SETTINGS: OnceLock<RwLock<QuickAccessSettings>> =
    OnceLock::new();

fn runtime_settings() -> &'static RwLock<QuickAccessSettings> {
    QUICK_ACCESS_SETTINGS.get_or_init(|| {
        RwLock::new(
            crate::deadlock::load_quick_access_settings(),
        )
    })
}

pub fn settings() -> QuickAccessSettings {
    *runtime_settings()
        .read()
        .unwrap_or_else(|error| error.into_inner())
}

pub fn apply_settings(settings: QuickAccessSettings) {
    *runtime_settings()
        .write()
        .unwrap_or_else(|error| error.into_inner()) = settings;
}

pub fn is_enabled() -> bool {
    settings().enabled
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QuickAccessState {
    visible: bool,
    interactive: bool,
}


pub fn is_visible() -> bool {
    QUICK_ACCESS_VISIBLE.load(Ordering::SeqCst)
}


pub fn is_interactive() -> bool {
    QUICK_ACCESS_INTERACTIVE.load(Ordering::SeqCst)
}


pub fn state() -> QuickAccessState {
    QuickAccessState {
        visible: is_visible(),
        interactive: is_interactive(),
    }
}


fn emit_interaction_mode(
    window: &WebviewWindow,
    active: bool,
) {
    if let Err(error) = window.emit(
        "quick-access-interaction",
        active,
    ) {
        eprintln!(
            "[SPLIT][QA] Could not emit interaction={active}: {error}"
        );
    }
}


fn deadlock_rect() -> Result<RECT, String> {
    crate::deadlock::deadlock_window_rect()
}


pub fn owns_window_handle(
    app: &AppHandle,
    hwnd: HWND,
) -> bool {
    app.get_webview_window(QUICK_ACCESS_LABEL)
        .and_then(|window| window.hwnd().ok())
        .is_some_and(|quick_access_hwnd| {
            quick_access_hwnd.0 == hwnd
        })
}


fn window_x(
    rect: &RECT,
    width: u32,
    position: QuickAccessPosition,
) -> i32 {
    match position {
        QuickAccessPosition::Left =>
            rect.left + QUICK_ACCESS_MARGIN,
        QuickAccessPosition::Right =>
            rect.right - width as i32 - QUICK_ACCESS_MARGIN,
    }
}


fn position_window(
    window: &WebviewWindow,
    width: u32,
) -> Result<(), String> {
    let rect =
        deadlock_rect()?;

    let game_height =
        rect.bottom - rect.top;

    let height =
        (game_height -
            QUICK_ACCESS_MARGIN * 2)
            .max(500) as u32;

    window
        .set_size(
            PhysicalSize::new(
                width,
                height,
            ),
        )
        .map_err(|error| {
            format!(
                "Could not size Quick Access: {error}"
            )
        })?;

    window
        .set_position(
            PhysicalPosition::new(
                window_x(
                    &rect,
                    width,
                    settings().position,
                ),
                rect.top +
                    QUICK_ACCESS_MARGIN,
            ),
        )
        .map_err(|error| {
            format!(
                "Could not position Quick Access: {error}"
            )
        })?;

    Ok(())
}


fn quick_access_ex_style(
    current: u32,
    mode: QuickAccessWindowMode,
) -> u32 {
    let base =
        (current | WS_EX_TOOLWINDOW) &
            !WS_EX_APPWINDOW;

    match mode {
        QuickAccessWindowMode::Passive =>
            base | WS_EX_NOACTIVATE,
        QuickAccessWindowMode::Interactive =>
            base & !WS_EX_NOACTIVATE,
    }
}


fn read_window_ex_style(
    hwnd: HWND,
) -> Result<u32, String> {
    unsafe {
        SetLastError(ERROR_SUCCESS);
    }
    let current = unsafe {
        GetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
        )
    };
    let read_error = unsafe { GetLastError() };

    if current == 0 && read_error != ERROR_SUCCESS {
        return Err(format!(
            "Could not read Quick Access extended styles: {}",
            std::io::Error::from_raw_os_error(read_error as i32),
        ));
    }

    Ok(current as u32)
}


fn apply_window_mode_style(
    window: &WebviewWindow,
    mode: QuickAccessWindowMode,
) -> Result<(), String> {
    let hwnd = window
        .hwnd()
        .map_err(|error| {
            format!(
                "Could not read Quick Access window handle: {error}"
            )
        })?
        .0;

    let current = read_window_ex_style(hwnd)?;

    let next = quick_access_ex_style(
        current,
        mode,
    );

    if next != current {
        unsafe {
            SetLastError(ERROR_SUCCESS);
        }
        let previous = unsafe {
            SetWindowLongPtrW(
                hwnd,
                GWL_EXSTYLE,
                next as isize,
            )
        };
        let write_error = unsafe { GetLastError() };

        if previous == 0 && write_error != ERROR_SUCCESS {
            return Err(format!(
                "Could not update Quick Access extended styles: {}",
                std::io::Error::from_raw_os_error(write_error as i32),
            ));
        }

        if unsafe {
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED |
                    SWP_NOMOVE |
                    SWP_NOSIZE |
                    SWP_NOZORDER |
                    SWP_NOACTIVATE,
            )
        } == 0
        {
            return Err(format!(
                "Could not refresh Quick Access window frame: {}",
                std::io::Error::last_os_error(),
            ));
        }

        println!(
            "[SPLIT][QA] Applied {:?} window styles",
            mode,
        );
    }

    Ok(())
}


fn show_without_activation(
    window: &WebviewWindow,
) -> Result<(), String> {
    let hwnd = window
        .hwnd()
        .map_err(|error| {
            format!(
                "Could not read Quick Access window handle: {error}"
            )
        })?
        .0;

    if unsafe {
        SetWindowPos(
            hwnd,
            std::ptr::null_mut(),
            0,
            0,
            0,
            0,
            SWP_SHOWWINDOW |
                SWP_NOACTIVATE |
                SWP_NOMOVE |
                SWP_NOSIZE |
                SWP_NOZORDER,
        )
    } == 0
    {
        return Err(format!(
            "Could not show Quick Access without activation: {}",
            std::io::Error::last_os_error(),
        ));
    }

    Ok(())
}


fn set_quick_access_owner(
    quick_access_hwnd: HWND,
    owner_hwnd: Option<HWND>,
) -> Result<(), String> {
    let owner = owner_hwnd.unwrap_or(std::ptr::null_mut());

    unsafe {
        SetLastError(ERROR_SUCCESS);
    }
    let previous = unsafe {
        SetWindowLongPtrW(
            quick_access_hwnd,
            GWLP_HWNDPARENT,
            owner as isize,
        )
    };
    let error = unsafe { GetLastError() };

    if previous == 0 && error != ERROR_SUCCESS {
        return Err(format!(
            "Could not {} Quick Access owner: {}",
            if owner.is_null() {
                "detach"
            } else {
                "attach"
            },
            std::io::Error::from_raw_os_error(error as i32),
        ));
    }

    if previous != owner as isize {
        println!(
            "[SPLIT][QA] owner {}",
            if owner.is_null() {
                "detached"
            } else {
                "attached to Deadlock"
            },
        );
    }

    Ok(())
}


fn get_or_create(
    app: &AppHandle,
) -> Result<WebviewWindow, String> {
    if let Some(window) =
        app.get_webview_window(
            QUICK_ACCESS_LABEL,
        )
    {
        return Ok(window);
    }

    let window = WebviewWindowBuilder::new(
        app,
        QUICK_ACCESS_LABEL,
        WebviewUrl::App(
            "index.html?quick-access=1"
                .into(),
        ),
    )
    .title("SPLIT Quick Access")
    .inner_size(
        QUICK_ACCESS_WIDTH as f64,
        720.0,
    )
    .decorations(false)
    .resizable(false)
    .maximizable(false)
    .minimizable(false)
    .always_on_top(true)
    .focusable(false)
    .skip_taskbar(true)
    .visible(false)
    .build()
    .map_err(|error| {
        format!(
            "Could not create Quick Access window: {error}"
        )
    })?;

    Ok(window)
}


pub fn show(
    app: &AppHandle,
) -> Result<(), String> {
    if !is_enabled() {
        return Ok(());
    }

    let owner_hwnd =
        crate::deadlock::foreground_deadlock_window()
            .ok_or_else(|| {
                "Could not find the foreground Deadlock window"
                    .to_string()
            })?;
    let window =
        get_or_create(app)?;
    let quick_access_hwnd = window
        .hwnd()
        .map_err(|error| {
            format!(
                "Could not read Quick Access window handle: {error}"
            )
        })?
        .0;
    let foreground_before = unsafe {
        GetForegroundWindow()
    };

    println!(
        "[SPLIT][QA] Passive show foreground before: {}",
        if foreground_before == owner_hwnd {
            "Deadlock"
        } else if foreground_before == quick_access_hwnd {
            "Quick Access"
        } else {
            "Other"
        },
    );

    set_quick_access_owner(
        quick_access_hwnd,
        Some(owner_hwnd),
    )?;

    apply_window_mode_style(
        &window,
        QuickAccessWindowMode::Passive,
    )?;

    window
        .set_focusable(false)
        .map_err(|error| {
            format!(
                "Could not make Quick Access passive: {error}"
            )
        })?;

    QUICK_ACCESS_INTERACTIVE.store(
        false,
        Ordering::SeqCst,
    );

    emit_interaction_mode(
        &window,
        false,
    );

    position_window(
        &window,
        QUICK_ACCESS_WIDTH,
    )?;

    show_without_activation(&window)?;

    let foreground_after = unsafe {
        GetForegroundWindow()
    };

    println!(
        "[SPLIT][QA] Passive show foreground after: {}",
        if foreground_after == owner_hwnd {
            "Deadlock"
        } else if foreground_after == quick_access_hwnd {
            "Quick Access"
        } else {
            "Other"
        },
    );

    if foreground_after != foreground_before {
        eprintln!(
            "[SPLIT][QA] Passive show changed foreground: before={foreground_before:p}, after={foreground_after:p}"
        );
    }

    QUICK_ACCESS_VISIBLE.store(
        true,
        Ordering::SeqCst,
    );

    /*
     * Au premier affichage React charge
     * automatiquement les données.
     *
     * Aux affichages suivants, cet event
     * force un refresh des slots.
     */
    let _ =
        window.emit(
            "quick-access-refresh",
            (),
        );

    Ok(())
}


fn hide_internal(
    app: &AppHandle,
    restore_deadlock_focus: bool,
) -> Result<(), String> {
    if let Some(window) =
        app.get_webview_window(
            QUICK_ACCESS_LABEL,
        )
    {
        window
            .set_focusable(false)
            .map_err(|error| {
                format!(
                    "Could not make Quick Access passive: {error}"
                )
            })?;

        apply_window_mode_style(
            &window,
            QuickAccessWindowMode::Passive,
        )?;

        QUICK_ACCESS_INTERACTIVE.store(
            false,
            Ordering::SeqCst,
        );

        emit_interaction_mode(
            &window,
            false,
        );

        window
            .hide()
            .map_err(|error| {
                format!(
                    "Could not hide Quick Access: {error}"
                )
            })?;

        match window.hwnd() {
            Ok(hwnd) => {
                if let Err(error) =
                    set_quick_access_owner(
                        hwnd.0,
                        None,
                    )
                {
                    eprintln!(
                        "[SPLIT][QA] {error}"
                    );
                }
            }
            Err(error) => eprintln!(
                "[SPLIT][QA] Could not read Quick Access window handle while detaching owner: {error}"
            ),
        }
    }

    QUICK_ACCESS_INTERACTIVE.store(
        false,
        Ordering::SeqCst,
    );
    QUICK_ACCESS_VISIBLE.store(
        false,
        Ordering::SeqCst,
    );

    if restore_deadlock_focus {
        if let Err(error) = crate::deadlock::focus_deadlock_window() {
            eprintln!(
                "[SPLIT][QA] Could not return focus to Deadlock: {error}"
            );
        }
    }

    Ok(())
}


pub fn hide(
    app: &AppHandle,
) -> Result<(), String> {
    hide_internal(app, true)
}


pub(crate) fn hide_without_focus(
    app: &AppHandle,
) -> Result<(), String> {
    hide_internal(app, false)
}


pub fn set_viewer_open(
    app: &AppHandle,
    open: bool,
) -> Result<(), String> {
    let window = get_or_create(app)?;
    let rect = deadlock_rect()?;
    let width = if open {
        let game_width =
            (rect.right - rect.left).max(0) as f64;

        (game_width * 0.58)
            .round()
            .clamp(
                QUICK_ACCESS_VIEWER_MIN_WIDTH as f64,
                QUICK_ACCESS_VIEWER_MAX_WIDTH as f64,
            ) as u32
    } else {
        QUICK_ACCESS_WIDTH
    };

    let height = window
        .inner_size()
        .map_err(|error| {
            format!(
                "Could not read Quick Access size: {error}"
            )
        })?
        .height;

    window
        .set_size(PhysicalSize::new(width, height))
        .map_err(|error| {
            format!(
                "Could not resize Quick Access viewer: {error}"
            )
        })?;

    window
        .set_position(PhysicalPosition::new(
            window_x(
                &rect,
                width,
                settings().position,
            ),
            rect.top + QUICK_ACCESS_MARGIN,
        ))
        .map_err(|error| {
            format!(
                "Could not position Quick Access viewer: {error}"
            )
        })
}


pub fn reposition_if_visible(
    app: &AppHandle,
) -> Result<(), String> {
    if !is_visible() {
        return Ok(());
    }

    let window = app
        .get_webview_window(QUICK_ACCESS_LABEL)
        .ok_or_else(|| {
            "Quick Access window does not exist"
                .to_string()
        })?;
    let size = window
        .inner_size()
        .map_err(|error| {
            format!(
                "Could not read Quick Access size: {error}"
            )
        })?;
    let rect = deadlock_rect()?;

    window
        .set_position(PhysicalPosition::new(
            window_x(
                &rect,
                size.width,
                settings().position,
            ),
            rect.top + QUICK_ACCESS_MARGIN,
        ))
        .map_err(|error| {
            format!(
                "Could not reposition Quick Access: {error}"
            )
        })
}


fn quick_access_has_focus(
    window: &WebviewWindow,
    hwnd: HWND,
) -> bool {
    (unsafe { GetForegroundWindow() }) == hwnd &&
        window.is_focused().unwrap_or(false)
}


fn wait_for_quick_access_focus(
    window: &WebviewWindow,
    hwnd: HWND,
) -> bool {
    for _ in 0..15 {
        if quick_access_has_focus(window, hwnd) {
            return true;
        }

        thread::sleep(
            Duration::from_millis(10),
        );
    }

    quick_access_has_focus(window, hwnd)
}


fn rollback_failed_interaction(
    window: &WebviewWindow,
    reason: String,
) -> String {
    let focusable_error = window
        .set_focusable(false)
        .err()
        .map(|error| error.to_string());
    let style_error = apply_window_mode_style(
        window,
        QuickAccessWindowMode::Passive,
    )
    .err();

    QUICK_ACCESS_INTERACTIVE.store(
        false,
        Ordering::SeqCst,
    );
    emit_interaction_mode(
        window,
        false,
    );

    let mut rollback_errors = Vec::new();
    if let Some(error) = focusable_error {
        rollback_errors.push(format!(
            "could not make Quick Access non-focusable: {error}"
        ));
    }
    if let Some(error) = style_error {
        rollback_errors.push(format!(
            "could not restore passive styles: {error}"
        ));
    }

    if rollback_errors.is_empty() {
        reason
    } else {
        format!(
            "{reason}; passive rollback failed: {}",
            rollback_errors.join("; "),
        )
    }
}


pub fn enter_interaction_mode(
    app: &AppHandle,
) -> Result<(), String> {
    let window = app
        .get_webview_window(
            QUICK_ACCESS_LABEL,
        )
        .ok_or_else(|| {
            "Quick Access window does not exist"
                .to_string()
        })?;

    if !window
        .is_visible()
        .unwrap_or(false)
    {
        let _ = window.set_focusable(false);

        QUICK_ACCESS_INTERACTIVE.store(
            false,
            Ordering::SeqCst,
        );

        emit_interaction_mode(
            &window,
            false,
        );

        QUICK_ACCESS_VISIBLE.store(
            false,
            Ordering::SeqCst,
        );

        return Err(
            "Quick Access is not visible"
                .to_string(),
        );
    }

    if let Err(error) = window.set_focusable(true) {
        return Err(rollback_failed_interaction(
            &window,
            format!(
                "Could not make Quick Access interactive: {error}"
            ),
        ));
    }

    if let Err(error) = apply_window_mode_style(
        &window,
        QuickAccessWindowMode::Interactive,
    ) {
        return Err(rollback_failed_interaction(
            &window,
            format!(
                "Could not apply Quick Access interactive styles: {error}"
            ),
        ));
    }

    let quick_access_hwnd = window
        .hwnd()
        .map_err(|error| {
            rollback_failed_interaction(
                &window,
                format!(
                    "Could not read Quick Access window handle: {error}"
                ),
            )
        })?
        .0;
    let interactive_style = read_window_ex_style(
        quick_access_hwnd,
    )
    .map_err(|error| {
        rollback_failed_interaction(
            &window,
            error,
        )
    })?;

    if interactive_style & WS_EX_NOACTIVATE != 0 {
        eprintln!(
            "[SPLIT][QA] Interactive style still had WS_EX_NOACTIVATE"
        );
        return Err(rollback_failed_interaction(
            &window,
            "Interactive Quick Access still had WS_EX_NOACTIVATE"
                .to_string(),
        ));
    }

    let first_focus_error = window
        .set_focus()
        .err()
        .map(|error| error.to_string());
    let mut focused = wait_for_quick_access_focus(
        &window,
        quick_access_hwnd,
    );

    let mut fallback_details = None;

    if !focused {
        let brought_to_top = unsafe {
            BringWindowToTop(quick_access_hwnd)
        } != 0;
        let set_foreground = unsafe {
            SetForegroundWindow(quick_access_hwnd)
        } != 0;
        let retry_focus_error = window
            .set_focus()
            .err()
            .map(|error| error.to_string());

        fallback_details = Some(format!(
            "firstFocusError={first_focus_error:?}, BringWindowToTop={brought_to_top}, SetForegroundWindow={set_foreground}, retryFocusError={retry_focus_error:?}"
        ));
        focused = wait_for_quick_access_focus(
            &window,
            quick_access_hwnd,
        );
    }

    if !focused {
        eprintln!(
            "[SPLIT][QA] Quick Access focus acquisition failed"
        );
        return Err(rollback_failed_interaction(
            &window,
            format!(
                "Quick Access focus acquisition failed ({})",
                fallback_details.unwrap_or_else(|| {
                    format!(
                        "firstFocusError={first_focus_error:?}"
                    )
                }),
            ),
        ));
    }

    println!(
        "[SPLIT][QA] Quick Access focus acquired"
    );

    QUICK_ACCESS_INTERACTIVE.store(
        true,
        Ordering::SeqCst,
    );

    emit_interaction_mode(
        &window,
        true,
    );
    Ok(())
}


pub fn exit_interaction_mode(
    app: &AppHandle,
) -> Result<(), String> {
    let window = app
        .get_webview_window(
            QUICK_ACCESS_LABEL,
        )
        .ok_or_else(|| {
            "Quick Access window does not exist"
                .to_string()
        })?;

    if let Err(error) =
        crate::deadlock::focus_deadlock_window()
    {
        eprintln!(
            "[SPLIT][QA] Could not transfer focus to Deadlock: {error}"
        );

        if let Err(restore_error) = window.set_focus() {
            eprintln!(
                "[SPLIT][QA] Could not restore Quick Access focus after failed Deadlock activation: {restore_error}"
            );
        }

        return Err(format!(
            "Could not transfer focus to Deadlock: {error}"
        ));
    }

    if let Err(error) = window.set_focusable(false) {
        if let Err(restore_error) = window.set_focusable(true) {
            eprintln!(
                "[SPLIT][QA] Could not restore Quick Access focusability: {restore_error}"
            );
        }

        return Err(format!(
            "Could not make Quick Access passive: {error}"
        ));
    }

    if let Err(error) = apply_window_mode_style(
        &window,
        QuickAccessWindowMode::Passive,
    ) {
        let _ = apply_window_mode_style(
            &window,
            QuickAccessWindowMode::Interactive,
        );

        if let Err(restore_error) = window.set_focusable(true) {
            eprintln!(
                "[SPLIT][QA] Could not restore Quick Access focusability after passive style failure: {restore_error}"
            );
        }

        return Err(format!(
            "Could not restore Quick Access passive window style: {error}"
        ));
    }

    QUICK_ACCESS_INTERACTIVE.store(
        false,
        Ordering::SeqCst,
    );

    emit_interaction_mode(
        &window,
        false,
    );

    Ok(())
}


#[cfg(test)]
mod tests {
    use super::*;

    fn rect() -> RECT {
        RECT {
            left: 100,
            top: 40,
            right: 2020,
            bottom: 1120,
        }
    }

    #[test]
    fn left_position_uses_left_margin() {
        assert_eq!(
            window_x(
                &rect(),
                QUICK_ACCESS_WIDTH,
                QuickAccessPosition::Left,
            ),
            114,
        );
    }

    #[test]
    fn quick_access_defaults_enabled_on_left() {
        assert_eq!(
            QuickAccessSettings::default(),
            QuickAccessSettings {
                enabled: true,
                position: QuickAccessPosition::Left,
            },
        );
    }

    #[test]
    fn right_position_uses_width_and_right_margin() {
        assert_eq!(
            window_x(
                &rect(),
                QUICK_ACCESS_WIDTH,
                QuickAccessPosition::Right,
            ),
            1616,
        );
    }

    #[test]
    fn right_viewer_expands_toward_deadlock_interior() {
        assert_eq!(
            window_x(
                &rect(),
                1100,
                QuickAccessPosition::Right,
            ),
            906,
        );
    }

    #[test]
    fn passive_style_preserves_other_extended_styles() {
        let preserved_style = 0x0000_0008;
        let current =
            preserved_style |
                WS_EX_APPWINDOW;
        let updated =
            quick_access_ex_style(
                current,
                QuickAccessWindowMode::Passive,
            );

        assert_ne!(updated & WS_EX_TOOLWINDOW, 0);
        assert_eq!(updated & WS_EX_APPWINDOW, 0);
        assert_ne!(updated & WS_EX_NOACTIVATE, 0);
        assert_ne!(updated & preserved_style, 0);
        assert_eq!(
            quick_access_ex_style(
                updated,
                QuickAccessWindowMode::Passive,
            ),
            updated,
        );
    }

    #[test]
    fn interactive_style_removes_noactivate_and_is_idempotent() {
        let preserved_style = 0x0000_0008;
        let current =
            preserved_style |
                WS_EX_APPWINDOW |
                WS_EX_NOACTIVATE;
        let updated =
            quick_access_ex_style(
                current,
                QuickAccessWindowMode::Interactive,
            );

        assert_ne!(updated & WS_EX_TOOLWINDOW, 0);
        assert_eq!(updated & WS_EX_APPWINDOW, 0);
        assert_eq!(updated & WS_EX_NOACTIVATE, 0);
        assert_ne!(updated & preserved_style, 0);
        assert_eq!(
            quick_access_ex_style(
                updated,
                QuickAccessWindowMode::Interactive,
            ),
            updated,
        );
    }

    #[test]
    fn window_mode_style_round_trip_is_stable() {
        let current = 0x0000_0008;
        let passive = quick_access_ex_style(
            current,
            QuickAccessWindowMode::Passive,
        );
        let interactive = quick_access_ex_style(
            passive,
            QuickAccessWindowMode::Interactive,
        );
        let passive_again = quick_access_ex_style(
            interactive,
            QuickAccessWindowMode::Passive,
        );

        assert_eq!(passive_again, passive);
    }
}
