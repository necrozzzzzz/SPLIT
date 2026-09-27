mod borderless;
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

pub fn legacy_panorama_renderer_active() -> bool {
    match CURRENT {
        Edition::Borderless => borderless::legacy_panorama_renderer_active(),
        Edition::Panorama => panorama::legacy_panorama_renderer_active(),
    }
}

pub fn start_quick_access_runtime(app: AppHandle) -> Result<(), String> {
    match CURRENT {
        Edition::Borderless => borderless::start_quick_access_runtime(app),
        Edition::Panorama => panorama::start_quick_access_runtime(app),
    }
}

pub fn stop_quick_access_runtime() -> Result<(), String> {
    match CURRENT {
        Edition::Borderless => borderless::stop_quick_access_runtime(),
        Edition::Panorama => panorama::stop_quick_access_runtime(),
    }
}

pub fn start_notification_runtime() -> Result<(), String> {
    match CURRENT {
        Edition::Borderless => borderless::start_notification_runtime(),
        Edition::Panorama => panorama::start_notification_runtime(),
    }
}

pub fn stop_notification_runtime() -> Result<(), String> {
    match CURRENT {
        Edition::Borderless => borderless::stop_notification_runtime(),
        Edition::Panorama => panorama::stop_notification_runtime(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_edition_matches_build_environment() {
        assert_eq!(CURRENT.label().to_ascii_lowercase(), env!("SPLIT_EDITION"));
    }
}
