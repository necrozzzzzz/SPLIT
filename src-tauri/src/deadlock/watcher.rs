use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, RecvTimeoutError},
        Mutex,
    },
    thread,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

use serde::Serialize;
use tauri::AppHandle;

use super::{
    mouse_tracker::MouseSnapshot,
    parser::{PositionAssembler, PositionSnapshot},
};

static LAST_POSITION: Mutex<Option<PositionSnapshot>> = Mutex::new(None);

pub fn get_last_position() -> Option<PositionSnapshot> {
    LAST_POSITION.lock().ok()?.clone()
}

struct PendingSave {
    slot: u8,
    generation: u64,
    requested_at: Instant,

    /*
     * La caméra est capturée immédiatement
     * au Alt+F1..F8, pas lorsque console.log
     * répond quelques millisecondes après.
     */
    camera: MouseSnapshot,
}

static PENDING_SAVE: Mutex<Option<PendingSave>> = Mutex::new(None);
static SAVE_GENERATION: AtomicU64 = AtomicU64::new(0);
const SAVE_TIMEOUT: Duration = Duration::from_secs(2);

struct WatcherRuntime {
    stop: mpsc::Sender<()>,
    thread: JoinHandle<()>,
}

static WATCHER_RUNTIME: Mutex<Option<WatcherRuntime>> = Mutex::new(None);

pub fn stop() -> Result<(), String> {
    let previous = WATCHER_RUNTIME
        .lock()
        .map_err(|_| "Console watcher manager lock poisoned".to_string())?
        .take();

    if let Some(previous) = previous {
        let _ = previous.stop.send(());
        previous
            .thread
            .join()
            .map_err(|_| "Console watcher panicked while stopping".to_string())?;
    }

    Ok(())
}

#[derive(Clone, Serialize)]
pub struct SaveFailedPayload {
    slot: u8,
    reason: String,
}

pub(crate) fn report_save_failed(app: &AppHandle, slot: u8, reason: impl Into<String>) {
    let payload = SaveFailedPayload {
        slot,
        reason: reason.into(),
    };

    crate::ui::emit_to_main_if_present(app, "deadlock-save-failed", payload);
    crate::notifications::show(crate::notifications::Notification::SaveFailed);
}

fn take_expired_generation(pending: &mut Option<PendingSave>, generation: u64) -> Option<u8> {
    if pending.as_ref().is_some_and(|request| {
        request.generation == generation && request.requested_at.elapsed() >= SAVE_TIMEOUT
    }) {
        pending.take().map(|request| request.slot)
    } else {
        None
    }
}

pub fn request_save_slot(app: AppHandle, slot: u8) -> Result<u64, String> {
    if !(1..=8).contains(&slot) {
        return Err(format!("Invalid slot {slot}"));
    }

    let camera = super::mouse_tracker::snapshot();

    let mut pending = PENDING_SAVE
        .lock()
        .map_err(|_| "Pending save lock poisoned".to_string())?;

    /*
     * Empêche deux captures de s'écraser.
     */
    if let Some(existing) = pending.as_ref() {
        if existing.requested_at.elapsed() < SAVE_TIMEOUT {
            return Err(format!("Slot {} capture is already pending", existing.slot,));
        }
    }

    let generation = SAVE_GENERATION.fetch_add(1, Ordering::Relaxed) + 1;
    *pending = Some(PendingSave {
        slot,
        generation,
        requested_at: Instant::now(),
        camera,
    });
    drop(pending);

    thread::spawn(move || {
        thread::sleep(SAVE_TIMEOUT);

        let expired_slot = PENDING_SAVE
            .lock()
            .ok()
            .and_then(|mut pending| take_expired_generation(&mut pending, generation));

        if let Some(expired_slot) = expired_slot {
            let reason = "Timed out waiting for Deadlock getpos_exact response";
            eprintln!("[SPLIT] Save {expired_slot} failed: {reason}");
            report_save_failed(&app, expired_slot, reason);
        }
    });

    Ok(generation)
}

pub fn cancel_pending_save(generation: u64) {
    if let Ok(mut pending) = PENDING_SAVE.lock() {
        if pending
            .as_ref()
            .is_some_and(|request| request.generation == generation)
        {
            *pending = None;
        }
    }
}

pub fn has_pending_save() -> bool {
    let Ok(pending) = PENDING_SAVE.lock() else {
        return false;
    };

    pending
        .as_ref()
        .is_some_and(|request| request.requested_at.elapsed() < SAVE_TIMEOUT)
}

enum PendingSaveResult {
    None,

    Ready { slot: u8, camera: MouseSnapshot },

    Expired(u8),
}

fn take_pending_save_slot() -> PendingSaveResult {
    let Ok(mut pending) = PENDING_SAVE.lock() else {
        return PendingSaveResult::None;
    };

    let Some(request) = pending.take() else {
        return PendingSaveResult::None;
    };

    /*
     * Si getpos n'arrive pas en 2 sec,
     * on refuse qu'un futur getpos manuel
     * sauvegarde accidentellement le slot.
     */
    if request.requested_at.elapsed() > SAVE_TIMEOUT {
        println!("[SPLIT] Pending save {} expired", request.slot,);

        return PendingSaveResult::Expired(request.slot);
    }

    PendingSaveResult::Ready {
        slot: request.slot,
        camera: request.camera,
    }
}

fn set_last_position(position: PositionSnapshot) {
    if let Ok(mut last) = LAST_POSITION.lock() {
        *last = Some(position);
    }
}

struct ConsoleTail {
    path: PathBuf,
    offset: u64,
    pending: String,
}

impl ConsoleTail {
    fn new(path: PathBuf) -> Self {
        /*
         * On démarre à la fin du fichier :
         * aucune ancienne ligne n'est reparsée.
         */
        let offset = std::fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);

        Self {
            path,
            offset,
            pending: String::new(),
        }
    }

    fn read_appended(&mut self) -> std::io::Result<Vec<String>> {
        let metadata = std::fs::metadata(&self.path)?;

        /*
         * Deadlock peut tronquer/recréer console.log.
         */
        if metadata.len() < self.offset {
            self.offset = 0;
            self.pending.clear();
        }

        /*
         * Rien de nouveau.
         *
         * Très important :
         * dans ce cas aucune ouverture/lecture
         * du fichier n'est effectuée.
         */
        if metadata.len() == self.offset {
            return Ok(Vec::new());
        }

        let mut file = File::open(&self.path)?;

        file.seek(SeekFrom::Start(self.offset))?;

        let mut bytes = Vec::new();

        file.read_to_end(&mut bytes)?;

        self.offset = file.stream_position()?;

        self.pending.push_str(&String::from_utf8_lossy(&bytes));

        let mut lines = Vec::new();

        /*
         * On ne traite que les lignes terminées.
         *
         * Le reste reste dans pending jusqu'à
         * ce que Deadlock écrive le \n.
         */
        while let Some(newline) = self.pending.find('\n') {
            let mut line = self.pending.drain(..=newline).collect::<String>();

            let length = line.trim_end_matches(['\r', '\n']).len();

            line.truncate(length);

            lines.push(line);
        }

        Ok(lines)
    }
}

fn event_touches_console(event: &Event) -> bool {
    event.paths.iter().any(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.eq_ignore_ascii_case("console.log"))
    })
}

fn process_lines(app: &AppHandle, lines: Vec<String>, assembler: &mut PositionAssembler) {
    for line in lines {
        if line.contains("setpos") || line.contains("setang") || line.contains("getpos") {
            println!("[SPLIT] Deadlock console -> {}", line);
        }

        let Some(position) = assembler.push_line(&line) else {
            continue;
        };

        println!(
            "[SPLIT] Position parsed -> {}",
            position.to_deadlock_command()
        );

        set_last_position(position.clone());

        /*
         * Si cette position vient d'un
         * Alt+F1..F8, sauvegarder directement
         * dans le slot demandé.
         */
        match take_pending_save_slot() {
            PendingSaveResult::Ready { slot, camera } => {
                let mut saved_position = position.clone();

                saved_position.camera = Some(camera);

                println!(
                    "[SPLIT] Camera captured for slot {slot}: session={} X={} Y={}",
                    camera.session_id, camera.x, camera.y,
                );

                match super::persist_slot_position(slot, saved_position) {
                    Ok(saved) => {
                        println!("[SPLIT] Hotkey save completed: slot {slot}");

                        crate::notifications::show(crate::notifications::Notification::SlotSaved {
                            slot,
                            favorite: super::favorite_mode_for_bank(saved.bank),
                        });

                        crate::ui::emit_to_main_if_present(app, "deadlock-slots", saved.slots);
                        if saved.history_changed {
                            super::emit_history_state(app, saved.history_state);
                        }
                    }

                    Err(error) => {
                        eprintln!("[SPLIT] Hotkey save failed for slot {slot}: {error}");
                        report_save_failed(app, slot, error);
                    }
                }
            }
            PendingSaveResult::Expired(slot) => {
                report_save_failed(
                    app,
                    slot,
                    "Timed out waiting for Deadlock getpos_exact response",
                );
            }
            PendingSaveResult::None => {}
        }

        crate::ui::emit_to_main_if_present(app, "deadlock-position", position);
    }
}

pub fn start(app: AppHandle, console_log: PathBuf) -> Result<(), String> {
    let watch_root = console_log
        .parent()
        .ok_or_else(|| "console.log has no parent directory".to_string())?
        .to_path_buf();

    if !watch_root.is_dir() {
        return Err(format!(
            "Deadlock log directory does not exist: {}",
            watch_root.display()
        ));
    }

    let mut runtime = WATCHER_RUNTIME
        .lock()
        .map_err(|_| "Console watcher manager lock poisoned".to_string())?;

    if let Some(previous) = runtime.take() {
        let _ = previous.stop.send(());
        previous
            .thread
            .join()
            .map_err(|_| "Previous console watcher panicked while stopping".to_string())?;
    }

    let (tx, rx) = mpsc::channel::<notify::Result<Event>>();
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |event| {
        let _ = tx.send(event);
    })
    .map_err(|error| format!("Watcher init failed: {error}"))?;

    watcher
        .watch(&watch_root, RecursiveMode::NonRecursive)
        .map_err(|error| format!("Could not watch {}: {error}", watch_root.display()))?;

    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let handle = thread::Builder::new()
        .name("split-console-watcher".to_string())
        .spawn(move || {
            let _watcher = watcher;

            println!(
                "[SPLIT] Watching Deadlock console: {}",
                console_log.display()
            );

            println!("[SPLIT] Safety tail active: 100 ms");

            let mut tail = ConsoleTail::new(console_log.clone());

            let mut assembler = PositionAssembler::default();

            /*
             * notify reste la méthode principale.
             *
             * recv_timeout ajoute seulement un
             * filet de sécurité toutes les 100 ms.
             */
            loop {
                if stop_rx.try_recv().is_ok() {
                    println!("[SPLIT] Console watcher stopped");
                    break;
                }

                match rx.recv_timeout(Duration::from_millis(100)) {
                    /*
                     * notify a vu une modification :
                     * lecture immédiate.
                     */
                    Ok(Ok(event)) => {
                        if !event_touches_console(&event) {
                            continue;
                        }

                        if let Ok(lines) = tail.read_appended() {
                            process_lines(&app, lines, &mut assembler);
                        }
                    }

                    /*
                     * notify n'a rien envoyé depuis
                     * 100 ms.
                     *
                     * On vérifie simplement si la
                     * taille du fichier a changé.
                     */
                    Err(RecvTimeoutError::Timeout) => {
                        if let Ok(lines) = tail.read_appended() {
                            if !lines.is_empty() {
                                process_lines(&app, lines, &mut assembler);
                            }
                        }
                    }

                    /*
                     * Le watcher natif a disparu.
                     */
                    Err(RecvTimeoutError::Disconnected) => {
                        eprintln!("[SPLIT] Console watcher disconnected");

                        break;
                    }

                    /*
                     * Erreur notify ponctuelle.
                     * Le fallback continue.
                     */
                    Ok(Err(error)) => {
                        eprintln!("[SPLIT] notify error: {error}");
                    }
                }
            }
        })
        .map_err(|error| format!("failed to spawn console watcher: {error}"))?;

    *runtime = Some(WatcherRuntime {
        stop: stop_tx,
        thread: handle,
    });

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_old_timeout_cannot_cancel_a_newer_capture() {
        let mut pending = Some(PendingSave {
            slot: 2,
            generation: 12,
            requested_at: Instant::now() - SAVE_TIMEOUT,
            camera: MouseSnapshot::default(),
        });

        assert_eq!(take_expired_generation(&mut pending, 11), None);
        assert_eq!(pending.as_ref().map(|request| request.generation), Some(12));
        assert_eq!(take_expired_generation(&mut pending, 12), Some(2));
        assert!(pending.is_none());
    }
}
