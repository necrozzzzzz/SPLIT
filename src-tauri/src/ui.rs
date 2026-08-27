use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};

pub fn emit_to_main_if_present<R, S>(app: &AppHandle<R>, event: &str, payload: S)
where
    R: Runtime,
    S: Serialize + Clone,
{
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit(event, payload);
    }
}
