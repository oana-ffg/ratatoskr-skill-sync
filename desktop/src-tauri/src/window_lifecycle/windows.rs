//! Windows keeps the tray alive, creating an interactive window only on demand.
use super::{background_argument, launch_mode_path, main_window, show};
use anyhow::{Context, Result};
use std::sync::{Condvar, Mutex};
use std::time::Duration;
use tauri::Manager;

// Never hold this lock on the event thread: WebView2 creation needs its message pump.
static OPENER: WindowOpener = WindowOpener::new();

enum ShowOutcome {
    Visible,
    Missing,
    Failed(anyhow::Error),
}

struct WindowOpener {
    serial: Mutex<()>,
    destruction: Destruction,
}
impl WindowOpener {
    const fn new() -> Self {
        Self {
            serial: Mutex::new(()),
            destruction: Destruction::new(),
        }
    }
    fn open(
        &self,
        mut create: impl FnMut() -> Result<()>,
        mut show: impl FnMut() -> Result<ShowOutcome>,
    ) -> Result<()> {
        let _guard = self
            .serial
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        loop {
            let generation = self.destruction.generation()?;
            create()?;
            match show()? {
                ShowOutcome::Visible => return Ok(()),
                ShowOutcome::Missing => continue,
                ShowOutcome::Failed(error) => {
                    // A queued destroy must finish before another build can reuse `main`.
                    // Keep real show failures if no destruction occurs.
                    if !self.destruction.wait(generation, Duration::from_secs(5))? {
                        return Err(error);
                    }
                }
            }
        }
    }
}

struct Destruction {
    generation: Mutex<u64>,
    changed: Condvar,
}
impl Destruction {
    const fn new() -> Self {
        Self {
            generation: Mutex::new(0),
            changed: Condvar::new(),
        }
    }
    fn generation(&self) -> Result<u64> {
        Ok(*self
            .generation
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?)
    }
    fn notify(&self) -> Result<()> {
        let mut generation = self
            .generation
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        *generation += 1;
        self.changed.notify_all();
        Ok(())
    }
    fn wait(&self, previous: u64, timeout: Duration) -> Result<bool> {
        let generation = self
            .generation
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let (generation, _) = self
            .changed
            .wait_timeout_while(generation, timeout, |g| *g == previous)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        Ok(*generation != previous)
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

/// Restart after a user's explicit choice, keeping the window's current visibility.
pub(crate) fn prepare_restart(app: &tauri::AppHandle) -> Result<()> {
    let hidden = app.get_webview_window("main").is_none();
    save_restart(app, hidden)
}
/// Unattended installs restart hidden, so they never open or focus a window.
pub(crate) fn prepare_quiet_restart(app: &tauri::AppHandle) -> Result<()> {
    save_restart(app, true)
}
fn save_restart(app: &tauri::AppHandle, hidden: bool) -> Result<()> {
    let result = ratatoskr_core::tray::save_restart_mode(&launch_mode_path()?, hidden);
    crate::tray::restart_outcome(app, &result)?;
    result
}
pub(crate) fn restart(app: &tauri::AppHandle) -> ! {
    crate::instance::release();
    app.restart()
}
/// Unattended restart uses the native app restart path.
pub(crate) fn quiet_restart(app: &tauri::AppHandle) -> Result<()> {
    restart(app)
}
pub(crate) fn cancel_restart(app: &tauri::AppHandle) -> Result<()> {
    let result = ratatoskr_core::tray::clear_restart_mode(&launch_mode_path()?);
    crate::tray::restart_outcome(app, &result)?;
    result
}

/// Enqueue creation; tray, single-instance and setup callbacks must return immediately.
pub(crate) fn open(app: &tauri::AppHandle) -> Result<()> {
    let handle = app.clone();
    std::thread::Builder::new()
        .name("workspace-window".into())
        .spawn(move || {
            if let Err(error) = open_serialized(&handle) {
                report(&handle, error);
            }
        })
        .context("Cannot start window worker")?;
    Ok(())
}
fn open_serialized(app: &tauri::AppHandle) -> Result<()> {
    OPENER.open(
        || {
            // Do not retain a stale handle while waiting for destruction.
            drop(main_window(app)?);
            Ok(())
        },
        || {
            let (send, receive) = std::sync::mpsc::sync_channel(1);
            let handle = app.clone();
            app.run_on_main_thread(move || {
                let result = (|| -> Result<bool> {
                    let Some(window) = handle.get_webview_window("main") else {
                        return Ok(false);
                    };
                    // A Destroy message can retire the native window before Tauri removes its
                    // label. Setters alone return Ok for that stale handle. A getter detects it.
                    window.is_visible().context("Cannot inspect main window")?;
                    show(&window)?;
                    Ok(true)
                })();
                if let Err(error) = send.send(result) {
                    report(
                        &handle,
                        anyhow::anyhow!("Cannot complete window open: {error}"),
                    );
                }
            })?;
            Ok(
                match receive.recv().context("Window open callback stopped")? {
                    Ok(true) => ShowOutcome::Visible,
                    Ok(false) => ShowOutcome::Missing,
                    Err(error) => ShowOutcome::Failed(error),
                },
            )
        },
    )
}

pub(crate) fn destroyed(app: &tauri::AppHandle) -> Result<()> {
    crate::commands::set_app_error_listener(false);
    crate::update_scheduler::window_destroyed(app);
    OPENER.destruction.notify()
}
pub(crate) fn prevent_exit(code: Option<i32>) -> bool {
    code.is_none()
}
pub(crate) fn setup(app: &tauri::AppHandle) -> Result<()> {
    let hidden = match ratatoskr_core::tray::consume_restart_mode(
        &launch_mode_path()?,
        background_argument(),
    ) {
        Ok(hidden) => hidden,
        Err(error) => {
            // Keep background work alive and surface the corrupt one-shot intent.
            eprintln!("Ratatoskr restart mode: {error:#}");
            crate::tray::restart_outcome(app, &Err(error))?;
            true
        }
    };
    if !hidden {
        open(app)?;
    }
    Ok(())
}

/// Quit is an explicit application exit, including when there is no window.
pub(crate) fn quit(app: &tauri::AppHandle) {
    app.exit(0);
}
pub(crate) fn report(app: &tauri::AppHandle, error: anyhow::Error) {
    crate::tray::report(app, error);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_implicit_last_window_exit_is_prevented() {
        assert!(prevent_exit(None));
        assert!(!prevent_exit(Some(0)));
        assert!(!prevent_exit(Some(tauri::RESTART_EXIT_CODE)));
    }
    #[test]
    fn destruction_before_or_during_wait_releases_reopen() {
        let state = std::sync::Arc::new(Destruction::new());
        let previous = state.generation().unwrap();
        state.notify().unwrap();
        assert!(state.wait(previous, Duration::ZERO).unwrap());
        let previous = state.generation().unwrap();
        let signal = state.clone();
        let thread = std::thread::spawn(move || signal.notify().unwrap());
        assert!(state.wait(previous, Duration::from_secs(5)).unwrap());
        thread.join().unwrap();
        assert!(
            !state
                .wait(state.generation().unwrap(), Duration::ZERO)
                .unwrap()
        );
    }
    #[test]
    fn simultaneous_opens_build_only_one_window() {
        let opener = std::sync::Arc::new(WindowOpener::new());
        let builds = std::sync::Arc::new(Mutex::new(0));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let threads: Vec<_> = (0..2)
            .map(|_| {
                let (opener, builds, barrier) = (opener.clone(), builds.clone(), barrier.clone());
                std::thread::spawn(move || {
                    barrier.wait();
                    opener
                        .open(
                            || {
                                let missing = *builds.lock().unwrap() == 0;
                                if missing {
                                    std::thread::yield_now();
                                    *builds.lock().unwrap() += 1;
                                }
                                Ok(())
                            },
                            || Ok(ShowOutcome::Visible),
                        )
                        .unwrap();
                })
            })
            .collect();
        barrier.wait();
        for thread in threads {
            thread.join().unwrap();
        }
        assert_eq!(*builds.lock().unwrap(), 1);
    }
    #[test]
    fn an_open_racing_destruction_rebuilds_and_shows() {
        let opener = WindowOpener::new();
        let mut attempts = 0;
        let mut shown = 0;
        opener
            .open(
                || {
                    attempts += 1;
                    Ok(())
                },
                || {
                    shown += 1;
                    if shown == 1 {
                        opener.destruction.notify().unwrap();
                        Ok(ShowOutcome::Failed(anyhow::anyhow!(
                            "old native window retired"
                        )))
                    } else {
                        Ok(ShowOutcome::Visible)
                    }
                },
            )
            .unwrap();
        assert_eq!((attempts, shown), (2, 2));
        // Also cover destruction removing the manager entry before show executes.
        let mut shown = 0;
        opener
            .open(
                || Ok(()),
                || {
                    shown += 1;
                    Ok(if shown == 2 {
                        ShowOutcome::Visible
                    } else {
                        ShowOutcome::Missing
                    })
                },
            )
            .unwrap();
        assert_eq!(shown, 2);
        // Dispatch and build failures must propagate, without retries.
        assert!(
            opener
                .open(|| anyhow::bail!("build failed"), || unreachable!())
                .is_err()
        );
        assert!(
            opener
                .open(|| Ok(()), || anyhow::bail!("dispatch failed"))
                .is_err()
        );
    }
    #[test]
    fn background_launch_is_hidden_and_restart_intent_wins_once() {
        let temp = tempfile::tempdir().unwrap();
        let path = dunce::canonicalize(temp.path())
            .unwrap()
            .join("restart.json");
        assert_eq!(super::super::AUTOSTART_ARGS, ["--background"]);
        for contents in [
            include_str!("../../tauri.macos.conf.json"),
            include_str!("../../tauri.windows.conf.json"),
        ] {
            let config: serde_json::Value = serde_json::from_str(contents).unwrap();
            assert_eq!(config["app"]["windows"][0]["visible"], false);
            assert_eq!(config["app"]["windows"][0]["create"], false);
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
