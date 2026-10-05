#[cfg(split_edition = "borderless")]
mod borderless;
#[cfg(split_edition = "fullscreen")]
mod fullscreen;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edition {
    Borderless,
    Fullscreen,
}

#[cfg(split_edition = "borderless")]
pub const CURRENT: Edition = Edition::Borderless;

#[cfg(split_edition = "fullscreen")]
pub const CURRENT: Edition = Edition::Fullscreen;

impl Edition {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Borderless => "Borderless",
            Self::Fullscreen => "Fullscreen",
        }
    }
}

pub const fn windows_quick_access_enabled() -> bool {
    matches!(CURRENT, Edition::Borderless)
}

pub const fn native_notifications_enabled() -> bool {
    matches!(CURRENT, Edition::Borderless)
}

#[cfg(split_edition = "borderless")]
pub fn start_notification_runtime() -> Result<(), String> {
    borderless::start_notification_runtime()
}

#[cfg(split_edition = "fullscreen")]
pub fn start_notification_runtime() -> Result<(), String> {
    fullscreen::start_notification_runtime()
}

#[cfg(split_edition = "borderless")]
pub fn stop_notification_runtime() -> Result<(), String> {
    borderless::stop_notification_runtime()
}

#[cfg(split_edition = "fullscreen")]
pub fn stop_notification_runtime() -> Result<(), String> {
    fullscreen::stop_notification_runtime()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiled_edition_matches_build_environment() {
        assert_eq!(CURRENT.label().to_ascii_lowercase(), env!("SPLIT_EDITION"));
    }

    #[test]
    fn renderer_capabilities_match_the_compiled_edition() {
        let borderless = matches!(CURRENT, Edition::Borderless);
        assert_eq!(windows_quick_access_enabled(), borderless);
        assert_eq!(native_notifications_enabled(), borderless);
    }
}
