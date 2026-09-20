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

use windows_sys::Win32::{
    Foundation::RECT,
    UI::WindowsAndMessaging::GetWindowRect,
};

const QUICK_ACCESS_LABEL: &str =
    "quick-access";

const QUICK_ACCESS_WIDTH: u32 = 320;
const QUICK_ACCESS_MARGIN: i32 = 14;


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

    window
        .set_focus()
        .map_err(|error| {
            format!(
                "Could not focus Quick Access: {error}"
            )
        })?;

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
            .hide()
            .map_err(|error| {
                format!(
                    "Could not hide Quick Access: {error}"
                )
            })?;
    }

    /*
     * Ne pas faire échouer la fermeture
     * si Deadlock s'est fermé entre-temps.
     */
    if let Err(error) =
        crate::deadlock::
            focus_deadlock_window()
    {
        eprintln!(
            "[SPLIT] Could not restore Deadlock focus after Quick Access: {error}"
        );
    }

    Ok(())
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