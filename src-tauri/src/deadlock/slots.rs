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
const PRESET_COUNT: usize = 4;
const SLOT_FILE_VERSION: u32 = 2;


#[derive(
    Debug,
    Clone,
    Serialize,
    Deserialize,
)]
#[serde(rename_all = "camelCase")]
struct SlotsFile {
    version: u32,
    active_preset: u8,

    presets:
        Vec<Vec<Option<PositionSnapshot>>>,
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
#[derive(
    Debug,
    Deserialize,
)]
struct LegacySlotsFile {
    #[allow(dead_code)]
    version: u32,

    slots:
        Vec<Option<PositionSnapshot>>,
}


#[derive(
    Debug,
    Deserialize,
)]
#[serde(untagged)]
enum SlotsDisk {
    Current(
        SlotsFile,
    ),

    Legacy(
        LegacySlotsFile,
    ),
}


fn empty_slots(
) -> Vec<Option<PositionSnapshot>> {
    vec![
        None;
        SLOT_COUNT
    ]
}


fn default_state(
) -> SlotsFile {
    SlotsFile {
        version:
            SLOT_FILE_VERSION,

        active_preset:
            1,

        presets:
            (0..PRESET_COUNT)
                .map(
                    |_| {
                        empty_slots()
                    },
                )
                .collect(),
    }
}


fn normalize_slots(
    slots:
        &mut Vec<Option<PositionSnapshot>>,
) {
    slots.truncate(
        SLOT_COUNT,
    );

    while slots.len()
        < SLOT_COUNT
    {
        slots.push(
            None,
        );
    }
}


fn normalize_state(
    state: &mut SlotsFile,
) {
    state.version =
        SLOT_FILE_VERSION;

    if !(1..=PRESET_COUNT as u8)
        .contains(
            &state.active_preset,
        )
    {
        state.active_preset =
            1;
    }

    state.presets.truncate(
        PRESET_COUNT,
    );

    while state.presets.len()
        < PRESET_COUNT
    {
        state.presets.push(
            empty_slots(),
        );
    }

    for preset
        in &mut state.presets
    {
        normalize_slots(
            preset,
        );
    }
}


fn slots_file_path(
) -> Result<PathBuf, String> {
    let appdata =
        env::var_os(
            "APPDATA",
        )
        .ok_or_else(|| {
            "APPDATA is unavailable"
                .to_string()
        })?;

    Ok(
        PathBuf::from(
            appdata,
        )
        .join(
            "SPLIT",
        )
        .join(
            "slots.json",
        ),
    )
}


fn read_state(
) -> Result<SlotsFile, String> {
    let path =
        slots_file_path()?;

    if !path.is_file() {
        return Ok(
            default_state(),
        );
    }

    let content =
        fs::read_to_string(
            &path,
        )
        .map_err(
            |error| {
                format!(
                    "Could not read slots.json: {error}"
                )
            },
        )?;


    let disk =
        serde_json::from_str::<SlotsDisk>(
            &content,
        )
        .map_err(
            |error| {
                format!(
                    "Could not parse slots.json: {error}"
                )
            },
        )?;


    let mut state =
        match disk {
            SlotsDisk::Current(
                state,
            ) => {
                state
            }


            /*
             * Migration V1 -> V2.
             *
             * Les anciens slots deviennent
             * le Preset 1.
             */
            SlotsDisk::Legacy(
                legacy,
            ) => {
                println!(
                    "[SPLIT] Migrating slots.json v1 -> v2 presets"
                );

                let mut state =
                    default_state();

                let mut old_slots =
                    legacy.slots;

                normalize_slots(
                    &mut old_slots,
                );

                state.presets[0] =
                    old_slots;

                state
            }
        };


    normalize_state(
        &mut state,
    );

    Ok(
        state,
    )
}


fn write_state(
    state: &SlotsFile,
) -> Result<(), String> {
    let path =
        slots_file_path()?;

    let Some(parent) =
        path.parent()
    else {
        return Err(
            "slots.json has no parent directory"
                .to_string(),
        );
    };


    fs::create_dir_all(
        parent,
    )
    .map_err(
        |error| {
            format!(
                "Could not create SPLIT data directory: {error}"
            )
        },
    )?;


    let json =
        serde_json::to_string_pretty(
            state,
        )
        .map_err(
            |error| {
                format!(
                    "Could not serialize slots: {error}"
                )
            },
        )?;


    fs::write(
        &path,
        json,
    )
    .map_err(
        |error| {
            format!(
                "Could not write slots.json: {error}"
            )
        },
    )?;


    Ok(())
}


pub fn load_slots(
) -> Result<
    Vec<Option<PositionSnapshot>>,
    String,
> {
    let state =
        read_state()?;


    let index =
        usize::from(
            state.active_preset - 1,
        );


    Ok(
        state.presets[index]
            .clone(),
    )
}


pub fn get_active_preset(
) -> Result<u8, String> {
    Ok(
        read_state()?
            .active_preset,
    )
}


pub fn set_active_preset(
    preset: u8,
) -> Result<
    Vec<Option<PositionSnapshot>>,
    String,
> {
    if !(1..=PRESET_COUNT as u8)
        .contains(
            &preset,
        )
    {
        return Err(
            format!(
                "Invalid preset {preset}"
            ),
        );
    }


    let mut state =
        read_state()?;


    state.active_preset =
        preset;


    write_state(
        &state,
    )?;


    println!(
        "[SPLIT] Active preset changed to {preset}"
    );


    let index =
        usize::from(
            preset - 1,
        );


    Ok(
        state.presets[index]
            .clone(),
    )
}


pub fn save_slot(
    slot: u8,
    position: PositionSnapshot,
) -> Result<
    Vec<Option<PositionSnapshot>>,
    String,
> {
    if !(1..=SLOT_COUNT as u8)
        .contains(
            &slot,
        )
    {
        return Err(
            format!(
                "Invalid slot {slot}"
            ),
        );
    }


    let mut state =
        read_state()?;


    let preset_index =
        usize::from(
            state.active_preset - 1,
        );


    let slot_index =
        usize::from(
            slot - 1,
        );


    state.presets
        [preset_index]
        [slot_index] =
        Some(
            position,
        );


    write_state(
        &state,
    )?;


    println!(
        "[SPLIT] Saved position to preset {} slot {}",
        state.active_preset,
        slot,
    );


    Ok(
        state.presets
            [preset_index]
            .clone(),
    )
}