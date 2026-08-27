use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};

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
