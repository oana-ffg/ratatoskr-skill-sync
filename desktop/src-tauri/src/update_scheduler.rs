//! Native request polling; non-macOS platforms also own the resident nightly clock.
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tauri::{Emitter, Manager};

#[derive(Default)]
pub(crate) struct Runtime {
    busy: AtomicBool,
    #[cfg(target_os = "macos")]
    expired: AtomicBool,
    #[cfg(not(target_os = "macos"))]
    startup: tokio::sync::OnceCell<Result<bool, String>>,
}

pub(crate) fn busy(app: &tauri::AppHandle) -> bool {
    app.state::<Runtime>().busy.load(Ordering::Acquire)
}
pub(crate) fn set_busy(app: &tauri::AppHandle, busy: bool) {
    app.state::<Runtime>().busy.store(busy, Ordering::Release);
}

#[cfg(windows)]
pub(crate) fn window_destroyed(app: &tauri::AppHandle) {
    app.state::<Runtime>().window_destroyed();
}
impl Runtime {
    #[cfg(any(windows, all(test, not(target_os = "macos"))))]
    fn window_destroyed(&self) {
        self.busy.store(false, Ordering::Release);
    }
    #[cfg(not(target_os = "macos"))]
    async fn window_startup(
        &self,
        initialize: impl std::future::Future<Output = Result<bool, String>>,
    ) -> Result<bool, String> {
        // Completed startup belongs to the tray session, not each new page. The
        // workspace's durable job status supplies current update diagnostics.
        if self.startup.get().is_some() {
            return Ok(false);
        }
        self.startup.get_or_init(|| initialize).await.clone()
    }
}

pub(crate) async fn window_startup(app: tauri::AppHandle) -> Result<bool, String> {
    #[cfg(not(target_os = "macos"))]
    return app
        .state::<Runtime>()
        .window_startup(crate::commands::automatic_update(app.clone(), true, true))
        .await;
    #[cfg(target_os = "macos")]
    startup(app).await
}

/// Both native startup and the UI await the same decision before enabling work.
#[cfg(not(target_os = "macos"))]
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
        #[cfg(windows)]
        let error = error.or(ratatoskr_core::windows_update::failure(
            &ratatoskr_core::updates::desktop_data_dir()?,
        )?);
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

/// CLI requests are polled on their own task, so request work never delays the
/// nightly policy's one-minute clock.
const REQUEST_POLL: Duration = Duration::from_secs(5);

#[cfg(not(target_os = "macos"))]
fn poll_requests(handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        if let Err(error) = crate::commands::settle_requested_update() {
            report(&handle, &Err(error));
        }
        let mut last_error = None;
        loop {
            tokio::time::sleep(REQUEST_POLL).await;
            // Report a persistent request failure once, not on every poll.
            let failure = crate::commands::requested_update(handle.clone())
                .await
                .err();
            if let Some(error) = &failure
                && failure != last_error
            {
                report(&handle, &Err(error.clone()));
            }
            last_error = failure;
        }
    });
}

pub(crate) fn setup(app: &tauri::App) {
    let handle = app.handle().clone();
    poll_requests(handle.clone());
    #[cfg(not(target_os = "macos"))]
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

#[cfg(target_os = "macos")]
pub(crate) async fn startup(_app: tauri::AppHandle) -> Result<bool, String> {
    Ok(false)
}

#[cfg(target_os = "macos")]
pub(crate) fn expired(app: &tauri::AppHandle) -> bool {
    app.state::<Runtime>().expired.load(Ordering::Acquire)
}

#[cfg(target_os = "macos")]
fn pending_request() -> anyhow::Result<bool> {
    Ok(ratatoskr_core::updates::pending_desktop_request(
        &ratatoskr_core::updates::desktop_data_dir()?,
        &std::env::current_exe()?,
    )?
    .is_some())
}

#[cfg(target_os = "macos")]
fn headless_done(window: bool, installing: bool, pending: bool) -> bool {
    !window && !installing && !pending
}

#[cfg(target_os = "macos")]
fn poll_requests(handle: tauri::AppHandle) {
    // The independent deadline also covers a stalled download. Never terminate
    // in the updater's bundle replacement critical section.
    let deadline_handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(Duration::from_secs(30 * 60)).await;
        if crate::window_lifecycle::has_window(&deadline_handle) {
            return;
        }
        deadline_handle
            .state::<Runtime>()
            .expired
            .store(true, Ordering::Release);
        let message = "Headless desktop update exceeded its 30-minute deadline";
        // Alert at the deadline even if bundle replacement cannot safely stop yet.
        crate::commands::report_headless_error(message);
        while crate::window_lifecycle::installing() {
            tokio::time::sleep(REQUEST_POLL).await;
        }
        if let Err(error) = crate::commands::fail_headless_request(message) {
            crate::commands::report_headless_error(&format!(
                "{message}; recording failure: {error:#}"
            ));
        }
        deadline_handle.exit(1);
    });
    tauri::async_runtime::spawn(async move {
        if let Err(error) = crate::commands::settle_requested_update() {
            crate::commands::report_headless_error(&error);
            if !crate::window_lifecycle::has_window(&handle) {
                handle.exit(1);
                return;
            }
            report(&handle, &Err(error));
        }
        loop {
            // Disk reads/locks stay off the main thread and async executor.
            let pending = tauri::async_runtime::spawn_blocking(pending_request).await;
            match pending {
                Ok(Ok(pending)) => {
                    if headless_done(
                        crate::window_lifecycle::has_window(&handle),
                        crate::window_lifecycle::installing(),
                        pending,
                    ) {
                        handle.exit(0);
                        return;
                    }
                }
                error => {
                    crate::commands::report_headless_error(&format!(
                        "Read desktop update request: {error:?}"
                    ));
                    if !crate::window_lifecycle::has_window(&handle) {
                        handle.exit(1);
                        return;
                    }
                }
            }
            if let Err(error) = crate::commands::requested_update(handle.clone()).await {
                crate::commands::report_headless_error(&error);
            }
            tokio::time::sleep(REQUEST_POLL).await;
        }
    });
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    #[test]
    fn background_exits_without_a_request_but_an_open_window_or_install_keeps_it_alive() {
        assert!(headless_done(false, false, false));
        assert!(!headless_done(false, false, true));
        assert!(!headless_done(false, true, false));
        assert!(!headless_done(true, false, false));
    }
}

#[cfg(all(test, not(target_os = "macos")))]
mod window_tests {
    use super::*;
    #[test]
    fn destroying_a_busy_editor_releases_update_deferral() {
        let runtime = Runtime::default();
        runtime.busy.store(true, Ordering::Release);
        runtime.window_destroyed();
        assert!(!runtime.busy.load(Ordering::Acquire));
    }
    #[test]
    fn reopened_windows_do_not_replay_old_startup_failures() {
        tauri::async_runtime::block_on(async {
            let runtime = Runtime::default();
            assert_eq!(
                runtime
                    .window_startup(async { Err("old failure".into()) })
                    .await,
                Err("old failure".into())
            );
            assert_eq!(
                runtime
                    .window_startup(async { panic!("startup must run once") })
                    .await,
                Ok(false)
            );
        });
    }
    #[test]
    fn a_window_opened_during_startup_waits_for_the_same_decision() {
        tauri::async_runtime::block_on(async {
            let runtime = std::sync::Arc::new(Runtime::default());
            let native = runtime.clone();
            let (started, waiting) = tokio::sync::oneshot::channel();
            let (finish, finished) = tokio::sync::oneshot::channel();
            let task = tauri::async_runtime::spawn(async move {
                native
                    .startup
                    .get_or_init(|| async {
                        started.send(()).unwrap();
                        finished.await.unwrap();
                        Err("first failure".into())
                    })
                    .await
                    .clone()
            });
            waiting.await.unwrap();
            let ui = runtime.window_startup(async { panic!("must share native startup") });
            let mut ui = std::pin::pin!(ui);
            // Poll once while the native operation is deliberately unfinished.
            assert!(
                std::future::poll_fn(|cx| std::task::Poll::Ready(ui.as_mut().poll(cx)))
                    .await
                    .is_pending()
            );
            finish.send(()).unwrap();
            assert_eq!(ui.await, Err("first failure".into()));
            assert_eq!(task.await.unwrap(), Err("first failure".into()));
        });
    }
}
