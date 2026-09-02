use std::{
    env, fs,
    path::PathBuf,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use super::parser::PositionSnapshot;
use crate::storage::atomic_write;

const SLOT_COUNT: usize = 8;
const PRESET_COUNT: usize = 4;

/*
 * v5 introduit les métadonnées de slot :
 *
 * - name
 * - savedAt
 * - color
 *
 * L'API publique continue cependant
 * d'exposer des Option<PositionSnapshot>
 * pour ne casser aucun comportement existant.
 */
const SLOT_FILE_VERSION: u32 = 5;

static STORAGE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotBank {
    Preset(u8),
    Favorites,
}

pub(crate) struct SlotChangeResult {
    pub bank: SlotBank,
    pub slot: u8,
    pub before: SlotEntry,
    pub after: SlotEntry,
    pub slots: Vec<Option<PositionSnapshot>>,
}

/*
 * Nouveau format interne v5.
 *
 * snapshot reste Option<> afin qu'un slot vide
 * puisse quand même posséder une structure
 * stable et, plus tard, des métadonnées.
 */
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SlotEntry {
    pub(crate) snapshot: Option<PositionSnapshot>,

    #[serde(default)]
    pub(crate) name: String,

    #[serde(default)]
    pub(crate) saved_at: Option<u64>,

    #[serde(default)]
    pub(crate) color: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlotMetadata {
    pub name: String,
    pub saved_at: Option<u64>,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SlotsFile {
    version: u32,
    active_preset: u8,

    presets: Vec<Vec<SlotEntry>>,
    favorites: Vec<SlotEntry>,
}

/*
 * Format SPLIT 2 v4 :
 *
 * {
 *   "version": 4,
 *   "activePreset": 1,
 *   "presets": [[PositionSnapshot...]],
 *   "favorites": [PositionSnapshot...]
 * }
 */
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviousSlotsFileWithFavorites {
    #[allow(dead_code)]
    version: u32,

    active_preset: u8,

    presets: Vec<Vec<Option<PositionSnapshot>>>,

    favorites: Vec<Option<PositionSnapshot>>,
}

/*
 * Formats SPLIT 2 précédents avec presets
 * mais sans banque Favorites persistée.
 */
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PreviousSlotsFile {
    #[allow(dead_code)]
    version: u32,

    active_preset: u8,

    presets: Vec<Vec<Option<PositionSnapshot>>>,
}

/*
 * Premier ancien format SPLIT 2 :
 *
 * {
 *   "version": 1,
 *   "slots": [...]
 * }
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
    /*
     * Toujours essayer v5 en premier.
     */
    Current(SlotsFile),

    /*
     * Puis le format avec Favorites
     * utilisé avant les métadonnées.
     */
    PreviousWithFavorites(PreviousSlotsFileWithFavorites),

    /*
     * Puis les anciens presets.
     */
    Previous(PreviousSlotsFile),

    /*
     * Enfin le tout premier format.
     */
    Legacy(LegacySlotsFile),
}

fn default_slot_name(bank: SlotBank, slot_index: usize) -> String {
    let number = slot_index + 1;

    match bank {
        SlotBank::Favorites => {
            format!("Favorite {number}")
        }

        SlotBank::Preset(_) => {
            format!("Slot {number}")
        }
    }
}

fn automatic_saved_name(bank: SlotBank, slot_index: usize) -> String {
    let number = slot_index + 1;

    match bank {
        SlotBank::Favorites => {
            format!("Favorite Save {number}")
        }

        SlotBank::Preset(_) => {
            format!("Save {number}")
        }
    }
}

fn unix_timestamp_now() -> Result<u64, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| format!("System clock is before UNIX epoch: {error}"))
}

fn apply_save_to_entry(
    entry: &mut SlotEntry,
    bank: SlotBank,
    slot_index: usize,
    position: PositionSnapshot,
    saved_at: u64,
) {
    /*
     * SPLIT 1 renommait automatiquement
     * uniquement les noms encore par défaut.
     *
     * Un futur nom personnalisé devra donc
     * survivre aux Overwrite.
     */
    if entry.name == default_slot_name(bank, slot_index) {
        entry.name = automatic_saved_name(bank, slot_index);
    }

    entry.snapshot = Some(position);

    entry.saved_at = Some(saved_at);

    /*
     * color n'est volontairement PAS touché.
     * Un Overwrite conservera donc son tag.
     */
}

fn apply_rename_to_entry(entry: &mut SlotEntry, name: &str) -> Result<(), String> {
    let name = name.trim();

    if name.is_empty() {
        return Err("Slot name cannot be empty".to_string());
    }

    entry.name = name.to_string();

    Ok(())
}

fn apply_clear_to_entry(entry: &mut SlotEntry, bank: SlotBank, slot_index: usize) {
    *entry = empty_entry(bank, slot_index);
}

fn empty_entry(bank: SlotBank, slot_index: usize) -> SlotEntry {
    SlotEntry {
        snapshot: None,

        name: default_slot_name(bank, slot_index),

        saved_at: None,

        color: None,
    }
}

fn empty_slots(bank: SlotBank) -> Vec<SlotEntry> {
    (0..SLOT_COUNT)
        .map(|index| empty_entry(bank, index))
        .collect()
}

fn entries_from_snapshots(
    snapshots: Vec<Option<PositionSnapshot>>,
    bank: SlotBank,
) -> Vec<SlotEntry> {
    snapshots
        .into_iter()
        .enumerate()
        .map(|(index, snapshot)| {
            SlotEntry {
                snapshot,

                name: default_slot_name(bank, index),

                /*
                 * Les anciens formats
                 * ne possédaient pas
                 * ces informations.
                 */
                saved_at: None,

                color: None,
            }
        })
        .collect()
}

fn snapshots_from_entries(entries: &[SlotEntry]) -> Vec<Option<PositionSnapshot>> {
    entries.iter().map(|entry| entry.snapshot.clone()).collect()
}

fn metadata_from_entries(entries: &[SlotEntry]) -> Vec<SlotMetadata> {
    entries
        .iter()
        .map(|entry| SlotMetadata {
            name: entry.name.clone(),

            saved_at: entry.saved_at,

            color: entry.color.clone(),
        })
        .collect()
}

fn default_state() -> SlotsFile {
    SlotsFile {
        version: SLOT_FILE_VERSION,

        active_preset: 1,

        presets: (1..=PRESET_COUNT)
            .map(|preset| empty_slots(SlotBank::Preset(preset as u8)))
            .collect(),

        favorites: empty_slots(SlotBank::Favorites),
    }
}

fn normalize_entries(entries: &mut Vec<SlotEntry>, bank: SlotBank) {
    entries.truncate(SLOT_COUNT);

    while entries.len() < SLOT_COUNT {
        let index = entries.len();

        entries.push(empty_entry(bank, index));
    }

    for (index, entry) in entries.iter_mut().enumerate() {
        /*
         * Un ancien/futur fichier incomplet
         * ne doit jamais produire un nom vide.
         */
        if entry.name.trim().is_empty() {
            entry.name = default_slot_name(bank, index);
        }

        /*
         * Pas de couleur ni de timestamp
         * sur un slot réellement vide.
         *
         * Le nom reste conservé :
         * cela permettra plus tard de gérer
         * proprement les noms personnalisés.
         */
        if entry.snapshot.is_none() {
            entry.saved_at = None;
            entry.color = None;
        }
    }
}

fn normalize_state(state: &mut SlotsFile) {
    state.version = SLOT_FILE_VERSION;

    if !(1..=PRESET_COUNT as u8).contains(&state.active_preset) {
        state.active_preset = 1;
    }

    state.presets.truncate(PRESET_COUNT);

    while state.presets.len() < PRESET_COUNT {
        let preset = state.presets.len() + 1;

        state
            .presets
            .push(empty_slots(SlotBank::Preset(preset as u8)));
    }

    for (index, preset) in state.presets.iter_mut().enumerate() {
        normalize_entries(preset, SlotBank::Preset((index + 1) as u8));
    }

    normalize_entries(&mut state.favorites, SlotBank::Favorites);
}

fn bank_entries_mut(state: &mut SlotsFile, bank: SlotBank) -> Result<&mut Vec<SlotEntry>, String> {
    match bank {
        SlotBank::Preset(preset) => {
            if !(1..=PRESET_COUNT as u8).contains(&preset) {
                return Err(format!("Invalid preset {preset}"));
            }

            Ok(&mut state.presets[usize::from(preset - 1)])
        }

        SlotBank::Favorites => Ok(&mut state.favorites),
    }
}

#[cfg(test)]
fn set_slot_in_state(
    state: &mut SlotsFile,
    bank: SlotBank,
    slot_index: usize,
    value: Option<PositionSnapshot>,
) -> Result<(Option<PositionSnapshot>, Vec<Option<PositionSnapshot>>), String> {
    let entries = bank_entries_mut(state, bank)?;

    let entry = entries
        .get_mut(slot_index)
        .ok_or_else(|| format!("Invalid slot index {slot_index}"))?;

    let before = entry.snapshot.clone();

    /*
     * IMPORTANT :
     *
     * Pour cette première étape v5,
     * on change UNIQUEMENT le snapshot.
     *
     * name / savedAt / color seront
     * branchés progressivement dans les
     * prochaines features.
     *
     * Cela garantit que le comportement
     * Save/Load actuel ne change pas.
     */
    entry.snapshot = value;

    let snapshots = snapshots_from_entries(entries);

    Ok((before, snapshots))
}

fn slots_file_path() -> Result<PathBuf, String> {
    let appdata = env::var_os("APPDATA").ok_or_else(|| "APPDATA is unavailable".to_string())?;

    Ok(PathBuf::from(appdata).join("SPLIT").join("slots.json"))
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

fn read_state_unlocked() -> Result<SlotsFile, String> {
    let path = slots_file_path()?;

    if !path.is_file() {
        return Ok(default_state());
    }

    let content =
        fs::read_to_string(&path).map_err(|error| format!("Could not read slots.json: {error}"))?;

    let disk = serde_json::from_str::<SlotsDisk>(&content)
        .map_err(|error| format!("Could not parse slots.json: {error}"))?;

    /*
     * Même un fichier ayant déjà
     * la structure v5 est réécrit
     * si son numéro de version
     * n'est pas le courant.
     */
    let needs_rewrite = match &disk {
        SlotsDisk::Current(state) => state.version != SLOT_FILE_VERSION,

        _ => true,
    };

    let mut state = match disk {
        SlotsDisk::Current(state) => state,

        SlotsDisk::PreviousWithFavorites(previous) => {
            println!("[SPLIT] Migrating slots.json with favorites -> v5 metadata");

            SlotsFile {
                version: SLOT_FILE_VERSION,

                active_preset: previous.active_preset,

                presets: previous
                    .presets
                    .into_iter()
                    .enumerate()
                    .map(|(index, snapshots)| {
                        entries_from_snapshots(snapshots, SlotBank::Preset((index + 1) as u8))
                    })
                    .collect(),

                favorites: entries_from_snapshots(previous.favorites, SlotBank::Favorites),
            }
        }

        SlotsDisk::Previous(previous) => {
            println!("[SPLIT] Migrating previous preset slots.json -> v5 metadata");

            SlotsFile {
                version: SLOT_FILE_VERSION,

                active_preset: previous.active_preset,

                presets: previous
                    .presets
                    .into_iter()
                    .enumerate()
                    .map(|(index, snapshots)| {
                        entries_from_snapshots(snapshots, SlotBank::Preset((index + 1) as u8))
                    })
                    .collect(),

                favorites: empty_slots(SlotBank::Favorites),
            }
        }

        SlotsDisk::Legacy(legacy) => {
            println!("[SPLIT] Migrating legacy slots.json -> v5 metadata");

            let mut state = default_state();

            state.presets[0] = entries_from_snapshots(legacy.slots, SlotBank::Preset(1));

            state
        }
    };

    normalize_state(&mut state);

    /*
     * Migration réellement persistée
     * immédiatement.
     *
     * On ne dépend donc pas d'un futur
     * Save pour convertir le fichier.
     */
    if needs_rewrite {
        write_state_unlocked(&state)?;

        println!("[SPLIT] slots.json migration complete -> v{SLOT_FILE_VERSION}");
    }

    Ok(state)
}

pub(crate) fn load_bank(bank: SlotBank) -> Result<Vec<Option<PositionSnapshot>>, String> {
    let _guard = STORAGE_LOCK
        .lock()
        .map_err(|_| "Slots storage lock poisoned".to_string())?;

    let state = read_state_unlocked()?;

    match bank {
        SlotBank::Preset(preset) if (1..=PRESET_COUNT as u8).contains(&preset) => Ok(
            snapshots_from_entries(&state.presets[usize::from(preset - 1)]),
        ),

        SlotBank::Favorites => Ok(snapshots_from_entries(&state.favorites)),

        SlotBank::Preset(preset) => Err(format!("Invalid preset {preset}")),
    }
}

pub(crate) fn load_metadata(bank: SlotBank) -> Result<Vec<SlotMetadata>, String> {
    let _guard = STORAGE_LOCK
        .lock()
        .map_err(|_| "Slots storage lock poisoned".to_string())?;

    let state = read_state_unlocked()?;

    match bank {
        SlotBank::Preset(preset) if (1..=PRESET_COUNT as u8).contains(&preset) => Ok(
            metadata_from_entries(&state.presets[usize::from(preset - 1)]),
        ),

        SlotBank::Favorites => Ok(metadata_from_entries(&state.favorites)),

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

    Ok(snapshots_from_entries(&state.presets[index]))
}

pub fn save_slot(
    bank: SlotBank,
    slot: u8,
    position: PositionSnapshot,
) -> Result<SlotChangeResult, String> {
    if !(1..=SLOT_COUNT as u8).contains(&slot) {
        return Err(format!("Invalid slot {slot}"));
    }

    let _guard = STORAGE_LOCK
        .lock()
        .map_err(|_| "Slots storage lock poisoned".to_string())?;

    let mut state = read_state_unlocked()?;

    let slot_index = usize::from(slot - 1);

    let saved_at = unix_timestamp_now()?;

    let entries = bank_entries_mut(&mut state, bank)?;

    let entry = entries
        .get_mut(slot_index)
        .ok_or_else(|| format!("Invalid slot index {slot_index}"))?;

    /*
     * Snapshot COMPLET avant Save :
     *
     * position
     * nom
     * timestamp
     * couleur
     */
    let before = entry.clone();

    apply_save_to_entry(entry, bank, slot_index, position, saved_at);

    let after = entry.clone();

    let saved_slots = snapshots_from_entries(entries);

    write_state_unlocked(&state)?;

    println!(
        "[SPLIT] Saved position to {:?} slot {} at {}",
        bank, slot, saved_at,
    );

    Ok(SlotChangeResult {
        bank,
        slot,
        before,
        after,
        slots: saved_slots,
    })
}

pub(crate) fn rename_slot(
    bank: SlotBank,
    slot: u8,
    name: String,
) -> Result<SlotChangeResult, String> {
    if !(1..=SLOT_COUNT as u8).contains(&slot) {
        return Err(format!("Invalid slot {slot}"));
    }

    let _guard = STORAGE_LOCK
        .lock()
        .map_err(|_| "Slots storage lock poisoned".to_string())?;

    let mut state = read_state_unlocked()?;
    let slot_index = usize::from(slot - 1);

    let entries = bank_entries_mut(&mut state, bank)?;

    let entry = entries
        .get_mut(slot_index)
        .ok_or_else(|| format!("Invalid slot index {slot_index}"))?;

    let before = entry.clone();

    apply_rename_to_entry(entry, &name)?;

    let after = entry.clone();
    let saved_slots = snapshots_from_entries(entries);

    write_state_unlocked(&state)?;

    println!(
        "[SPLIT] Renamed {:?} slot {} to {:?}",
        bank, slot, after.name,
    );

    Ok(SlotChangeResult {
        bank,
        slot,
        before,
        after,
        slots: saved_slots,
    })
}

pub(crate) fn clear_slot(bank: SlotBank, slot: u8) -> Result<SlotChangeResult, String> {
    if !(1..=SLOT_COUNT as u8).contains(&slot) {
        return Err(format!("Invalid slot {slot}"));
    }

    let _guard = STORAGE_LOCK
        .lock()
        .map_err(|_| "Slots storage lock poisoned".to_string())?;

    let mut state = read_state_unlocked()?;
    let slot_index = usize::from(slot - 1);

    let entries = bank_entries_mut(&mut state, bank)?;

    let entry = entries
        .get_mut(slot_index)
        .ok_or_else(|| format!("Invalid slot index {slot_index}"))?;

    let before = entry.clone();

    apply_clear_to_entry(entry, bank, slot_index);

    let after = entry.clone();
    let saved_slots = snapshots_from_entries(entries);

    write_state_unlocked(&state)?;

    println!("[SPLIT] Cleared {:?} slot {}", bank, slot,);

    Ok(SlotChangeResult {
        bank,
        slot,
        before,
        after,
        slots: saved_slots,
    })
}

pub(crate) fn restore_slot(
    bank: SlotBank,
    slot: u8,
    value: SlotEntry,
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

    let entries = bank_entries_mut(&mut state, bank)?;

    let entry = entries
        .get_mut(slot_index)
        .ok_or_else(|| format!("Invalid slot index {slot_index}"))?;

    /*
     * Undo / Redo restaure maintenant
     * l'entrée ENTIÈRE.
     */
    *entry = value;

    let saved = snapshots_from_entries(entries);

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
            camera: None,
        }
    }

    #[test]
    fn normalization_preserves_four_presets_of_eight_slots() {
        let mut state = default_state();

        state.version = 99;
        state.active_preset = 8;

        state.presets = vec![(0..12)
            .map(|index| SlotEntry {
                snapshot: Some(position(index as f64)),

                name: String::new(),

                saved_at: Some(123),

                color: Some("#ffffff".to_string()),
            })
            .collect()];

        normalize_state(&mut state);

        assert_eq!(state.version, SLOT_FILE_VERSION,);

        assert_eq!(state.active_preset, 1,);

        assert_eq!(state.presets.len(), PRESET_COUNT,);

        assert!(state
            .presets
            .iter()
            .all(|preset| { preset.len() == SLOT_COUNT }),);

        assert_eq!(state.presets[0][0].name, "Slot 1",);
    }

    #[test]
    fn version_four_migrates_snapshots_and_favorites() {
        let disk = serde_json::from_str::<SlotsDisk>(
            r#"{
                    "version": 4,
                    "activePreset": 2,
                    "presets": [
                        [
                            {
                                "x": 1,
                                "y": 2,
                                "z": 3,
                                "pitch": 4,
                                "yaw": 5,
                                "roll": 6
                            }
                        ]
                    ],
                    "favorites": [
                        {
                            "x": 10,
                            "y": 20,
                            "z": 30,
                            "pitch": 40,
                            "yaw": 50,
                            "roll": 60
                        }
                    ]
                }"#,
        )
        .expect("v4 state should deserialize");

        let SlotsDisk::PreviousWithFavorites(previous) = disk else {
            panic!("v4 should select previous-with-favorites variant");
        };

        let mut state = SlotsFile {
            version: SLOT_FILE_VERSION,

            active_preset: previous.active_preset,

            presets: previous
                .presets
                .into_iter()
                .enumerate()
                .map(|(index, snapshots)| {
                    entries_from_snapshots(snapshots, SlotBank::Preset((index + 1) as u8))
                })
                .collect(),

            favorites: entries_from_snapshots(previous.favorites, SlotBank::Favorites),
        };

        normalize_state(&mut state);

        assert_eq!(state.active_preset, 2,);

        assert_eq!(state.presets[0][0].snapshot.as_ref().unwrap().x, 1.0,);

        assert_eq!(state.favorites[0].snapshot.as_ref().unwrap().x, 10.0,);

        assert_eq!(state.presets[0][0].name, "Slot 1",);

        assert_eq!(state.favorites[0].name, "Favorite 1",);
    }

    #[test]
    fn previous_presets_migrate_with_empty_favorites() {
        let disk = serde_json::from_str::<SlotsDisk>(
            r#"{
                    "version": 2,
                    "activePreset": 2,
                    "presets": [
                        [null],
                        [
                            {
                                "x": 9,
                                "y": 9,
                                "z": 9,
                                "pitch": 9,
                                "yaw": 9,
                                "roll": 9
                            }
                        ]
                    ]
                }"#,
        )
        .expect("old preset state should deserialize");

        let SlotsDisk::Previous(previous) = disk else {
            panic!("old preset state should select previous variant");
        };

        let mut state = SlotsFile {
            version: SLOT_FILE_VERSION,

            active_preset: previous.active_preset,

            presets: previous
                .presets
                .into_iter()
                .enumerate()
                .map(|(index, snapshots)| {
                    entries_from_snapshots(snapshots, SlotBank::Preset((index + 1) as u8))
                })
                .collect(),

            favorites: empty_slots(SlotBank::Favorites),
        };

        normalize_state(&mut state);

        assert_eq!(state.active_preset, 2,);

        assert_eq!(state.presets[1][0].snapshot.as_ref().unwrap().x, 9.0,);

        assert!(state
            .favorites
            .iter()
            .all(|entry| { entry.snapshot.is_none() }),);
    }

    #[test]
    fn legacy_slots_migrate_into_first_preset() {
        let disk = serde_json::from_str::<SlotsDisk>(
            r#"{
                    "version": 1,
                    "slots": [
                        {
                            "x": 1,
                            "y": 2,
                            "z": 3,
                            "pitch": 4,
                            "yaw": 5,
                            "roll": 6
                        }
                    ]
                }"#,
        )
        .expect("legacy state should deserialize");

        let SlotsDisk::Legacy(legacy) = disk else {
            panic!("legacy state should select legacy variant");
        };

        let mut state = default_state();

        state.presets[0] = entries_from_snapshots(legacy.slots, SlotBank::Preset(1));

        normalize_state(&mut state);

        assert_eq!(state.presets[0][0].snapshot.as_ref().unwrap().x, 1.0,);

        assert_eq!(state.presets[0][0].name, "Slot 1",);
    }

    #[test]
    fn snapshot_api_remains_compatible() {
        let entries = vec![
            SlotEntry {
                snapshot: Some(position(42.0)),

                name: "Custom name".to_string(),

                saved_at: Some(123),

                color: Some("#fff".to_string()),
            },
            empty_entry(SlotBank::Preset(1), 1),
        ];

        let snapshots = snapshots_from_entries(&entries);

        assert_eq!(snapshots.len(), 2,);

        assert_eq!(snapshots[0].as_ref().unwrap().x, 42.0,);

        assert!(snapshots[1].is_none(),);
    }

    #[test]
    fn favorite_and_preset_snapshots_remain_isolated() {
        let mut state = default_state();

        set_slot_in_state(&mut state, SlotBank::Favorites, 0, Some(position(1.0))).unwrap();

        assert!(state
            .presets
            .iter()
            .flatten()
            .all(|entry| { entry.snapshot.is_none() }),);

        set_slot_in_state(&mut state, SlotBank::Preset(3), 1, Some(position(2.0))).unwrap();

        assert_eq!(state.favorites[0].snapshot, Some(position(1.0),),);

        assert_eq!(state.presets[2][1].snapshot, Some(position(2.0),),);
    }

    #[test]
    fn save_metadata_uses_automatic_name_and_timestamp() {
        let mut entry = empty_entry(SlotBank::Preset(1), 0);

        apply_save_to_entry(&mut entry, SlotBank::Preset(1), 0, position(10.0), 123456);

        assert_eq!(entry.name, "Save 1",);

        assert_eq!(entry.saved_at, Some(123456),);

        assert_eq!(entry.snapshot.as_ref().unwrap().x, 10.0,);
    }

    #[test]
    fn overwrite_preserves_custom_name_and_color() {
        let mut entry = SlotEntry {
            snapshot: Some(position(1.0)),

            name: "Mid rooftop".to_string(),

            saved_at: Some(100),

            color: Some("#abcdef".to_string()),
        };

        apply_save_to_entry(&mut entry, SlotBank::Preset(1), 0, position(2.0), 200);

        assert_eq!(entry.name, "Mid rooftop",);

        assert_eq!(entry.color.as_deref(), Some("#abcdef"),);

        assert_eq!(entry.saved_at, Some(200),);

        assert_eq!(entry.snapshot.as_ref().unwrap().x, 2.0,);
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
