//! Shared resident-window lifecycle. Windows keeps its tray; macOS keeps its Dock.
use anyhow::{Context, Result};
use tauri::{Emitter, Manager};

pub(crate) const AUTOSTART_ARGS: [&str; 1] = ["--background"];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Intent {
    CloseWindow,
    Reopen,
    #[cfg(any(windows, test))]
    Quit,
}
#[derive(Debug, PartialEq, Eq)]
enum Action {
    Hide,
    Show,
    #[cfg(any(windows, test))]
    Exit,
}
fn action(intent: Intent) -> Action {
    match intent {
        Intent::CloseWindow => Action::Hide,
        Intent::Reopen => Action::Show,
        #[cfg(any(windows, test))]
        Intent::Quit => Action::Exit,
    }
}

/// Refresh arguments only for an existing opt-in login entry.
pub(crate) fn refresh_login(app: &tauri::AppHandle) -> Result<bool> {
    use tauri_plugin_autostart::ManagerExt;
    refresh_login_with(
        || app.autolaunch().is_enabled().map_err(Into::into),
        || app.autolaunch().enable().map_err(Into::into),
    )
}
fn refresh_login_with(
    is_enabled: impl FnOnce() -> Result<bool>,
    enable: impl FnOnce() -> Result<()>,
) -> Result<bool> {
    let enabled = is_enabled()?;
    if enabled {
        enable()?;
    }
    Ok(enabled)
}

fn launch_mode_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf> {
    Ok(app.path().app_data_dir()?.join("restart-background.json"))
}
pub(crate) fn prepare_restart(app: &tauri::AppHandle) -> Result<()> {
    let hidden = !app
        .get_webview_window("main")
        .context("Main window missing")?
        .is_visible()?;
    let result = ratatoskr_core::tray::save_restart_mode(&launch_mode_path(app)?, hidden);
    #[cfg(windows)]
    crate::tray::restart_outcome(app, &result)?;
    result
}
pub(crate) fn cancel_restart(app: &tauri::AppHandle) -> Result<()> {
    let result = ratatoskr_core::tray::clear_restart_mode(&launch_mode_path(app)?);
    #[cfg(windows)]
    crate::tray::restart_outcome(app, &result)?;
    result
}

fn perform(app: &tauri::AppHandle, intent: Intent) -> Result<()> {
    match action(intent) {
        Action::Hide => {
            let window = app
                .get_webview_window("main")
                .context("Main window missing")?;
            window.hide().context("Cannot hide main window")?;
            window
                .emit("workspace-window-visible", false)
                .context("Cannot notify workspace that the window is hidden")?;
        }
        Action::Show => {
            let window = app
                .get_webview_window("main")
                .context("Main window missing")?;
            window.show().context("Cannot show main window")?;
            window.unminimize().context("Cannot restore main window")?;
            window.set_focus().context("Cannot focus main window")?;
            window
                .emit("workspace-window-visible", true)
                .context("Cannot notify workspace that the window is visible")?;
        }
        #[cfg(any(windows, test))]
        Action::Exit => app.exit(0),
    }
    Ok(())
}

pub(crate) fn open(app: &tauri::AppHandle) -> Result<()> {
    perform(app, Intent::Reopen)
}
pub(crate) fn setup(app: &tauri::AppHandle) -> Result<()> {
    let window = app
        .get_webview_window("main")
        .context("Main window missing")?;
    let handle = app.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            if let Err(error) = perform(&handle, Intent::CloseWindow) {
                report(&handle, error);
            }
        }
    });
    let hidden = match ratatoskr_core::tray::consume_restart_mode(
        &launch_mode_path(app)?,
        std::env::args_os().any(|arg| arg == "--background"),
    ) {
        Ok(hidden) => hidden,
        Err(error) => {
            // Keep background work alive and surface the corrupt one-shot intent.
            eprintln!("Ratatoskr restart mode: {error:#}");
            #[cfg(windows)]
            crate::tray::restart_outcome(app, &Err(error))?;
            #[cfg(not(windows))]
            report(app, error);
            true
        }
    };
    if !hidden {
        open(app)?;
    }
    Ok(())
}

/// Never intercept application exit; Cmd+Q and the Windows Quit menu still exit.
#[cfg(windows)]
pub(crate) fn quit(app: &tauri::AppHandle) {
    if let Err(error) = perform(app, Intent::Quit) {
        report(app, error);
    }
}

pub(crate) fn report(app: &tauri::AppHandle, error: anyhow::Error) {
    #[cfg(windows)]
    crate::tray::report(app, error);
    #[cfg(not(windows))]
    {
        let message = format!("Desktop lifecycle: {error:#}");
        eprintln!("Ratatoskr: {message}");
        let visible = app
            .get_webview_window("main")
            .map(|window| window.is_visible())
            .transpose();
        let visible = match visible {
            Ok(value) => value.unwrap_or(false),
            Err(error) => {
                eprintln!("Ratatoskr: cannot read window visibility: {error:#}");
                false
            }
        };
        if use_frontend_error(visible, crate::commands::app_error_listener_ready()) {
            match app.emit("app-error", Some(&message)) {
                Ok(()) => return,
                Err(error) => eprintln!("Ratatoskr: cannot show lifecycle error: {error:#}"),
            }
        }
        // Early and hidden-window failures need a native dialog. A visible,
        // listening frontend receives the error only through app-error.
        use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
        app.dialog()
            .message(message)
            .title("Ratatoskr Skill Sync")
            .kind(MessageDialogKind::Error)
            .show(|_| {});
    }
}

#[cfg(any(target_os = "macos", test))]
fn use_frontend_error(visible: bool, listener_ready: bool) -> bool {
    visible && listener_ready
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn early_or_hidden_errors_use_native_dialog_and_visible_ready_errors_use_frontend() {
        assert!(!use_frontend_error(false, false));
        assert!(!use_frontend_error(false, true));
        assert!(!use_frontend_error(true, false));
        assert!(use_frontend_error(true, true));
    }
    #[test]
    fn closing_hides_reopening_shows_and_quit_exits() {
        assert_eq!(action(Intent::CloseWindow), Action::Hide);
        assert_eq!(action(Intent::Reopen), Action::Show);
        assert_eq!(action(Intent::Quit), Action::Exit);
    }
    #[test]
    fn background_launch_is_hidden_and_restart_intent_wins_once() {
        let temp = tempfile::tempdir().unwrap();
        let path = dunce::canonicalize(temp.path())
            .unwrap()
            .join("restart.json");
        assert_eq!(AUTOSTART_ARGS, ["--background"]);
        for contents in [
            include_str!("../tauri.macos.conf.json"),
            include_str!("../tauri.windows.conf.json"),
        ] {
            let config: serde_json::Value = serde_json::from_str(contents).unwrap();
            assert_eq!(config["app"]["windows"][0]["visible"], false);
        }
        assert!(!ratatoskr_core::tray::consume_restart_mode(&path, false).unwrap());
        assert!(ratatoskr_core::tray::consume_restart_mode(&path, true).unwrap());
        ratatoskr_core::tray::save_restart_mode(&path, false).unwrap();
        assert!(!ratatoskr_core::tray::consume_restart_mode(&path, true).unwrap());
        assert!(ratatoskr_core::tray::consume_restart_mode(&path, true).unwrap());
    }
    #[test]
    fn login_argument_refresh_preserves_opt_out_and_reports_errors() {
        assert!(!refresh_login_with(|| Ok(false), || panic!("must preserve opt-out")).unwrap());
        let mut refreshed = false;
        assert!(
            refresh_login_with(
                || Ok(true),
                || {
                    refreshed = true;
                    Ok(())
                }
            )
            .unwrap()
        );
        assert!(refreshed);
        assert!(
            refresh_login_with(
                || anyhow::bail!("login state unreadable"),
                || unreachable!()
            )
            .is_err()
        );
        assert!(
            refresh_login_with(|| Ok(true), || anyhow::bail!("login entry read-only")).is_err()
        );
    }
}
