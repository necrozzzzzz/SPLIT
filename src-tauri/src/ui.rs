use serde::Serialize;
use tauri::{
    AppHandle,
    Emitter,
    Manager,
    Runtime,
};

pub fn emit_to_main_if_present<R, S>(
    app: &AppHandle<R>,
    event: &str,
    payload: S,
)
where
    R: Runtime,
    S: Serialize + Clone,
{
    for label in [
        "main",
        "quick-access",
    ] {
        if let Some(window) =
            app.get_webview_window(label)
        {
            let _ =
                window.emit(
                    event,
                    payload.clone(),
                );
        }
    }
}