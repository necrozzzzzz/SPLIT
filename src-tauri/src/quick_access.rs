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

use serde::Serialize;

use std::sync::atomic::{
    AtomicBool,
    Ordering,
};

use windows_sys::Win32::{
    Foundation::RECT,
    UI::WindowsAndMessaging::GetWindowRect,
};

const QUICK_ACCESS_LABEL: &str =
    "quick-access";

const QUICK_ACCESS_WIDTH: u32 = 390;
const QUICK_ACCESS_MARGIN: i32 = 14;

static QUICK_ACCESS_VISIBLE: AtomicBool =
    AtomicBool::new(false);
static QUICK_ACCESS_INTERACTIVE: AtomicBool =
    AtomicBool::new(false);

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


#[cfg(test)]
pub fn set_visible_for_test(visible: bool) {
    QUICK_ACCESS_VISIBLE.store(
        visible,
        Ordering::SeqCst,
    );
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
    let hwnd =
        crate::deadlock::foreground_deadlock_window()
            .ok_or_else(|| {
                "Deadlock is not in the foreground"
                    .to_string()
            })?;

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };

    let success =
        unsafe {
            GetWindowRect(
                hwnd,
                &mut rect,
            )
        };

    if success == 0 {
        return Err(format!(
            "Could not read Deadlock window bounds: {}",
            std::io::Error::last_os_error(),
        ));
    }

    Ok(rect)
}


fn position_window(
    window: &WebviewWindow,
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
                QUICK_ACCESS_WIDTH,
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
                rect.left +
                    QUICK_ACCESS_MARGIN,
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


pub fn hide(
    app: &AppHandle,
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
    println!("[SPLIT][QA] interaction=false");

    QUICK_ACCESS_VISIBLE.store(
        false,
        Ordering::SeqCst,
    );

    match crate::deadlock::focus_deadlock_window() {
        Ok(()) => println!(
            "[SPLIT][QA] focus returned to Deadlock"
        ),
        Err(error) => eprintln!(
            "[SPLIT][QA] Could not return focus to Deadlock: {error}"
        ),
    }

    Ok(())
}


pub fn enter_interaction_mode(
    app: &AppHandle,
) -> Result<(), String> {
    println!("[SPLIT][QA] enter_interaction_mode()");

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

    QUICK_ACCESS_INTERACTIVE.store(
        true,
        Ordering::SeqCst,
    );

    emit_interaction_mode(
        &window,
        true,
    );
    println!("[SPLIT][QA] interaction=true");

    let focus_window = window.clone();

    if let Err(error) = window.run_on_main_thread(
        move || {
            if let Err(error) = focus_window.set_focus() {
                let _ = focus_window.set_focusable(false);

                QUICK_ACCESS_INTERACTIVE.store(
                    false,
                    Ordering::SeqCst,
                );

                emit_interaction_mode(
                    &focus_window,
                    false,
                );

                eprintln!(
                    "[SPLIT][QA] Could not focus Quick Access: {error}"
                );
            }
        },
    ) {
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
            "Could not schedule Quick Access focus: {error}"
        ));
    }

    Ok(())
}


pub fn exit_interaction_mode(
    app: &AppHandle,
) -> Result<(), String> {
    println!("[SPLIT][QA] exit_interaction_mode()");

    let mut error = None;

    QUICK_ACCESS_INTERACTIVE.store(
        false,
        Ordering::SeqCst,
    );

    if let Some(window) =
        app.get_webview_window(
            QUICK_ACCESS_LABEL,
        )
    {
        if let Err(reason) =
            window.set_focusable(false)
        {
            error = Some(format!(
                "Could not make Quick Access passive: {reason}"
            ));
        }

        emit_interaction_mode(
            &window,
            false,
        );
    }

    println!("[SPLIT][QA] interaction=false");

    match crate::deadlock::focus_deadlock_window() {
        Ok(()) => println!(
            "[SPLIT][QA] focus returned to Deadlock"
        ),
        Err(reason) => {
            if error.is_none() {
                error = Some(reason);
            }
        }
    }

    match error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}


pub fn toggle(
    app: &AppHandle,
) -> Result<(), String> {
    if let Some(window) =
        app.get_webview_window(
            QUICK_ACCESS_LABEL,
        )
    {
        if window
            .is_visible()
            .unwrap_or(false)
        {
            return hide(app);
        }
    }

    show(app)
}
