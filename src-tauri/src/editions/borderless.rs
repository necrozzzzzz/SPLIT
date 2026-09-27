use tauri::AppHandle;

pub(super) fn legacy_panorama_renderer_active() -> bool {
    crate::panorama_bridge::is_panorama_active()
}

pub(super) fn start_quick_access_runtime(app: AppHandle) -> Result<(), String> {
    // Compatibility only: this starts the existing experimental PNG transport.
    // The future Panorama edition renderer must not be added here.
    crate::panorama_bridge::start(app)
}

pub(super) fn stop_quick_access_runtime() -> Result<(), String> {
    crate::panorama_bridge::stop()
}

pub(super) fn start_notification_runtime() -> Result<(), String> {
    crate::notifications::start()
}

pub(super) fn stop_notification_runtime() -> Result<(), String> {
    crate::notifications::stop()
}
