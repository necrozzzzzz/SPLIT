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
    previous: Option<bool>,
}

impl TransitionDetector {
    fn observe(&mut self, running: bool) -> Option<bool> {
        match self.previous.replace(running) {
            Some(previous) if previous != running => Some(running),
            _ => None,
        }
    }
}

fn apply_transition(app: &AppHandle, running: bool) {
    if running {
        if let Err(error) = super::repair_integration_on_startup() {
            eprintln!("[SPLIT] Deadlock start integration repair failed: {error}");
        }
        if let Err(error) = super::hotkeys::apply_generated_cfg_now() {
            eprintln!("[SPLIT] Deadlock start CFG application failed: {error}");
        }
        if let Err(error) = super::start_console_watcher(app.clone()) {
            eprintln!("[SPLIT] Deadlock start console watcher failed: {error}");
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
            let _ = detector.observe(super::process::is_deadlock_running());

            loop {
                match stop_rx.recv_timeout(Duration::from_millis(1_500)) {
                    Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if let Some(running) =
                            detector.observe(super::process::is_deadlock_running())
                        {
                            println!(
                                "[SPLIT] Deadlock process transition: {}",
                                if running { "running" } else { "stopped" }
                            );
                            apply_transition(&app, running);
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
    fn process_transitions_emit_once_per_change() {
        let mut detector = TransitionDetector::default();
        assert_eq!(detector.observe(false), None);
        assert_eq!(detector.observe(false), None);
        assert_eq!(detector.observe(true), Some(true));
        assert_eq!(detector.observe(true), None);
        assert_eq!(detector.observe(false), Some(false));
        assert_eq!(detector.observe(false), None);
    }
}
