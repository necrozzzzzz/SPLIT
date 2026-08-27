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
fn get_slots(
) -> Result<
    Vec<Option<deadlock::PositionSnapshot>>,
    String,
> {
    deadlock::get_slots()
}

#[tauri::command]
fn save_slot(
    slot: u8,
) -> Result<
    Vec<Option<deadlock::PositionSnapshot>>,
    String,
> {
    deadlock::save_slot(
        slot,
    )
}

#[tauri::command]
fn sync_slots_to_deadlock(
) -> Result<(), String> {
    deadlock::sync_slots_to_deadlock()
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

                        if let Err(error) =
                deadlock::start_hotkeys()
            {
                eprintln!(
                    "SPLIT hotkeys unavailable: {error}"
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
                get_slots,
                save_slot,
                sync_slots_to_deadlock,
            ],
        )
        .run(
            tauri::generate_context!()
        )
        .expect(
            "error while running SPLIT"
        );
}