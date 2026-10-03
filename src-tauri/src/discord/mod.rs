mod config;
mod presence;
mod state;

pub use config::{DiscordPresenceConfig, DiscordPresenceSection, PartyDisplay};
pub(crate) use state::{canonicalize_hero_key, is_known_hero_key};

pub(crate) fn config() -> DiscordPresenceConfig {
    config::current()
}

pub(crate) fn apply_config(config: DiscordPresenceConfig) -> Result<(), String> {
    config::apply(config)
}

pub(crate) fn start(enabled: bool) -> Result<(), String> {
    presence::start(enabled)
}

pub(crate) fn set_enabled(enabled: bool) -> Result<(), String> {
    presence::set_enabled(enabled)
}

pub(crate) fn stop() -> Result<(), String> {
    presence::stop()
}
