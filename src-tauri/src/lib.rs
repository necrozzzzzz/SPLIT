mod deadlock;

#[tauri::command]
fn get_deadlock_status() -> deadlock::DeadlockStatus {
    deadlock::get_status()
}

#[tauri::command]
fn get_deadlock_setup(
) -> deadlock::DeadlockSetupState {
    deadlock::get_setup_state()
}

#[tauri::command]
fn scan_deadlock_path() -> Option<String> {
    deadlock::scan_deadlock_path()
}

#[tauri::command]
fn get_last_position(
) -> Option<deadlock::PositionSnapshot> {
    deadlock::get_last_position()
}

#[tauri::command]
fn confirm_deadlock_path(
    app: tauri::AppHandle,
    path: String,
) -> Result<deadlock::DeadlockStatus, String> {
    deadlock::confirm_deadlock_path(
        app,
        path,
    )
}

#[cfg_attr(
    mobile,
    tauri::mobile_entry_point
)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_dialog::init()
        )
        .setup(|app| {
            if let Err(error) =
                deadlock::start_console_watcher(
                    app.handle().clone(),
                )
            {
                eprintln!(
                    "SPLIT console watcher unavailable: {error}"
                );
            }

            Ok(())
        })
        .invoke_handler(
            tauri::generate_handler![
                get_deadlock_status,
                get_deadlock_setup,
                scan_deadlock_path,
                confirm_deadlock_path,
                get_last_position,
            ],
        )
        .run(
            tauri::generate_context!()
        )
        .expect(
            "error while running SPLIT"
        );
}