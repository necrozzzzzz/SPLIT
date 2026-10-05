use std::{
    sync::{mpsc, Mutex},
    thread::{self, JoinHandle},
    time::Duration,
};

use tauri::AppHandle;

struct ProcessMonitorRuntime {
    stop: mpsc::Sender<()>,
    thread: JoinHandle<()>,
}

static PROCESS_MONITOR: Mutex<Option<ProcessMonitorRuntime>> = Mutex::new(None);

#[derive(Debug, Default)]
struct TransitionDetector {
    previous_pid: Option<u32>,
    has_observed: bool,
}

impl TransitionDetector {
    fn observe(&mut self, pid: Option<u32>) -> Option<ProcessTransition> {
        let initial_observation = !self.has_observed;
        self.has_observed = true;

        if self.previous_pid == pid {
            return None;
        }

        let previous_pid = self.previous_pid;
        self.previous_pid = pid;

        match pid {
            Some(pid) => Some(ProcessTransition::Started {
                pid,
                initial_observation,
            }),
            None if previous_pid.is_some() => Some(ProcessTransition::Stopped),
            None => None,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum ProcessTransition {
    Started { pid: u32, initial_observation: bool },
    Stopped,
}

fn should_focus_for_cfg_reload(initial_observation: bool, focus_on_startup: bool) -> bool {
    !initial_observation || focus_on_startup
}

fn apply_transition(app: &AppHandle, transition: ProcessTransition) {
    if let ProcessTransition::Started {
        pid,
        initial_observation,
    } = transition
    {
        println!("[SPLIT][Deadlock] new game session detected (PID {pid}), applying savestate.cfg");

        super::interfaces::initialize_schema_runtime(pid);

        if let Err(error) = super::repair_integration_on_startup() {
            eprintln!("[SPLIT] Deadlock start integration repair failed: {error}");
        }

        let focus_on_startup = super::get_focus_deadlock_on_startup();
        if should_focus_for_cfg_reload(initial_observation, focus_on_startup) {
            if let Err(error) = super::hotkeys::reload_savestate_cfg_now() {
                eprintln!("[SPLIT] Deadlock start CFG application failed: {error}");
            } else {
                println!("[SPLIT][Deadlock] savestate.cfg applied");
            }
        } else {
            println!(
                "[SPLIT][Deadlock] startup CFG activation deferred: startup focus is disabled"
            );
        }

        if !super::watcher::is_running() {
            if let Err(error) = super::start_console_watcher(app.clone()) {
                eprintln!("[SPLIT] Deadlock start console watcher failed: {error}");
            }
        }

        for _ in 0..30 {
            if super::watcher::is_running() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
    } else if let Err(error) = super::watcher::stop() {
        eprintln!("[SPLIT] Deadlock stop console watcher failed: {error}");
    }

    crate::ui::emit_to_main_if_present(app, "deadlock-status-changed", super::get_status());
}

pub fn start(app: AppHandle) -> Result<(), String> {
    let mut runtime = PROCESS_MONITOR
        .lock()
        .map_err(|_| "Process monitor lock poisoned".to_string())?;
    if runtime.is_some() {
        return Ok(());
    }

    let (stop_tx, stop_rx) = mpsc::channel();
    let handle = thread::Builder::new()
        .name("split-deadlock-process-monitor".to_string())
        .spawn(move || {
            let mut detector = TransitionDetector::default();

            if let Some(transition) = detector.observe(super::process::deadlock_pid()) {
                apply_transition(&app, transition);
            }

            loop {
                match stop_rx.recv_timeout(Duration::from_millis(1_500)) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if let Some(transition) = detector.observe(super::process::deadlock_pid()) {
                            println!("[SPLIT] Deadlock process transition: {transition:?}");
                            apply_transition(&app, transition);
                        }
                    }
                }
            }
        })
        .map_err(|error| format!("Could not start Deadlock process monitor: {error}"))?;

    *runtime = Some(ProcessMonitorRuntime {
        stop: stop_tx,
        thread: handle,
    });
    Ok(())
}

pub fn stop() -> Result<(), String> {
    let runtime = PROCESS_MONITOR
        .lock()
        .map_err(|_| "Process monitor lock poisoned".to_string())?
        .take();
    if let Some(runtime) = runtime {
        let _ = runtime.stop.send(());
        runtime
            .thread
            .join()
            .map_err(|_| "Deadlock process monitor panicked while stopping".to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_transitions_emit_once_per_pid() {
        let mut detector = TransitionDetector::default();
        assert_eq!(detector.observe(None), None);
        assert_eq!(
            detector.observe(Some(41)),
            Some(ProcessTransition::Started {
                pid: 41,
                initial_observation: false,
            })
        );
        assert_eq!(detector.observe(Some(41)), None);
        assert_eq!(
            detector.observe(Some(84)),
            Some(ProcessTransition::Started {
                pid: 84,
                initial_observation: false,
            })
        );
        assert_eq!(detector.observe(None), Some(ProcessTransition::Stopped));
        assert_eq!(detector.observe(None), None);
    }

    #[test]
    fn initial_running_process_is_marked_as_startup_observation() {
        let mut detector = TransitionDetector::default();
        assert_eq!(
            detector.observe(Some(41)),
            Some(ProcessTransition::Started {
                pid: 41,
                initial_observation: true,
            })
        );
    }

    #[test]
    fn startup_focus_decision_only_gates_the_initial_observation() {
        assert!(!should_focus_for_cfg_reload(true, false));
        assert!(should_focus_for_cfg_reload(true, true));
        assert!(should_focus_for_cfg_reload(false, false));
        assert!(should_focus_for_cfg_reload(false, true));
    }
}
