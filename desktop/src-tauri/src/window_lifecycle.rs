//! Explicit show/hide signals supplement native focus/resize events. WebViews
//! do not reliably dispatch DOM visibilitychange when their window is hidden.
use tauri::{Emitter, Manager};

pub(crate) fn open(app: &tauri::AppHandle) -> tauri::Result<()> {
    let window = app
        .get_webview_window("main")
        .ok_or(tauri::Error::WindowNotFound)?;
    window.show()?;
    window.unminimize()?;
    window.set_focus()?;
    window.emit("workspace-window-visible", true)
}
