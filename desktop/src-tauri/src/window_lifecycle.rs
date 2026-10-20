//! Explicit show/hide signals supplement native focus/resize events. WebViews
//! do not reliably dispatch DOM visibilitychange when their window is hidden.
use anyhow::{Context, Result};
use tauri::{Emitter, Manager};

pub(crate) fn open(app: &tauri::AppHandle) -> Result<()> {
    let window = app
        .get_webview_window("main")
        .context("Main window missing")?;
    window.show().context("Cannot show main window")?;
    window.unminimize().context("Cannot restore main window")?;
    window.set_focus().context("Cannot focus main window")?;
    window
        .emit("workspace-window-visible", true)
        .context("Cannot notify workspace that the window is visible")
}
