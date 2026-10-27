//! One native update clock for every desktop platform, independent of WebView life.
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tauri::{Emitter, Manager};

#[derive(Default)]
pub(crate) struct Runtime {
    busy: AtomicBool,
    startup: tokio::sync::OnceCell<Result<bool, String>>,
}

pub(crate) fn busy(app: &tauri::AppHandle) -> bool {
    app.state::<Runtime>().busy.load(Ordering::Acquire)
}
pub(crate) fn set_busy(app: &tauri::AppHandle, busy: bool) {
    app.state::<Runtime>().busy.store(busy, Ordering::Release);
}

/// Both native startup and the UI await the same decision before enabling work.
pub(crate) async fn startup(app: tauri::AppHandle) -> Result<bool, String> {
    app.state::<Runtime>()
        .startup
        .get_or_init(|| crate::commands::automatic_update(app.clone(), true, true))
        .await
        .clone()
}

fn report(app: &tauri::AppHandle, result: &Result<bool, String>) {
    let diagnostic = (|| -> anyhow::Result<()> {
        let config = ratatoskr_core::default_config_path()?;
        let error = if config.try_exists()? {
            ratatoskr_core::job_status(&ratatoskr_core::load(&config)?)?
                .desktop_update_error
                .or_else(|| result.as_ref().err().cloned())
        } else {
            result.as_ref().err().cloned()
        };
        if let Some(error) = &error {
            eprintln!("Ratatoskr desktop update: {error}");
        }
        app.emit("app-error", &error)?;
        #[cfg(windows)]
        crate::tray::update_outcome(app, error)?;
        Ok(())
    })();
    if let Err(error) = diagnostic {
        let message = format!("Desktop update diagnostics failed: {error:#}; result: {result:?}");
        eprintln!("Ratatoskr: {message}");
        if let Err(error) = app.emit("app-error", Some(&message)) {
            eprintln!("Ratatoskr: cannot show desktop update error: {error:#}");
        }
        #[cfg(windows)]
        crate::tray::report(app, error);
    }
}

/// CLI requests are polled often; the nightly policy keeps its one-minute clock.
const REQUEST_POLL: Duration = Duration::from_secs(5);
const POLICY_TICKS: u32 = 12;

fn finish_request() -> Result<(), String> {
    let data = ratatoskr_core::updates::desktop_data_dir().map_err(|e| e.to_string())?;
    ratatoskr_core::updates::finish_desktop_request(
        &data,
        env!("CARGO_PKG_VERSION"),
        chrono::Local::now().fixed_offset(),
    )
    .map_err(|e| format!("Desktop update request: {e:#}"))
}

pub(crate) fn setup(app: &tauri::App) {
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        if let Err(error) = finish_request() {
            report(&handle, &Err(error));
        }
        report(&handle, &startup(handle.clone()).await);
        let mut ticks = 0;
        let mut request_error = None;
        loop {
            // Wall-clock policy is reevaluated after every wake, including after
            // sleep. No dependency on a JS interval or the window being visible.
            tokio::time::sleep(REQUEST_POLL).await;
            // Report a persistent request failure once, not on every poll.
            let failure = crate::commands::requested_update(handle.clone())
                .await
                .err();
            if let Some(error) = &failure
                && failure != request_error
            {
                report(&handle, &Err(error.clone()));
            }
            request_error = failure;
            ticks += 1;
            if ticks < POLICY_TICKS {
                continue;
            }
            ticks = 0;
            let checked = crate::commands::automatic_update(handle.clone(), false, false).await;
            report(&handle, &checked);
            if matches!(checked, Ok(true)) && !busy(&handle) {
                let installed =
                    crate::commands::automatic_update(handle.clone(), true, false).await;
                report(&handle, &installed);
            }
        }
    });
}
