//! Windows owns its sync/update timers in Rust, independent of WebView timers.
use anyhow::{Context, Result};
use ratatoskr_core::{
    self as core,
    tray::{LastSuccess, RunGate, TrayPreferences},
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
    pub error: Option<String>,
    pub migration_error: Option<String>,
}
pub(crate) struct Runtime {
    status: Mutex<Status>,
    preferences: Mutex<()>,
    gate: RunGate,
    sync_requested: AtomicBool,
    migrate_requested: AtomicBool,
    stopped: AtomicBool,
    pub(crate) desktop_busy: AtomicBool,
    pause_item: MenuItem<tauri::Wry>,
}
impl Runtime {
    fn status(&self) -> Result<Status> {
        Ok(self
            .status
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
            .clone())
    }
    fn begin_run(&self) -> Result<Option<core::tray::RunPermit<'_>>> {
        let mut state = self
            .status
            .lock()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        if state.paused || state.migration_error.is_some() {
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
    app.state::<Runtime>().status()
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
    core::tray::save_restart_mode(&launch_mode_path(app)?, hidden)
}
pub(crate) fn cancel_restart(app: &tauri::AppHandle) -> Result<()> {
    core::tray::clear_restart_mode(&launch_mode_path(app)?)
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

pub(crate) fn set_preferences(
    app: &tauri::AppHandle,
    paused: bool,
    interval_seconds: u32,
) -> Result<Status> {
    let runtime = app.state::<Runtime>();
    let _preferences = runtime
        .preferences
        .lock()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let settings = core::load(&core::default_config_path()?)?;
    TrayPreferences {
        paused,
        interval_seconds,
    }
    .save(&settings)?;
    runtime.change(|s| {
        s.paused = paused;
        s.interval_seconds = interval_seconds;
    })?;
    if !paused {
        runtime.sync_requested.store(true, Ordering::Release);
    }
    refresh(app)?;
    runtime.status()
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
    let message = format!("{error:#}");
    eprintln!("Ratatoskr tray: {message}");
    let result = (|| -> Result<()> {
        app.state::<Runtime>()
            .change(|s| s.error = Some(message.clone()))?;
        core::record_background_problem(&core::default_config_path()?, Some(&message))?;
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
        sync_requested: AtomicBool::new(true),
        migrate_requested: AtomicBool::new(true),
        stopped: AtomicBool::new(false),
        desktop_busy: AtomicBool::new(false),
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
                        let state = status(app)?;
                        set_preferences(app, !state.paused, state.interval_seconds)?;
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
        let result = core::migrate_windows_schedule(&settings, &config);
        let runtime = app.state::<Runtime>();
        runtime.change(|s| s.migration_error = result.err().map(|e| format!("{e:#}")))?;
        let preferences = TrayPreferences::load(&settings)?;
        let last = LastSuccess::load(&settings)?;
        runtime.change(|s| {
            s.paused = preferences.paused;
            s.interval_seconds = preferences.interval_seconds;
            s.last_success = last.time;
            s.skills = last.skills;
        })?;
        core::tray::record_problem(&settings, runtime.status()?.migration_error.as_deref())
    })();
    if let Err(error) = migration {
        report(app.handle(), error);
    }
    use tauri_plugin_autostart::ManagerExt;
    let autostart = (|| -> Result<()> {
        if app.autolaunch().is_enabled()? {
            app.autolaunch().enable()?;
        }
        Ok(())
    })();
    if let Err(error) = autostart {
        report(app.handle(), error);
    }
    let hidden = match start_hidden(app.handle()) {
        Ok(hidden) => hidden,
        Err(error) => {
            report(app.handle(), error);
            false
        }
    };
    if !hidden {
        open(app.handle())?;
    }
    refresh(app.handle())?;
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        // Updates precede the first sync, as on macOS. Hidden windows need no JS.
        if let Err(error) = crate::commands::automatic_update(handle.clone(), true, true).await {
            report(&handle, anyhow::anyhow!(error));
        }
        let worker = handle.clone();
        tauri::async_runtime::spawn_blocking(move || run_loop(&worker));
        let mut next_update = Instant::now() + Duration::from_secs(60);
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if handle.state::<Runtime>().stopped.load(Ordering::Acquire) {
                return;
            }
            if Instant::now() >= next_update {
                next_update = Instant::now() + Duration::from_secs(60);
                match crate::commands::automatic_update(handle.clone(), false, false).await {
                    Ok(true)
                        if !handle
                            .state::<Runtime>()
                            .desktop_busy
                            .load(Ordering::Acquire) =>
                    {
                        if let Err(error) =
                            crate::commands::automatic_update(handle.clone(), true, false).await
                        {
                            report(&handle, anyhow::anyhow!(error));
                        }
                    }
                    Ok(_) => (),
                    Err(error) => report(&handle, anyhow::anyhow!(error)),
                }
            }
        }
    });
    Ok(())
}

fn run_loop(app: &tauri::AppHandle) {
    let mut next_sync = Instant::now();
    let mut next_migration = Instant::now();
    let mut config_seen = None;
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
                config_seen = Some(config.clone());
                next_migration = Instant::now();
                let last = LastSuccess::load(&settings)?;
                runtime.change(|s| {
                    s.last_success = last.time;
                    s.skills = last.skills;
                })?;
            }
            if runtime.migrate_requested.swap(false, Ordering::AcqRel)
                || Instant::now() >= next_migration
            {
                next_migration = Instant::now() + Duration::from_secs(60);
                let _preferences = runtime
                    .preferences
                    .lock()
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                let migration = core::migrate_windows_schedule(&settings, &config);
                runtime
                    .change(|s| s.migration_error = migration.err().map(|e| format!("{e:#}")))?;
                let preferences = TrayPreferences::load(&settings)?;
                runtime.change(|s| {
                    s.paused = preferences.paused;
                    s.interval_seconds = preferences.interval_seconds;
                })?;
            }
            let state = runtime.status()?;
            let manual = runtime.sync_requested.swap(false, Ordering::AcqRel);
            if state.migration_error.is_none()
                && !state.paused
                && (manual || Instant::now() >= next_sync)
            {
                let Some(_permit) = runtime.begin_run()? else {
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
                    Ok(None) => (), // Existing operation holds sync.lock; no run was started.
                    Err(error) => runtime.change(|s| s.error = Some(format!("{error:#}")))?,
                }
            }
            let state = runtime.status()?;
            let error = state.migration_error.as_deref().or(state.error.as_deref());
            core::tray::record_problem(&settings, error)?;
            refresh(app)
        })();
        if let Err(error) = result {
            report(app, error);
        }
        let notify = (|| -> Result<()> {
            let state = runtime.status()?;
            let error = state.migration_error.as_deref().or(state.error.as_deref());
            core::notify_background(&core::default_config_path()?, error, |body| {
                app.notification()
                    .builder()
                    .title("Ratatoskr Skill Sync")
                    .body(body)
                    .show()?;
                Ok(())
            })
        })();
        if let Err(error) = notify {
            report(app, error);
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}
