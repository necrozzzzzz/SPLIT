use std::{
    fs,
    path::Path,
};

use super::parser::PositionSnapshot;

const LOAD_TRANSPORT_KEYS: [&str; 8] = [
    "u",
    "i",
    "o",
    "j",
    "k",
    "l",
    "n",
    "m",
];

const LOAD_HOTKEYS: [&str; 8] = [
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
];

pub fn write_savestate_cfg(
    cfg_file: &Path,
    slots: &[Option<PositionSnapshot>],
) -> Result<(), String> {
    let Some(parent) = cfg_file.parent() else {
        return Err(
            "savestate.cfg has no parent directory"
                .to_string(),
        );
    };

    fs::create_dir_all(parent)
        .map_err(|error| {
            format!(
                "Could not create Deadlock CFG directory: {error}"
            )
        })?;

    let mut output = String::new();

    output.push_str(
        "// SPLIT 2 - auto-generated, do not edit manually\n\n",
    );

    /*
     * Position capture transport.
     */
    output.push_str(
        "alias \"savestate_getpos\" \"exec savestate; getpos_exact\"\n",
    );

    output.push_str(
        "bind \"h\" \"savestate_getpos\"\n\n",
    );

    for index in 0..8 {
        let slot_number = index + 1;
        let transport_key =
            LOAD_TRANSPORT_KEYS[index];

        let load_hotkey =
            LOAD_HOTKEYS[index];

        match slots.get(index).and_then(|slot| slot.as_ref()) {
            Some(position) => {
                output.push_str(
                    &format!(
                        "// Slot {slot_number}\n"
                    ),
                );

                output.push_str(
                    &format!(
                        "alias \"load_slot_{slot_number}\" \"{};noclip\"\n",
                        position.to_deadlock_command(),
                    ),
                );
            }

            None => {
                output.push_str(
                    &format!(
                        "// Slot {slot_number}: empty\n"
                    ),
                );

                output.push_str(
                    &format!(
                        "alias \"load_slot_{slot_number}\" \"echo SPLIT Slot {slot_number} empty\"\n",
                    ),
                );
            }
        }

        output.push_str(
            &format!(
                "bind \"{transport_key}\" \"exec savestate; load_slot_{slot_number}\"\n",
            ),
        );

        output.push_str(
            &format!(
                "bind \"{load_hotkey}\" \"exec savestate; load_slot_{slot_number}\"\n\n",
            ),
        );
    }

    fs::write(
        cfg_file,
        output,
    )
    .map_err(|error| {
        format!(
            "Could not write savestate.cfg: {error}"
        )
    })?;

    println!(
        "[SPLIT] savestate.cfg updated: {}",
        cfg_file.display(),
    );

    Ok(())
}

pub fn ensure_autoexec(
    autoexec: &Path,
) -> Result<(), String> {
    const COMMAND: &str = "exec savestate";

    let Some(parent) = autoexec.parent() else {
        return Err(
            "autoexec.cfg has no parent directory"
                .to_string(),
        );
    };

    fs::create_dir_all(parent)
        .map_err(|error| {
            format!(
                "Could not create Deadlock CFG directory: {error}"
            )
        })?;

    if autoexec.is_file() {
        let content =
            fs::read_to_string(autoexec)
                .unwrap_or_default();

        if content
            .to_ascii_lowercase()
            .contains(COMMAND)
        {
            return Ok(());
        }

        let mut updated = content;

        if !updated.ends_with('\n') {
            updated.push('\n');
        }

        updated.push_str(
            "\nexec savestate // Added by SPLIT 2\n",
        );

        fs::write(
            autoexec,
            updated,
        )
        .map_err(|error| {
            format!(
                "Could not update autoexec.cfg: {error}"
            )
        })?;
    } else {
        fs::write(
            autoexec,
            "exec savestate // Added by SPLIT 2\n",
        )
        .map_err(|error| {
            format!(
                "Could not create autoexec.cfg: {error}"
            )
        })?;
    }

    println!(
        "[SPLIT] autoexec.cfg configured"
    );

    Ok(())
}