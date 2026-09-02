mod camera;
mod cfg;
mod history;
mod hotkeys;
mod parser;
mod paths;
mod process;
mod slots;
mod watcher;
pub use history::HistoryState;
pub use parser::PositionSnapshot;
pub use slots::SlotMetadata;

pub(crate) fn foreground_deadlock_window() -> Option<windows_sys::Win32::Foundation::HWND> {
    hotkeys::foreground_deadlock_window()
}

use std::{
    fs,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
};

use serde::Serialize;
use tauri::AppHandle;

static SLOT_OPERATION_LOCK: Mutex<()> = Mutex::new(());
static FAVORITE_MODE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryOperationResult {
    preset: u8,
    slots: Vec<Option<PositionSnapshot>>,
    history_state: HistoryState,
    favorite_active: bool,
    performed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveBankResult {
    preset: u8,
    slots: Vec<Option<PositionSnapshot>>,
    favorite_active: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotEditResult {
    preset: u8,
    slots: Vec<Option<PositionSnapshot>>,
    history_state: HistoryState,
    favorite_active: bool,
}

pub(crate) struct PersistSlotResult {
    pub bank: slots::SlotBank,
    pub slots: Vec<Option<PositionSnapshot>>,
    pub history_state: HistoryState,
    pub history_changed: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadlockStatus {
    deadlock_running: bool,
    deadlock_path: Option<String>,
    console_log_path: Option<String>,
    console_log_exists: bool,
    cfg_dir_exists: bool,

    savestate_cfg_exists: bool,
    prepare_cfg_exists: bool,
    autoexec_exists: bool,

    savestate_cfg_valid: bool,
    prepare_cfg_valid: bool,
    autoexec_valid: bool,

    integration_healthy: bool,

    hotkeys_running: bool,
    hotkeys_error: Option<String>,

    console_watcher_running: bool,
    console_watcher_error: Option<String>,

    teleports_ready: bool,
    presentation_mask_active: bool,

    camera_runtime_checked: bool,
    camera_runtime_ready: bool,
    camera_runtime_error: Option<String>,

    source: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeadlockSetupState {
    configured_path: Option<String>,
    detected_path: Option<String>,
    needs_setup: bool,
}

pub fn get_last_position() -> Option<PositionSnapshot> {
    watcher::get_last_position()
}

pub fn get_slots() -> Result<Vec<Option<PositionSnapshot>>, String> {
    slots::load_bank(current_slot_bank()?)
}

pub fn get_slot_metadata() -> Result<Vec<SlotMetadata>, String> {
    slots::load_metadata(current_slot_bank()?)
}

pub fn get_active_preset() -> Result<u8, String> {
    slots::get_active_preset()
}

fn favorite_mode_active() -> bool {
    FAVORITE_MODE.load(Ordering::SeqCst)
}

fn current_slot_bank() -> Result<slots::SlotBank, String> {
    if favorite_mode_active() {
        Ok(slots::SlotBank::Favorites)
    } else {
        Ok(bank_for_mode(false, slots::get_active_preset()?))
    }
}

fn bank_for_mode(favorite_active: bool, preset: u8) -> slots::SlotBank {
    if favorite_active {
        slots::SlotBank::Favorites
    } else {
        slots::SlotBank::Preset(preset)
    }
}

fn favorite_mode_for_bank(bank: slots::SlotBank) -> bool {
    bank == slots::SlotBank::Favorites
}

pub fn get_favorite_mode() -> bool {
    favorite_mode_active()
}

pub fn get_notification_settings() -> crate::notifications::NotificationSettings {
    paths::load_notification_settings()
}

pub fn update_notification_settings(
    settings: crate::notifications::NotificationSettings,
) -> Result<crate::notifications::NotificationSettings, String> {
    let saved = paths::save_notification_settings(settings)?;
    crate::notifications::apply_settings(saved.clone());
    Ok(saved)
}

pub fn set_active_preset(preset: u8) -> Result<Vec<Option<PositionSnapshot>>, String> {
    let _operation = SLOT_OPERATION_LOCK
        .lock()
        .map_err(|_| "Slot operation lock poisoned".to_string())?;
    ensure_bank_change_allowed(watcher::has_pending_save())?;
    let bank_changed = favorite_mode_active() || slots::get_active_preset()? != preset;
    let saved = set_active_preset_locked(preset)?;
    if bank_changed {
        crate::notifications::show(crate::notifications::Notification::Preset(preset));
    }
    Ok(saved)
}

fn set_active_preset_locked(preset: u8) -> Result<Vec<Option<PositionSnapshot>>, String> {
    let saved = slots::set_active_preset(preset)?;

    /*
     * Dès qu'on change de preset,
     * savestate.cfg doit représenter
     * les 8 slots de CE preset.
     */
    let deadlock = paths::configured_deadlock_paths()
        .ok_or_else(|| "Deadlock directory is not configured".to_string())?;

    cfg::write_savestate_cfg(&deadlock.cfg_file, &saved)?;

    cfg::ensure_autoexec(&deadlock.autoexec)?;

    FAVORITE_MODE.store(false, Ordering::SeqCst);

    Ok(saved)
}

pub fn cycle_active_preset() -> Result<Option<(u8, Vec<Option<PositionSnapshot>>)>, String> {
    let _operation = SLOT_OPERATION_LOCK
        .lock()
        .map_err(|_| "Slot operation lock poisoned".to_string())?;
    /*
     * Ne jamais changer de preset
     * pendant qu'une capture Save
     * attend son getpos_exact.
     */
    if watcher::has_pending_save() {
        return Err("Cannot switch preset while a save capture is pending".to_string());
    }

    let favorite_active = favorite_mode_active();
    if favorite_active {
        println!("[SPLIT] Preset cycle ignored while Favorite Mode is active");
        return Ok(None);
    }

    let current = slots::get_active_preset()?;

    let next = next_preset(current, favorite_active);

    /*
     * Réutilise set_active_preset(),
     * donc :
     *
     * - slots.json est mis à jour
     * - savestate.cfg est régénéré
     * - autoexec est vérifié
     */
    let saved = set_active_preset_locked(next)?;

    Ok(Some((next, saved)))
}

fn next_preset(current: u8, favorite_active: bool) -> u8 {
    if favorite_active || current >= 4 {
        if favorite_active {
            current
        } else {
            1
        }
    } else {
        current + 1
    }
}

pub(crate) fn persist_slot_position(
    slot: u8,
    position: PositionSnapshot,
) -> Result<PersistSlotResult, String> {
    let _operation = SLOT_OPERATION_LOCK
        .lock()
        .map_err(|_| "Slot operation lock poisoned".to_string())?;
    let bank = current_slot_bank()?;
    let saved = slots::save_slot(bank, slot, position)?;

    let deadlock = paths::configured_deadlock_paths()
        .ok_or_else(|| "Deadlock directory is not configured".to_string())?;

    cfg::write_savestate_cfg(&deadlock.cfg_file, &saved.slots)?;

    cfg::ensure_autoexec(&deadlock.autoexec)?;

    let (history_changed, history_state) = history::record(history::SlotAction {
        bank: saved.bank,
        slot: saved.slot,
        before: saved.before,
        after: saved.after,
    })?;

    Ok(PersistSlotResult {
        bank: saved.bank,
        slots: saved.slots,
        history_state,
        history_changed,
    })
}

pub fn save_slot(slot: u8) -> Result<Vec<Option<PositionSnapshot>>, String> {
    let position = watcher::get_last_position()
        .ok_or_else(|| "No position captured yet. Run getpos_exact first.".to_string())?;

    persist_slot_position(slot, position).map(|result| result.slots)
}

pub fn rename_slot(slot: u8, name: String) -> Result<SlotEditResult, String> {
    let _operation = SLOT_OPERATION_LOCK
        .lock()
        .map_err(|_| "Slot operation lock poisoned".to_string())?;

    ensure_history_action_allowed(watcher::has_pending_save())?;

    let bank = current_slot_bank()?;

    let changed = slots::rename_slot(bank, slot, name)?;

    let (_, history_state) = history::record(history::SlotAction {
        bank: changed.bank,
        slot: changed.slot,
        before: changed.before,
        after: changed.after,
    })?;

    Ok(SlotEditResult {
        preset: slots::get_active_preset()?,
        slots: changed.slots,
        history_state,
        favorite_active: favorite_mode_for_bank(bank),
    })
}

pub fn clear_slot(slot: u8) -> Result<SlotEditResult, String> {
    let _operation = SLOT_OPERATION_LOCK
        .lock()
        .map_err(|_| "Slot operation lock poisoned".to_string())?;

    ensure_history_action_allowed(watcher::has_pending_save())?;

    let bank = current_slot_bank()?;

    let changed = slots::clear_slot(bank, slot)?;

    /*
     * Clear modifie réellement les slots
     * utilisés par Deadlock uniquement si
     * une position existait auparavant.
     */
    if changed.before.snapshot != changed.after.snapshot {
        let deadlock = paths::configured_deadlock_paths()
            .ok_or_else(|| "Deadlock directory is not configured".to_string())?;

        cfg::write_savestate_cfg(&deadlock.cfg_file, &changed.slots)?;

        cfg::ensure_autoexec(&deadlock.autoexec)?;
    }

    let (_, history_state) = history::record(history::SlotAction {
        bank: changed.bank,
        slot: changed.slot,
        before: changed.before,
        after: changed.after,
    })?;

    Ok(SlotEditResult {
        preset: slots::get_active_preset()?,
        slots: changed.slots,
        history_state,
        favorite_active: favorite_mode_for_bank(bank),
    })
}

pub fn get_history_state() -> Result<HistoryState, String> {
    history::state()
}

fn apply_history_action(undo: bool) -> Result<HistoryOperationResult, String> {
    let _operation = SLOT_OPERATION_LOCK
        .lock()
        .map_err(|_| "Slot operation lock poisoned".to_string())?;

    ensure_history_action_allowed(watcher::has_pending_save())?;

    let action = if undo {
        history::peek_undo()?
    } else {
        history::peek_redo()?
    };

    let Some(action) = action else {
        return Ok(HistoryOperationResult {
            preset: slots::get_active_preset()?,
            slots: slots::load_bank(current_slot_bank()?)?,
            history_state: history::state()?,
            favorite_active: favorite_mode_active(),
            performed: false,
        });
    };

    let snapshot_changed = action.snapshot_changed();

    let value = if undo {
        action.before.clone()
    } else {
        action.after.clone()
    };

    let saved = slots::restore_slot(action.bank, action.slot, value)?;

    if snapshot_changed {
        let deadlock = paths::configured_deadlock_paths()
            .ok_or_else(|| "Deadlock directory is not configured".to_string())?;

        cfg::write_savestate_cfg(&deadlock.cfg_file, &saved)?;

        cfg::ensure_autoexec(&deadlock.autoexec)?;
    }

    let history_state = if undo {
        history::complete_undo()?
    } else {
        history::complete_redo()?
    };

    let favorite_active = favorite_mode_for_bank(action.bank);
    FAVORITE_MODE.store(favorite_active, Ordering::SeqCst);

    Ok(HistoryOperationResult {
        preset: slots::get_active_preset()?,
        slots: saved,
        history_state,
        favorite_active,
        performed: true,
    })
}

fn ensure_history_action_allowed(save_pending: bool) -> Result<(), String> {
    if save_pending {
        Err("Cannot use Undo or Redo while a save capture is pending".to_string())
    } else {
        Ok(())
    }
}

pub fn undo_last_action() -> Result<HistoryOperationResult, String> {
    let result = apply_history_action(true)?;
    crate::notifications::show(history_notification(true, result.performed));
    Ok(result)
}

pub fn redo_last_action() -> Result<HistoryOperationResult, String> {
    let result = apply_history_action(false)?;
    crate::notifications::show(history_notification(false, result.performed));
    Ok(result)
}

fn history_notification(undo: bool, performed: bool) -> crate::notifications::Notification {
    match (undo, performed) {
        (true, true) => crate::notifications::Notification::Undo,
        (true, false) => crate::notifications::Notification::NothingToUndo,
        (false, true) => crate::notifications::Notification::Redo,
        (false, false) => crate::notifications::Notification::NothingToRedo,
    }
}

pub(crate) fn emit_history_state(app: &AppHandle, state: HistoryState) {
    crate::ui::emit_to_main_if_present(app, "deadlock-history-state", state);
}

pub(crate) fn emit_history_operation(app: &AppHandle, result: &HistoryOperationResult) {
    crate::ui::emit_to_main_if_present(app, "deadlock-slots", &result.slots);
    crate::ui::emit_to_main_if_present(app, "deadlock-preset", result.preset);
    emit_favorite_mode(app, result.favorite_active);
    emit_history_state(app, result.history_state);
}

pub(crate) fn emit_favorite_mode(app: &AppHandle, active: bool) {
    crate::ui::emit_to_main_if_present(app, "deadlock-favorite-mode", active);
}

fn ensure_bank_change_allowed(save_pending: bool) -> Result<(), String> {
    if save_pending {
        Err("Cannot change slot bank while a save capture is pending".to_string())
    } else {
        Ok(())
    }
}

pub fn toggle_favorite_mode() -> Result<ActiveBankResult, String> {
    let _operation = SLOT_OPERATION_LOCK
        .lock()
        .map_err(|_| "Slot operation lock poisoned".to_string())?;
    ensure_bank_change_allowed(watcher::has_pending_save())?;

    let active = !favorite_mode_active();
    let preset = slots::get_active_preset()?;
    let bank = bank_for_mode(active, preset);
    let saved = slots::load_bank(bank)?;
    let deadlock = paths::configured_deadlock_paths()
        .ok_or_else(|| "Deadlock directory is not configured".to_string())?;
    cfg::write_savestate_cfg(&deadlock.cfg_file, &saved)?;
    cfg::ensure_autoexec(&deadlock.autoexec)?;
    FAVORITE_MODE.store(active, Ordering::SeqCst);

    crate::notifications::show(crate::notifications::Notification::Favorites(active));

    Ok(ActiveBankResult {
        preset,
        slots: saved,
        favorite_active: active,
    })
}

pub(crate) fn active_slot_state(slot: u8) -> Result<(bool, Option<PositionSnapshot>), String> {
    if !(1..=8).contains(&slot) {
        return Err(format!("Invalid load slot {slot}"));
    }

    let bank = current_slot_bank()?;

    let slots = slots::load_bank(bank)?;

    let snapshot = slots[usize::from(slot - 1)].clone();

    Ok((favorite_mode_for_bank(bank), snapshot))
}

pub(crate) fn emit_active_bank(app: &AppHandle, result: &ActiveBankResult) {
    crate::ui::emit_to_main_if_present(app, "deadlock-slots", &result.slots);
    emit_favorite_mode(app, result.favorite_active);
}

pub fn sync_slots_to_deadlock() -> Result<(), String> {
    let saved = slots::load_bank(current_slot_bank()?)?;

    let deadlock = paths::configured_deadlock_paths()
        .ok_or_else(|| "Deadlock directory is not configured".to_string())?;

    cfg::write_savestate_cfg(&deadlock.cfg_file, &saved)?;

    cfg::ensure_autoexec(&deadlock.autoexec)?;

    Ok(())
}

pub fn repair_integration_on_startup() -> Result<bool, String> {
    /*
     * Premier lancement :
     * aucun chemin Deadlock n'est encore confirmé.
     * Ce n'est pas une erreur.
     */
    let Some(deadlock) = paths::configured_deadlock_paths() else {
        return Ok(false);
    };

    /*
     * On recharge la banque actuellement active
     * et on régénère toute l'intégration CFG.
     *
     * Cela répare notamment :
     *
     * - savestate.cfg
     * - savestate_prepare.cfg
     * - savestate_slot_X.cfg
     * - les binds internes SPLIT
     * - l'entrée "exec savestate" dans autoexec.cfg
     */
    let saved = slots::load_bank(current_slot_bank()?)?;

    cfg::write_savestate_cfg(&deadlock.cfg_file, &saved)?;
    cfg::ensure_autoexec(&deadlock.autoexec)?;

    Ok(true)
}

pub fn repair_integration() -> Result<DeadlockStatus, String> {
    /*
     * Régénère toute l'intégration SPLIT
     * depuis la banque actuellement active :
     *
     * - savestate.cfg
     * - savestate_prepare.cfg
     * - savestate_slot_X.cfg
     * - autoexec.cfg
     */
    sync_slots_to_deadlock()?;

    /*
     * Retourne immédiatement le nouvel état
     * au frontend pour mettre à jour le Health Check.
     */
    Ok(get_status())
}

pub fn retry_camera_runtime() -> DeadlockStatus {
    /*
     * Action volontaire de l'utilisateur.
     *
     * Contrairement à Refresh, cette commande
     * a le droit de résoudre/scanner la caméra.
     */
    if process::is_deadlock_running() {
        if let Err(error) = camera::capture() {
            eprintln!("[SPLIT] Camera diagnostic retry failed: {error}");
        }
    }

    /*
     * capture() a déjà mis à jour CameraHealth.
     */
    get_status()
}

pub fn retry_console_watcher(app: AppHandle) -> DeadlockStatus {
    let Some(deadlock) = paths::configured_deadlock_paths() else {
        return get_status();
    };

    /*
     * watcher::start() sait déjà arrêter proprement
     * l'ancien watcher avant d'en créer un nouveau.
     */
    if let Err(error) = watcher::start(app, deadlock.console_log) {
        eprintln!("[SPLIT] Console watcher retry failed: {error}");
    }

    /*
     * Le thread positionne WATCHER_RUNNING
     * juste après son démarrage.
     *
     * On lui laisse au maximum 300 ms afin
     * que l'UI reçoive directement le bon état.
     */
    for _ in 0..30 {
        if watcher::is_running() {
            break;
        }

        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    get_status()
}

pub fn prepare_teleports_now() -> Result<DeadlockStatus, String> {
    if !process::is_deadlock_running() {
        return Err("Deadlock must be running before teleport points can be prepared.".to_string());
    }

    hotkeys::prepare_teleports_from_ui()?;

    Ok(get_status())
}

pub fn resume_presentation_now() -> Result<DeadlockStatus, String> {
    if !process::is_deadlock_running() {
        return Err("Deadlock must be running before presentation can be resumed.".to_string());
    }

    hotkeys::resume_presentation_from_ui()?;

    Ok(get_status())
}

fn status_from_paths(found: paths::DeadlockPaths) -> DeadlockStatus {
    let prepare_cfg = found.cfg_dir.join("savestate_prepare.cfg");

    let savestate_cfg_exists = found.cfg_file.is_file();
    let prepare_cfg_exists = prepare_cfg.is_file();
    let autoexec_exists = found.autoexec.is_file();

    /*
     * savestate.cfg :
     * vérifie uniquement les éléments critiques
     * dont SPLIT dépend réellement.
     *
     * On ne compare PAS le fichier entier :
     * les aliases de slots changent selon
     * la banque et les positions sauvegardées.
     */
    let savestate_cfg_valid = fs::read_to_string(&found.cfg_file)
        .map(|content| {
            content.contains("alias \"savestate_getpos\"")
                && content.contains("bind \"F13\" \"exec savestate_prepare\"")
                && content.contains("bind \"F10\" \"r_force_no_present 0\"")
                && content.contains("modifier_citadel_root")
        })
        .unwrap_or(false);

    /*
     * savestate_prepare.cfg doit nettoyer
     * l'ancienne génération de point_teleport.
     *
     * On n'exige PAS ent_create ici :
     * une banque entièrement vide est valide.
     */
    let prepare_cfg_valid = fs::read_to_string(&prepare_cfg)
        .map(|content| content.contains("ent_fire split_tp_* Kill"))
        .unwrap_or(false);

    /*
     * L'autoexec peut contenir plein d'autres
     * commandes utilisateur.
     *
     * SPLIT exige uniquement exec savestate.
     */
    let autoexec_valid = fs::read_to_string(&found.autoexec)
        .map(|content| content.to_ascii_lowercase().contains("exec savestate"))
        .unwrap_or(false);

    let integration_healthy = savestate_cfg_valid && prepare_cfg_valid && autoexec_valid;

    let (camera_runtime_checked, camera_runtime_ready, camera_runtime_error) =
        camera::runtime_status();

    let (hotkeys_running, hotkeys_error) = hotkeys::runtime_status();

    let (console_watcher_running, console_watcher_error) = watcher::runtime_status();

    DeadlockStatus {
        deadlock_running: process::is_deadlock_running(),

        deadlock_path: Some(paths::path_to_string(&found.root)),

        console_log_path: Some(paths::path_to_string(&found.console_log)),

        console_log_exists: found.console_log.is_file(),

        cfg_dir_exists: found.cfg_dir.is_dir(),

        savestate_cfg_exists,
        prepare_cfg_exists,
        autoexec_exists,

        savestate_cfg_valid,
        prepare_cfg_valid,
        autoexec_valid,

        integration_healthy,

        hotkeys_running,
        hotkeys_error,

        console_watcher_running,
        console_watcher_error,

        teleports_ready: !cfg::teleports_dirty(),
        presentation_mask_active: hotkeys::presentation_mask_active(),

        camera_runtime_checked,
        camera_runtime_ready,
        camera_runtime_error,

        source: found.source.as_str(),
    }
}

pub fn get_setup_state() -> DeadlockSetupState {
    /*
     * Un chemin déjà confirmé par l'utilisateur
     * reste prioritaire.
     */
    if let Some(found) = paths::configured_deadlock_paths() {
        return DeadlockSetupState {
            configured_path: Some(paths::path_to_string(&found.root)),

            detected_path: None,

            needs_setup: false,
        };
    }

    /*
     * Aucun chemin configuré :
     * scan automatique.
     */
    let detected = paths::scan_deadlock_root();

    DeadlockSetupState {
        configured_path: None,

        detected_path: detected.as_deref().map(paths::path_to_string),

        needs_setup: true,
    }
}

pub fn scan_deadlock_path() -> Option<String> {
    paths::scan_deadlock_root()
        .as_deref()
        .map(paths::path_to_string)
}

pub fn confirm_deadlock_path(app: AppHandle, path: String) -> Result<DeadlockStatus, String> {
    let found = paths::save_deadlock_root(Path::new(&path))?;

    sync_slots_to_deadlock()?;

    /*
     * Maintenant seulement on démarre
     * le watcher sur le dossier CONFIRMÉ.
     */
    watcher::start(app, found.console_log.clone())?;

    Ok(status_from_paths(found))
}

pub fn get_status() -> DeadlockStatus {
    match paths::configured_deadlock_paths() {
        Some(found) => status_from_paths(found),

        None => DeadlockStatus {
            deadlock_running: process::is_deadlock_running(),

            deadlock_path: None,
            console_log_path: None,
            console_log_exists: false,
            cfg_dir_exists: false,

            savestate_cfg_exists: false,
            prepare_cfg_exists: false,
            autoexec_exists: false,

            savestate_cfg_valid: false,
            prepare_cfg_valid: false,
            autoexec_valid: false,

            integration_healthy: false,

            hotkeys_running: hotkeys::is_running(),
            hotkeys_error: hotkeys::runtime_status().1,

            console_watcher_running: watcher::is_running(),
            console_watcher_error: watcher::runtime_status().1,

            teleports_ready: false,
            presentation_mask_active: hotkeys::presentation_mask_active(),

            camera_runtime_checked: false,
            camera_runtime_ready: false,
            camera_runtime_error: None,

            source: "not-found",
        },
    }
}

pub fn diagnostic_report() -> String {
    let status = get_status();

    let deadlock_process = if status.deadlock_running {
        "Running"
    } else {
        "Not running"
    };

    let integration = if status.integration_healthy {
        "Healthy"
    } else {
        "Needs attention"
    };

    let cfg_directory = if status.cfg_dir_exists {
        "Ready"
    } else {
        "Missing"
    };

    let savestate_cfg = if status.savestate_cfg_valid {
        "Valid"
    } else if status.savestate_cfg_exists {
        "Invalid"
    } else {
        "Missing"
    };

    let prepare_cfg = if status.prepare_cfg_valid {
        "Valid"
    } else if status.prepare_cfg_exists {
        "Invalid"
    } else {
        "Missing"
    };

    let autoexec = if status.autoexec_valid {
        "Configured"
    } else if status.autoexec_exists {
        "Missing SPLIT entry"
    } else {
        "Missing"
    };

    let hotkeys = if status.hotkeys_running {
        "Running"
    } else {
        "Down"
    };

    let watcher = if status.console_watcher_running {
        "Running"
    } else {
        "Down"
    };

    let teleports = if status.teleports_ready {
        "Ready"
    } else {
        "Pending"
    };

    let presentation = if status.presentation_mask_active {
        "Active"
    } else {
        "Normal"
    };

    let camera = if !status.camera_runtime_checked {
        "Not tested"
    } else if status.camera_runtime_ready {
        "Ready"
    } else {
        "Unavailable"
    };

    let preset = slots::get_active_preset()
        .map(|preset| preset.to_string())
        .unwrap_or_else(|error| format!("Unavailable ({error})"));

    let active_bank = if favorite_mode_active() {
        "Favorites".to_string()
    } else {
        format!("Preset {preset}")
    };

    let deadlock_path = status.deadlock_path.as_deref().unwrap_or("Not configured");

    let console_log_path = status.console_log_path.as_deref().unwrap_or("Unavailable");

    let hotkey_error = status.hotkeys_error.as_deref().unwrap_or("None");

    let watcher_error = status.console_watcher_error.as_deref().unwrap_or("None");

    let camera_error = status.camera_runtime_error.as_deref().unwrap_or("None");

    format!(
        "SPLIT 2 Diagnostic Report\n\
         =========================\n\
         SPLIT version: {}\n\
         Platform: {} / {}\n\
         \n\
         Deadlock\n\
         --------\n\
         Process: {deadlock_process}\n\
         Detection source: {}\n\
         Game folder: {deadlock_path}\n\
         Console log: {console_log_path}\n\
         Active bank: {active_bank}\n\
         \n\
         Integration\n\
         -----------\n\
         Overall: {integration}\n\
         CFG directory: {cfg_directory}\n\
         savestate.cfg: {savestate_cfg}\n\
         savestate_prepare.cfg: {prepare_cfg}\n\
         autoexec.cfg: {autoexec}\n\
         \n\
         Runtime\n\
         -------\n\
         Hotkey hook: {hotkeys}\n\
         Console watcher: {watcher}\n\
         Teleport preparation: {teleports}\n\
         Presentation mask: {presentation}\n\
         Camera runtime: {camera}\n\
         \n\
         Errors\n\
         ------\n\
         Hotkey hook: {hotkey_error}\n\
         Console watcher: {watcher_error}\n\
         Camera runtime: {camera_error}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        status.source,
    )
}

pub fn load_slot(slot: u8) -> Result<(), String> {
    hotkeys::load_slot_from_ui(slot)
}

pub fn capture_slot(app: AppHandle, slot: u8) -> Result<(), String> {
    hotkeys::save_slot_from_ui(app, slot)
}

pub fn start_hotkeys(app: AppHandle) -> Result<(), String> {
    hotkeys::start(app)
}

pub fn start_console_watcher(app: AppHandle) -> Result<(), String> {
    /*
     * Premier lancement :
     * aucun watcher avant confirmation.
     */
    let Some(paths) = paths::configured_deadlock_paths() else {
        return Ok(());
    };

    watcher::start(app, paths.console_log)
}

pub fn shutdown_background_services() {
    if let Err(error) = hotkeys::stop() {
        eprintln!("[SPLIT] Could not stop hotkeys cleanly: {error}");
    }
    if let Err(error) = watcher::stop() {
        eprintln!("[SPLIT] Could not stop console watcher cleanly: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_actions_are_refused_while_save_is_pending() {
        assert!(ensure_history_action_allowed(true).is_err());
        assert!(ensure_history_action_allowed(false).is_ok());
    }

    #[test]
    fn favorite_mode_keeps_preset_and_blocks_cycle() {
        assert_eq!(bank_for_mode(true, 3), slots::SlotBank::Favorites);
        assert_eq!(bank_for_mode(false, 3), slots::SlotBank::Preset(3));
        assert!(favorite_mode_for_bank(slots::SlotBank::Favorites));
        assert!(!favorite_mode_for_bank(slots::SlotBank::Preset(2)));
        assert_eq!(next_preset(3, true), 3);
        assert_eq!(next_preset(3, false), 4);
        assert_eq!(next_preset(4, false), 1);
    }

    #[test]
    fn history_notification_distinguishes_performed_and_empty_actions() {
        assert_eq!(
            history_notification(true, true),
            crate::notifications::Notification::Undo
        );
        assert_eq!(
            history_notification(true, false),
            crate::notifications::Notification::NothingToUndo
        );
        assert_eq!(
            history_notification(false, true),
            crate::notifications::Notification::Redo
        );
        assert_eq!(
            history_notification(false, false),
            crate::notifications::Notification::NothingToRedo
        );
    }

    #[test]
    fn rename_preserves_slot_contents() {
        let mut entry = SlotEntry {
            snapshot: Some(position(10.0)),
            name: "Save 1".to_string(),
            saved_at: Some(123456),
            color: Some("#abcdef".to_string()),
        };

        apply_rename_to_entry(&mut entry, "  Mid rooftop  ").unwrap();

        assert_eq!(entry.name, "Mid rooftop",);

        assert_eq!(entry.snapshot, Some(position(10.0)),);

        assert_eq!(entry.saved_at, Some(123456),);

        assert_eq!(entry.color.as_deref(), Some("#abcdef"),);
    }

    #[test]
    fn clear_resets_complete_slot_entry() {
        let mut entry = SlotEntry {
            snapshot: Some(position(10.0)),
            name: "Mid rooftop".to_string(),
            saved_at: Some(123456),
            color: Some("#abcdef".to_string()),
        };

        apply_clear_to_entry(&mut entry, SlotBank::Preset(1), 2);

        assert_eq!(entry, empty_entry(SlotBank::Preset(1), 2,),);

        assert_eq!(entry.name, "Slot 3",);
    }
}
