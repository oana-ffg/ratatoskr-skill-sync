use anyhow::{Context, Result};
use tauri::{Emitter, Manager};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub(crate) use macos::*;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(crate) use windows::*;

pub(crate) const AUTOSTART_ARGS: [&str; 1] = ["--background"];

fn launch_mode_path() -> Result<std::path::PathBuf> {
    Ok(ratatoskr_core::updates::desktop_data_dir()?.join("restart-background.json"))
}
fn background_argument() -> bool {
    std::env::args_os().any(|arg| arg == "--background")
}
fn main_window(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow> {
    if let Some(window) = app.get_webview_window("main") {
        return Ok(window);
    }
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|w| w.label == "main")
        .context("Main window config missing")?;
    Ok(tauri::WebviewWindowBuilder::from_config(app, config)?.build()?)
}
fn show(window: &tauri::WebviewWindow) -> Result<()> {
    window.show().context("Cannot show main window")?;
    window.unminimize().context("Cannot restore main window")?;
    window.set_focus().context("Cannot focus main window")?;
    window.emit("workspace-window-visible", ())?;
    Ok(())
}
