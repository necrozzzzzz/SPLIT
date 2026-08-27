mod parser;
mod paths;
mod process;
mod watcher;
pub use parser::PositionSnapshot;

use std::path::Path;

use serde::Serialize;
use tauri::AppHandle;

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

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadlockSetupState {
    configured_path: Option<String>,
    detected_path: Option<String>,
    needs_setup: bool,
}

pub fn get_last_position(
) -> Option<PositionSnapshot> {
    watcher::get_last_position()
}

fn status_from_paths(
    found: paths::DeadlockPaths,
) -> DeadlockStatus {
    DeadlockStatus {
        deadlock_running:
            process::is_deadlock_running(),

        deadlock_path:
            Some(paths::path_to_string(
                &found.root,
            )),

        console_log_path:
            Some(paths::path_to_string(
                &found.console_log,
            )),

        console_log_exists:
            found.console_log.is_file(),

        cfg_dir_exists:
            found.cfg_dir.is_dir(),

        source:
            found.source.as_str(),
    }
}

pub fn get_setup_state() -> DeadlockSetupState {
    /*
     * Un chemin déjà confirmé par l'utilisateur
     * reste prioritaire.
     */
    if let Some(found) =
        paths::configured_deadlock_paths()
    {
        return DeadlockSetupState {
            configured_path:
                Some(paths::path_to_string(
                    &found.root,
                )),

            detected_path: None,

            needs_setup: false,
        };
    }

    /*
     * Aucun chemin configuré :
     * scan automatique.
     */
    let detected =
        paths::scan_deadlock_root();

    DeadlockSetupState {
        configured_path: None,

        detected_path:
            detected.as_deref()
                .map(paths::path_to_string),

        needs_setup: true,
    }
}

pub fn scan_deadlock_path() -> Option<String> {
    paths::scan_deadlock_root()
        .as_deref()
        .map(paths::path_to_string)
}

pub fn confirm_deadlock_path(
    app: AppHandle,
    path: String,
) -> Result<DeadlockStatus, String> {
    let found =
        paths::save_deadlock_root(
            Path::new(&path),
        )?;

    /*
     * Maintenant seulement on démarre
     * le watcher sur le dossier CONFIRMÉ.
     */
    watcher::start(
        app,
        found.console_log.clone(),
    )?;

    Ok(status_from_paths(found))
}

pub fn get_status() -> DeadlockStatus {
    match paths::configured_deadlock_paths() {
        Some(found) =>
            status_from_paths(found),

        None => DeadlockStatus {
            deadlock_running:
                process::is_deadlock_running(),

            deadlock_path: None,
            console_log_path: None,
            console_log_exists: false,
            cfg_dir_exists: false,
            source: "not-found",
        },
    }
}

pub fn start_console_watcher(
    app: AppHandle,
) -> Result<(), String> {
    /*
     * Premier lancement :
     * aucun watcher avant confirmation.
     */
    let Some(paths) =
        paths::configured_deadlock_paths()
    else {
        return Ok(());
    };

    watcher::start(
        app,
        paths.console_log,
    )
}