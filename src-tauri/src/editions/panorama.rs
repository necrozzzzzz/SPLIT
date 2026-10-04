use tauri::AppHandle;

pub(super) fn panorama_renderer_active() -> bool {
    true
}

pub(super) fn start_quick_access_runtime(app: AppHandle) -> Result<(), String> {
    crate::panorama_runtime::start(app)
}

pub(super) fn stop_quick_access_runtime() -> Result<(), String> {
    crate::panorama_runtime::stop()
}

pub(super) fn start_notification_runtime() -> Result<(), String> {
    // Reserved for future Panorama notifications.
    Ok(())
}

pub(super) fn stop_notification_runtime() -> Result<(), String> {
    Ok(())
}
