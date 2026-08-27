use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::{mpsc, Mutex},
    thread,
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
        let offset = std::fs::metadata(&path)
            .map(|meta| meta.len())
            .unwrap_or(0);

        Self {
            path,
            offset,
            pending: String::new(),
        }
    }

    fn read_appended(&mut self) -> std::io::Result<Vec<String>> {
        let metadata = std::fs::metadata(&self.path)?;

        // Deadlock peut recréer/tronquer console.log.
        if metadata.len() < self.offset {
            self.offset = 0;
            self.pending.clear();
        }

        if metadata.len() == self.offset {
            return Ok(Vec::new());
        }

        let mut file = File::open(&self.path)?;
        file.seek(SeekFrom::Start(self.offset))?;

        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;

        self.offset = file.stream_position()?;

        self.pending
            .push_str(&String::from_utf8_lossy(&bytes));

        let mut lines = Vec::new();

        while let Some(newline) = self.pending.find('\n') {
            let mut line = self
                .pending
                .drain(..=newline)
                .collect::<String>();

            let trimmed_length = line
                .trim_end_matches(['\r', '\n'])
                .len();

            line.truncate(trimmed_length);

            lines.push(line);
        }

        /*
         * Certains programmes écrivent une ligne complète avant
         * d'ajouter le retour à la ligne.
         *
         * On expose donc aussi temporairement le buffer courant
         * sans le vider.
         */
        if !self.pending.trim().is_empty() {
            lines.push(self.pending.clone());
        }

        Ok(lines)
    }
}

fn event_touches_console(event: &Event) -> bool {
    event.paths.iter().any(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.eq_ignore_ascii_case("console.log")
            })
    })
}

pub fn start(
    app: AppHandle,
    console_log: PathBuf,
) -> Result<(), String> {
    let watch_root = console_log
        .parent()
        .ok_or_else(|| {
            "console.log has no parent directory".to_string()
        })?
        .to_path_buf();

    if !watch_root.is_dir() {
        return Err(format!(
            "Deadlock log directory does not exist: {}",
            watch_root.display()
        ));
    }

    thread::Builder::new()
        .name("split-console-watcher".to_string())
        .spawn(move || {
            let (tx, rx) =
                mpsc::channel::<notify::Result<Event>>();

            let mut watcher: RecommendedWatcher =
                match notify::recommended_watcher(
                    move |event| {
                        let _ = tx.send(event);
                    },
                ) {
                    Ok(watcher) => watcher,

                    Err(error) => {
                        eprintln!(
                            "SPLIT console watcher failed to initialize: {error}"
                        );
                        return;
                    }
                };

            if let Err(error) = watcher.watch(
                &watch_root,
                RecursiveMode::NonRecursive,
            ) {
                eprintln!(
                    "SPLIT could not watch {}: {error}",
                    watch_root.display()
                );

                return;
            }

            println!(
                "[SPLIT] Watching Deadlock console: {}",
                console_log.display()
            );

            let mut tail =
                ConsoleTail::new(console_log.clone());

            let mut position_assembler =
                PositionAssembler::default();

            for event in rx {
                let Ok(event) = event else {
                    continue;
                };

                /*
                 * Avant on exigeait que le chemin remonté par
                 * notify soit strictement identique au PathBuf
                 * original.
                 *
                 * Sous Windows ce n'est pas toujours garanti.
                 */
                if !event_touches_console(&event) {
                    continue;
                }

                let lines = match tail.read_appended() {
                    Ok(lines) => lines,

                    Err(error) => {
                        eprintln!(
                            "[SPLIT] Failed reading console.log: {error}"
                        );

                        continue;
                    }
                };

                for line in lines {
                    /*
                     * Diagnostic temporaire :
                     * on n'affiche que les lignes intéressantes.
                     */
                    if line.contains("setpos")
                        || line.contains("setang")
                        || line.contains("getpos")
                    {
                        println!(
                            "[SPLIT] Deadlock console -> {}",
                            line
                        );
                    }

                    if let Some(position) =
    position_assembler.push_line(&line)
{
    println!(
        "[SPLIT] Position parsed -> {}",
        position.to_deadlock_command()
    );

    /*
     * Toujours conserver la dernière position
     * côté Rust.
     */
    set_last_position(
        position.clone(),
    );

    /*
     * Envoyer explicitement l'événement
     * à la WebView principale.
     */
    match app.emit_to(
        EventTarget::webview_window("main"),
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
            }
        })
        .map_err(|error| {
            format!(
                "failed to spawn console watcher: {error}"
            )
        })?;

    Ok(())
}