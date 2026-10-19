//! Windows owns its sync timer and tray independently of the WebView.
use anyhow::{Context, Result};
use ratatoskr_core::{
    self as core, NotificationKind,
    tray::{LastSuccess, Problem, RunGate, TrayPreferences},
};
use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tauri::{
    Manager,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_notification::NotificationExt;

#[derive(Clone, Default, Serialize)]
pub(crate) struct Status {
    pub paused: bool,
    pub interval_seconds: u32,
    pub running: bool,
    pub last_success: Option<u64>,
    pub skills: usize,
    pub login_enabled: bool,
    #[serde(serialize_with = "problem_message")]
    pub error: Option<Problem>,
    #[serde(skip_serializing)]
    update_error: Option<Problem>,
    #[serde(skip_serializing)]
    notification_error: Option<Problem>,
    #[serde(skip_serializing)]
    login_error: Option<Problem>,
    #[serde(skip_serializing)]
    restart_error: Option<Problem>,
    #[serde(skip_serializing)]
    preferences_error: Option<Problem>,
    #[serde(skip_serializing)]
    preferences_observed: bool,
    #[serde(serialize_with = "problem_message")]
    pub migration_error: Option<Problem>,
}
fn problem_message<S: serde::Serializer>(
    problem: &Option<Problem>,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    problem
        .as_ref()
        .map(|problem| &problem.message)
        .serialize(serializer)
}
impl Status {
    fn current_error(&self) -> Option<Problem> {
        let errors: Vec<_> = [
            &self.error,
            &self.update_error,
            &self.notification_error,
            &self.login_error,
            &self.restart_error,
            &self.preferences_error,
        ]
        .into_iter()
        .flatten()
        .map(|problem| problem.message.clone())
        .collect();
        (!errors.is_empty()).then(|| Problem::message("Tray status", errors.join("; ")))
    }
    fn notification_outcomes(&self) -> Vec<(NotificationKind, Option<&Problem>)> {
        let mut outcomes = vec![
            (NotificationKind::Sync, self.error.as_ref()),
            (NotificationKind::Migration, self.migration_error.as_ref()),
            (NotificationKind::DesktopUpdate, self.update_error.as_ref()),
            (
                NotificationKind::Notifications,
                self.notification_error.as_ref(),
            ),
            (NotificationKind::Login, self.login_error.as_ref()),
            (NotificationKind::Restart, self.restart_error.as_ref()),
        ];
        if self.preferences_observed {
            outcomes.push((
                NotificationKind::Preferences,
                self.preferences_error.as_ref(),
            ));
        }
        outcomes
    }
    fn restore(&mut self, mut saved: Self) {
        saved.login_enabled = self.login_enabled;
        saved.notification_error = self.notification_error.take();
        saved.login_error = self.login_error.take();
        saved.restart_error = self.restart_error.take();
        saved.preferences_error = self.preferences_error.take();
        saved.preferences_observed = self.preferences_observed;
        *self = saved;
    }
}
fn restored_status(settings: &core::Settings) -> Result<Status> {
    let preferences = TrayPreferences::load(settings)?;
    let last = LastSuccess::load(settings)?;
    let jobs = core::job_status(settings)?;
    Ok(Status {
        paused: preferences.paused,
        interval_seconds: preferences.interval_seconds,
        last_success: last.time,
        skills: last.skills,
        error: jobs.last_run.and_then(|run| {
            run.error.map(|message| {
                let mut problem = Problem::message("Sync", message);
                if let Some(identity) = run.problem_identity {
                    problem.identity = identity;
                }
                problem
            })
        }),
        update_error: jobs
            .desktop_update_error
            .map(|message| Problem::message("Desktop update", message)),
        ..Default::default()
    })
}
pub(crate) struct Runtime {
    status: Mutex<Status>,
    preferences: Mutex<()>,
    gate: RunGate,
    sync_requested: AtomicBool,
    migrate_requested: AtomicBool,
    stopped: AtomicBool,
    pause_item: MenuItem<tauri::Wry>,
}
impl Runtime {
    fn status(&self) -> Result<Status> {
        let mut status = self
            .status
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
            .clone();
        status.error = status.current_error();
        Ok(status)
    }
    fn begin_run(&self, manual: bool) -> Result<Option<core::tray::RunPermit<'_>>> {
        let mut state = self
            .status
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        if (state.paused && !manual) || state.migration_error.is_some() {
            return Ok(None);
        }
        let permit = self.gate.enter(false);
        if permit.is_some() {
            state.running = true;
        }
        Ok(permit)
    }
    fn change(&self, action: impl FnOnce(&mut Status)) -> Result<()> {
        let mut state = self
            .status
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        action(&mut state);
        Ok(())
    }
}

pub(crate) fn status(app: &tauri::AppHandle) -> Result<Status> {
    use tauri_plugin_autostart::ManagerExt;
    let enabled = app.autolaunch().is_enabled()?;
    app.state::<Runtime>()
        .change(|s| s.login_enabled = enabled)?;
    app.state::<Runtime>().status()
}
fn migrate(
    app: &tauri::AppHandle,
    settings: &core::Settings,
    config: &std::path::Path,
) -> Result<bool> {
    use tauri_plugin_autostart::ManagerExt;
    core::migrate_windows_schedule(settings, config, || {
        let result = (|| -> Result<bool> {
            app.autolaunch().enable()?;
            anyhow::ensure!(
                app.autolaunch().is_enabled()?,
                "Open Ratatoskr at login is still disabled"
            );
            Ok(true)
        })();
        app.state::<Runtime>().change(|s| {
            s.login_error = result
                .as_ref()
                .err()
                .map(|error| Problem::from_error("Open at login", error));
            if let Ok(enabled) = &result {
                s.login_enabled = *enabled;
            }
        })?;
        result.map(|_| ())
    })
}

fn launch_mode_path(app: &tauri::AppHandle) -> Result<PathBuf> {
    Ok(app.path().app_data_dir()?.join("restart-background.json"))
}
/// One-shot intent overrides stale launch arguments after a foreground reopen.
/// The NSIS updater relaunches itself, so this also covers its process exit.
pub(crate) fn prepare_restart(app: &tauri::AppHandle) -> Result<()> {
    let hidden = !app
        .get_webview_window("main")
        .context("Main window missing")?
        .is_visible()?;
    core::tray::save_restart_mode(&launch_mode_path(app)?, hidden)?;
    app.state::<Runtime>().change(|s| s.restart_error = None)
}
pub(crate) fn cancel_restart(app: &tauri::AppHandle) -> Result<()> {
    core::tray::clear_restart_mode(&launch_mode_path(app)?)?;
    app.state::<Runtime>().change(|s| s.restart_error = None)
}
fn start_hidden(app: &tauri::AppHandle) -> Result<bool> {
    core::tray::consume_restart_mode(
        &launch_mode_path(app)?,
        std::env::args_os().any(|arg| arg == "--background"),
    )
}

pub(crate) fn open(app: &tauri::AppHandle) -> Result<()> {
    let window = app
        .get_webview_window("main")
        .context("Main window missing")?;
    window.show()?;
    window.unminimize()?;
    window.set_focus()?;
    Ok(())
}

fn edit_preferences(
    app: &tauri::AppHandle,
    edit: impl FnOnce(&mut TrayPreferences),
) -> Result<Status> {
    let runtime = app.state::<Runtime>();
    let result = (|| -> Result<()> {
        let _preferences = runtime
            .preferences
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        let settings = core::load(&core::default_config_path()?)?;
        let mut preferences = TrayPreferences::load(&settings)?;
        edit(&mut preferences);
        preferences.save(&settings)?;
        runtime.change(|s| {
            s.paused = preferences.paused;
            s.interval_seconds = preferences.interval_seconds;
        })?;
        if !preferences.paused {
            runtime.sync_requested.store(true, Ordering::Release);
        }
        Ok(())
    })();
    runtime.change(|s| {
        s.preferences_observed = true;
        s.preferences_error = result
            .as_ref()
            .err()
            .map(|error| Problem::from_error("Sync settings", error));
    })?;
    refresh(app)?;
    result?;
    runtime.status()
}
pub(crate) fn set_preferences(
    app: &tauri::AppHandle,
    paused: Option<bool>,
    interval_seconds: Option<u32>,
) -> Result<Status> {
    anyhow::ensure!(
        paused.is_some() || interval_seconds.is_some(),
        "Choose a background sync setting to change"
    );
    edit_preferences(app, |preferences| {
        if let Some(paused) = paused {
            preferences.paused = paused;
        }
        if let Some(seconds) = interval_seconds {
            preferences.interval_seconds = seconds;
        }
    })
}
pub(crate) fn login_outcome(
    app: &tauri::AppHandle,
    result: &std::result::Result<bool, String>,
) -> Result<()> {
    app.state::<Runtime>().change(|s| {
        s.login_error = result
            .as_ref()
            .err()
            .map(|message| Problem::message("Open at login", message));
        if let Ok(enabled) = result {
            s.login_enabled = *enabled;
        }
    })?;
    refresh(app)
}
pub(crate) fn request_sync(app: &tauri::AppHandle) {
    app.state::<Runtime>()
        .sync_requested
        .store(true, Ordering::Release);
}
pub(crate) fn request_migration(app: &tauri::AppHandle) {
    app.state::<Runtime>()
        .migrate_requested
        .store(true, Ordering::Release);
}

fn icon(app: &tauri::AppHandle, state: &Status) -> Result<tauri::image::Image<'static>> {
    let base = app
        .default_window_icon()
        .context("Application icon missing")?;
    let mut rgba = base.rgba().to_vec();
    // Preserve the squirrel identity; a small status badge remains legible in
    // Windows' 16/20/24px tray. Red takes precedence over paused amber.
    let color = if state.error.is_some() || state.migration_error.is_some() {
        Some([235, 100, 100, 255])
    } else if state.paused {
        Some([232, 185, 96, 255])
    } else {
        None
    };
    if let Some(color) = color {
        let (width, height) = (base.width() as i32, base.height() as i32);
        let radius = (width.min(height) / 5).max(2);
        let (cx, cy) = (width - radius - 1, height - radius - 1);
        for y in 0..height {
            for x in 0..width {
                if (x - cx).pow(2) + (y - cy).pow(2) <= radius.pow(2) {
                    let index = ((y * width + x) * 4) as usize;
                    rgba[index..index + 4].copy_from_slice(&color);
                }
            }
        }
    }
    Ok(tauri::image::Image::new_owned(
        rgba,
        base.width(),
        base.height(),
    ))
}
fn refresh(app: &tauri::AppHandle) -> Result<()> {
    let runtime = app.state::<Runtime>();
    let state = runtime.status()?;
    runtime.pause_item.set_text(if state.paused {
        "Resume sync"
    } else {
        "Pause sync"
    })?;
    let time = state
        .last_success
        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds as i64, 0))
        .map(|date| {
            date.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_else(|| "never".into());
    let mode = if state.migration_error.is_some() {
        "Blocked"
    } else if state.paused {
        "Paused"
    } else {
        "Active"
    };
    let tray = app.tray_by_id("sync").context("Sync tray missing")?;
    tray.set_tooltip(Some(format!(
        "Ratatoskr · {mode}\nLast success: {time}\n{} skills",
        state.skills
    )))?;
    tray.set_icon(Some(icon(app, &state)?))?;
    Ok(())
}

/// Surface callback failures in the visible status and CLI ledger. If even the
/// diagnostic store is inaccessible, stderr still reports both failures.
pub(crate) fn report(app: &tauri::AppHandle, error: anyhow::Error) {
    let problem = Problem::from_error("Sync", &error);
    let message = &problem.message;
    eprintln!("Ratatoskr tray: {message}");
    let result = (|| -> Result<()> {
        app.state::<Runtime>()
            .change(|s| s.error = Some(problem.clone()))?;
        core::record_background_problem(&core::default_config_path()?, Some(&problem))?;
        refresh(app)
    })();
    if let Err(error) = result {
        eprintln!("Ratatoskr tray diagnostics failed: {error:#}; original: {message}");
    }
}

pub(crate) fn setup(app: &mut tauri::App) -> Result<()> {
    let open_item = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
    let sync_item = MenuItem::with_id(app, "sync", "Sync now", true, None::<&str>)?;
    let pause_item = MenuItem::with_id(app, "pause", "Pause sync", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_item, &sync_item, &pause_item, &quit_item])?;
    app.manage(Runtime {
        status: Mutex::new(Status {
            interval_seconds: 60,
            ..Default::default()
        }),
        preferences: Mutex::new(()),
        gate: RunGate::default(),
        sync_requested: AtomicBool::new(false),
        migrate_requested: AtomicBool::new(true),
        stopped: AtomicBool::new(false),
        pause_item,
    });
    TrayIconBuilder::with_id("sync")
        .icon(
            app.default_window_icon()
                .context("Application icon missing")?
                .clone(),
        )
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| {
            let result = (|| -> Result<()> {
                match event.id.as_ref() {
                    "open" => open(app)?,
                    "sync" => request_sync(app),
                    "pause" => {
                        if let Err(error) = edit_preferences(app, |preferences| {
                            preferences.paused = !preferences.paused
                        }) {
                            eprintln!("Ratatoskr sync settings: {error:#}");
                        }
                    }
                    "quit" => {
                        app.state::<Runtime>()
                            .stopped
                            .store(true, Ordering::Release);
                        app.exit(0);
                    }
                    _ => (),
                }
                Ok(())
            })();
            if let Err(error) = result {
                report(app, error);
            }
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) && let Err(error) = open(tray.app_handle())
            {
                report(tray.app_handle(), error);
            }
        })
        .build(app)?;
    let window = app
        .get_webview_window("main")
        .context("Main window missing")?;
    let handle = app.handle().clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let result = handle
                .get_webview_window("main")
                .context("Main window missing")
                .and_then(|w| w.hide().map_err(Into::into));
            if let Err(error) = result {
                report(&handle, error);
            }
        }
    });
    // Remove owned legacy persistence before a network update check can delay
    // startup long enough for the old minute trigger to run again.
    let migration = (|| -> Result<()> {
        let config = core::default_config_path()?;
        if !config.try_exists()? {
            return Ok(());
        }
        let settings = core::load(&config)?;
        let result = migrate(app.handle(), &settings, &config);
        let mut saved = restored_status(&settings)?;
        saved.migration_error = result
            .err()
            .map(|error| Problem::from_error("Task migration", &error));
        let runtime = app.state::<Runtime>();
        runtime.change(|s| s.restore(saved))?;
        core::tray::record_problem(&settings, runtime.status()?.migration_error.as_ref())
    })();
    if let Err(error) = migration {
        report(app.handle(), error);
    }
    use tauri_plugin_autostart::ManagerExt;
    let autostart = (|| -> Result<()> {
        let enabled = app.autolaunch().is_enabled()?;
        if enabled {
            app.autolaunch().enable()?;
        }
        app.state::<Runtime>()
            .change(|s| s.login_enabled = enabled)?;
        Ok(())
    })();
    if let Err(error) = autostart {
        eprintln!("Ratatoskr open at login: {error:#}");
        app.state::<Runtime>()
            .change(|s| s.login_error = Some(Problem::from_error("Open at login", &error)))?;
    }
    let hidden = match start_hidden(app.handle()) {
        Ok(hidden) => hidden,
        Err(error) => {
            eprintln!("Ratatoskr restart mode: {error:#}");
            app.state::<Runtime>()
                .change(|s| s.restart_error = Some(Problem::from_error("Restart", &error)))?;
            true
        }
    };
    if !hidden {
        open(app.handle())?;
    }
    refresh(app.handle())?;
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        // The shared native updater completes startup before the first sync.
        if let Err(error) = crate::update_scheduler::startup(handle.clone()).await {
            eprintln!("Ratatoskr update: {error}");
        }
        tauri::async_runtime::spawn_blocking(move || run_loop(&handle));
    });
    Ok(())
}

pub(crate) fn update_outcome(app: &tauri::AppHandle, error: Option<String>) -> Result<()> {
    app.state::<Runtime>().change(|s| {
        s.update_error = error.map(|message| Problem::message("Desktop update", message))
    })?;
    refresh(app)
}

fn run_loop(app: &tauri::AppHandle) {
    let mut next_sync = Instant::now();
    let mut config_seen = None;
    let mut notifications = core::BackgroundNotifications::default();
    loop {
        let runtime = app.state::<Runtime>();
        if runtime.stopped.load(Ordering::Acquire) {
            return;
        }
        let result = (|| -> Result<()> {
            let config = core::default_config_path()?;
            if !config.try_exists()? {
                return Ok(());
            }
            let settings = match core::load(&config) {
                Ok(settings) => settings,
                Err(error) => {
                    // Preserve the same malformed-config run ledger as normal background runs.
                    if Instant::now() >= next_sync {
                        next_sync = Instant::now() + Duration::from_secs(60);
                        core::recorded_sync(&config)?;
                    }
                    return Err(error);
                }
            };
            if config_seen.as_ref() != Some(&config) {
                runtime.migrate_requested.store(true, Ordering::Release);
                let _preferences = runtime
                    .preferences
                    .lock()
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                let saved = restored_status(&settings)?;
                runtime.change(|s| s.restore(saved))?;
                config_seen = Some(config.clone());
            }
            if runtime.migrate_requested.swap(false, Ordering::AcqRel) {
                let _preferences = runtime
                    .preferences
                    .lock()
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                let migration = migrate(app, &settings, &config);
                if matches!(migration, Ok(true)) {
                    runtime.sync_requested.store(true, Ordering::Release);
                }
                runtime.change(|s| {
                    s.migration_error = migration
                        .err()
                        .map(|error| Problem::from_error("Task migration", &error))
                })?;
                let preferences = match TrayPreferences::load(&settings) {
                    Ok(preferences) => preferences,
                    Err(error) => {
                        runtime.change(|s| {
                            s.migration_error = Some(Problem::from_error("Sync settings", &error))
                        })?;
                        return Err(error);
                    }
                };
                runtime.change(|s| {
                    s.paused = preferences.paused;
                    s.interval_seconds = preferences.interval_seconds;
                })?;
            }
            let state = runtime.status()?;
            let manual = runtime.sync_requested.swap(false, Ordering::AcqRel);
            if state.migration_error.is_none()
                && (manual || (!state.paused && Instant::now() >= next_sync))
            {
                let Some(_permit) = runtime.begin_run(manual)? else {
                    return Ok(());
                };
                let outcome = core::recorded_sync(&config);
                runtime.change(|s| s.running = false)?;
                next_sync = Instant::now() + Duration::from_secs(state.interval_seconds.into());
                match outcome {
                    Ok(Some(plan)) => {
                        let last = LastSuccess {
                            time: Some(
                                std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)?
                                    .as_secs(),
                            ),
                            skills: plan.skills,
                        };
                        last.save(&settings)?;
                        runtime.change(|s| {
                            s.last_success = last.time;
                            s.skills = last.skills;
                            s.error = None;
                        })?;
                    }
                    Ok(None) => {
                        // A manual request remains pending while another operation
                        // owns sync.lock, including when automatic sync is paused.
                        if manual {
                            runtime.sync_requested.store(true, Ordering::Release);
                        }
                    }
                    Err(error) => {
                        runtime.change(|s| s.error = Some(Problem::from_error("Sync", &error)))?
                    }
                }
            }
            let state = runtime.status()?;
            let error = state.migration_error.as_ref().or(state.error.as_ref());
            core::tray::record_problem(&settings, error)?;
            refresh(app)
        })();
        if let Err(error) = result {
            report(app, error);
        }
        let notify = (|| -> Result<()> {
            let state = runtime
                .status
                .lock()
                .map_err(|e| anyhow::anyhow!(e.to_string()))?
                .clone();
            notifications.outcome(
                &core::default_config_path()?,
                &state.notification_outcomes(),
                |kind, message, recovered| {
                    let body = if recovered {
                        format!("{} recovered", kind.label())
                    } else {
                        message.to_owned()
                    };
                    app.notification()
                        .builder()
                        .title(format!("Ratatoskr Skill Sync · {}", kind.label()))
                        .body(body)
                        .show()
                        .context("Show Windows tray notification")?;
                    Ok(())
                },
            )
        })();
        let notification_error = notify
            .err()
            .map(|error| Problem::from_error("Notifications", &error));
        if let Some(error) = &notification_error {
            eprintln!("Ratatoskr notification: {error}");
        }
        if let Err(error) = runtime.change(|s| s.notification_error = notification_error) {
            report(app, error);
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sync_recovery_keeps_independent_notification_failures_active() {
        let mut state = Status {
            error: Some(Problem::message("Sync", "sync failed")),
            notification_error: Some(Problem::message(
                "Notifications",
                "notification ledger is read-only",
            )),
            ..Default::default()
        };
        state.error = None;
        assert_eq!(
            state.current_error().as_deref(),
            Some("notification ledger is read-only")
        );
        state.notification_error = None;
        assert!(state.current_error().is_none());
    }
    #[test]
    fn restoring_sync_state_preserves_app_health_and_independent_notification_identities() {
        let mut state = Status {
            error: Some(Problem::message("Sync", "old sync failure")),
            notification_error: Some(Problem::message(
                "Notifications",
                "notification ledger is read-only",
            )),
            login_error: Some(Problem::message("Open at login", "Run key is read-only")),
            restart_error: Some(Problem::message("Restart", "restart marker is invalid")),
            preferences_error: Some(Problem::message("Sync settings", "settings are read-only")),
            preferences_observed: true,
            ..Default::default()
        };
        state.restore(Status {
            paused: true,
            update_error: Some(Problem::message("Desktop update", "update unavailable")),
            ..Default::default()
        });
        let outcomes = state.notification_outcomes();
        assert_eq!(
            outcomes
                .iter()
                .find(|(kind, _)| *kind == NotificationKind::Sync)
                .unwrap()
                .1,
            None
        );
        for kind in [
            NotificationKind::Notifications,
            NotificationKind::Login,
            NotificationKind::Restart,
            NotificationKind::Preferences,
            NotificationKind::DesktopUpdate,
        ] {
            assert!(
                outcomes
                    .iter()
                    .find(|(source, _)| *source == kind)
                    .unwrap()
                    .1
                    .is_some()
            );
        }
        assert!(state.paused);
        assert!(
            !Status::default()
                .notification_outcomes()
                .iter()
                .any(|(kind, _)| *kind == NotificationKind::Preferences)
        );
    }
    #[test]
    fn paused_restart_preserves_sync_failure_until_a_run_actually_recovers() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let settings = core::Settings {
            source: root.join("source"),
            machine: "test".into(),
            state_dir: root.join("state"),
            ..Default::default()
        };
        std::fs::create_dir(&settings.source).unwrap();
        let config = root.join("config.toml");
        core::save(&config, &settings).unwrap();
        assert!(core::recorded_sync(&config).is_err());
        TrayPreferences {
            paused: true,
            interval_seconds: 60,
        }
        .save(&settings)
        .unwrap();
        let state = restored_status(&settings).unwrap();
        assert!(state.paused && state.error.is_some());
        let mut notifications = core::tray::NotificationState::default();
        notifications
            .deliver(state.error.as_ref(), true, |_| Ok(()))
            .unwrap();
        let reopened = restored_status(&settings).unwrap();
        notifications
            .deliver(reopened.error.as_ref(), true, |_| {
                panic!("A paused restart must not announce recovery")
            })
            .unwrap();
        std::fs::write(
            settings.state_dir.join("test/tray-preferences.json"),
            "invalid",
        )
        .unwrap();
        assert!(
            restored_status(&settings).is_err(),
            "Unreadable preferences must not select active defaults"
        );
    }
}
