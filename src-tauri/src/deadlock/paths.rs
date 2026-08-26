use std::{env, fs, path::{Path, PathBuf}};

use serde::Deserialize;

#[derive(Debug, Clone, Copy)]
pub enum PathSource {
    LegacyConfig,
    SteamDefault,
}

impl PathSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LegacyConfig => "legacy-config",
            Self::SteamDefault => "steam-default",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeadlockPaths {
    pub root: PathBuf,
    pub console_log: PathBuf,
    pub cfg_dir: PathBuf,
    pub cfg_file: PathBuf,
    pub autoexec: PathBuf,
    pub source: PathSource,
}

#[derive(Debug, Deserialize)]
struct LegacyConfig {
    deadlock_path: Option<String>,
}

impl DeadlockPaths {
    fn from_root(root: PathBuf, source: PathSource) -> Option<Self> {
        let cfg_dir = root.join("game").join("citadel").join("cfg");

        if !cfg_dir.is_dir() {
            return None;
        }

        let citadel_dir = root.join("game").join("citadel");

        Some(Self {
            root,
            console_log: citadel_dir.join("console.log"),
            cfg_file: cfg_dir.join("savestate.cfg"),
            autoexec: cfg_dir.join("autoexec.cfg"),
            cfg_dir,
            source,
        })
    }
}

fn legacy_deadlock_path() -> Option<PathBuf> {
    let app_data = env::var_os("APPDATA")?;
    let config_path = PathBuf::from(app_data)
        .join("SPLIT")
        .join("deadlock_savestate_config.json");

    let raw = fs::read_to_string(config_path).ok()?;
    let config: LegacyConfig = serde_json::from_str(&raw).ok()?;
    config.deadlock_path.map(PathBuf::from)
}

fn steam_default_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    for env_key in ["ProgramFiles(x86)", "ProgramFiles"] {
        let Some(program_files) = env::var_os(env_key) else {
            continue;
        };

        let candidate = PathBuf::from(program_files)
            .join("Steam")
            .join("steamapps")
            .join("common")
            .join("Deadlock");

        if !candidates.iter().any(|known| known == &candidate) {
            candidates.push(candidate);
        }
    }

    candidates
}

pub fn detect_deadlock_paths() -> Option<DeadlockPaths> {
    if let Some(root) = legacy_deadlock_path() {
        if let Some(paths) = DeadlockPaths::from_root(root, PathSource::LegacyConfig) {
            return Some(paths);
        }
    }

    steam_default_candidates()
        .into_iter()
        .find_map(|root| DeadlockPaths::from_root(root, PathSource::SteamDefault))
}

pub fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
