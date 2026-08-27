use std::{env, fs, path::PathBuf, sync::Mutex};

use serde::{Deserialize, Serialize};

use super::parser::PositionSnapshot;
use crate::storage::atomic_write;

const SLOT_COUNT: usize = 8;
const PRESET_COUNT: usize = 4;
const SLOT_FILE_VERSION: u32 = 3;
static STORAGE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotBank {
    Preset(u8),
    Favorites,
}

pub(crate) struct SlotSaveResult {
    pub bank: SlotBank,
    pub slot: u8,
    pub before: Option<PositionSnapshot>,
    pub after: Option<PositionSnapshot>,
    pub slots: Vec<Option<PositionSnapshot>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SlotsFile {
    version: u32,
    active_preset: u8,

    presets: Vec<Vec<Option<PositionSnapshot>>>,
    favorites: Vec<Option<PositionSnapshot>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviousSlotsFile {
    #[allow(dead_code)]
    version: u32,
    active_preset: u8,
    presets: Vec<Vec<Option<PositionSnapshot>>>,
}

/*
 * Ancien format SPLIT 2 :
 *
 * {
 *   "version": 1,
 *   "slots": [...]
 * }
 *
 * On le garde pour migrer automatiquement
 * vers les 4 presets.
 */
#[derive(Debug, Deserialize)]
struct LegacySlotsFile {
    #[allow(dead_code)]
    version: u32,

    slots: Vec<Option<PositionSnapshot>>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum SlotsDisk {
    Current(SlotsFile),

    Previous(PreviousSlotsFile),

    Legacy(LegacySlotsFile),
}

fn empty_slots() -> Vec<Option<PositionSnapshot>> {
    vec![None; SLOT_COUNT]
}

fn default_state() -> SlotsFile {
    SlotsFile {
        version: SLOT_FILE_VERSION,

        active_preset: 1,

        presets: (0..PRESET_COUNT).map(|_| empty_slots()).collect(),

        favorites: empty_slots(),
    }
}

fn normalize_slots(slots: &mut Vec<Option<PositionSnapshot>>) {
    slots.truncate(SLOT_COUNT);

    while slots.len() < SLOT_COUNT {
        slots.push(None);
    }
}

fn normalize_state(state: &mut SlotsFile) {
    state.version = SLOT_FILE_VERSION;

    if !(1..=PRESET_COUNT as u8).contains(&state.active_preset) {
        state.active_preset = 1;
    }

    state.presets.truncate(PRESET_COUNT);

    while state.presets.len() < PRESET_COUNT {
        state.presets.push(empty_slots());
    }

    for preset in &mut state.presets {
        normalize_slots(preset);
    }

    normalize_slots(&mut state.favorites);
}

fn set_slot_in_state(
    state: &mut SlotsFile,
    bank: SlotBank,
    slot_index: usize,
    value: Option<PositionSnapshot>,
) -> Result<(Option<PositionSnapshot>, Vec<Option<PositionSnapshot>>), String> {
    match bank {
        SlotBank::Preset(preset) => {
            if !(1..=PRESET_COUNT as u8).contains(&preset) {
                return Err(format!("Invalid preset {preset}"));
            }
            let preset_index = usize::from(preset - 1);
            let before = state.presets[preset_index][slot_index].clone();
            state.presets[preset_index][slot_index] = value;
            Ok((before, state.presets[preset_index].clone()))
        }
        SlotBank::Favorites => {
            let before = state.favorites[slot_index].clone();
            state.favorites[slot_index] = value;
            Ok((before, state.favorites.clone()))
        }
    }
}

fn slots_file_path() -> Result<PathBuf, String> {
    let appdata = env::var_os("APPDATA").ok_or_else(|| "APPDATA is unavailable".to_string())?;

    Ok(PathBuf::from(appdata).join("SPLIT").join("slots.json"))
}

fn read_state_unlocked() -> Result<SlotsFile, String> {
    let path = slots_file_path()?;

    if !path.is_file() {
        return Ok(default_state());
    }

    let content =
        fs::read_to_string(&path).map_err(|error| format!("Could not read slots.json: {error}"))?;

    let disk = serde_json::from_str::<SlotsDisk>(&content)
        .map_err(|error| format!("Could not parse slots.json: {error}"))?;

    let mut state = match disk {
        SlotsDisk::Current(state) => state,

        SlotsDisk::Previous(previous) => SlotsFile {
            version: SLOT_FILE_VERSION,
            active_preset: previous.active_preset,
            presets: previous.presets,
            favorites: empty_slots(),
        },

        /*
         * Migration V1 -> V2.
         *
         * Les anciens slots deviennent
         * le Preset 1.
         */
        SlotsDisk::Legacy(legacy) => {
            println!("[SPLIT] Migrating slots.json v1 -> v2 presets");

            let mut state = default_state();

            let mut old_slots = legacy.slots;

            normalize_slots(&mut old_slots);

            state.presets[0] = old_slots;

            state
        }
    };

    normalize_state(&mut state);

    Ok(state)
}

fn write_state_unlocked(state: &SlotsFile) -> Result<(), String> {
    let path = slots_file_path()?;

    let Some(parent) = path.parent() else {
        return Err("slots.json has no parent directory".to_string());
    };

    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create SPLIT data directory: {error}"))?;

    let json = serde_json::to_string_pretty(state)
        .map_err(|error| format!("Could not serialize slots: {error}"))?;

    atomic_write(&path, json).map_err(|error| format!("Could not write slots.json: {error}"))?;

    Ok(())
}

pub(crate) fn load_bank(bank: SlotBank) -> Result<Vec<Option<PositionSnapshot>>, String> {
    let _guard = STORAGE_LOCK
        .lock()
        .map_err(|_| "Slots storage lock poisoned".to_string())?;
    let state = read_state_unlocked()?;

    match bank {
        SlotBank::Preset(preset) if (1..=PRESET_COUNT as u8).contains(&preset) => {
            Ok(state.presets[usize::from(preset - 1)].clone())
        }
        SlotBank::Favorites => Ok(state.favorites.clone()),
        SlotBank::Preset(preset) => Err(format!("Invalid preset {preset}")),
    }
}

pub fn get_active_preset() -> Result<u8, String> {
    let _guard = STORAGE_LOCK
        .lock()
        .map_err(|_| "Slots storage lock poisoned".to_string())?;
    Ok(read_state_unlocked()?.active_preset)
}

pub fn set_active_preset(preset: u8) -> Result<Vec<Option<PositionSnapshot>>, String> {
    if !(1..=PRESET_COUNT as u8).contains(&preset) {
        return Err(format!("Invalid preset {preset}"));
    }

    let _guard = STORAGE_LOCK
        .lock()
        .map_err(|_| "Slots storage lock poisoned".to_string())?;
    let mut state = read_state_unlocked()?;

    state.active_preset = preset;

    write_state_unlocked(&state)?;

    println!("[SPLIT] Active preset changed to {preset}");

    let index = usize::from(preset - 1);

    Ok(state.presets[index].clone())
}

pub fn save_slot(
    bank: SlotBank,
    slot: u8,
    position: PositionSnapshot,
) -> Result<SlotSaveResult, String> {
    if !(1..=SLOT_COUNT as u8).contains(&slot) {
        return Err(format!("Invalid slot {slot}"));
    }

    let _guard = STORAGE_LOCK
        .lock()
        .map_err(|_| "Slots storage lock poisoned".to_string())?;
    let mut state = read_state_unlocked()?;

    let slot_index = usize::from(slot - 1);
    let (before, saved_slots) =
        set_slot_in_state(&mut state, bank, slot_index, Some(position.clone()))?;

    write_state_unlocked(&state)?;

    println!("[SPLIT] Saved position to {:?} slot {}", bank, slot,);

    Ok(SlotSaveResult {
        bank,
        slot,
        before,
        after: Some(position),
        slots: saved_slots,
    })
}

pub(crate) fn restore_slot(
    bank: SlotBank,
    slot: u8,
    value: Option<PositionSnapshot>,
) -> Result<Vec<Option<PositionSnapshot>>, String> {
    if !(1..=SLOT_COUNT as u8).contains(&slot) {
        return Err(format!("Invalid slot {slot}"));
    }

    let _guard = STORAGE_LOCK
        .lock()
        .map_err(|_| "Slots storage lock poisoned".to_string())?;
    let mut state = read_state_unlocked()?;
    let slot_index = usize::from(slot - 1);
    if let SlotBank::Preset(preset) = bank {
        state.active_preset = preset;
    }
    let (_, saved) = set_slot_in_state(&mut state, bank, slot_index, value)?;
    write_state_unlocked(&state)?;
    Ok(saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn position(value: f64) -> PositionSnapshot {
        PositionSnapshot {
            x: value,
            y: value,
            z: value,
            pitch: value,
            yaw: value,
            roll: value,
        }
    }

    #[test]
    fn normalization_preserves_four_presets_of_eight_slots() {
        let mut state = SlotsFile {
            version: 99,
            active_preset: 8,
            presets: vec![vec![Some(position(1.0)); 12]],
            favorites: vec![Some(position(2.0)); 12],
        };

        normalize_state(&mut state);

        assert_eq!(state.version, SLOT_FILE_VERSION);
        assert_eq!(state.active_preset, 1);
        assert_eq!(state.presets.len(), PRESET_COUNT);
        assert!(state
            .presets
            .iter()
            .all(|preset| preset.len() == SLOT_COUNT));
        assert!(state.presets[1].iter().all(Option::is_none));
        assert_eq!(state.favorites.len(), SLOT_COUNT);
    }

    #[test]
    fn legacy_slots_migrate_into_first_preset() {
        let disk: SlotsDisk = serde_json::from_str(
            r#"{"version":1,"slots":[{"x":1,"y":2,"z":3,"pitch":4,"yaw":5,"roll":6}]}"#,
        )
        .expect("legacy state should deserialize");

        let SlotsDisk::Legacy(legacy) = disk else {
            panic!("legacy format should select legacy variant");
        };
        let mut state = default_state();
        let mut old_slots = legacy.slots;
        normalize_slots(&mut old_slots);
        state.presets[0] = old_slots;
        normalize_state(&mut state);

        assert_eq!(state.presets.len(), 4);
        assert_eq!(state.presets[0].len(), 8);
        assert_eq!(state.presets[0][0].as_ref().unwrap().x, 1.0);
        assert!(state.presets[1].iter().all(Option::is_none));
    }

    #[test]
    fn changing_preset_keeps_all_slot_sets_isolated() {
        let mut state = default_state();
        state.presets[0][0] = Some(position(10.0));
        state.active_preset = 2;
        state.presets[1][0] = Some(position(20.0));
        normalize_state(&mut state);

        assert_eq!(state.active_preset, 2);
        assert_eq!(state.presets[0][0], Some(position(10.0)));
        assert_eq!(state.presets[1][0], Some(position(20.0)));
        assert_eq!(state.presets.len(), 4);
        assert!(state.presets.iter().all(|preset| preset.len() == 8));
    }

    #[test]
    fn version_two_migrates_with_empty_favorites_and_preserved_presets() {
        let disk: SlotsDisk = serde_json::from_str(
            r#"{"version":2,"activePreset":2,"presets":[[null],[{"x":9,"y":9,"z":9,"pitch":9,"yaw":9,"roll":9}]]}"#,
        )
        .expect("version two state should deserialize");
        let SlotsDisk::Previous(previous) = disk else {
            panic!("version two should select previous variant");
        };
        let mut state = SlotsFile {
            version: SLOT_FILE_VERSION,
            active_preset: previous.active_preset,
            presets: previous.presets,
            favorites: empty_slots(),
        };
        normalize_state(&mut state);

        assert_eq!(state.active_preset, 2);
        assert_eq!(state.presets[1][0], Some(position(9.0)));
        assert_eq!(state.favorites, empty_slots());
    }

    #[test]
    fn favorite_and_preset_saves_are_isolated() {
        let mut state = default_state();
        set_slot_in_state(&mut state, SlotBank::Favorites, 0, Some(position(1.0))).unwrap();
        assert!(state.presets.iter().flatten().all(Option::is_none));

        set_slot_in_state(&mut state, SlotBank::Preset(3), 1, Some(position(2.0))).unwrap();
        assert_eq!(state.favorites[0], Some(position(1.0)));
        assert_eq!(state.presets[2][1], Some(position(2.0)));
    }
}
