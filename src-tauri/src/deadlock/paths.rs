use std::{
    env, fs,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use super::hotkeys::HotkeySettings;
use super::process::running_deadlock_root;
use crate::discord::{DiscordPresenceConfig, DiscordPresenceSection};
use crate::notifications::NotificationSettings;
use crate::quick_access::QuickAccessSettings;
use crate::storage::atomic_write;
use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;

static CONFIG_LOCK: Mutex<()> = Mutex::new(());
static DEADLOCK_SCAN_RUNNING: AtomicBool = AtomicBool::new(false);
static STARTUP_TRACE: Mutex<Vec<String>> = Mutex::new(Vec::new());
static STARTUP_INCIDENT: AtomicBool = AtomicBool::new(false);
static STARTUP_PROBLEM: Mutex<Option<String>> = Mutex::new(None);

const DEADLOCK_SCAN_TIMEOUT: Duration = Duration::from_secs(4);
const DRIVE_REMOVABLE: u32 = 2;
const DRIVE_FIXED: u32 = 3;
const DRIVE_RAMDISK: u32 = 6;

fn startup_directory() -> Option<PathBuf> {
    env::var_os("APPDATA").map(|app_data| PathBuf::from(app_data).join("SPLIT"))
}

fn cleanup_legacy_startup_logs(directory: &Path) {
    for legacy_name in ["startup-boot.log", "startup-scan.log"] {
        match fs::remove_file(directory.join(legacy_name)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {}
        }
    }
}

pub(crate) fn trace_startup(reset: bool, message: &str) {
    if reset {
        if let Some(directory) = startup_directory() {
            cleanup_legacy_startup_logs(&directory);
        }
    }

    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default();
    let sanitized = message.chars().take(512).collect::<String>();
    if let Ok(mut trace) = STARTUP_TRACE.lock() {
        if reset {
            trace.clear();
            STARTUP_INCIDENT.store(false, Ordering::Release);
            if let Ok(mut problem) = STARTUP_PROBLEM.lock() {
                *problem = None;
            }
        }
        if trace.len() >= 128 {
            trace.remove(0);
        }
        trace.push(format!("[{unix_ms}] {sanitized}"));
    }
}

pub(crate) fn persist_startup_diagnostics(problem: &str) {
    STARTUP_INCIDENT.store(true, Ordering::Release);
    if let Ok(mut stored_problem) = STARTUP_PROBLEM.lock() {
        *stored_problem = Some(problem.chars().take(1_024).collect());
    }
    write_startup_diagnostics(problem);
}

pub(crate) fn finalize_startup_diagnostics() {
    if !STARTUP_INCIDENT.load(Ordering::Acquire) {
        return;
    }
    let problem = STARTUP_PROBLEM
        .lock()
        .ok()
        .and_then(|problem| problem.clone())
        .unwrap_or_else(|| "unspecified startup incident".to_string());
    write_startup_diagnostics(&problem);
}

fn write_startup_diagnostics(problem: &str) {
    let Some(app_data) = env::var_os("APPDATA") else {
        return;
    };
    let directory = PathBuf::from(app_data).join("SPLIT");
    let trace = STARTUP_TRACE
        .lock()
        .map(|trace| trace.clone())
        .unwrap_or_default();
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_millis())
        .unwrap_or_default();
    let problem = problem.chars().take(1_024).collect::<String>();
    let mut contents = format!(
        "SPLIT startup diagnostics\nversion: {}\ntimestamp_unix_ms: {unix_ms}\nproblem: {problem}\n\nstartup trace:\n",
        env!("CARGO_PKG_VERSION")
    );
    for entry in trace {
        contents.push_str(&entry);
        contents.push('\n');
    }

    let _ = atomic_write(&directory.join("startup-diagnostics.log"), contents);
}

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
    last_launch_at: Option<u64>,
    focus_deadlock_on_startup: bool,
    #[serde(deserialize_with = "deserialize_notification_settings")]
    notifications: NotificationSettings,
    #[serde(deserialize_with = "deserialize_hotkey_settings")]
    hotkeys: HotkeySettings,
    #[serde(deserialize_with = "deserialize_quick_access_settings")]
    quick_access: QuickAccessSettings,
    #[serde(deserialize_with = "deserialize_startup_sound_settings")]
    startup_sound: StartupSoundSettings,
    discord_presence_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    discord_presence: Option<DiscordPresenceConfig>,
}

impl Default for SplitConfig {
    fn default() -> Self {
        Self {
            deadlock_path: String::new(),
            last_launch_at: None,
            focus_deadlock_on_startup: false,
            notifications: NotificationSettings::default(),
            hotkeys: HotkeySettings::default(),
            quick_access: QuickAccessSettings::default(),
            startup_sound: StartupSoundSettings::default(),
            discord_presence_enabled: true,
            discord_presence: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StartupSoundSettings {
    pub enabled: bool,
    pub volume: u8,
}

impl Default for StartupSoundSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            volume: 40,
        }
    }
}

impl StartupSoundSettings {
    fn normalized(self) -> Self {
        Self {
            volume: self.volume.min(100),
            ..self
        }
    }
}

const THIRTY_DAYS_SECONDS: u64 = 30 * 24 * 60 * 60;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchFolderState {
    pub configured_path: Option<String>,
    pub path_valid: bool,
    pub reminder_due: bool,
}

fn reminder_due(last_launch_at: Option<u64>, now: u64) -> bool {
    last_launch_at.is_some_and(|last| now.saturating_sub(last) > THIRTY_DAYS_SECONDS)
}

pub fn record_successful_launch(now: u64) -> Result<LaunchFolderState, String> {
    let (configured_path, launch_was_stale) = {
        let _guard = CONFIG_LOCK
            .lock()
            .map_err(|_| "SPLIT configuration lock poisoned".to_string())?;
        let path = config_path()?;
        update_successful_launch_at_path(&path, now)?
    };

    Ok(launch_folder_state(configured_path, launch_was_stale))
}

#[cfg(test)]
fn record_successful_launch_at_path(path: &Path, now: u64) -> Result<LaunchFolderState, String> {
    let (configured_path, launch_was_stale) = update_successful_launch_at_path(path, now)?;
    Ok(launch_folder_state(configured_path, launch_was_stale))
}

fn update_successful_launch_at_path(
    path: &Path,
    now: u64,
) -> Result<(Option<String>, bool), String> {
    let mut config = load_config_for_write(path, "record_successful_launch")?;

    let configured_path =
        (!config.deadlock_path.trim().is_empty()).then(|| config.deadlock_path.clone());
    let launch_was_stale = reminder_due(config.last_launch_at, now);

    config.last_launch_at = Some(now);
    write_config(path, &config, "record_successful_launch")?;

    Ok((configured_path, launch_was_stale))
}

fn launch_folder_state(
    configured_path: Option<String>,
    launch_was_stale: bool,
) -> LaunchFolderState {
    let path_valid = configured_path.as_ref().is_some_and(|root| {
        DeadlockPaths::from_root(PathBuf::from(root), PathSource::UserConfig).is_some()
    });

    LaunchFolderState {
        configured_path,
        path_valid,
        reminder_due: path_valid && launch_was_stale,
    }
}

pub fn configured_deadlock_root() -> Option<PathBuf> {
    let _guard = CONFIG_LOCK.lock().ok()?;
    let path = config_path().ok()?;
    let raw = fs::read_to_string(path).ok()?;
    let config: SplitConfig = serde_json::from_str(&raw).ok()?;
    (!config.deadlock_path.trim().is_empty()).then(|| PathBuf::from(config.deadlock_path))
}

fn deserialize_hotkey_settings<'de, D>(deserializer: D) -> Result<HotkeySettings, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value)
        .ok()
        .and_then(|settings: HotkeySettings| settings.normalized().ok())
        .unwrap_or_default())
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

fn deserialize_quick_access_settings<'de, D>(
    deserializer: D,
) -> Result<QuickAccessSettings, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).unwrap_or_default())
}

fn deserialize_startup_sound_settings<'de, D>(
    deserializer: D,
) -> Result<StartupSoundSettings, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<StartupSoundSettings>(value)
        .unwrap_or_default()
        .normalized())
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

fn load_config_for_write(path: &Path, writer: &str) -> Result<SplitConfig, String> {
    match fs::read_to_string(path) {
        Ok(raw) => serde_json::from_str::<SplitConfig>(&raw).map_err(|error| {
            format!(
                "Could not parse SPLIT configuration for {writer}; refusing to overwrite it: {error}"
            )
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(SplitConfig::default()),
        Err(error) => Err(format!(
            "Could not read SPLIT configuration for {writer}: {error}"
        )),
    }
}

fn write_config(path: &Path, config: &SplitConfig, writer: &str) -> Result<(), String> {
    let json = serde_json::to_string_pretty(config)
        .map_err(|error| format!("Could not serialize SPLIT configuration: {error}"))?;
    atomic_write(path, json)
        .map_err(|error| format!("Could not save SPLIT configuration ({writer}): {error}"))
}

pub fn configured_deadlock_paths() -> Option<DeadlockPaths> {
    let configured_path = {
        let _guard = CONFIG_LOCK.lock().ok()?;
        let path = config_path().ok()?;
        let raw = fs::read_to_string(path).ok()?;
        let config: SplitConfig = serde_json::from_str(&raw).ok()?;
        config.deadlock_path
    };

    DeadlockPaths::from_root(PathBuf::from(configured_path), PathSource::UserConfig)
}

pub fn save_deadlock_root(root: &Path) -> Result<DeadlockPaths, String> {
    let _guard = CONFIG_LOCK
        .lock()
        .map_err(|_| "SPLIT configuration lock poisoned".to_string())?;
    let config_path = config_path()?;
    save_deadlock_root_at_path(root, &config_path)
}

fn save_deadlock_root_at_path(root: &Path, config_path: &Path) -> Result<DeadlockPaths, String> {
    let paths =
        DeadlockPaths::from_root(root.to_path_buf(), PathSource::UserConfig).ok_or_else(|| {
            format!(
                "This is not a valid Deadlock installation:\n{}",
                root.display()
            )
        })?;

    let Some(parent) = config_path.parent() else {
        return Err("SPLIT configuration directory is invalid".to_string());
    };

    fs::create_dir_all(parent)
        .map_err(|error| format!("Could not create SPLIT configuration directory: {error}"))?;

    let mut config = load_config_for_write(config_path, "save_deadlock_root")?;
    config.deadlock_path = path_to_string(&paths.root);
    write_config(config_path, &config, "save_deadlock_root")?;

    let persisted = load_config_for_write(config_path, "save_deadlock_root verification")?;
    if persisted.deadlock_path != config.deadlock_path {
        return Err(
            "Deadlock directory verification failed after saving SPLIT configuration".to_string(),
        );
    }

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
    save_notification_settings_at_path(&path, settings)
}

fn save_notification_settings_at_path(
    path: &Path,
    settings: NotificationSettings,
) -> Result<NotificationSettings, String> {
    settings.validate()?;
    let mut config = load_config_for_write(path, "save_notification_settings")?;
    config.notifications = settings.clone();
    write_config(path, &config, "save_notification_settings")?;
    Ok(settings)
}

pub fn load_hotkey_settings() -> HotkeySettings {
    let Ok(_guard) = CONFIG_LOCK.lock() else {
        return HotkeySettings::default();
    };
    let Ok(path) = config_path() else {
        return HotkeySettings::default();
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<SplitConfig>(&raw).ok())
        .and_then(|config| config.hotkeys.normalized().ok())
        .unwrap_or_default()
}

pub fn save_hotkey_settings(settings: HotkeySettings) -> Result<HotkeySettings, String> {
    let settings = settings.normalized()?;
    let _guard = CONFIG_LOCK
        .lock()
        .map_err(|_| "SPLIT configuration lock poisoned".to_string())?;
    let path = config_path()?;
    save_hotkey_settings_at_path(&path, settings)
}

fn save_hotkey_settings_at_path(
    path: &Path,
    settings: HotkeySettings,
) -> Result<HotkeySettings, String> {
    let settings = settings.normalized()?;
    let mut config = load_config_for_write(path, "save_hotkey_settings")?;
    config.hotkeys = settings.clone();
    write_config(path, &config, "save_hotkey_settings")?;
    Ok(settings)
}

pub fn load_quick_access_settings() -> QuickAccessSettings {
    let Ok(_guard) = CONFIG_LOCK.lock() else {
        return QuickAccessSettings::default();
    };
    let Ok(path) = config_path() else {
        return QuickAccessSettings::default();
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<SplitConfig>(&raw).ok())
        .map(|config| config.quick_access)
        .unwrap_or_default()
}

pub fn save_quick_access_settings(
    settings: QuickAccessSettings,
) -> Result<QuickAccessSettings, String> {
    let _guard = CONFIG_LOCK
        .lock()
        .map_err(|_| "SPLIT configuration lock poisoned".to_string())?;
    let path = config_path()?;
    save_quick_access_settings_at_path(&path, settings)
}

fn save_quick_access_settings_at_path(
    path: &Path,
    settings: QuickAccessSettings,
) -> Result<QuickAccessSettings, String> {
    let mut config = load_config_for_write(path, "save_quick_access_settings")?;
    config.quick_access = settings;
    write_config(path, &config, "save_quick_access_settings")?;
    Ok(settings)
}

pub fn load_startup_sound_settings() -> StartupSoundSettings {
    let Ok(_guard) = CONFIG_LOCK.lock() else {
        return StartupSoundSettings::default();
    };
    let Ok(path) = config_path() else {
        return StartupSoundSettings::default();
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<SplitConfig>(&raw).ok())
        .map(|config| config.startup_sound.normalized())
        .unwrap_or_default()
}

pub fn load_focus_deadlock_on_startup() -> bool {
    let Ok(_guard) = CONFIG_LOCK.lock() else {
        return false;
    };
    let Ok(path) = config_path() else {
        return false;
    };
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<SplitConfig>(&raw).ok())
        .map(|config| config.focus_deadlock_on_startup)
        .unwrap_or(false)
}

pub fn save_focus_deadlock_on_startup(enabled: bool) -> Result<bool, String> {
    let _guard = CONFIG_LOCK
        .lock()
        .map_err(|_| "SPLIT configuration lock poisoned".to_string())?;
    let path = config_path()?;
    save_focus_deadlock_on_startup_at_path(&path, enabled)
}

fn save_focus_deadlock_on_startup_at_path(path: &Path, enabled: bool) -> Result<bool, String> {
    let mut config = load_config_for_write(path, "save_focus_deadlock_on_startup")?;
    config.focus_deadlock_on_startup = enabled;
    write_config(path, &config, "save_focus_deadlock_on_startup")?;
    Ok(enabled)
}

pub fn save_startup_sound_settings(
    settings: StartupSoundSettings,
) -> Result<StartupSoundSettings, String> {
    let _guard = CONFIG_LOCK
        .lock()
        .map_err(|_| "SPLIT configuration lock poisoned".to_string())?;
    let path = config_path()?;
    save_startup_sound_settings_at_path(&path, settings)
}

fn save_startup_sound_settings_at_path(
    path: &Path,
    settings: StartupSoundSettings,
) -> Result<StartupSoundSettings, String> {
    let settings = settings.normalized();
    let mut config = load_config_for_write(path, "save_startup_sound_settings")?;
    config.startup_sound = settings;
    write_config(path, &config, "save_startup_sound_settings")?;
    Ok(settings)
}

fn effective_discord_presence_config(config: &SplitConfig) -> DiscordPresenceConfig {
    config.discord_presence.clone().unwrap_or_else(|| {
        let mut presence = DiscordPresenceConfig::default();
        presence.global.enabled = config.discord_presence_enabled;
        presence
    })
}

pub fn load_discord_presence_config() -> DiscordPresenceConfig {
    let Ok(_guard) = CONFIG_LOCK.lock() else {
        return DiscordPresenceConfig::default();
    };

    let Ok(path) = config_path() else {
        return DiscordPresenceConfig::default();
    };

    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<SplitConfig>(&raw).ok())
        .map(|config| effective_discord_presence_config(&config))
        .unwrap_or_default()
}

pub fn save_discord_presence_config(
    presence: DiscordPresenceConfig,
) -> Result<DiscordPresenceConfig, String> {
    presence.validate()?;
    let _guard = CONFIG_LOCK
        .lock()
        .map_err(|_| "SPLIT configuration lock poisoned".to_string())?;
    let path = config_path()?;

    save_discord_presence_config_at_path(&path, presence)
}

fn save_discord_presence_config_at_path(
    path: &Path,
    presence: DiscordPresenceConfig,
) -> Result<DiscordPresenceConfig, String> {
    presence.validate()?;
    let mut config = load_config_for_write(path, "save_discord_presence_config")?;
    config.discord_presence_enabled = presence.global.enabled;
    config.discord_presence = Some(presence.clone());
    write_config(path, &config, "save_discord_presence_config")?;
    Ok(presence)
}

pub fn reset_discord_presence_config(
    section: Option<DiscordPresenceSection>,
) -> Result<DiscordPresenceConfig, String> {
    let _guard = CONFIG_LOCK
        .lock()
        .map_err(|_| "SPLIT configuration lock poisoned".to_string())?;
    let path = config_path()?;
    reset_discord_presence_config_at_path(&path, section)
}

fn reset_discord_presence_config_at_path(
    path: &Path,
    section: Option<DiscordPresenceSection>,
) -> Result<DiscordPresenceConfig, String> {
    let persisted = load_config_for_write(&path, "reset_discord_presence_config")?;
    let mut presence = effective_discord_presence_config(&persisted);

    if let Some(section) = section {
        presence.reset_section(section);
    } else {
        presence = DiscordPresenceConfig::default();
    }

    save_discord_presence_config_at_path(path, presence)
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

fn steam_library_roots_from_vdf(steam_root: &Path, raw: &str) -> Vec<PathBuf> {
    let mut libraries = Vec::new();

    push_unique(&mut libraries, steam_root.to_path_buf());

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

fn steam_library_roots(steam_root: &Path, log: &mut ScanTrace) -> Vec<PathBuf> {
    let vdf = steam_root.join("steamapps").join("libraryfolders.vdf");
    log.write(format!("reading library file: {}", vdf.display()));

    match fs::read_to_string(&vdf) {
        Ok(raw) => {
            log.write(format!("read library file: {}", vdf.display()));
            steam_library_roots_from_vdf(steam_root, &raw)
        }
        Err(error) => {
            log.write(format!(
                "library file unavailable: {} ({error})",
                vdf.display()
            ));
            if error.kind() != std::io::ErrorKind::NotFound {
                persist_startup_diagnostics(&format!(
                    "could not read Steam library file {}: {error}",
                    vdf.display()
                ));
            }
            vec![steam_root.to_path_buf()]
        }
    }
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

fn supported_drive_type(drive_type: u32) -> bool {
    matches!(drive_type, DRIVE_REMOVABLE | DRIVE_FIXED | DRIVE_RAMDISK)
}

fn available_local_volume(path: &Path) -> bool {
    let wide: Vec<u16> = path.as_os_str().encode_wide().collect();

    if wide.starts_with(&['\\' as u16, '\\' as u16]) {
        return false;
    }
    if wide.len() < 2 || wide[1] != ':' as u16 {
        return false;
    }

    let root = [wide[0], ':' as u16, '\\' as u16, 0];
    supported_drive_type(unsafe { GetDriveTypeW(root.as_ptr()) })
}

struct ScanTrace {
    started: Instant,
}

impl ScanTrace {
    fn start() -> Self {
        let mut trace = Self {
            started: Instant::now(),
        };
        trace.write("scan started");
        trace
    }

    fn write(&mut self, message: impl AsRef<str>) {
        trace_startup(
            false,
            &format!(
                "scan +{}ms: {}",
                self.started.elapsed().as_millis(),
                message.as_ref()
            ),
        );
    }
}

#[cfg(test)]
fn first_valid_candidate<F>(
    candidates: impl IntoIterator<Item = PathBuf>,
    mut valid: F,
) -> Option<PathBuf>
where
    F: FnMut(&Path) -> bool,
{
    candidates.into_iter().find(|candidate| valid(candidate))
}

fn scan_deadlock_root_inner() -> Option<PathBuf> {
    let mut log = ScanTrace::start();

    /*
     * 1. Si Deadlock tourne,
     * son installation est un excellent candidat.
     */
    log.write("checking running Deadlock process");
    let running_root = running_deadlock_root();
    log.write(format!(
        "running process root: {}",
        running_root
            .as_deref()
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| "none".to_string())
    ));
    if let Some(root) = running_root {
        if available_local_volume(&root) {
            log.write(format!("validating: {}", root.display()));
            let valid = DeadlockPaths::from_root(root.clone(), PathSource::UserConfig).is_some();
            log.write(format!(
                "validation result: {} ({})",
                root.display(),
                if valid { "valid" } else { "invalid" }
            ));
            if valid {
                log.write(format!("found from running process: {}", root.display()));
                return Some(root);
            }
        }
    }

    /*
     * 2. Steam principal + toutes les bibliothèques
     * déclarées dans libraryfolders.vdf.
     */
    for steam_root in primary_steam_roots() {
        log.write(format!("primary Steam root: {}", steam_root.display()));
        if !available_local_volume(&steam_root) {
            log.write(format!(
                "skipping unavailable/non-local Steam root: {}",
                steam_root.display()
            ));
            continue;
        }

        for library in steam_library_roots(&steam_root, &mut log) {
            log.write(format!("library detected: {}", library.display()));
            if !available_local_volume(&library) {
                log.write(format!(
                    "skipping unavailable/non-local library: {}",
                    library.display()
                ));
                continue;
            }
            let deadlock_root = library.join("steamapps").join("common").join("Deadlock");

            log.write(format!("candidate: {}", deadlock_root.display()));
            log.write(format!("validating: {}", deadlock_root.display()));

            let valid =
                DeadlockPaths::from_root(deadlock_root.clone(), PathSource::UserConfig).is_some();
            log.write(format!(
                "validation result: {} ({})",
                deadlock_root.display(),
                if valid { "valid" } else { "invalid" }
            ));
            if valid {
                println!("[SPLIT] Deadlock detected: {}", deadlock_root.display());
                log.write(format!("found: {}", deadlock_root.display()));
                return Some(deadlock_root);
            }
        }
    }

    println!("[SPLIT] Deadlock was not automatically detected");
    log.write("completed: Deadlock not found");

    None
}

struct ScanRunningGuard;

impl Drop for ScanRunningGuard {
    fn drop(&mut self) {
        DEADLOCK_SCAN_RUNNING.store(false, Ordering::Release);
    }
}

pub fn scan_deadlock_root() -> Option<PathBuf> {
    if DEADLOCK_SCAN_RUNNING
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return None;
    }

    let (sender, receiver) = mpsc::sync_channel(1);
    let spawn_result = std::thread::Builder::new()
        .name("deadlock-install-scan".to_string())
        .spawn(move || {
            let _guard = ScanRunningGuard;
            let _ = sender.send(scan_deadlock_root_inner());
        });

    if spawn_result.is_err() {
        DEADLOCK_SCAN_RUNNING.store(false, Ordering::Release);
        persist_startup_diagnostics("could not start Deadlock installation scan worker");
        return None;
    }

    match receiver.recv_timeout(DEADLOCK_SCAN_TIMEOUT) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            trace_startup(false, "scan timeout after 4000ms");
            persist_startup_diagnostics("Deadlock installation scan timed out after 4000ms");
            None
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            trace_startup(false, "scan worker disconnected unexpectedly");
            persist_startup_diagnostics("Deadlock installation scan worker disconnected");
            None
        }
    }
}

pub fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::notifications::NotificationPosition;

    fn temporary_test_directory(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "split-path-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }

    fn create_valid_deadlock_root(base: &Path) -> PathBuf {
        let root = base.join("Deadlock");
        let cfg = root.join("game").join("citadel").join("cfg");
        let exe = root
            .join("game")
            .join("bin")
            .join("win64")
            .join("deadlock.exe");
        fs::create_dir_all(cfg).unwrap();
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(exe, []).unwrap();
        root
    }

    #[test]
    fn legacy_config_without_notifications_uses_defaults() {
        let config: SplitConfig =
            serde_json::from_str(r#"{"deadlockPath":"C:\\Deadlock"}"#).unwrap();

        assert_eq!(config.deadlock_path, r"C:\Deadlock");
        assert_eq!(config.notifications, NotificationSettings::default());
        assert_eq!(config.hotkeys, HotkeySettings::default());
        assert_eq!(config.quick_access, QuickAccessSettings::default());
        assert_eq!(config.startup_sound, StartupSoundSettings::default());
        assert_eq!(config.last_launch_at, None);
        assert!(!config.focus_deadlock_on_startup);
    }

    #[test]
    fn startup_focus_defaults_to_disabled() {
        assert!(!SplitConfig::default().focus_deadlock_on_startup);

        let legacy: SplitConfig = serde_json::from_str(r#"{"deadlockPath":""}"#).unwrap();
        assert!(!legacy.focus_deadlock_on_startup);
    }

    #[test]
    fn explicit_startup_focus_values_round_trip() {
        for enabled in [true, false] {
            let config: SplitConfig = serde_json::from_value(serde_json::json!({
                "focusDeadlockOnStartup": enabled
            }))
            .unwrap();
            assert_eq!(config.focus_deadlock_on_startup, enabled);

            let serialized = serde_json::to_value(config).unwrap();
            assert_eq!(serialized["focusDeadlockOnStartup"], enabled);
        }
    }

    #[test]
    fn startup_focus_writer_preserves_other_settings() {
        let directory = temporary_test_directory("startup-focus-preserves-config");
        let path = directory.join("split2-config.json");
        let original = SplitConfig {
            deadlock_path: r"C:\Deadlock".to_string(),
            last_launch_at: Some(12_345),
            discord_presence_enabled: false,
            ..SplitConfig::default()
        };
        write_config(&path, &original, "test setup").unwrap();

        assert!(save_focus_deadlock_on_startup_at_path(&path, true).unwrap());
        let saved = load_config_for_write(&path, "test read").unwrap();
        assert!(saved.focus_deadlock_on_startup);
        assert_eq!(saved.deadlock_path, original.deadlock_path);
        assert_eq!(saved.last_launch_at, original.last_launch_at);
        assert_eq!(
            saved.discord_presence_enabled,
            original.discord_presence_enabled
        );

        assert!(!save_focus_deadlock_on_startup_at_path(&path, false).unwrap());
        assert!(
            !load_config_for_write(&path, "test read")
                .unwrap()
                .focus_deadlock_on_startup
        );

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn legacy_discord_enabled_flag_migrates_when_nested_config_is_absent() {
        let config: SplitConfig =
            serde_json::from_str(r#"{"discordPresenceEnabled":false}"#).unwrap();
        let presence = effective_discord_presence_config(&config);

        assert!(!presence.global.enabled);
        assert_eq!(
            presence.explore_nyc,
            DiscordPresenceConfig::default().explore_nyc
        );
    }

    #[test]
    fn partial_nested_discord_config_uses_section_defaults() {
        let config: SplitConfig = serde_json::from_str(
            r#"{
                "discordPresence": {
                    "exploreNyc": { "showDistrict": false }
                }
            }"#,
        )
        .unwrap();
        let presence = effective_discord_presence_config(&config);

        assert!(!presence.explore_nyc.show_district);
        assert!(presence.explore_nyc.show_hero_in_details);
        assert_eq!(presence.explore_nyc.district_prefix, "› ");
        assert!(presence.hideout.use_official_hero_phrase);
    }

    #[test]
    fn legacy_match_party_boolean_migrates_in_persisted_config() {
        for (legacy, expected) in [
            (true, crate::discord::PartyDisplay::Compact),
            (false, crate::discord::PartyDisplay::Hidden),
        ] {
            let config: SplitConfig = serde_json::from_value(serde_json::json!({
                "discordPresence": {
                    "match": {
                        "showHeroImage": true,
                        "showParty": legacy
                    }
                }
            }))
            .unwrap();
            let presence = effective_discord_presence_config(&config);

            assert_eq!(presence.r#match.party_display, expected);
        }
    }

    #[test]
    fn discord_config_persists_and_reset_restores_section_or_all_defaults() {
        let directory = temporary_test_directory("discord-config");
        let path = directory.join("split2-config.json");
        let mut presence = DiscordPresenceConfig::default();
        presence.explore_nyc.district_prefix = "• ".to_string();
        presence.hideout.party_display = crate::discord::PartyDisplay::Hidden;
        presence.hideout.state_prefix = "› ".to_string();
        presence.spectating.match_id_prefix = "# ".to_string();
        save_discord_presence_config_at_path(&path, presence).unwrap();

        let saved_json: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        for section in [
            "hideout",
            "exploreNyc",
            "mainMenu",
            "matchmaking",
            "match",
            "postMatch",
        ] {
            assert!(saved_json["discordPresence"][section]
                .get("partyDisplay")
                .is_some());
            assert!(saved_json["discordPresence"][section]
                .get("showParty")
                .is_none());
        }
        assert_eq!(
            saved_json["discordPresence"]["hideout"]["statePrefix"],
            "› "
        );
        assert_eq!(
            saved_json["discordPresence"]["spectating"]["matchIdPrefix"],
            "# "
        );
        assert!(saved_json["discordPresence"]["spectating"]
            .get("partyDisplay")
            .is_none());
        assert!(saved_json["discordPresence"]["spectating"]
            .get("showParty")
            .is_none());

        let section_reset =
            reset_discord_presence_config_at_path(&path, Some(DiscordPresenceSection::ExploreNyc))
                .unwrap();
        assert_eq!(section_reset.explore_nyc.district_prefix, "› ");
        assert_eq!(
            section_reset.hideout.party_display,
            crate::discord::PartyDisplay::Hidden
        );
        assert_eq!(section_reset.hideout.state_prefix, "› ");
        assert_eq!(section_reset.spectating.match_id_prefix, "# ");

        let full_reset = reset_discord_presence_config_at_path(&path, None).unwrap();
        assert_eq!(full_reset, DiscordPresenceConfig::default());
        let persisted = load_config_for_write(&path, "test read").unwrap();
        assert_eq!(
            effective_discord_presence_config(&persisted),
            DiscordPresenceConfig::default()
        );

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn startup_sound_volume_is_clamped_during_deserialization() {
        let config: SplitConfig = serde_json::from_str(
            r#"{
                "startupSound": {
                    "enabled": false,
                    "volume": 180
                }
            }"#,
        )
        .unwrap();

        assert_eq!(
            config.startup_sound,
            StartupSoundSettings {
                enabled: false,
                volume: 100,
            }
        );
    }

    #[test]
    fn launch_reminder_uses_strict_thirty_day_threshold() {
        let now = 10_000_000;
        assert!(!reminder_due(Some(now - THIRTY_DAYS_SECONDS), now));
        assert!(!reminder_due(Some(now - 20 * 24 * 60 * 60), now));
        assert!(reminder_due(Some(now - THIRTY_DAYS_SECONDS - 1), now));
        assert!(!reminder_due(None, now));
    }

    #[test]
    fn setting_writers_preserve_deadlock_path() {
        let directory = temporary_test_directory("preserve-config");
        let path = directory.join("split2-config.json");
        let config = SplitConfig {
            deadlock_path: r"C:\Deadlock".to_string(),
            focus_deadlock_on_startup: true,
            ..SplitConfig::default()
        };
        write_config(&path, &config, "test setup").unwrap();

        record_successful_launch_at_path(&path, 10_000_000).unwrap();
        save_hotkey_settings_at_path(&path, HotkeySettings::default()).unwrap();
        save_notification_settings_at_path(&path, NotificationSettings::default()).unwrap();
        save_quick_access_settings_at_path(&path, QuickAccessSettings::default()).unwrap();
        save_startup_sound_settings_at_path(&path, StartupSoundSettings::default()).unwrap();

        let saved = load_config_for_write(&path, "test read").unwrap();
        assert_eq!(saved.deadlock_path, r"C:\Deadlock");
        assert_eq!(saved.last_launch_at, Some(10_000_000));
        assert!(saved.focus_deadlock_on_startup);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn save_deadlock_root_survives_all_follow_up_writers() {
        let directory = temporary_test_directory("first-setup");
        let root = create_valid_deadlock_root(&directory);
        let path = directory.join("split2-config.json");

        save_deadlock_root_at_path(&root, &path).unwrap();
        record_successful_launch_at_path(&path, 10_000_000).unwrap();
        save_hotkey_settings_at_path(&path, HotkeySettings::default()).unwrap();
        save_notification_settings_at_path(&path, NotificationSettings::default()).unwrap();
        save_quick_access_settings_at_path(&path, QuickAccessSettings::default()).unwrap();
        save_startup_sound_settings_at_path(&path, StartupSoundSettings::default()).unwrap();

        let saved = load_config_for_write(&path, "test read").unwrap();
        assert_eq!(saved.deadlock_path, path_to_string(&root));

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn malformed_config_is_never_replaced_with_defaults() {
        let directory = temporary_test_directory("malformed-config");
        let path = directory.join("split2-config.json");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, b"{\"deadlockPath\":\"C:\\\\Deadlock\",\"hotkeys\":").unwrap();
        let before = fs::read(&path).unwrap();

        let error =
            save_notification_settings_at_path(&path, NotificationSettings::default()).unwrap_err();

        assert!(error.contains("refusing to overwrite"));
        assert_eq!(fs::read(&path).unwrap(), before);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn startup_sound_writer_preserves_all_other_settings() {
        let directory = temporary_test_directory("startup-sound-preserves-config");
        let path = directory.join("split2-config.json");
        let notifications = NotificationSettings {
            enabled: false,
            ..NotificationSettings::default()
        };
        let hotkeys = HotkeySettings::default();
        let quick_access = QuickAccessSettings {
            enabled: false,
            position: crate::quick_access::QuickAccessPosition::Right,
        };
        let original = SplitConfig {
            deadlock_path: r"C:\Deadlock".to_string(),
            last_launch_at: Some(12_345),
            focus_deadlock_on_startup: true,
            notifications: notifications.clone(),
            hotkeys: hotkeys.clone(),
            quick_access,
            startup_sound: StartupSoundSettings::default(),
            discord_presence_enabled: true,
            discord_presence: None,
        };
        write_config(&path, &original, "test setup").unwrap();

        let saved_settings = save_startup_sound_settings_at_path(
            &path,
            StartupSoundSettings {
                enabled: false,
                volume: 255,
            },
        )
        .unwrap();
        let saved = load_config_for_write(&path, "test read").unwrap();

        assert_eq!(saved_settings.volume, 100);
        assert!(!saved_settings.enabled);
        assert_eq!(saved.deadlock_path, r"C:\Deadlock");
        assert_eq!(saved.last_launch_at, Some(12_345));
        assert!(saved.focus_deadlock_on_startup);
        assert_eq!(saved.notifications, notifications);
        assert_eq!(saved.hotkeys, hotkeys);
        assert_eq!(saved.quick_access, quick_access);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn startup_sound_writer_refuses_malformed_config() {
        let directory = temporary_test_directory("malformed-startup-sound-config");
        let path = directory.join("split2-config.json");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, b"{\"deadlockPath\":\"C:\\\\Deadlock\",\"hotkeys\":").unwrap();
        let before = fs::read(&path).unwrap();

        let error = save_startup_sound_settings_at_path(&path, StartupSoundSettings::default())
            .unwrap_err();

        assert!(error.contains("refusing to overwrite"));
        assert_eq!(fs::read(&path).unwrap(), before);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn startup_focus_writer_refuses_malformed_config() {
        let directory = temporary_test_directory("malformed-startup-focus-config");
        let path = directory.join("split2-config.json");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, b"{\"deadlockPath\":\"C:\\\\Deadlock\",\"hotkeys\":").unwrap();
        let before = fs::read(&path).unwrap();

        let error = save_focus_deadlock_on_startup_at_path(&path, true).unwrap_err();

        assert!(error.contains("refusing to overwrite"));
        assert_eq!(fs::read(&path).unwrap(), before);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn successful_launch_update_rejects_malformed_config_without_hanging() {
        let directory = temporary_test_directory("malformed-launch-config");
        let path = directory.join("split2-config.json");
        fs::create_dir_all(&directory).unwrap();
        fs::write(&path, b"{\"deadlockPath\":\"\",\"lastLaunchAt\":123").unwrap();
        let before = fs::read(&path).unwrap();
        let (sender, receiver) = mpsc::channel();

        std::thread::spawn({
            let path = path.clone();
            move || {
                let result = record_successful_launch_at_path(&path, 10_000_000);
                let _ = sender.send(result);
            }
        });

        let result = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("malformed config handling must return instead of hanging");
        assert!(result.unwrap_err().contains("refusing to overwrite"));
        assert_eq!(fs::read(&path).unwrap(), before);

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn deadlock_path_validation_distinguishes_valid_and_invalid_folders() {
        let root = std::env::temp_dir().join(format!(
            "split-path-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let cfg = root.join("game").join("citadel").join("cfg");
        let exe = root
            .join("game")
            .join("bin")
            .join("win64")
            .join("deadlock.exe");
        fs::create_dir_all(&cfg).unwrap();
        fs::create_dir_all(exe.parent().unwrap()).unwrap();

        assert!(DeadlockPaths::from_root(root.clone(), PathSource::UserConfig).is_none());
        fs::write(&exe, []).unwrap();
        assert!(DeadlockPaths::from_root(root.clone(), PathSource::UserConfig).is_some());

        let _ = fs::remove_dir_all(root);
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
        assert!(config.notifications.use_slot_color);
    }

    #[test]
    fn invalid_hotkeys_fall_back_to_defaults() {
        let config: SplitConfig = serde_json::from_str(
            r#"{
                "deadlockPath": "C:\\Deadlock",
                "hotkeys": {
                    "loadSlots": [],
                    "saveSlots": [],
                    "undo": { "key": "F13", "ctrl": false, "alt": false, "shift": false }
                }
            }"#,
        )
        .unwrap();

        assert_eq!(config.hotkeys, HotkeySettings::default());
    }

    #[test]
    fn quick_access_settings_deserialize_disabled_on_right() {
        let config: SplitConfig = serde_json::from_str(
            r#"{
                "quickAccess": {
                    "enabled": false,
                    "position": "right"
                }
            }"#,
        )
        .unwrap();

        assert!(!config.quick_access.enabled);
        assert_eq!(
            config.quick_access.position,
            crate::quick_access::QuickAccessPosition::Right,
        );
    }

    #[test]
    fn libraryfolders_keeps_primary_adds_libraries_and_removes_duplicates() {
        let primary = Path::new(r"C:\Program Files (x86)\Steam");
        let raw = r#"
            "path" "D:\\SteamLibrary"
            "path" "E:\\Games\\Steam"
            "path" "d:\\steamlibrary"
        "#;

        let libraries = steam_library_roots_from_vdf(primary, raw);

        assert_eq!(libraries.len(), 3);
        assert_eq!(libraries[0], primary);
        assert_eq!(libraries[1], PathBuf::from(r"D:\SteamLibrary"));
        assert_eq!(libraries[2], PathBuf::from(r"E:\Games\Steam"));
    }

    #[test]
    fn inaccessible_and_network_drive_types_are_rejected() {
        assert!(!supported_drive_type(0));
        assert!(!supported_drive_type(1));
        assert!(!supported_drive_type(4));
        assert!(supported_drive_type(DRIVE_REMOVABLE));
        assert!(supported_drive_type(DRIVE_FIXED));
    }

    #[test]
    fn startup_log_migration_removes_only_legacy_scan_logs() {
        let directory = temporary_test_directory("startup-log-migration");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("startup-boot.log"), b"old boot").unwrap();
        fs::write(directory.join("startup-scan.log"), b"old scan").unwrap();
        fs::write(directory.join("split_debug.log"), b"keep").unwrap();

        cleanup_legacy_startup_logs(&directory);

        assert!(!directory.join("startup-boot.log").exists());
        assert!(!directory.join("startup-scan.log").exists());
        assert_eq!(
            fs::read(directory.join("split_debug.log")).unwrap(),
            b"keep"
        );

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn candidate_scan_finds_second_library_and_stops_after_match() {
        let candidates = vec![
            PathBuf::from(r"C:\Steam\Deadlock"),
            PathBuf::from(r"D:\SteamLibrary\Deadlock"),
            PathBuf::from(r"E:\SteamLibrary\Deadlock"),
        ];
        let mut checked = Vec::new();

        let found = first_valid_candidate(candidates, |candidate| {
            checked.push(candidate.to_path_buf());
            candidate.starts_with(r"D:\")
        });

        assert_eq!(found, Some(PathBuf::from(r"D:\SteamLibrary\Deadlock")));
        assert_eq!(checked.len(), 2);
    }
}
