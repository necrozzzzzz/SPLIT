use std::path::PathBuf;

use sysinfo::{ProcessesToUpdate, System};

const DEADLOCK_PROCESS_NAME: &str = "deadlock.exe";

fn system_processes() -> System {
    let mut system = System::new();

    system.refresh_processes(ProcessesToUpdate::All, true);

    system
}

fn is_deadlock_process(process: &sysinfo::Process) -> bool {
    process
        .name()
        .to_string_lossy()
        .eq_ignore_ascii_case(DEADLOCK_PROCESS_NAME)
}

pub(crate) fn deadlock_pid() -> Option<u32> {
    let system = system_processes();

    system.processes().iter().find_map(|(pid, process)| {
        if is_deadlock_process(process) {
            Some(pid.as_u32())
        } else {
            None
        }
    })
}

pub fn is_deadlock_running() -> bool {
    let system = system_processes();

    system.processes().values().any(is_deadlock_process)
}

pub fn running_deadlock_root() -> Option<PathBuf> {
    let system = system_processes();

    for process in system.processes().values() {
        if !is_deadlock_process(process) {
            continue;
        }

        let Some(exe) = process.exe() else {
            continue;
        };

        /*
         * Exemple :
         *
         * Deadlock
         * └── game
         *     └── bin
         *         └── win64
         *             └── deadlock.exe
         */

        let Some(root) = exe
            .parent()
            .and_then(|path| path.parent())
            .and_then(|path| path.parent())
            .and_then(|path| path.parent())
        else {
            continue;
        };

        if root
            .join("game")
            .join("bin")
            .join("win64")
            .join("deadlock.exe")
            .is_file()
        {
            return Some(root.to_path_buf());
        }
    }

    None
}
