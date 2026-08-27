use tauri::{
    menu::MenuBuilder,
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App,
};

const OPEN_ID: &str = "open-split";
const QUIT_ID: &str = "quit-split";

pub fn setup(app: &App) -> Result<(), String> {
    let menu = MenuBuilder::new(app)
        .text(OPEN_ID, "Open SPLIT")
        .text(QUIT_ID, "Quit SPLIT")
        .build()
        .map_err(|error| format!("Could not build tray menu: {error}"))?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| "SPLIT tray icon is unavailable".to_string())?;

    TrayIconBuilder::with_id("split-tray")
        .icon(icon)
        .tooltip("SPLIT")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            OPEN_ID => {
                if let Err(error) = crate::app_window::open_main_window(app.clone()) {
                    eprintln!("[SPLIT] Could not open window from tray: {error}");
                }
            }
            QUIT_ID => crate::app_window::request_true_quit(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                if let Err(error) = crate::app_window::open_main_window(tray.app_handle().clone()) {
                    eprintln!("[SPLIT] Could not open window from tray click: {error}");
                }
            }
        })
        .build(app)
        .map_err(|error| format!("Could not create tray icon: {error}"))?;

    Ok(())
}
