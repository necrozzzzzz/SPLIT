#[cfg(split_edition = "borderless")]
mod borderless;
#[cfg(split_edition = "panorama")]
mod panorama;

use tauri::AppHandle;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edition {
    Borderless,
    Panorama,
}

#[cfg(split_edition = "borderless")]
pub const CURRENT: Edition = Edition::Borderless;

#[cfg(split_edition = "panorama")]
pub const CURRENT: Edition = Edition::Panorama;

impl Edition {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Borderless => "Borderless",
            Self::Panorama => "Panorama",
        }
    }
}

pub const fn windows_quick_access_enabled() -> bool {
    matches!(CURRENT, Edition::Borderless)
}

pub const fn native_notifications_enabled() -> bool {
    matches!(CURRENT, Edition::Borderless)
}

pub const fn uses_production_panorama_runtime() -> bool {
    matches!(CURRENT, Edition::Panorama)
}

#[cfg(split_edition = "borderless")]
pub fn panorama_renderer_active() -> bool {
    borderless::legacy_panorama_renderer_active()
}

#[cfg(split_edition = "panorama")]
pub fn panorama_renderer_active() -> bool {
    panorama::panorama_renderer_active()
}

#[cfg(split_edition = "borderless")]
pub fn start_quick_access_runtime(app: AppHandle) -> Result<(), String> {
    borderless::start_quick_access_runtime(app)
}

#[cfg(split_edition = "panorama")]
pub fn start_quick_access_runtime(app: AppHandle) -> Result<(), String> {
    panorama::start_quick_access_runtime(app)
}

#[cfg(split_edition = "borderless")]
pub fn stop_quick_access_runtime() -> Result<(), String> {
    borderless::stop_quick_access_runtime()
}

#[cfg(split_edition = "panorama")]
pub fn stop_quick_access_runtime() -> Result<(), String> {
    panorama::stop_quick_access_runtime()
}

#[cfg(split_edition = "borderless")]
pub fn start_notification_runtime() -> Result<(), String> {
    borderless::start_notification_runtime()
}

#[cfg(split_edition = "panorama")]
pub fn start_notification_runtime() -> Result<(), String> {
    panorama::start_notification_runtime()
}

#[cfg(split_edition = "borderless")]
pub fn stop_notification_runtime() -> Result<(), String> {
    borderless::stop_notification_runtime()
}

#[cfg(split_edition = "panorama")]
pub fn stop_notification_runtime() -> Result<(), String> {
    panorama::stop_notification_runtime()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_edition_matches_build_environment() {
        assert_eq!(CURRENT.label().to_ascii_lowercase(), env!("SPLIT_EDITION"));
    }

    #[test]
    fn only_panorama_edition_selects_production_panorama_runtime() {
        assert_eq!(
            uses_production_panorama_runtime(),
            matches!(CURRENT, Edition::Panorama)
        );
    }
}
