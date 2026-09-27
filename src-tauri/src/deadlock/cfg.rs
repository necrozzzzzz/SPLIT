use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Mutex, OnceLock,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use super::parser::PositionSnapshot;
use crate::storage::atomic_write;

const LOAD_TRANSPORT_KEYS: [&str; 8] = [
    "F15", "F16", "F17", "F18", "F19", "F20", "F21", "F22",
];
const LEGACY_TRANSPORT_BINDS: [&str; 9] = [
    "bind \"h\" \"savestate_getpos\"",
    "bind \"u\" \"exec savestate; load_slot_1\"",
    "bind \"i\" \"exec savestate; load_slot_2\"",
    "bind \"o\" \"exec savestate; load_slot_3\"",
    "bind \"j\" \"exec savestate; load_slot_4\"",
    "bind \"k\" \"exec savestate; load_slot_5\"",
    "bind \"l\" \"exec savestate; load_slot_6\"",
    "bind \"n\" \"exec savestate; load_slot_7\"",
    "bind \"m\" \"exec savestate; load_slot_8\"",
];

pub(crate) const PREPARE_BIND: &str =
    "bind \"F13\" \"exec savestate; exec savestate_prepare\"";
pub(crate) const PRESENTATION_RESUME_BIND: &str = "bind \"F24\" \"r_force_no_present 0\"";
pub(crate) const MOMENTUM_RESET_BIND: &str = "bind \"F14\" \"ent_fire !self addmodifier modifier_citadel_root; ent_fire !self removemodifier modifier_citadel_root\"";
pub(crate) const LEGACY_MOMENTUM_RESET_BIND: &str = "bind \"F9\" \"ent_fire !self addmodifier modifier_citadel_root; ent_fire !self removemodifier modifier_citadel_root\"";

static TELEPORT_GENERATION: AtomicU64 = AtomicU64::new(0);

static TELEPORTS_DIRTY: AtomicBool = AtomicBool::new(false);
static PENDING_CYCLE_BIND_CLEANUP: Mutex<Option<(PathBuf, String)>> = Mutex::new(None);

pub(crate) fn teleports_dirty() -> bool {
    TELEPORTS_DIRTY.load(Ordering::SeqCst)
}

pub(crate) fn mark_teleports_prepared() {
    TELEPORTS_DIRTY.store(false, Ordering::SeqCst);
    let pending = PENDING_CYCLE_BIND_CLEANUP
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take();
    if let Some((path, cleanup_line)) = pending {
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(250));
            let Ok(content) = fs::read_to_string(&path) else {
                return;
            };
            let mut updated = content
                .lines()
                .filter(|line| line.trim() != cleanup_line)
                .collect::<Vec<_>>()
                .join("\n");
            updated.push('\n');
            if let Err(error) = atomic_write(&path, updated) {
                eprintln!("[SPLIT] Could not finish legacy Cycle Preset bind cleanup: {error}");
            }
        });
    }
}

fn owned_cycle_bind_key(content: &str) -> Option<String> {
    if !content.starts_with("// SPLIT 2 - auto-generated") {
        return None;
    }
    content
        .lines()
        .take_while(|line| !line.trim().starts_with("alias \"savestate_getpos\""))
        .find_map(|line| {
            let remainder = line.trim().strip_prefix("bind \"")?;
            let (key, _) = remainder.split_once("\" \"")?;
            (!key.is_empty() && !key.contains('"')).then(|| key.to_string())
        })
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

    let previous_cfg = fs::read_to_string(cfg_file).ok();
    let legacy_transport_cleanup = previous_cfg.as_ref().is_some_and(|content| {
            LEGACY_TRANSPORT_BINDS
                .iter()
                .any(|binding| content.contains(binding))
                || content.contains("bind \"F10\" \"r_force_no_present 0\"")
        });
    let owned_cycle_key = previous_cfg.as_deref().and_then(owned_cycle_bind_key);

    let mut output = String::new();

    let namespace = teleport_namespace();

    let prepare_cfg_path = parent.join("savestate_prepare.cfg");

    let mut prepare_output = String::from(
        "// SPLIT 2 - prepared teleport points\n\n\
        // Remove stale SPLIT teleport points before creating the new generation.\n\
        ent_fire split_tp_* Kill\n\n",
    );

    output.push_str("// SPLIT 2 - auto-generated, do not edit manually\n\n");

    if legacy_transport_cleanup {
        output.push_str(
            "// One-time migration away from legacy letter transports.\n\
             unbind \"h\"\n\
             unbind \"u\"\n\
             unbind \"i\"\n\
             unbind \"o\"\n\
             unbind \"j\"\n\
             unbind \"k\"\n\
             unbind \"l\"\n\
             unbind \"n\"\n\
             unbind \"m\"\n\
             unbind \"F10\"\n\n",
        );
    }

    if let Some(key) = &owned_cycle_key {
        output.push_str("// Remove the obsolete SPLIT-owned Cycle Preset bind once.\n");
        output.push_str(&format!("unbind \"{key}\"\n\n"));
    }

    /*
     * Position capture transport.
     */
    output.push_str("alias \"savestate_getpos\" \"exec savestate; getpos_exact\"\n");

    output.push_str("bind \"F23\" \"savestate_getpos\"\n\n");

    /*
     * Transports internes SPLIT.
     *
     * F13 est une touche virtuelle interne utilisée
     * uniquement pour préparer les point_teleport.
     *
     * F14 injecté par SPLIT réinitialise le momentum
     * après un vrai Load.
     *
     * F24 injecté par SPLIT réactive la présentation
     * après le masque d'un Load.
     *
     * F10 physique reste Redo grâce au hook SPLIT.
     * F11 physique reste Favorite Mode.
     * F12 n'est jamais bindé côté Deadlock : le défaut Cycle Preset
     * est intercepté directement par SPLIT.
     */
    output.push_str(
        "bind \"F13\" \"exec savestate; exec savestate_prepare\"\n\
        bind \"F24\" \"r_force_no_present 0\"\n\
        bind \"F14\" \"ent_fire !self addmodifier modifier_citadel_root; ent_fire !self removemodifier modifier_citadel_root\"\n\n",
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

    *PENDING_CYCLE_BIND_CLEANUP
        .lock()
        .unwrap_or_else(|error| error.into_inner()) = owned_cycle_key
        .map(|key| (cfg_file.to_path_buf(), format!("unbind \"{key}\"")));

    println!("[SPLIT] savestate.cfg updated: {}", cfg_file.display(),);

    TELEPORTS_DIRTY.store(true, Ordering::SeqCst);

    Ok(())
}

fn shutdown_cfg_contents() -> String {
    let mut output = String::from(
        "// SPLIT 2 - transport disabled after application shutdown\n\n\
         unbind \"h\"\n\
         unbind \"u\"\n\
         unbind \"i\"\n\
         unbind \"o\"\n\
         unbind \"j\"\n\
         unbind \"k\"\n\
         unbind \"l\"\n\
         unbind \"n\"\n\
         unbind \"m\"\n\
         unbind \"F10\"\n\
         unbind \"F13\"\n\
         unbind \"F14\"\n\
         unbind \"F15\"\n\
         unbind \"F16\"\n\
         unbind \"F17\"\n\
         unbind \"F18\"\n\
         unbind \"F19\"\n\
         unbind \"F20\"\n\
         unbind \"F21\"\n\
         unbind \"F22\"\n\
         unbind \"F23\"\n\
         unbind \"F24\"\n",
    );
    output.push_str(
        "alias \"savestate_getpos\" \"\"\n",
    );
    for slot in 1..=8 {
        output.push_str(&format!("alias \"load_slot_{slot}\" \"\"\n"));
    }
    output.push_str(
        "ent_fire split_tp_* Kill\n\
         r_force_no_present 0\n\
         bind \"F13\" \"exec savestate; exec savestate_prepare\"\n",
    );
    output
}

pub(crate) fn write_shutdown_prepare(cfg_file: &Path) -> Result<(), String> {
    let parent = cfg_file
        .parent()
        .ok_or_else(|| "savestate.cfg has no parent directory".to_string())?;
    let path = parent.join("savestate_prepare.cfg");
    atomic_write(&path, shutdown_cfg_contents())
        .map_err(|error| format!("Could not write shutdown prepare CFG: {error}"))
}

pub(crate) fn write_shutdown_main(cfg_file: &Path) -> Result<(), String> {
    atomic_write(cfg_file, shutdown_cfg_contents())
        .map_err(|error| format!("Could not write shutdown savestate.cfg: {error}"))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_cfg_uses_only_f13_through_f24_for_internal_transport() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "split-cfg-momentum-test-{}-{unique}",
            std::process::id()
        ));
        let cfg_file = directory.join("savestate.cfg");

        let empty_slots: [Option<PositionSnapshot>; 8] = std::array::from_fn(|_| None);
        write_savestate_cfg(&cfg_file, &empty_slots).unwrap();
        let content = fs::read_to_string(&cfg_file).unwrap();

        assert!(content.contains(MOMENTUM_RESET_BIND));
        assert!(!content.contains(LEGACY_MOMENTUM_RESET_BIND));
        assert!(!content.contains("bind \"F9\""));
        assert!(content.contains(PREPARE_BIND));
        assert!(content.contains(PRESENTATION_RESUME_BIND));
        assert!(content.contains("bind \"F23\" \"savestate_getpos\""));
        assert!(!content.contains("bind \"F12\""));
        for (index, key) in LOAD_TRANSPORT_KEYS.iter().enumerate() {
            assert!(content.contains(&format!(
                "bind \"{key}\" \"exec savestate; load_slot_{}\"",
                index + 1
            )));
        }
        for key in ["h", "u", "i", "o", "j", "k", "l", "n", "m"] {
            assert!(!content.contains(&format!("bind \"{key}\"")));
        }

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn shutdown_cfg_neutralizes_every_split_transport() {
        let content = shutdown_cfg_contents();
        for key in [
            "h", "u", "i", "o", "j", "k", "l", "n", "m", "F10", "F13", "F14", "F15",
            "F16", "F17", "F18", "F19", "F20", "F21", "F22", "F23", "F24",
        ] {
            assert!(content.contains(&format!("unbind \"{key}\"")));
        }
        for slot in 1..=8 {
            assert!(content.contains(&format!("alias \"load_slot_{slot}\" \"\"")));
        }
        assert!(content.contains("ent_fire split_tp_* Kill"));
        assert!(content.contains("r_force_no_present 0"));
        assert!(content.ends_with("bind \"F13\" \"exec savestate; exec savestate_prepare\"\n"));
    }
}
