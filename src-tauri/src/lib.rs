mod app_window;
mod deadlock;
mod notifications;
mod panorama_bridge;
mod quick_access;
mod storage;
mod tray;
mod ui;

use tauri::Manager;

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct StartupSoundLaunch {
    settings: deadlock::StartupSoundSettings,
    should_play: bool,
}

#[tauri::command]
fn get_deadlock_status() -> deadlock::DeadlockStatus {
    deadlock::get_status()
}

#[tauri::command]
fn get_diagnostic_report() -> String {
    deadlock::diagnostic_report()
}

#[tauri::command]
fn get_deadlock_setup() -> deadlock::DeadlockSetupState {
    deadlock::trace_startup(false, "frontend init command: get_deadlock_setup");
    deadlock::trace_startup(false, "get_deadlock_setup begin");
    let state = deadlock::get_setup_state();
    deadlock::trace_startup(false, "get_deadlock_setup end");
    deadlock::finalize_startup_diagnostics();
    state
}

#[tauri::command]
fn record_successful_launch() -> Result<deadlock::LaunchFolderState, String> {
    deadlock::trace_startup(true, "frontend init command: record_successful_launch");
    deadlock::trace_startup(false, "record_successful_launch begin");
    let result = deadlock::record_successful_launch();
    deadlock::trace_startup(
        false,
        if result.is_ok() {
            "record_successful_launch end: ok"
        } else {
            "record_successful_launch end: error"
        },
    );
    if let Err(error) = &result {
        deadlock::persist_startup_diagnostics(&format!(
            "record_successful_launch failed: {error}"
        ));
    }
    result
}

#[tauri::command]
fn report_startup_error(message: String) {
    deadlock::trace_startup(false, "frontend reported an unexpected setup error");
    deadlock::persist_startup_diagnostics(&message);
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
fn get_slot_metadata() -> Result<Vec<deadlock::SlotMetadata>, String> {
    deadlock::get_slot_metadata()
}

#[tauri::command]
fn get_active_preset() -> Result<u8, String> {
    deadlock::get_active_preset()
}

#[tauri::command]
fn get_preset_names() -> Result<Vec<String>, String> {
    deadlock::get_preset_names()
}

#[tauri::command]
fn export_preset(preset: u8) -> Result<deadlock::PresetExport, String> {
    deadlock::export_preset(preset)
}

#[tauri::command]
fn export_preset_archive(preset: u8, destination: String) -> Result<(), String> {
    deadlock::export_preset_archive(preset, destination)
}

#[tauri::command]
fn import_preset_archive(preset: u8, source: String) -> Result<deadlock::SlotEditResult, String> {
    deadlock::import_preset_archive(preset, source)
}

#[tauri::command]
fn import_preset(
    preset: u8,
    imported: deadlock::PresetExport,
) -> Result<deadlock::SlotEditResult, String> {
    deadlock::import_preset(preset, imported)
}

#[tauri::command]
fn rename_preset(
    app: tauri::AppHandle,
    preset: u8,
    name: String,
) -> Result<Vec<String>, String> {
    let names = deadlock::rename_preset(preset, name)?;
    ui::emit_to_main_if_present(
        &app,
        "quick-access-refresh",
        (),
    );
    Ok(names)
}

#[tauri::command]
fn clear_preset(preset: u8) -> Result<deadlock::SlotEditResult, String> {
    deadlock::clear_preset(preset)
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
fn get_favorite_slot_summaries() -> Result<Vec<deadlock::FavoriteSlotSummary>, String> {
    deadlock::get_favorite_slot_summaries()
}

#[tauri::command]
fn copy_slot_to_favorite(
    source_slot: u8,
    favorite_slot: u8,
    overwrite: bool,
) -> Result<deadlock::FavoriteSlotSummary, String> {
    deadlock::copy_slot_to_favorite(source_slot, favorite_slot, overwrite)
}

#[tauri::command]
fn get_notification_settings() -> notifications::NotificationSettings {
    deadlock::get_notification_settings()
}

#[tauri::command]
fn claim_startup_sound(app: tauri::AppHandle) -> StartupSoundLaunch {
    StartupSoundLaunch {
        settings: deadlock::get_startup_sound_settings(),
        should_play: app_window::claim_visible_startup_sound(&app),
    }
}

#[tauri::command]
fn update_startup_sound_settings(
    settings: deadlock::StartupSoundSettings,
) -> Result<deadlock::StartupSoundSettings, String> {
    deadlock::update_startup_sound_settings(settings)
}

#[tauri::command]
fn update_notification_settings(
    settings: notifications::NotificationSettings,
) -> Result<notifications::NotificationSettings, String> {
    deadlock::update_notification_settings(settings)
}

#[tauri::command]
fn test_notification(app: tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "SPLIT main window is unavailable".to_string())?;
    let hwnd = window
        .hwnd()
        .map_err(|error| format!("Could not read the SPLIT window handle: {error}"))?;

    notifications::show_test(hwnd.0)
}

#[tauri::command]
fn get_hotkey_settings() -> deadlock::HotkeySettings {
    deadlock::get_hotkey_settings()
}

#[tauri::command]
async fn update_hotkey_settings(
    settings: deadlock::HotkeySettings,
) -> Result<deadlock::HotkeySettings, String> {
    tauri::async_runtime::spawn_blocking(move || {
        deadlock::update_hotkey_settings(settings)
    })
    .await
    .map_err(|error| {
        format!("Hotkey settings task failed: {error}")
    })?
}

#[tauri::command]
async fn reset_hotkey_settings() -> Result<deadlock::HotkeySettings, String> {
    tauri::async_runtime::spawn_blocking(
        deadlock::reset_hotkey_settings,
    )
    .await
    .map_err(|error| {
        format!("Hotkey reset task failed: {error}")
    })?
}

#[tauri::command]
fn get_quick_access_settings(
) -> quick_access::QuickAccessSettings {
    deadlock::get_quick_access_settings()
}

#[tauri::command]
async fn update_quick_access_settings(
    app: tauri::AppHandle,
    settings: quick_access::QuickAccessSettings,
) -> Result<quick_access::QuickAccessSettings, String> {
    tauri::async_runtime::spawn_blocking(move || {
        deadlock::update_quick_access_settings(
            &app,
            settings,
        )
    })
    .await
    .map_err(|error| {
        format!("Quick Access settings task failed: {error}")
    })?
}

#[tauri::command]
fn toggle_favorite_mode(
    app: tauri::AppHandle,
) -> Result<deadlock::ActiveBankResult, String> {
    let result = deadlock::toggle_favorite_mode()?;
    deadlock::emit_active_bank(&app, &result);
    Ok(result)
}

#[tauri::command]
fn undo_last_action(
    app: tauri::AppHandle,
) -> Result<deadlock::HistoryOperationResult, String> {
    let result = deadlock::undo_last_action()?;
    deadlock::emit_history_operation(&app, &result);
    Ok(result)
}

#[tauri::command]
fn redo_last_action(
    app: tauri::AppHandle,
) -> Result<deadlock::HistoryOperationResult, String> {
    let result = deadlock::redo_last_action()?;
    deadlock::emit_history_operation(&app, &result);
    Ok(result)
}

#[tauri::command]
fn set_active_preset(
    app: tauri::AppHandle,
    preset: u8,
) -> Result<
    Vec<Option<deadlock::PositionSnapshot>>,
    String,
> {
    let slots =
        deadlock::set_active_preset(
            preset,
        )?;

    ui::emit_to_main_if_present(
        &app,
        "deadlock-slots",
        &slots,
    );

    ui::emit_to_main_if_present(
        &app,
        "deadlock-preset",
        preset,
    );

    ui::emit_to_main_if_present(
        &app,
        "deadlock-favorite-mode",
        false,
    );

    Ok(slots)
}

#[tauri::command]
fn save_slot(slot: u8) -> Result<Vec<Option<deadlock::PositionSnapshot>>, String> {
    deadlock::save_slot(slot)
}

#[tauri::command]
fn rename_slot(
    app: tauri::AppHandle,
    slot: u8,
    name: String,
) -> Result<deadlock::SlotEditResult, String> {
    let result = deadlock::rename_slot(slot, name)?;
    deadlock::emit_slot_edit(&app, &result);
    Ok(result)
}

#[tauri::command]
fn clear_slot(
    app: tauri::AppHandle,
    slot: u8,
) -> Result<deadlock::SlotEditResult, String> {
    let result = deadlock::clear_slot(slot)?;
    deadlock::emit_slot_edit(&app, &result);
    Ok(result)
}

#[tauri::command]
fn set_slot_color(
    app: tauri::AppHandle,
    slot: u8,
    color: Option<String>,
) -> Result<deadlock::SlotEditResult, String> {
    let result = deadlock::set_slot_color(slot, color)?;
    deadlock::emit_slot_edit(&app, &result);
    Ok(result)
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
fn repair_deadlock_integration() -> Result<deadlock::DeadlockStatus, String> {
    deadlock::repair_integration()
}

#[tauri::command]
fn retry_camera_runtime() -> deadlock::DeadlockStatus {
    deadlock::retry_camera_runtime()
}

#[tauri::command]
fn retry_console_watcher(app: tauri::AppHandle) -> deadlock::DeadlockStatus {
    deadlock::retry_console_watcher(app)
}

#[tauri::command]
fn prepare_teleports_now() -> Result<deadlock::DeadlockStatus, String> {
    deadlock::prepare_teleports_now()
}

#[tauri::command]
fn resume_deadlock_presentation() -> Result<deadlock::DeadlockStatus, String> {
    deadlock::resume_presentation_now()
}

#[tauri::command]
fn confirm_deadlock_path(
    app: tauri::AppHandle,
    path: String,
) -> Result<deadlock::DeadlockStatus, String> {
    deadlock::confirm_deadlock_path(app, path)
}


#[tauri::command]
fn get_start_minimized_to_tray(
    app: tauri::AppHandle,
) -> bool {
    app_window::start_minimized_to_tray_enabled(
        &app,
    )
}


#[tauri::command]
fn set_start_minimized_to_tray(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<bool, String> {
    app_window::set_start_minimized_to_tray(
        &app,
        enabled,
    )
}

#[tauri::command]
fn get_close_to_tray(
    app: tauri::AppHandle,
) -> bool {
    app_window::close_to_tray_enabled(
        &app,
    )
}


#[tauri::command]
fn set_close_to_tray(
    app: tauri::AppHandle,
    enabled: bool,
) -> Result<bool, String> {
    app_window::set_close_to_tray(
        &app,
        enabled,
    )
}

#[tauri::command]
fn reset_main_window(
    app: tauri::AppHandle,
) -> Result<(), String> {
    app_window::reset_main_window(
        &app,
    )
}

#[tauri::command]
fn launch_deadlock() -> Result<(), String> {
    deadlock::launch_deadlock()
}

#[tauri::command]
fn hide_quick_access(
    app: tauri::AppHandle,
) -> Result<(), String> {
    quick_access::hide(&app)?;
    deadlock::quick_access_hidden();
    Ok(())
}

#[tauri::command]
fn get_quick_access_state(
) -> quick_access::QuickAccessState {
    quick_access::state()
}

#[tauri::command]
fn set_quick_access_viewer_open(
    app: tauri::AppHandle,
    open: bool,
) -> Result<(), String> {
    quick_access::set_viewer_open(&app, open)
}

#[tauri::command]
async fn set_quick_access_text_input_active(
    active: bool,
) -> Result<(), String> {
    println!(
        "[SPLIT][QA] text input command active={active}"
    );

    tauri::async_runtime::spawn_blocking(move || {
        deadlock::set_quick_access_text_input_active(active)
    })
    .await
    .map_err(|error| {
        format!(
            "Quick Access text input command task failed: {error}"
        )
    })?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Err(error) = app_window::open_main_window(app.clone()) {
                eprintln!(
                    "[SPLIT] Could not open window from second instance: {error}"
                );
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--autostart"]),
        ))
        .plugin(
            tauri_plugin_window_state::Builder::default()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::SIZE,
                )
                .build(),
        )
        .setup(|app| {
            tray::setup(app)?;

            if app_window::should_start_minimized_to_tray(app.handle()) {
                if let Err(error) =
                    app_window::close_main_window_to_background(
                        app.handle().clone(),
                    )
                {
                    eprintln!(
                        "[SPLIT] Could not start minimized to tray: {error}"
                    );
                }
            }

            /*
            * IMPORTANT :
             *
             * Ne jamais bloquer le thread de setup Tauri
             * avec les services Deadlock / notifications.
             *
             * La fenêtre et le WebView doivent devenir
             * interactifs immédiatement.
             */
            let background_app = app.handle().clone();

            std::thread::Builder::new()
                .name("split-background-startup".to_string())
                .spawn(move || {
                    println!("[SPLIT] Background services starting...");

                    /*
                     * Avant de démarrer les services,
                     * vérifier/régénérer l'intégration Deadlock.
                     *
                     * Cela permet notamment de récupérer
                     * automatiquement après :
                     *
                     * - suppression accidentelle des CFG,
                     * - autoexec modifié,
                     * - fichiers SPLIT manquants,
                     * - certains changements après une update Deadlock.
                     */
                    match deadlock::repair_integration_on_startup() {
                        Ok(true) => {
                            println!("[SPLIT] Deadlock integration verified/regenerated");
                        }

                        Ok(false) => {
                            println!("[SPLIT] Deadlock integration skipped: path not configured");
                        }

                        Err(error) => {
                            eprintln!("[SPLIT] Deadlock integration repair failed: {error}");
                        }
                    }

                    if app_window::exit_requested() {
                        return;
                    }

                    /*
                     * Le watcher doit être prêt avant
                     * les hotkeys de Save.
                     */
                    if let Err(error) = deadlock::start_console_watcher(background_app.clone()) {
                        eprintln!("[SPLIT] Console watcher unavailable: {error}");
                    }

                    if let Err(error) = panorama_bridge::start(background_app.clone()) {
                        eprintln!("[SPLIT] Panorama bridge unavailable: {error}");
                    }

                    if let Err(error) = deadlock::start_hotkeys(background_app.clone()) {
                        eprintln!("[SPLIT] Hotkeys unavailable: {error}");
                    }

                    if let Err(error) = deadlock::apply_generated_cfg_now() {
                        eprintln!("[SPLIT] Startup CFG application failed: {error}");
                    }

                    if let Err(error) = deadlock::start_process_monitor(background_app.clone()) {
                        eprintln!("[SPLIT] Deadlock process monitor unavailable: {error}");
                    }

                    /*
                     * Les notifications sont les moins
                     * critiques et peuvent être les dernières.
                     *
                     * Leur initialisation peut attendre
                     * ready_receiver.recv() sans bloquer l'UI.
                     */
                    if let Err(error) = notifications::start() {
                        eprintln!("[SPLIT] Native notifications unavailable: {error}");
                    }

                    println!("[SPLIT] Background services ready");
                })
                .map_err(|error| format!("Could not start SPLIT background services: {error}"))?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" || app_window::exit_requested() {
                return;
            }

            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();

                if app_window::close_to_tray_enabled(
                    window.app_handle(),
                ) {
                    app_window::show_background_notice_once(
                        window.app_handle(),
                    );

                    if let Err(error) =
                        app_window::close_main_window_to_background(
                            window.app_handle().clone(),
                        )
                    {
                        eprintln!(
                            "[SPLIT] Could not close main window to background: {error}"
                        );
                    }
                } else {
                    app_window::request_true_quit(
                        window.app_handle(),
                    );
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            launch_deadlock,
            get_start_minimized_to_tray,
            set_start_minimized_to_tray,
            get_deadlock_status,
            get_diagnostic_report,
            get_deadlock_setup,
            record_successful_launch,
            report_startup_error,
            scan_deadlock_path,
            confirm_deadlock_path,
            get_close_to_tray,
            set_close_to_tray,
            reset_main_window,
            hide_quick_access,
            get_quick_access_state,
            set_quick_access_viewer_open,
            set_quick_access_text_input_active,
            get_last_position,
            get_slots,
            get_slot_metadata,
            get_active_preset,
            get_history_state,
            get_favorite_mode,
            get_favorite_slot_summaries,
            copy_slot_to_favorite,
            get_preset_names,
            export_preset,
            export_preset_archive,
            import_preset_archive,
            import_preset,
            rename_preset,
            clear_preset,
            get_notification_settings,
            update_notification_settings,
            claim_startup_sound,
            update_startup_sound_settings,
            test_notification,
            get_hotkey_settings,
            update_hotkey_settings,
            reset_hotkey_settings,
            get_quick_access_settings,
            update_quick_access_settings,
            toggle_favorite_mode,
            undo_last_action,
            redo_last_action,
            set_active_preset,
            save_slot,
            rename_slot,
            clear_slot,
            set_slot_color,
            load_slot,
            capture_slot,
            sync_slots_to_deadlock,
            repair_deadlock_integration,
            retry_camera_runtime,
            retry_console_watcher,
            prepare_teleports_now,
            resume_deadlock_presentation,
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
