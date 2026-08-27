use std::{
    env,
    fs,
    path::PathBuf,
};

use serde::{
    Deserialize,
    Serialize,
};

use super::parser::PositionSnapshot;

const SLOT_COUNT: usize = 8;
const SLOT_FILE_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct SlotsFile {
    version: u32,
    slots: Vec<Option<PositionSnapshot>>,
}

impl Default for SlotsFile {
    fn default() -> Self {
        Self {
            version: SLOT_FILE_VERSION,
            slots: empty_slots(),
        }
    }
}

fn empty_slots() -> Vec<Option<PositionSnapshot>> {
    (0..SLOT_COUNT)
        .map(|_| None)
        .collect()
}

fn normalize_slots(
    mut slots: Vec<Option<PositionSnapshot>>,
) -> Vec<Option<PositionSnapshot>> {
    if slots.len() > SLOT_COUNT {
        slots.truncate(SLOT_COUNT);
    }

    while slots.len() < SLOT_COUNT {
        slots.push(None);
    }

    slots
}

fn slots_path() -> Result<PathBuf, String> {
    let app_data =
        env::var_os("APPDATA")
            .ok_or_else(|| {
                "APPDATA environment variable is unavailable"
                    .to_string()
            })?;

    Ok(
        PathBuf::from(app_data)
            .join("SPLIT")
            .join("slots.json"),
    )
}

pub fn load_slots(
) -> Result<Vec<Option<PositionSnapshot>>, String> {
    let path = slots_path()?;

    /*
     * Premier lancement :
     * pas encore de slots.json.
     */
    if !path.is_file() {
        return Ok(empty_slots());
    }

    let raw =
        fs::read_to_string(&path)
            .map_err(|error| {
                format!(
                    "Could not read slots.json: {error}"
                )
            })?;

    let file: SlotsFile =
        serde_json::from_str(&raw)
            .map_err(|error| {
                format!(
                    "Could not parse slots.json: {error}"
                )
            })?;

    Ok(
        normalize_slots(file.slots)
    )
}

fn write_slots(
    slots: Vec<Option<PositionSnapshot>>,
) -> Result<(), String> {
    let path = slots_path()?;

    let Some(parent) = path.parent() else {
        return Err(
            "Invalid SPLIT data directory"
                .to_string(),
        );
    };

    fs::create_dir_all(parent)
        .map_err(|error| {
            format!(
                "Could not create SPLIT data directory: {error}"
            )
        })?;

    let file = SlotsFile {
        version: SLOT_FILE_VERSION,
        slots: normalize_slots(slots),
    };

    let json =
        serde_json::to_string_pretty(&file)
            .map_err(|error| {
                format!(
                    "Could not serialize slots: {error}"
                )
            })?;

    fs::write(&path, json)
        .map_err(|error| {
            format!(
                "Could not write slots.json: {error}"
            )
        })?;

    Ok(())
}

pub fn save_slot(
    slot: u8,
    position: PositionSnapshot,
) -> Result<Vec<Option<PositionSnapshot>>, String> {
    if !(1..=SLOT_COUNT as u8)
        .contains(&slot)
    {
        return Err(
            format!(
                "Invalid slot {slot}. Expected 1-{SLOT_COUNT}."
            ),
        );
    }

    let mut slots =
        load_slots()?;

    let index =
        usize::from(slot - 1);

    slots[index] =
        Some(position);

    write_slots(
        slots.clone(),
    )?;

    println!(
        "[SPLIT] Saved position to slot {slot}"
    );

    Ok(slots)
}