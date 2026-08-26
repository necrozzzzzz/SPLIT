mod paths;
mod process;

use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadlockStatus {
    deadlock_running: bool,
    deadlock_path: Option<String>,
    console_log_path: Option<String>,
    console_log_exists: bool,
    cfg_dir_exists: bool,
    source: &'static str,
}

pub fn get_status() -> DeadlockStatus {
    let deadlock_running = process::is_deadlock_running();
    let detected = paths::detect_deadlock_paths();

    match detected {
        Some(found) => DeadlockStatus {
            deadlock_running,
            deadlock_path: Some(paths::path_to_string(&found.root)),
            console_log_path: Some(paths::path_to_string(&found.console_log)),
            console_log_exists: found.console_log.is_file(),
            cfg_dir_exists: found.cfg_dir.is_dir(),
            source: found.source.as_str(),
        },
        None => DeadlockStatus {
            deadlock_running,
            deadlock_path: None,
            console_log_path: None,
            console_log_exists: false,
            cfg_dir_exists: false,
            source: "not-found",
        },
    }
}
