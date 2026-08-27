use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::{
        mpsc::{self, RecvTimeoutError},
        Mutex,
    },
    thread,
    time::{
        Duration,
        Instant,
    },
};

use notify::{
    Event,
    RecommendedWatcher,
    RecursiveMode,
    Watcher,
};

use tauri::{
    AppHandle,
    Emitter,
    EventTarget,
};

use super::parser::{
    PositionAssembler,
    PositionSnapshot,
};

static LAST_POSITION: Mutex<Option<PositionSnapshot>> =
    Mutex::new(None);

pub fn get_last_position() -> Option<PositionSnapshot> {
    LAST_POSITION
        .lock()
        .ok()?
        .clone()
}

struct PendingSave {
    slot: u8,
    requested_at: Instant,
}

static PENDING_SAVE:
    Mutex<Option<PendingSave>> =
    Mutex::new(None);

pub fn request_save_slot(
    slot: u8,
) -> Result<(), String> {
    if !(1..=8).contains(&slot) {
        return Err(
            format!(
                "Invalid slot {slot}"
            ),
        );
    }

    let mut pending =
        PENDING_SAVE
            .lock()
            .map_err(|_| {
                "Pending save lock poisoned"
                    .to_string()
            })?;

    /*
     * Empêche deux captures de s'écraser.
     */
    if let Some(existing) =
        pending.as_ref()
    {
        if existing
            .requested_at
            .elapsed()
            < Duration::from_secs(2)
        {
            return Err(
                format!(
                    "Slot {} capture is already pending",
                    existing.slot,
                ),
            );
        }
    }

    *pending =
        Some(PendingSave {
            slot,
            requested_at:
                Instant::now(),
        });

    Ok(())
}

pub fn cancel_pending_save() {
    if let Ok(mut pending) =
        PENDING_SAVE.lock()
    {
        *pending = None;
    }
}

pub fn has_pending_save() -> bool {
    let Ok(pending) =
        PENDING_SAVE.lock()
    else {
        return false;
    };

    pending
        .as_ref()
        .is_some_and(
            |request| {
                request
                    .requested_at
                    .elapsed()
                    < Duration::from_secs(2)
            },
        )
}

fn take_pending_save_slot(
) -> Option<u8> {
    let mut pending =
        PENDING_SAVE.lock().ok()?;

    let request =
        pending.take()?;

    /*
     * Si getpos n'arrive pas en 2 sec,
     * on refuse qu'un futur getpos manuel
     * sauvegarde accidentellement le slot.
     */
    if request
        .requested_at
        .elapsed()
        > Duration::from_secs(2)
    {
        println!(
            "[SPLIT] Pending save {} expired",
            request.slot,
        );

        return None;
    }

    Some(request.slot)
}

fn set_last_position(
    position: PositionSnapshot,
) {
    if let Ok(mut last) =
        LAST_POSITION.lock()
    {
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

    fn read_appended(
        &mut self,
    ) -> std::io::Result<Vec<String>> {
        let metadata =
            std::fs::metadata(&self.path)?;

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

        let mut file =
            File::open(&self.path)?;

        file.seek(
            SeekFrom::Start(self.offset),
        )?;

        let mut bytes = Vec::new();

        file.read_to_end(
            &mut bytes,
        )?;

        self.offset =
            file.stream_position()?;

        self.pending.push_str(
            &String::from_utf8_lossy(
                &bytes,
            ),
        );

        let mut lines = Vec::new();

        /*
         * On ne traite que les lignes terminées.
         *
         * Le reste reste dans pending jusqu'à
         * ce que Deadlock écrive le \n.
         */
        while let Some(newline) =
            self.pending.find('\n')
        {
            let mut line = self
                .pending
                .drain(..=newline)
                .collect::<String>();

            let length = line
                .trim_end_matches(
                    ['\r', '\n'],
                )
                .len();

            line.truncate(length);

            lines.push(line);
        }

        Ok(lines)
    }
}

fn event_touches_console(
    event: &Event,
) -> bool {
    event.paths.iter().any(
        |path| {
            path.file_name()
                .and_then(
                    |name| name.to_str(),
                )
                .is_some_and(
                    |name| {
                        name.eq_ignore_ascii_case(
                            "console.log",
                        )
                    },
                )
        },
    )
}

fn process_lines(
    app: &AppHandle,
    lines: Vec<String>,
    assembler: &mut PositionAssembler,
) {
    for line in lines {
        if line.contains("setpos")
            || line.contains("setang")
            || line.contains("getpos")
        {
            println!(
                "[SPLIT] Deadlock console -> {}",
                line
            );
        }

        let Some(position) =
            assembler.push_line(&line)
        else {
            continue;
        };

        println!(
            "[SPLIT] Position parsed -> {}",
            position.to_deadlock_command()
        );

        set_last_position(
            position.clone(),
        );

                /*
         * Si cette position vient d'un
         * Alt+F1..F8, sauvegarder directement
         * dans le slot demandé.
         */
        if let Some(slot) =
            take_pending_save_slot()
        {
            match super::persist_slot_position(
                slot,
                position.clone(),
            ) {
                Ok(saved_slots) => {
                    println!(
                        "[SPLIT] Hotkey save completed: slot {slot}"
                    );

                    if let Err(error) =
                        app.emit_to(
                            EventTarget::webview_window(
                                "main",
                            ),
                            "deadlock-slots",
                            saved_slots,
                        )
                    {
                        eprintln!(
                            "[SPLIT] Could not update slots UI: {error}"
                        );
                    }
                }

                Err(error) => {
                    eprintln!(
                        "[SPLIT] Hotkey save failed for slot {slot}: {error}"
                    );
                }
            }
        }

        match app.emit_to(
            EventTarget::webview_window(
                "main",
            ),
            "deadlock-position",
            position,
        ) {
            Ok(_) => {
                println!(
                    "[SPLIT] Position emitted to UI"
                );
            }

            Err(error) => {
                eprintln!(
                    "[SPLIT] Failed to emit position: {error}"
                );
            }
        }
    }
}

pub fn start(
    app: AppHandle,
    console_log: PathBuf,
) -> Result<(), String> {
    let watch_root = console_log
        .parent()
        .ok_or_else(|| {
            "console.log has no parent directory"
                .to_string()
        })?
        .to_path_buf();

    if !watch_root.is_dir() {
        return Err(
            format!(
                "Deadlock log directory does not exist: {}",
                watch_root.display()
            ),
        );
    }

    thread::Builder::new()
        .name(
            "split-console-watcher"
                .to_string(),
        )
        .spawn(move || {
            let (tx, rx) =
                mpsc::channel::<
                    notify::Result<Event>,
                >();

            let mut watcher:
                RecommendedWatcher =
                match notify::recommended_watcher(
                    move |event| {
                        let _ =
                            tx.send(event);
                    },
                ) {
                    Ok(watcher) =>
                        watcher,

                    Err(error) => {
                        eprintln!(
                            "[SPLIT] Watcher init failed: {error}"
                        );

                        return;
                    }
                };

            if let Err(error) =
                watcher.watch(
                    &watch_root,
                    RecursiveMode::NonRecursive,
                )
            {
                eprintln!(
                    "[SPLIT] Could not watch {}: {error}",
                    watch_root.display()
                );

                return;
            }

            println!(
                "[SPLIT] Watching Deadlock console: {}",
                console_log.display()
            );

            println!(
                "[SPLIT] Safety tail active: 100 ms"
            );

            let mut tail =
                ConsoleTail::new(
                    console_log.clone(),
                );

            let mut assembler =
                PositionAssembler::default();

            /*
             * notify reste la méthode principale.
             *
             * recv_timeout ajoute seulement un
             * filet de sécurité toutes les 100 ms.
             */
            loop {
                match rx.recv_timeout(
                    Duration::from_millis(100),
                ) {
                    /*
                     * notify a vu une modification :
                     * lecture immédiate.
                     */
                    Ok(Ok(event)) => {
                        if !event_touches_console(
                            &event,
                        ) {
                            continue;
                        }

                        if let Ok(lines) =
                            tail.read_appended()
                        {
                            process_lines(
                                &app,
                                lines,
                                &mut assembler,
                            );
                        }
                    }

                    /*
                     * notify n'a rien envoyé depuis
                     * 100 ms.
                     *
                     * On vérifie simplement si la
                     * taille du fichier a changé.
                     */
                    Err(
                        RecvTimeoutError::Timeout,
                    ) => {
                        if let Ok(lines) =
                            tail.read_appended()
                        {
                            if !lines.is_empty() {
                                process_lines(
                                    &app,
                                    lines,
                                    &mut assembler,
                                );
                            }
                        }
                    }

                    /*
                     * Le watcher natif a disparu.
                     */
                    Err(
                        RecvTimeoutError::Disconnected,
                    ) => {
                        eprintln!(
                            "[SPLIT] Console watcher disconnected"
                        );

                        break;
                    }

                    /*
                     * Erreur notify ponctuelle.
                     * Le fallback continue.
                     */
                    Ok(Err(error)) => {
                        eprintln!(
                            "[SPLIT] notify error: {error}"
                        );
                    }
                }
            }
        })
        .map_err(
            |error| {
                format!(
                    "failed to spawn console watcher: {error}"
                )
            },
        )?;

    Ok(())
}