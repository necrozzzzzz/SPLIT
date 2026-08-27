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

pub(crate) fn foreground_deadlock_window() -> Option<windows_sys::Win32::Foundation::HWND> {
    hotkeys::foreground_deadlock_window()
}

use std::{
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

    let value = if undo {
        action.before.clone()
    } else {
        action.after.clone()
    };
    let saved = slots::restore_slot(action.bank, action.slot, value)?;

    let deadlock = paths::configured_deadlock_paths()
        .ok_or_else(|| "Deadlock directory is not configured".to_string())?;
    cfg::write_savestate_cfg(&deadlock.cfg_file, &saved)?;
    cfg::ensure_autoexec(&deadlock.autoexec)?;

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

pub(crate) fn active_slot_state(slot: u8) -> Result<(bool, bool), String> {
    if !(1..=8).contains(&slot) {
        return Err(format!("Invalid load slot {slot}"));
    }

    let bank = current_slot_bank()?;
    let slots = slots::load_bank(bank)?;
    Ok((
        favorite_mode_for_bank(bank),
        slots[usize::from(slot - 1)].is_some(),
    ))
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

fn status_from_paths(found: paths::DeadlockPaths) -> DeadlockStatus {
    DeadlockStatus {
        deadlock_running: process::is_deadlock_running(),

        deadlock_path: Some(paths::path_to_string(&found.root)),

        console_log_path: Some(paths::path_to_string(&found.console_log)),

        console_log_exists: found.console_log.is_file(),

        cfg_dir_exists: found.cfg_dir.is_dir(),

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
            source: "not-found",
        },
    }
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
}
