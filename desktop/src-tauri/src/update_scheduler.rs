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

pub(crate) fn setup(app: &tauri::App) {
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        report(&handle, &startup(handle.clone()).await);
        loop {
            // Wall-clock policy is reevaluated after every wake, including after
            // sleep. No dependency on a JS interval or the window being visible.
            tokio::time::sleep(Duration::from_secs(60)).await;
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
