use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
};

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use tauri::{AppHandle, Emitter};

use super::parser::parse_position;

struct ConsoleTail {
    path: PathBuf,
    offset: u64,
    pending: String,
}

impl ConsoleTail {
    fn new(path: PathBuf) -> Self {
        let offset = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);

        Self {
            path,
            offset,
            pending: String::new(),
        }
    }

    fn read_appended(&mut self) -> std::io::Result<Vec<String>> {
        let metadata = std::fs::metadata(&self.path)?;

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

        self.pending.push_str(&String::from_utf8_lossy(&bytes));

        let mut lines = Vec::new();

        while let Some(newline) = self.pending.find('\n') {
            let mut line = self.pending.drain(..=newline).collect::<String>();
            line.truncate(line.trim_end_matches(['\r', '\n']).len());
            lines.push(line);
        }

        Ok(lines)
    }
}

fn event_touches_console(event: &Event, console_log: &Path) -> bool {
    matches!(event.kind, EventKind::Create(_) | EventKind::Modify(_))
        && event.paths.iter().any(|path| path == console_log)
}

pub fn start(app: AppHandle, console_log: PathBuf) -> Result<(), String> {
    let watch_root = console_log
        .parent()
        .ok_or_else(|| "console.log has no parent directory".to_string())?
        .to_path_buf();

    if !watch_root.is_dir() {
        return Err(format!("Deadlock log directory does not exist: {}", watch_root.display()));
    }

    thread::Builder::new()
        .name("split-console-watcher".to_string())
        .spawn(move || {
            let (tx, rx) = mpsc::channel::<notify::Result<Event>>();

            let mut watcher: RecommendedWatcher = match notify::recommended_watcher(move |event| {
                let _ = tx.send(event);
            }) {
                Ok(watcher) => watcher,
                Err(error) => {
                    eprintln!("SPLIT console watcher failed to initialize: {error}");
                    return;
                }
            };

            if let Err(error) = watcher.watch(&watch_root, RecursiveMode::NonRecursive) {
                eprintln!("SPLIT could not watch {}: {error}", watch_root.display());
                return;
            }

            let mut tail = ConsoleTail::new(console_log.clone());

            for event in rx {
                let Ok(event) = event else {
                    continue;
                };

                if !event_touches_console(&event, &console_log) {
                    continue;
                }

                let Ok(lines) = tail.read_appended() else {
                    continue;
                };

                for line in lines {
                    if let Some(position) = parse_position(&line) {
                        let _ = app.emit("deadlock://position", position);
                    }
                }
            }
        })
        .map_err(|error| format!("failed to spawn console watcher: {error}"))?;

    Ok(())
}
