//! Desktop update command orchestration; transport and policy stay separate.
use super::*;
#[cfg(not(target_os = "macos"))]
use crate::desktop_updates::automatic_update_with;
use crate::desktop_updates::{UpdateContext, manual_update_with};
use ratatoskr_core as core;
use tauri_plugin_updater::UpdaterExt;

fn install_desktop_update(
    app: &tauri::AppHandle,
    update: tauri_plugin_updater::Update,
) -> anyhow::Result<()> {
    // The updater verifies the signed download before invoking the installer.
    let bytes = tauri::async_runtime::block_on(update.download(|_, _| (), || ()))?;
    install_downloaded_update(app, &update, bytes, false)
}

/// `quiet` installs run unattended: the restart that follows keeps the window hidden.
fn install_downloaded_update(
    app: &tauri::AppHandle,
    update: &tauri_plugin_updater::Update,
    bytes: Vec<u8>,
    quiet: bool,
) -> anyhow::Result<()> {
    #[cfg(not(any(windows, target_os = "macos")))]
    let _ = (app, quiet);
    #[cfg(target_os = "macos")]
    let quiet = quiet && !crate::window_lifecycle::has_window(app);
    #[cfg(target_os = "macos")]
    let _installing = crate::window_lifecycle::InstallGuard::acquire();
    #[cfg(target_os = "macos")]
    anyhow::ensure!(
        !quiet || !crate::update_scheduler::expired(app),
        "Headless desktop update exceeded its 30-minute deadline"
    );
    #[cfg(any(windows, target_os = "macos"))]
    if quiet {
        crate::window_lifecycle::prepare_quiet_restart(app)?;
    } else {
        crate::window_lifecycle::prepare_restart(app)?;
    }
    #[cfg(windows)]
    {
        let result = core::windows_update::supervise(
            &std::env::current_exe()?,
            &core::updates::desktop_data_dir()?,
            &app.config().identifier,
            &update.version,
            &bytes,
        );
        if let Err(error) = result {
            crate::window_lifecycle::cancel_restart(app)?;
            return Err(error);
        }
        // The supervisor acknowledged its process handle before this exit. It
        // starts NSIS only after our executable has completely stopped.
        crate::instance::release();
        app.cleanup_before_exit();
        std::process::exit(0);
    }
    #[cfg(not(windows))]
    let result = update.install(bytes);
    #[cfg(not(windows))]
    {
        #[cfg(target_os = "macos")]
        if result.is_err() {
            crate::window_lifecycle::cancel_restart(app)?;
        }
        result.map_err(Into::into)
    }
}

fn desktop_updater(
    app: &tauri::AppHandle,
    settings: &core::updates::UpdateSettings,
) -> anyhow::Result<tauri_plugin_updater::Updater> {
    let builder = app
        .updater_builder()
        .pubkey(settings.tauri_public_key()?)
        .endpoints(vec![settings.endpoint().parse()?])?
        .timeout(std::time::Duration::from_secs(60));
    // CI's real signed-update journey trusts only its generated TLS certificate.
    // This feature is forbidden in release builds by main.rs.
    #[cfg(feature = "e2e-webdriver")]
    let builder = if let Some(path) = std::env::var_os("RATATOSKR_E2E_UPDATE_CA") {
        let certificate = reqwest::Certificate::from_der(&std::fs::read(path)?)?;
        builder.configure_client(move |client| client.add_root_certificate(certificate.clone()))
    } else {
        builder
    };
    Ok(builder.build()?)
}

#[cfg(not(target_os = "macos"))]
struct Nightly {
    pending: Option<(core::updates::UpdateSettings, tauri_plugin_updater::Update)>,
}
#[cfg(not(target_os = "macos"))]
static NIGHTLY: Mutex<Nightly> = Mutex::new(Nightly { pending: None });

fn update_result<T>(context: &UpdateContext, result: anyhow::Result<T>) -> Reply<T> {
    let error = result
        .as_ref()
        .err()
        .map(|e| format!("Desktop update: {e:#}"));
    context
        .report_outcome(error.as_deref())
        .map_err(|notification| {
            format!(
                "{}; alert delivery failed: {notification:#}",
                error.as_deref().unwrap_or("Update completed")
            )
        })?;
    result.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub(crate) async fn startup_update(app: tauri::AppHandle) -> Reply<bool> {
    crate::update_scheduler::window_startup(app).await
}

#[cfg(not(target_os = "macos"))]
pub(crate) async fn automatic_update(
    app: tauri::AppHandle,
    install: bool,
    launch: bool,
) -> Reply<bool> {
    tauri::async_runtime::spawn_blocking(move || {
        if install && !launch && crate::update_scheduler::busy(&app) {
            return Ok(false);
        }
        let _guard = if install {
            match MUTATION.try_lock() {
                Ok(guard) => Some(guard),
                Err(std::sync::TryLockError::WouldBlock) => return Ok(false),
                Err(error) => return Err(error.to_string()),
            }
        } else {
            None
        };
        let context = update_context()?;
        let mut installed = false;
        let result = (|| -> anyhow::Result<_> {
            let mut nightly = NIGHTLY.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
            automatic_update_with(
                &context,
                &mut nightly.pending,
                chrono::Local::now().fixed_offset(),
                install,
                launch,
                |settings| {
                    tauri::async_runtime::block_on(desktop_updater(&app, settings)?.check())
                        .map_err(Into::into)
                },
                |update| {
                    // A download can outlast the idle period. Keep the checked
                    // update pending when editing begins during that request.
                    let bytes = tauri::async_runtime::block_on(update.download(|_, _| (), || ()))?;
                    if crate::update_scheduler::busy(&app)
                        || !context
                            .installation_allowed(chrono::Local::now().fixed_offset(), launch)?
                    {
                        return Ok(false);
                    }
                    install_downloaded_update(&app, update, bytes, true)?;
                    installed = true;
                    Ok(true)
                },
            )
        })();
        let report = (cfg!(windows) && launch)
            || result
                .as_ref()
                .map_or(true, |outcome| outcome.checked || outcome.installed);
        let result = result.map(|outcome| {
            if install {
                outcome.installed
            } else {
                outcome.available
            }
        });
        let result = if report {
            update_result(&context, result)
        } else {
            result.map_err(|e| format!("{e:#}"))
        };
        let restart = installed;
        // Installation succeeded even if alert recovery delivery failed; never lose that error.
        if restart {
            if let Err(error) = &result {
                eprintln!("Ratatoskr: {error}");
            }
            if let Err(error) = crate::window_lifecycle::quiet_restart(&app) {
                // The old process keeps running; persist and alert like any update failure.
                return update_result(
                    &context,
                    Err(error.context("Installed the update, but restarting failed")),
                );
            }
        }
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Carries out an install queued by `ratatoskr update --desktop` with the nightly
/// path's signed updater and idle checks. Busy work defers to a later poll.
pub(crate) async fn requested_update(app: tauri::AppHandle) -> Reply<()> {
    use core::updates::DesktopRequestOutcome;
    tauri::async_runtime::spawn_blocking(move || {
        let data = core::updates::desktop_data_dir().map_err(|e| e.to_string())?;
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let Some(request) =
            core::updates::pending_desktop_request(&data, &exe).map_err(|e| format!("{e:#}"))?
        else {
            return Ok(());
        };
        if crate::update_scheduler::busy(&app) {
            return Ok(());
        }
        let _guard = match MUTATION.try_lock() {
            Ok(guard) => guard,
            Err(std::sync::TryLockError::WouldBlock) => return Ok(()),
            Err(error) => return Err(error.to_string()),
        };
        let context = update_context()?;
        // Same order as the manual path: serializes update-health writes with nightly checks.
        #[cfg(not(target_os = "macos"))]
        let _nightly = NIGHTLY.lock().map_err(|e| e.to_string())?;
        let result = (|| -> anyhow::Result<Option<DesktopRequestOutcome>> {
            // Defer before checking too: the spawning helper still owns sync.lock.
            // Retrying a contended lock must not perform a GET every five seconds.
            #[cfg(target_os = "macos")]
            let Some(_idle) = context.idle_guard(false)? else {
                return Ok(None);
            };
            let checked =
                tauri::async_runtime::block_on(desktop_updater(&app, context.settings())?.check())?;
            // Like a manual check: counts toward health without using an automatic slot.
            context.record_explicit_check(chrono::Local::now().fixed_offset())?;
            let Some(update) = checked else {
                return Ok(Some(DesktopRequestOutcome::Current));
            };
            #[cfg(not(target_os = "macos"))]
            let Some(_idle) = context.idle_guard(false)? else {
                return Ok(None);
            };
            let bytes = tauri::async_runtime::block_on(update.download(|_, _| (), || ()))?;
            if crate::update_scheduler::busy(&app) {
                return Ok(None);
            }
            #[cfg(target_os = "macos")]
            anyhow::ensure!(
                !crate::update_scheduler::expired(&app),
                "Headless desktop update exceeded its 30-minute deadline"
            );
            let installing = DesktopRequestOutcome::Installing {
                version: update.version.clone(),
            };
            // Windows exits inside the installer, so record the attempt first.
            core::updates::answer_desktop_request(
                &data,
                &request,
                chrono::Local::now().fixed_offset(),
                installing.clone(),
            )?;
            install_downloaded_update(&app, &update, bytes, true)?;
            // The restarted app confirms its version and records the final outcome.
            Ok(Some(installing))
        })();
        let outcome = match result {
            Ok(None) => return Ok(()),
            Ok(Some(outcome)) => outcome,
            Err(error) => DesktopRequestOutcome::Failed {
                error: format!("{error:#}"),
            },
        };
        let restart = matches!(outcome, DesktopRequestOutcome::Installing { .. });
        let answered = if restart {
            Ok(())
        } else {
            core::updates::answer_desktop_request(
                &data,
                &request,
                chrono::Local::now().fixed_offset(),
                outcome.clone(),
            )
        };
        // Keep both errors when the failure itself could not be recorded.
        let reported = update_result(
            &context,
            match (&outcome, answered) {
                (DesktopRequestOutcome::Failed { error }, Ok(())) => {
                    Err(anyhow::anyhow!("{error}"))
                }
                (DesktopRequestOutcome::Failed { error }, Err(write)) => Err(anyhow::anyhow!(
                    "{error}; recording this failure also failed: {write:#}"
                )),
                (_, Err(write)) => Err(write.context("Record requested update")),
                (_, Ok(())) => Ok(()),
            },
        );
        if let DesktopRequestOutcome::Installing { version } = &outcome {
            if let Err(error) = &reported {
                eprintln!("Ratatoskr: {error}");
            }
            if let Err(error) = crate::window_lifecycle::quiet_restart(&app) {
                // The old process keeps running: settle the request as failed and alert.
                let error = format!("Installed {version}, but restarting failed: {error:#}");
                let answered = core::updates::answer_desktop_request(
                    &data,
                    &request,
                    chrono::Local::now().fixed_offset(),
                    DesktopRequestOutcome::Failed {
                        error: error.clone(),
                    },
                );
                return update_result(
                    &context,
                    Err(match answered {
                        Ok(()) => anyhow::anyhow!("{error}"),
                        Err(write) => anyhow::anyhow!(
                            "{error}; recording this failure also failed: {write:#}"
                        ),
                    }),
                );
            }
        }
        reported
    })
    .await
    .map_err(|e| e.to_string())?
}

/// At startup, settle a requested install that ended with this restart and report it.
pub(crate) fn settle_requested_update() -> Reply<()> {
    use core::updates::DesktopRequestOutcome;
    let data = core::updates::desktop_data_dir().map_err(|e| e.to_string())?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let settled = core::updates::finish_desktop_request(
        &data,
        &exe,
        env!("CARGO_PKG_VERSION"),
        chrono::Local::now().fixed_offset(),
    )
    .map_err(|e| format!("Desktop update request: {e:#}"))?;
    match settled {
        None => Ok(()),
        Some(DesktopRequestOutcome::Failed { error }) => {
            update_result(&update_context()?, Err(anyhow::anyhow!("{error}")))
        }
        Some(_) => update_result(&update_context()?, Ok(())),
    }
}

fn update_context() -> Reply<UpdateContext> {
    let data_dir = core::updates::desktop_data_dir().map_err(|e| e.to_string())?;
    UpdateContext::load(&config()?, &data_dir).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub(crate) async fn read_update_settings() -> Reply<UpdateStatus> {
    off_main_thread(move || {
        let context = update_context()?;
        let settings = context.settings();
        Ok(UpdateStatus {
            endpoint: settings.endpoint().into(),
            override_active: settings.override_active(),
            automatic: settings.automatic,
            time: settings.time.clone(),
        })
    })
    .await
}

#[tauri::command]
pub(crate) async fn update_desktop(
    app: tauri::AppHandle,
    install: bool,
) -> Reply<crate::desktop_updates::DesktopUpdateResult> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
        let context = update_context()?;
        #[cfg(not(target_os = "macos"))]
        let mut nightly = NIGHTLY.lock().map_err(|e| e.to_string())?;
        #[cfg(not(target_os = "macos"))]
        {
            nightly.pending = None;
        }
        let result = manual_update_with(
            &context,
            chrono::Local::now().fixed_offset(),
            install,
            |settings| {
                tauri::async_runtime::block_on(desktop_updater(&app, settings)?.check())
                    .map_err(Into::into)
            },
            |update| update.version.clone(),
            |update| {
                install_desktop_update(&app, update)?;
                Ok(())
            },
        );
        update_result(&context, result)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub(crate) fn restart_desktop(app: tauri::AppHandle) -> Reply<()> {
    #[cfg(any(windows, target_os = "macos"))]
    crate::window_lifecycle::prepare_restart(&app).map_err(|e| format!("{e:#}"))?;
    crate::window_lifecycle::restart(&app);
}

/// Background lifecycle diagnostics must never open a native dialog.
#[cfg(target_os = "macos")]
pub(crate) fn report_headless_error(message: &str) {
    eprintln!("Ratatoskr: {message}");
    if let Err(error) = update_context().and_then(|context| {
        context
            .report_outcome(Some(message))
            .map_err(|e| format!("{e:#}"))
    }) {
        eprintln!("Ratatoskr: update alert delivery failed: {error}");
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn fail_headless_request(message: &str) -> anyhow::Result<()> {
    let data = core::updates::desktop_data_dir()?;
    let exe = std::env::current_exe()?;
    let (request, response) = core::updates::desktop_request_status(&data)?;
    if let Some(request) = request.filter(|q| q.app == exe)
        && response.is_none_or(|r| {
            matches!(
                r.outcome,
                core::updates::DesktopRequestOutcome::Installing { .. }
            )
        })
    {
        core::updates::answer_desktop_request(
            &data,
            &request,
            chrono::Local::now().fixed_offset(),
            core::updates::DesktopRequestOutcome::Failed {
                error: message.into(),
            },
        )?;
    }
    Ok(())
}
