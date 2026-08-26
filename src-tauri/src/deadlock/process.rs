use sysinfo::{ProcessesToUpdate, System};

const DEADLOCK_PROCESS_NAME: &str = "deadlock.exe";

pub fn is_deadlock_running() -> bool {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);

    system.processes().values().any(|process| {
        process
            .name()
            .to_string_lossy()
            .eq_ignore_ascii_case(DEADLOCK_PROCESS_NAME)
    })
}
