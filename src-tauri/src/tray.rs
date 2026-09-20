use tauri::{
    menu::{
        MenuBuilder,
        MenuItemBuilder,
    },
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App,
};

const OPEN_ID: &str = "open-split";

const LAUNCH_DEADLOCK_ID: &str =
    "launch-deadlock";

const QUIT_ID: &str = "quit-split";

const DEADLOCK_STATUS_ID: &str =
    "deadlock-status";

pub fn setup(app: &App) -> Result<(), String> {


    let deadlock_running =
        crate::deadlock::is_deadlock_running();

    let status_item =
        MenuItemBuilder::with_id(
            DEADLOCK_STATUS_ID,
            if deadlock_running {
                "Deadlock: Connected"
            } else {
                "Deadlock: Not running"
            },
        )
        .enabled(false)
        .build(app)
        .map_err(|error| {
            format!(
                "Could not create Deadlock tray status: {error}"
            )
        })?;

    let launch_item =
        MenuItemBuilder::with_id(
            LAUNCH_DEADLOCK_ID,
            "Launch Deadlock",
        )
        .enabled(!deadlock_running)
        .build(app)
        .map_err(|error| {
            format!(
                "Could not create Deadlock tray action: {error}"
            )
        })?;

    let menu = MenuBuilder::new(app)
        .item(&status_item)
        .separator()
        .text(
            OPEN_ID,
            "Open SPLIT",
        )
        .item(&launch_item)
        .separator()
        .text(
            QUIT_ID,
            "Quit SPLIT",
        )
        .build()
        .map_err(|error| {
            format!(
                "Could not build tray menu: {error}"
            )
        })?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or_else(|| "SPLIT tray icon is unavailable".to_string())?;


    let status_item_for_tray =
        status_item.clone();

    let launch_item_for_tray =
        launch_item.clone();    



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
            LAUNCH_DEADLOCK_ID => {
                if let Err(error) = crate::deadlock::launch_deadlock() {
                    eprintln!("[SPLIT] Could not launch Deadlock from tray: {error}");
                }
            }
            QUIT_ID => crate::app_window::request_true_quit(app),
            _ => {}
        })
        .on_tray_icon_event(
            move |tray, event| {
                match event {
                    TrayIconEvent::Click {
                        button:
                            MouseButton::Right,
                        button_state:
                            MouseButtonState::Up,
                        ..
                    } => {
                        let running =
                            crate::deadlock::
                                is_deadlock_running();

                        let _ =
                            status_item_for_tray
                                .set_text(
                                    if running {
                                        "Deadlock: Connected"
                                    } else {
                                        "Deadlock: Not running"
                                    },
                                );

                        let _ =
                            launch_item_for_tray
                                .set_enabled(
                                    !running,
                                );
                    }

                    TrayIconEvent::Click {
                        button:
                            MouseButton::Left,
                        button_state:
                            MouseButtonState::Up,
                        ..
                    } => {
                        if let Err(error) =
                            crate::app_window::
                                open_main_window(
                                    tray
                                        .app_handle()
                                        .clone(),
                                )
                        {
                            eprintln!(
                                "[SPLIT] Could not open window from tray click: {error}"
                            );
                        }
                    }

                    _ => {}
                }
            },
        )
        .build(app)
        .map_err(|error| format!("Could not create tray icon: {error}"))?;

    Ok(())
}
