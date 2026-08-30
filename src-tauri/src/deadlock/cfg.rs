use std::{
    fs,
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        OnceLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use super::parser::PositionSnapshot;
use crate::storage::atomic_write;

const LOAD_TRANSPORT_KEYS: [&str; 8] = ["u", "i", "o", "j", "k", "l", "n", "m"];

static TELEPORT_GENERATION: AtomicU64 = AtomicU64::new(0);

static TELEPORTS_DIRTY: AtomicBool = AtomicBool::new(false);

pub(crate) fn teleports_dirty() -> bool {
    TELEPORTS_DIRTY.load(Ordering::SeqCst)
}

pub(crate) fn mark_teleports_prepared() {
    TELEPORTS_DIRTY.store(false, Ordering::SeqCst);
}

static TELEPORT_SESSION: OnceLock<u128> = OnceLock::new();

fn teleport_namespace() -> String {
    let session = TELEPORT_SESSION.get_or_init(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    });

    let generation = TELEPORT_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;

    format!("{}_{}_{}", session, std::process::id(), generation,)
}

pub fn write_savestate_cfg(
    cfg_file: &Path,
    slots: &[Option<PositionSnapshot>],
) -> Result<(), String> {
    let Some(parent) = cfg_file.parent() else {
        return Err("savestate.cfg has no parent directory".to_string());
    };

    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create Deadlock CFG directory: {error}"))?;

    let mut output = String::new();

    let namespace = teleport_namespace();

    let prepare_cfg_path = parent.join("savestate_prepare.cfg");

    let mut prepare_output = String::from("// SPLIT 2 - prepared teleport points\n\n");

    output.push_str("// SPLIT 2 - auto-generated, do not edit manually\n\n");

    /*
     * Position capture transport.
     */
    output.push_str("alias \"savestate_getpos\" \"exec savestate; getpos_exact\"\n");

    output.push_str("bind \"h\" \"savestate_getpos\"\n\n");

    /*
     * Transports internes SPLIT.
     *
     * F13 est une touche virtuelle interne utilisée
     * uniquement pour préparer les point_teleport.
     *
     * F10 injecté par SPLIT réactive la présentation
     * après le masque d'un Load.
     *
     * F10 physique reste Redo grâce au hook SPLIT.
     * F11 physique reste Favorite Mode.
     * F12 reste totalement libre pour Steam.
     */
    output.push_str(
        "bind \"F13\" \"exec savestate_prepare\"\n\
        bind \"F10\" \"r_force_no_present 0\"\n\n",
    );

    for index in 0..8 {
        let slot_number = index + 1;
        let transport_key = LOAD_TRANSPORT_KEYS[index];

        match slots.get(index).and_then(|slot| slot.as_ref()) {
            Some(position) => {
                output.push_str(&format!("// Slot {slot_number}\n"));

                let slot_cfg_name = format!("savestate_slot_{slot_number}");
                let slot_cfg_path = parent.join(format!("{slot_cfg_name}.cfg"));

                let teleport_name = format!("split_tp_{}_{}", namespace, slot_number);

                /*
                 * Le point_teleport est créé à l'avance
                 * dans savestate_prepare.cfg.
                 */
                prepare_output.push_str(&format!(
                    "ent_create point_teleport \
                        {{\"targetname\" \"{}\" \
                        \"origin\" \"{} {} {}\" \
                        \"angles\" \"0 {} 0\"}}\n",
                    teleport_name, position.x, position.y, position.z, position.yaw,
                ));

                /*
                 * État qui fonctionnait :
                 *
                 * 1. freeze de la présentation
                 * 2. TP
                 * 3. setang
                 *
                 * Rust attend ensuite 35 ms avant
                 * de réafficher le jeu.
                 */
                let slot_cfg = format!(
                    "r_force_no_present 1\n\
                    ent_fire {} TeleportEntity !player\n\
                    setang_exact {} {} {}\n",
                    teleport_name, position.pitch, position.yaw, position.roll,
                );

                atomic_write(&slot_cfg_path, slot_cfg).map_err(|error| {
                    format!("Could not write {}: {error}", slot_cfg_path.display())
                })?;

                /*
                 * IMPORTANT :
                 * c'est cette ligne qui avait disparu.
                 */
                output.push_str(&format!(
                    "alias \"load_slot_{slot_number}\" \
                    \"exec {slot_cfg_name}\"\n",
                ));
            }

            None => {
                output.push_str(&format!("// Slot {slot_number}: empty\n"));

                output.push_str(&format!(
                    "alias \"load_slot_{slot_number}\" \
                    \"echo SPLIT Slot {slot_number} empty\"\n",
                ));
            }
        }

        /*
         * Transport normal.
         *
         * Le freeze n'est PAS ici.
         * Il est dans savestate_slot_X.cfg.
         */
        output.push_str(&format!(
            "bind \"{transport_key}\" \
            \"exec savestate; load_slot_{slot_number}\"\n\n",
        ));
    }

    atomic_write(&prepare_cfg_path, prepare_output)
        .map_err(|error| format!("Could not write {}: {error}", prepare_cfg_path.display(),))?;

    atomic_write(cfg_file, output)
        .map_err(|error| format!("Could not write savestate.cfg: {error}"))?;

    println!("[SPLIT] savestate.cfg updated: {}", cfg_file.display(),);

    TELEPORTS_DIRTY.store(true, Ordering::SeqCst);

    Ok(())
}

pub fn ensure_autoexec(autoexec: &Path) -> Result<(), String> {
    const COMMAND: &str = "exec savestate";

    let Some(parent) = autoexec.parent() else {
        return Err("autoexec.cfg has no parent directory".to_string());
    };

    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create Deadlock CFG directory: {error}"))?;

    if autoexec.is_file() {
        let content = fs::read_to_string(autoexec).unwrap_or_default();

        if content.to_ascii_lowercase().contains(COMMAND) {
            return Ok(());
        }

        let mut updated = content;

        if !updated.ends_with('\n') {
            updated.push('\n');
        }

        updated.push_str("\nexec savestate // Added by SPLIT 2\n");

        atomic_write(autoexec, updated)
            .map_err(|error| format!("Could not update autoexec.cfg: {error}"))?;
    } else {
        atomic_write(autoexec, "exec savestate // Added by SPLIT 2\n")
            .map_err(|error| format!("Could not create autoexec.cfg: {error}"))?;
    }

    println!("[SPLIT] autoexec.cfg configured");

    Ok(())
}
