use tauri::AppHandle;

pub(super) fn legacy_panorama_renderer_active() -> bool {
    false
}

pub(super) fn start_quick_access_runtime(_app: AppHandle) -> Result<(), String> {
    // Reserved for the future CitadelHTMLPanel/HTMLTitle bridge.
    Ok(())
}

pub(super) fn stop_quick_access_runtime() -> Result<(), String> {
    Ok(())
}

pub(super) fn start_notification_runtime() -> Result<(), String> {
    // Reserved for future Panorama notifications.
    Ok(())
}

pub(super) fn stop_notification_runtime() -> Result<(), String> {
    Ok(())
}
