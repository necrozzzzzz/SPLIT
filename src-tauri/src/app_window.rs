use std::{
    fs,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

use tauri_plugin_notification::NotificationExt;

use tauri::{AppHandle, Manager, WebviewWindowBuilder};

static EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);
static WINDOW_OPERATION_LOCK: Mutex<()> = Mutex::new(());

pub fn exit_requested() -> bool {
    EXIT_REQUESTED.load(Ordering::SeqCst)
}

pub fn open_main_window(app: AppHandle) -> Result<(), String> {
    std::thread::Builder::new()
        .name("split-window-open".to_string())
        .spawn(move || {
            let Ok(_operation) = WINDOW_OPERATION_LOCK.lock() else {
                eprintln!("[SPLIT] Window operation lock poisoned");
                return;
            };
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
                return;
            }

            let Some(config) = app
                .config()
                .app
                .windows
                .iter()
                .find(|config| config.label == "main")
                .cloned()
            else {
                eprintln!("[SPLIT] Main window configuration is missing");
                return;
            };

            match WebviewWindowBuilder::from_config(&app, &config)
                .and_then(|builder| builder.build())
            {
                Ok(window) => {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
                Err(error) => eprintln!("[SPLIT] Could not recreate main window: {error}"),
            }
        })
        .map_err(|error| format!("Could not start main window task: {error}"))?;

    Ok(())
}

fn start_minimized_marker(
    app: &AppHandle,
) -> Result<std::path::PathBuf, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| {
            format!(
                "Could not resolve app data directory: {error}"
            )
        })?;

    Ok(
        directory.join(
            "start-minimized-to-tray",
        ),
    )
}


pub fn start_minimized_to_tray_enabled(
    app: &AppHandle,
) -> bool {
    start_minimized_marker(app)
        .map(|path| path.exists())
        .unwrap_or(false)
}


pub fn set_start_minimized_to_tray(
    app: &AppHandle,
    enabled: bool,
) -> Result<bool, String> {
    let marker =
        start_minimized_marker(app)?;

    if enabled {
        if let Some(parent) =
            marker.parent()
        {
            fs::create_dir_all(parent)
                .map_err(|error| {
                    format!(
                        "Could not create app data directory: {error}"
                    )
                })?;
        }

        fs::write(
            &marker,
            b"enabled",
        )
        .map_err(|error| {
            format!(
                "Could not save minimized startup setting: {error}"
            )
        })?;
    } else if marker.exists() {
        fs::remove_file(
            &marker,
        )
        .map_err(|error| {
            format!(
                "Could not remove minimized startup setting: {error}"
            )
        })?;
    }

    Ok(enabled)
}


fn close_to_tray_marker(
    app: &AppHandle,
) -> Result<std::path::PathBuf, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| {
            format!(
                "Could not resolve app data directory: {error}"
            )
        })?;

    Ok(
        directory.join(
            "close-button-quits",
        ),
    )
}


pub fn close_to_tray_enabled(
    app: &AppHandle,
) -> bool {
    close_to_tray_marker(app)
        .map(|path| !path.exists())
        .unwrap_or(true)
}


pub fn set_close_to_tray(
    app: &AppHandle,
    enabled: bool,
) -> Result<bool, String> {
    let marker =
        close_to_tray_marker(app)?;

    if enabled {
        if marker.exists() {
            fs::remove_file(&marker)
                .map_err(|error| {
                    format!(
                        "Could not save close behavior: {error}"
                    )
                })?;
        }
    } else {
        if let Some(parent) =
            marker.parent()
        {
            fs::create_dir_all(parent)
                .map_err(|error| {
                    format!(
                        "Could not create app data directory: {error}"
                    )
                })?;
        }

        fs::write(
            &marker,
            b"quit",
        )
        .map_err(|error| {
            format!(
                "Could not save close behavior: {error}"
            )
        })?;
    }

    Ok(enabled)
}

pub fn show_background_notice_once(
    app: &AppHandle,
) {
    let marker = match app
        .path()
        .app_data_dir()
    {
        Ok(directory) => {
            directory.join(
                "tray-close-notice-shown",
            )
        }

        Err(error) => {
            eprintln!(
                "[SPLIT] Could not resolve app data directory: {error}"
            );
            return;
        }
    };

    if marker.exists() {
        return;
    }

    if let Err(error) = app
        .notification()
        .builder()
        .title("SPLIT is still running")
        .body(
            "SPLIT continues running in the background. \
             Use the tray icon to reopen or quit.",
        )
        .show()
    {
        eprintln!(
            "[SPLIT] Could not show background notification: {error}"
        );

        return;
    }

    if let Some(parent) = marker.parent() {
        if let Err(error) =
            fs::create_dir_all(parent)
        {
            eprintln!(
                "[SPLIT] Could not create notification state directory: {error}"
            );

            return;
        }
    }

    if let Err(error) =
        fs::write(&marker, b"shown")
    {
        eprintln!(
            "[SPLIT] Could not save background notification state: {error}"
        );
    }
}

pub fn close_main_window_to_background(app: AppHandle) -> Result<(), String> {
    std::thread::Builder::new()
        .name("split-window-close".to_string())
        .spawn(move || {
            let Ok(_operation) = WINDOW_OPERATION_LOCK.lock() else {
                eprintln!("[SPLIT] Window operation lock poisoned");
                return;
            };
            if let Some(window) = app.get_webview_window("main") {
                if let Err(error) = window.destroy() {
                    eprintln!("[SPLIT] Could not destroy main window: {error}");
                    let _ = window.hide();
                }
            }
        })
        .map_err(|error| format!("Could not start main window close task: {error}"))?;

    Ok(())
}

pub fn reset_main_window(
    app: &AppHandle,
) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| {
            "Main window is unavailable".to_string()
        })?;

    window
        .set_size(
            tauri::LogicalSize::new(
                980.0,
                680.0,
            ),
        )
        .map_err(|error| {
            format!(
                "Could not reset window size: {error}"
            )
        })?;

    window
        .center()
        .map_err(|error| {
            format!(
                "Could not center window: {error}"
            )
        })?;

    Ok(())
}

pub fn request_true_quit(app: &AppHandle) {
    EXIT_REQUESTED.store(true, Ordering::SeqCst);
    if let Err(error) = crate::notifications::stop() {
        eprintln!("[SPLIT] Could not stop native notifications: {error}");
    }
    crate::deadlock::shutdown_background_services();
    app.exit(0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_runtime_state_is_not_a_true_quit() {
        EXIT_REQUESTED.store(false, Ordering::SeqCst);
        assert!(!exit_requested());
        EXIT_REQUESTED.store(true, Ordering::SeqCst);
        assert!(exit_requested());
        EXIT_REQUESTED.store(false, Ordering::SeqCst);
    }
}
