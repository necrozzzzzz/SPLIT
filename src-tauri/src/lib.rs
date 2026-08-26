mod deadlock;

#[tauri::command]
fn get_deadlock_status() -> deadlock::DeadlockStatus {
    deadlock::get_status()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            if let Err(error) = deadlock::start_console_watcher(app.handle().clone()) {
                eprintln!("SPLIT console watcher unavailable: {error}");
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![get_deadlock_status])
        .run(tauri::generate_context!())
        .expect("error while running SPLIT");
}
