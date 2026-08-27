mod app_window;
mod deadlock;
mod notifications;
mod storage;
mod tray;
mod ui;

use tauri::Manager;

#[tauri::command]
fn get_deadlock_status() -> deadlock::DeadlockStatus {
    deadlock::get_status()
}

#[tauri::command]
fn get_deadlock_setup() -> deadlock::DeadlockSetupState {
    deadlock::get_setup_state()
}

#[tauri::command]
fn scan_deadlock_path() -> Option<String> {
    deadlock::scan_deadlock_path()
}

#[tauri::command]
fn get_last_position() -> Option<deadlock::PositionSnapshot> {
    deadlock::get_last_position()
}

#[tauri::command]
fn get_slots() -> Result<Vec<Option<deadlock::PositionSnapshot>>, String> {
    deadlock::get_slots()
}

#[tauri::command]
fn get_active_preset() -> Result<u8, String> {
    deadlock::get_active_preset()
}

#[tauri::command]
fn get_history_state() -> Result<deadlock::HistoryState, String> {
    deadlock::get_history_state()
}

#[tauri::command]
fn get_favorite_mode() -> bool {
    deadlock::get_favorite_mode()
}

#[tauri::command]
fn get_notification_settings() -> notifications::NotificationSettings {
    deadlock::get_notification_settings()
}

#[tauri::command]
fn update_notification_settings(
    settings: notifications::NotificationSettings,
) -> Result<notifications::NotificationSettings, String> {
    deadlock::update_notification_settings(settings)
}

#[tauri::command]
fn toggle_favorite_mode() -> Result<deadlock::ActiveBankResult, String> {
    deadlock::toggle_favorite_mode()
}

#[tauri::command]
fn undo_last_action() -> Result<deadlock::HistoryOperationResult, String> {
    deadlock::undo_last_action()
}

#[tauri::command]
fn redo_last_action() -> Result<deadlock::HistoryOperationResult, String> {
    deadlock::redo_last_action()
}

#[tauri::command]
fn set_active_preset(preset: u8) -> Result<Vec<Option<deadlock::PositionSnapshot>>, String> {
    deadlock::set_active_preset(preset)
}

#[tauri::command]
fn save_slot(slot: u8) -> Result<Vec<Option<deadlock::PositionSnapshot>>, String> {
    deadlock::save_slot(slot)
}

#[tauri::command]
fn load_slot(slot: u8) -> Result<(), String> {
    deadlock::load_slot(slot)
}

#[tauri::command]
fn capture_slot(app: tauri::AppHandle, slot: u8) -> Result<(), String> {
    deadlock::capture_slot(app, slot)
}

#[tauri::command]
fn sync_slots_to_deadlock() -> Result<(), String> {
    deadlock::sync_slots_to_deadlock()
}

#[tauri::command]
fn confirm_deadlock_path(
    app: tauri::AppHandle,
    path: String,
) -> Result<deadlock::DeadlockStatus, String> {
    deadlock::confirm_deadlock_path(app, path)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Err(error) = app_window::open_main_window(app.clone()) {
                eprintln!("[SPLIT] Could not open window from second instance: {error}");
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            tray::setup(app)?;

            if let Err(error) = notifications::start() {
                eprintln!("[SPLIT] Native notifications unavailable: {error}");
            }

            if let Err(error) = deadlock::start_console_watcher(app.handle().clone()) {
                eprintln!("SPLIT console watcher unavailable: {error}");
            }

            if let Err(error) = deadlock::start_hotkeys(app.handle().clone()) {
                eprintln!("SPLIT hotkeys unavailable: {error}");
            }

            if let Err(error) = deadlock::start_mouse_tracker() {
                eprintln!("SPLIT mouse tracker unavailable: {error}");
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" || app_window::exit_requested() {
                return;
            }

            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if let Err(error) =
                    app_window::close_main_window_to_background(window.app_handle().clone())
                {
                    eprintln!("[SPLIT] Could not close main window to background: {error}");
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_deadlock_status,
            get_deadlock_setup,
            scan_deadlock_path,
            confirm_deadlock_path,
            get_last_position,
            get_slots,
            get_active_preset,
            get_history_state,
            get_favorite_mode,
            get_notification_settings,
            update_notification_settings,
            toggle_favorite_mode,
            undo_last_action,
            redo_last_action,
            set_active_preset,
            save_slot,
            load_slot,
            capture_slot,
            sync_slots_to_deadlock,
        ])
        .build(tauri::generate_context!())
        .expect("error while building SPLIT");

    app.run(|_app, event| {
        if let tauri::RunEvent::ExitRequested { api, .. } = event {
            if !app_window::exit_requested() {
                api.prevent_exit();
            }
        }
    });
}
