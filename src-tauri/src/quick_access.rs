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

use windows_sys::Win32::Foundation::{HWND, RECT};

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

    WebviewWindowBuilder::new(
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
    })
}


pub fn show(
    app: &AppHandle,
) -> Result<(), String> {
    if !is_enabled() {
        return Ok(());
    }

    let window =
        get_or_create(app)?;

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

    window
        .show()
        .map_err(|error| {
            format!(
                "Could not show Quick Access: {error}"
            )
        })?;

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

    window
        .set_focusable(true)
        .map_err(|error| {
            format!(
                "Could not make Quick Access interactive: {error}"
            )
        })?;

    if let Err(error) = window.set_focus() {
        let _ = window.set_focusable(false);

        QUICK_ACCESS_INTERACTIVE.store(
            false,
            Ordering::SeqCst,
        );

        emit_interaction_mode(
            &window,
            false,
        );

        return Err(format!(
            "Could not focus Quick Access: {error}"
        ));
    }

    let mut focused = false;

    for _ in 0..15 {
        if window
            .is_focused()
            .unwrap_or(false)
        {
            focused = true;
            break;
        }

        thread::sleep(
            Duration::from_millis(10),
        );
    }

    if focused {
        println!(
            "[SPLIT][QA] Quick Access focus acquired"
        );
    } else {
        eprintln!(
            "[SPLIT][QA] Quick Access focus request did not become foreground"
        );
    }

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

}
