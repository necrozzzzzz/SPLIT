use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};

use super::process::running_deadlock_root;
use crate::notifications::NotificationSettings;
use crate::storage::atomic_write;

static CONFIG_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy)]
pub enum PathSource {
    UserConfig,
}

impl PathSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UserConfig => "user-config",
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct SplitConfig {
    deadlock_path: String,
    #[serde(deserialize_with = "deserialize_notification_settings")]
    notifications: NotificationSettings,
}

impl Default for SplitConfig {
    fn default() -> Self {
        Self {
            deadlock_path: String::new(),
            notifications: NotificationSettings::default(),
        }
    }
}

fn deserialize_notification_settings<'de, D>(
    deserializer: D,
) -> Result<NotificationSettings, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

impl DeadlockPaths {
    fn from_root(root: PathBuf, source: PathSource) -> Option<Self> {
        let deadlock_exe = root
            .join("game")
            .join("bin")
            .join("win64")
            .join("deadlock.exe");

        let citadel_dir = root.join("game").join("citadel");

        let cfg_dir = citadel_dir.join("cfg");

        /*
         * On exige ces deux éléments pour éviter
         * d'accepter n'importe quel dossier.
         */
        if !deadlock_exe.is_file() || !cfg_dir.is_dir() {
            return None;
        }

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

fn config_path() -> Result<PathBuf, String> {
    let app_data = env::var_os("APPDATA")
        .ok_or_else(|| "APPDATA environment variable is unavailable".to_string())?;

    Ok(PathBuf::from(app_data)
        .join("SPLIT")
        .join("split2-config.json"))
}

pub fn configured_deadlock_paths() -> Option<DeadlockPaths> {
    let _guard = CONFIG_LOCK.lock().ok()?;
    let path = config_path().ok()?;

    let raw = fs::read_to_string(path).ok()?;

    let config: SplitConfig = serde_json::from_str(&raw).ok()?;

    DeadlockPaths::from_root(PathBuf::from(config.deadlock_path), PathSource::UserConfig)
}

pub fn save_deadlock_root(root: &Path) -> Result<DeadlockPaths, String> {
    let paths =
        DeadlockPaths::from_root(root.to_path_buf(), PathSource::UserConfig).ok_or_else(|| {
            format!(
                "This is not a valid Deadlock installation:\n{}",
                root.display()
            )
        })?;

    let _guard = CONFIG_LOCK
        .lock()
        .map_err(|_| "SPLIT configuration lock poisoned".to_string())?;
    let config_path = config_path()?;

    let Some(parent) = config_path.parent() else {
        return Err("SPLIT configuration directory is invalid".to_string());
    };

    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create SPLIT configuration directory: {error}"))?;

    let mut config = fs::read_to_string(&config_path)
        .ok()
        .and_then(|raw| serde_json::from_str::<SplitConfig>(&raw).ok())
        .unwrap_or_default();
    config.deadlock_path = path_to_string(&paths.root);

    let json = serde_json::to_string_pretty(&config)
        .map_err(|error| format!("Could not serialize SPLIT configuration: {error}"))?;

    atomic_write(&config_path, json)
        .map_err(|error| format!("Could not save SPLIT configuration: {error}"))?;

    println!("[SPLIT] Deadlock directory saved: {}", paths.root.display());

    Ok(paths)
}

pub fn load_notification_settings() -> NotificationSettings {
    let Ok(_guard) = CONFIG_LOCK.lock() else {
        return NotificationSettings::default();
    };
    let Ok(path) = config_path() else {
        return NotificationSettings::default();
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<SplitConfig>(&raw).ok())
        .map(|config| config.notifications)
        .unwrap_or_default()
}

pub fn save_notification_settings(
    settings: NotificationSettings,
) -> Result<NotificationSettings, String> {
    settings.validate()?;
    let _guard = CONFIG_LOCK
        .lock()
        .map_err(|_| "SPLIT configuration lock poisoned".to_string())?;
    let path = config_path()?;
    let mut config = fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<SplitConfig>(&raw).ok())
        .unwrap_or_default();
    config.notifications = settings.clone();
    let json = serde_json::to_string_pretty(&config)
        .map_err(|error| format!("Could not serialize SPLIT configuration: {error}"))?;
    atomic_write(&path, json)
        .map_err(|error| format!("Could not save SPLIT configuration: {error}"))?;
    Ok(settings)
}

fn push_unique(candidates: &mut Vec<PathBuf>, candidate: PathBuf) {
    let candidate_string = candidate.to_string_lossy();

    let already_exists = candidates.iter().any(|known| {
        known
            .to_string_lossy()
            .eq_ignore_ascii_case(&candidate_string)
    });

    if !already_exists {
        candidates.push(candidate);
    }
}

fn extract_quoted_fields(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut inside_quotes = false;

    for character in line.chars() {
        if character == '"' {
            if inside_quotes {
                fields.push(current.clone());
                current.clear();
            }

            inside_quotes = !inside_quotes;
            continue;
        }

        if inside_quotes {
            current.push(character);
        }
    }

    fields
}

fn steam_library_roots(steam_root: &Path) -> Vec<PathBuf> {
    let mut libraries = Vec::new();

    push_unique(&mut libraries, steam_root.to_path_buf());

    let vdf = steam_root.join("steamapps").join("libraryfolders.vdf");

    let Ok(raw) = fs::read_to_string(vdf) else {
        return libraries;
    };

    for line in raw.lines() {
        let fields = extract_quoted_fields(line);

        if fields.len() < 2 {
            continue;
        }

        if !fields[0].eq_ignore_ascii_case("path") {
            continue;
        }

        /*
         * VDF encode généralement les chemins Windows
         * comme :
         *
         * H:\\SteamLibrary
         */
        let normalized = fields[1].replace("\\\\", "\\");

        push_unique(&mut libraries, PathBuf::from(normalized));
    }

    libraries
}

fn primary_steam_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();

    for variable in ["ProgramFiles(x86)", "ProgramFiles"] {
        let Some(program_files) = env::var_os(variable) else {
            continue;
        };

        push_unique(&mut roots, PathBuf::from(program_files).join("Steam"));
    }

    roots
}

pub fn scan_deadlock_root() -> Option<PathBuf> {
    let mut candidates = Vec::new();

    /*
     * 1. Si Deadlock tourne,
     * son installation est un excellent candidat.
     */
    if let Some(root) = running_deadlock_root() {
        push_unique(&mut candidates, root);
    }

    /*
     * 2. Steam principal + toutes les bibliothèques
     * déclarées dans libraryfolders.vdf.
     */
    for steam_root in primary_steam_roots() {
        for library in steam_library_roots(&steam_root) {
            let deadlock_root = library.join("steamapps").join("common").join("Deadlock");

            push_unique(&mut candidates, deadlock_root);
        }
    }

    println!("[SPLIT] Deadlock scan: {} candidate(s)", candidates.len());

    for candidate in candidates {
        println!("[SPLIT] Checking: {}", candidate.display());

        if DeadlockPaths::from_root(candidate.clone(), PathSource::UserConfig).is_some() {
            println!("[SPLIT] Deadlock detected: {}", candidate.display());

            return Some(candidate);
        }
    }

    println!("[SPLIT] Deadlock was not automatically detected");

    None
}

pub fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifications::NotificationPosition;

    #[test]
    fn legacy_config_without_notifications_uses_defaults() {
        let config: SplitConfig =
            serde_json::from_str(r#"{"deadlockPath":"C:\\Deadlock"}"#).unwrap();

        assert_eq!(config.deadlock_path, r"C:\Deadlock");
        assert_eq!(config.notifications, NotificationSettings::default());
    }

    #[test]
    fn invalid_notification_fields_fall_back_safely() {
        let config: SplitConfig = serde_json::from_str(
            r#"{
                "deadlockPath": "C:\\Deadlock",
                "notifications": {
                    "enabled": true,
                    "position": "somewhere",
                    "durationMs": 999999999
                }
            }"#,
        )
        .unwrap();

        assert_eq!(
            config.notifications.position,
            NotificationPosition::TopRight
        );
        assert_eq!(config.notifications.duration_ms, 1_500);
    }
}
