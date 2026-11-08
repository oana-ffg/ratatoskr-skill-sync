//! A macOS window is interactive; background updates never construct a webview.
use anyhow::{Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Emitter, Manager};

pub(crate) const AUTOSTART_ARGS: [&str; 1] = ["--background"];
static INSTALLING: AtomicBool = AtomicBool::new(false);

/// Held from bundle installation until the restart or failure path completes.
/// ExitRequested must not interrupt the updater between bundle rename and move.
pub(crate) struct InstallGuard;
impl InstallGuard {
    pub(crate) fn acquire() -> Self {
        INSTALLING.store(true, Ordering::Release);
        Self
    }
}
impl Drop for InstallGuard {
    fn drop(&mut self) {
        INSTALLING.store(false, Ordering::Release);
    }
}
pub(crate) fn installing() -> bool {
    INSTALLING.load(Ordering::Acquire)
}
fn launch_mode_path() -> Result<std::path::PathBuf> {
    Ok(ratatoskr_core::updates::desktop_data_dir()?.join("restart-background.json"))
}
fn background_argument() -> bool {
    std::env::args_os().any(|arg| arg == "--background")
}
pub(crate) fn launching_hidden() -> bool {
    match launch_mode_path()
        .and_then(|p| ratatoskr_core::tray::peek_restart_mode(&p, background_argument()))
    {
        Ok(hidden) => hidden,
        Err(error) => {
            eprintln!("Ratatoskr restart mode: {error:#}");
            true
        }
    }
}
pub(crate) fn has_window(app: &tauri::AppHandle) -> bool {
    app.get_webview_window("main").is_some()
}
pub(crate) fn prepare_restart(app: &tauri::AppHandle) -> Result<()> {
    ratatoskr_core::tray::save_restart_mode(&launch_mode_path()?, !has_window(app))
}
pub(crate) fn prepare_quiet_restart(_app: &tauri::AppHandle) -> Result<()> {
    ratatoskr_core::tray::save_restart_mode(&launch_mode_path()?, true)
}
pub(crate) fn cancel_restart(_app: &tauri::AppHandle) -> Result<()> {
    ratatoskr_core::tray::clear_restart_mode(&launch_mode_path()?)
}
pub(crate) fn restart(app: &tauri::AppHandle) -> ! {
    INSTALLING.store(false, Ordering::Release);
    tauri_plugin_single_instance::destroy(app);
    crate::instance::release();
    app.restart()
}
pub(crate) fn quiet_restart(app: &tauri::AppHandle) -> Result<()> {
    // Reopen can arrive while a signed download or bundle replacement is running.
    // Make the final restart decision on the main thread, after queued Reopens.
    let handle = app.clone();
    app.run_on_main_thread(move || {
        let result = (|| -> Result<()> {
            if has_window(&handle) {
                prepare_restart(&handle)?;
                restart(&handle);
            }
            prepare_quiet_restart(&handle)?;
            tauri_plugin_single_instance::destroy(&handle);
            crate::instance::release();
            if let Err(error) =
                ratatoskr_core::updates::start_desktop_hidden(&std::env::current_exe()?)
            {
                crate::instance::claim()
                    .context("Reclaim desktop instance after restart failure")?;
                return Err(error.context("Restart after update"));
            }
            INSTALLING.store(false, Ordering::Release);
            // Do not deliver the predecessor's plugin Exit event after spawning:
            // it would unlink the successor's newly claimed single-instance socket.
            handle.cleanup_before_exit();
            std::process::exit(0);
        })();
        if let Err(error) = result {
            INSTALLING.store(false, Ordering::Release);
            let mut message = format!("Desktop update restart: {error:#}");
            if let Err(error) = cancel_restart(&handle) {
                message.push_str(&format!("; clearing restart intent: {error:#}"));
            }
            if let Err(error) = crate::commands::fail_headless_request(&message) {
                crate::commands::report_headless_error(&format!(
                    "{message}; recording failure: {error:#}"
                ));
            } else {
                crate::commands::report_headless_error(&message);
            }
            handle.exit(1);
        }
    })?;
    Ok(())
}

pub(crate) fn open(app: &tauri::AppHandle) -> Result<()> {
    app.set_activation_policy(tauri::ActivationPolicy::Regular)?;
    let window = match app.get_webview_window("main") {
        Some(window) => window,
        None => {
            let config = app
                .config()
                .app
                .windows
                .iter()
                .find(|w| w.label == "main")
                .context("Main window config missing")?;
            tauri::WebviewWindowBuilder::from_config(app, config)?.build()?
        }
    };
    window.show()?;
    window.unminimize()?;
    window.set_focus()?;
    window.emit("workspace-window-visible", true)?;
    Ok(())
}
pub(crate) fn setup(app: &tauri::AppHandle) -> Result<()> {
    // A corrupt saved intent must be visible in diagnostics, never a hidden dialog.
    let hidden = match ratatoskr_core::tray::consume_restart_mode(
        &launch_mode_path()?,
        background_argument(),
    ) {
        Ok(hidden) => hidden,
        Err(error) => {
            crate::commands::report_headless_error(&format!("Desktop restart intent: {error:#}"));
            true
        }
    };
    #[cfg(feature = "e2e-webdriver")]
    if hidden {
        eprintln!(
            "Ratatoskr headless webviews: {}",
            app.webview_windows().len()
        );
    }
    if hidden {
        app.set_activation_policy(tauri::ActivationPolicy::Accessory)?;
    } else {
        open(app)?;
    }
    Ok(())
}
/// Retire old opt-in login entries; the launchd helper now owns background work.
pub(crate) fn retire_login_entry(app: &tauri::AppHandle) -> Result<()> {
    use tauri_plugin_autostart::ManagerExt;
    retire_login_entry_with(
        || app.autolaunch().is_enabled().map_err(Into::into),
        || app.autolaunch().disable().map_err(Into::into),
    )
}
fn retire_login_entry_with(
    enabled: impl FnOnce() -> Result<bool>,
    disable: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if enabled()? {
        disable()?;
    }
    Ok(())
}
pub(crate) fn report(app: &tauri::AppHandle, error: anyhow::Error) {
    let message = format!("Desktop lifecycle: {error:#}");
    if !has_window(app) {
        crate::commands::report_headless_error(&message);
        return;
    }
    eprintln!("Ratatoskr: {message}");
    if crate::commands::app_error_listener_ready() {
        match app.emit("app-error", Some(&message)) {
            Ok(()) => return,
            Err(error) => eprintln!("Ratatoskr: cannot show lifecycle error: {error:#}"),
        }
    }
    use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
    app.dialog()
        .message(message)
        .title("Ratatoskr Skill Sync")
        .kind(MessageDialogKind::Error)
        .show(|_| {});
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn background_has_no_window_and_installation_alone_prevents_exit() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.macos.conf.json")).unwrap();
        assert_eq!(config["app"]["windows"][0]["visible"], false);
        assert_eq!(config["app"]["windows"][0]["create"], false);
        assert!(!installing());
        let guard = InstallGuard::acquire();
        assert!(installing());
        drop(guard);
        assert!(!installing());
    }
    #[test]
    fn login_cleanup_disables_existing_entries_and_surfaces_failures() {
        retire_login_entry_with(|| Ok(false), || panic!("no entry")).unwrap();
        let mut disabled = false;
        retire_login_entry_with(
            || Ok(true),
            || {
                disabled = true;
                Ok(())
            },
        )
        .unwrap();
        assert!(disabled);
        assert!(
            retire_login_entry_with(|| anyhow::bail!("unreadable"), || unreachable!()).is_err()
        );
        assert!(retire_login_entry_with(|| Ok(true), || anyhow::bail!("cannot disable")).is_err());
    }
}
