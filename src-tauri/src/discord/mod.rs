mod presence;
mod state;

pub(crate) fn start(enabled: bool) -> Result<(), String> {
    presence::start(enabled)
}

pub(crate) fn set_enabled(enabled: bool) -> Result<(), String> {
    presence::set_enabled(enabled)
}

pub(crate) fn stop() -> Result<(), String> {
    presence::stop()
}
