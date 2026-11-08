//! macOS launchd helper policy. No desktop scheduler state is consulted.
use super::*;
use std::path::{Path, PathBuf};
type Time = chrono::DateTime<chrono::FixedOffset>;
const LEDGER: &str = "desktop-update-watchdog.json";
const NOTIFICATION: &str = "desktop-update-notification.json";
const INSTALL_FAILED: &str = "Desktop update install failed:";

/// A distinct schema deliberately ignores the old watchdog's desktop baseline.
/// Only our own check attempts establish health after migration.
#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
struct State {
    helper_checks: Option<Checks>,
}
#[derive(Deserialize, Serialize)]
struct Checks {
    settings: UpdateSettings,
    since: Time,
    attempt: Option<Time>,
    success: Option<Time>,
    error: Option<String>,
    pending: Option<String>,
    install_attempt: Option<Time>,
    install_error: Option<String>,
}
impl Checks {
    fn new(settings: &UpdateSettings, now: Time) -> Self {
        Self {
            settings: settings.clone(),
            since: now,
            attempt: None,
            success: None,
            error: None,
            pending: None,
            install_attempt: None,
            install_error: None,
        }
    }
    fn due(&self, now: Time) -> Result<bool> {
        let slot = desktop_slot(&self.settings, now)?;
        Ok(self.success.is_none_or(|last| last.naive_local() < slot)
            && self
                .attempt
                .is_none_or(|last| now - last >= chrono::Duration::hours(1)))
    }
    fn overdue(&self, now: Time) -> Result<bool> {
        let state = DesktopUpdateState {
            settings: Some(self.settings.clone()),
            last_success: self.success,
            monitoring_since: Some(self.since),
            ..Default::default()
        };
        state.overdue(&self.settings, now)
    }
}

#[cfg(target_os = "macos")]
pub(crate) fn helper_desktop_tick(settings: &crate::Settings) -> Result<()> {
    let exe = std::env::current_exe()?;
    // In-process desktop/library calls do not own helper checks.
    if exe.file_stem().is_none_or(|name| name != "ratatoskr") {
        return Ok(());
    }
    if !bundled_helper_path(&exe) {
        return clear_monitoring(settings);
    }
    let data = desktop_data_dir()?;
    tick(
        settings,
        &data,
        &exe,
        env!("CARGO_PKG_VERSION"),
        chrono::Local::now().fixed_offset(),
        || check(&settings.updates, env!("CARGO_PKG_VERSION")),
        || running_desktop(&data),
        start_desktop_hidden,
    )
}

fn clear_monitoring(settings: &crate::Settings) -> Result<()> {
    crate::notifications::update_for(settings, NOTIFICATION, |error| {
        error
            .filter(|e| !e.starts_with(DESKTOP_OVERDUE) && !e.starts_with(MONITORING_FAILED))
            .map(str::to_owned)
    })
}

/// Transport and process operations are injected; the persisted protocol is real in tests.
#[allow(clippy::too_many_arguments)]
fn tick(
    settings: &crate::Settings,
    data: &Path,
    exe: &Path,
    version: &str,
    now: Time,
    check: impl FnOnce() -> Result<Option<Release>>,
    running: impl FnOnce() -> Result<Option<PathBuf>>,
    spawn: impl FnOnce(&Path) -> Result<()>,
) -> Result<()> {
    if !bundled_helper_path(exe) || !settings.updates.automatic {
        return clear_monitoring(settings);
    }
    let path = settings.state_dir.join(&settings.machine).join(LEDGER);
    let mut failure = None;
    let result = (|| -> Result<Option<String>> {
        crate::config::no_symlinks(&path)?;
        let stored = if path.try_exists()? {
            Some(std::fs::read(&path)?)
        } else {
            None
        };
        let mut state: State = stored
            .as_deref()
            .map(serde_json::from_slice)
            .transpose()?
            .unwrap_or_default();
        if state
            .helper_checks
            .as_ref()
            .is_none_or(|s| s.settings != settings.updates)
        {
            state.helper_checks = Some(Checks::new(&settings.updates, now));
        }
        let checks = state.helper_checks.as_mut().expect("initialized checks");
        // A fresh helper (including an upgrade) checks before considering overdue alerts.
        if checks.due(now)? {
            checks.attempt = Some(now);
            // Persist before networking so failed/crashed attempts cannot run each minute.
            crate::config::atomic_write(&path, &serde_json::to_vec(&state)?)?;
            let checked = check();
            let checks = state.helper_checks.as_mut().expect("initialized checks");
            match checked {
                Ok(release) => {
                    checks.success = Some(now);
                    checks.error = None;
                    let pending = release.map(|r| r.version);
                    if checks.pending != pending {
                        checks.install_attempt = None;
                        checks.install_error = None;
                    }
                    checks.pending = pending;
                }
                Err(error) => {
                    let error = format!("{error:#}");
                    checks.error = Some(error.clone());
                    failure = Some(error);
                }
            }
        }
        let checks = state.helper_checks.as_mut().expect("initialized checks");
        if let Some(release) = checks.pending.clone() {
            if semver::Version::parse(version.trim_start_matches('v'))?
                >= semver::Version::parse(release.trim_start_matches('v'))?
            {
                checks.pending = None;
                checks.install_attempt = None;
                checks.install_error = None;
            } else {
                // Inspect an earlier request even while a headless app holds the instance.
                // This is how a hung installer becomes visible after one hour.
                let (request, response) = desktop_request_status(data)?;
                let app = exe.with_file_name("ratatoskr-desktop");
                let request = request.filter(|q| q.app == app);
                let response = response.filter(|_| request.is_some());
                if let Some(attempt) = checks.install_attempt
                    && now - attempt >= chrono::Duration::hours(1)
                {
                    let cause = match response.as_ref().map(|r| &r.outcome) {
                        Some(DesktopRequestOutcome::Failed { error }) => Some(error.as_str()),
                        Some(
                            DesktopRequestOutcome::Installed { .. }
                            | DesktopRequestOutcome::Current,
                        ) => None,
                        _ => {
                            Some("the request is still unanswered or installation has not settled")
                        }
                    };
                    if let Some(cause) = cause {
                        checks.install_error = Some(format!(
                            "{INSTALL_FAILED} install of version {release} did not finish: {cause}"
                        ));
                    }
                }
                let retry = checks
                    .install_attempt
                    .is_none_or(|last| now - last >= chrono::Duration::hours(1));
                if retry && running()?.is_none() {
                    checks.install_attempt = Some(now);
                    // Persist launch pacing before creating the request or spawning.
                    crate::config::atomic_write(&path, &serde_json::to_vec(&state)?)?;
                    let launched = (|| -> Result<()> {
                        // Reuse unanswered requests, including interrupted Installing responses.
                        if request.is_none()
                            || response.as_ref().is_some_and(|r| {
                                !matches!(r.outcome, DesktopRequestOutcome::Installing { .. })
                            })
                        {
                            request_desktop_update(data, &app, now)?;
                        }
                        spawn(&app)
                    })();
                    if let Err(error) = launched {
                        state
                            .helper_checks
                            .as_mut()
                            .expect("initialized checks")
                            .install_error =
                            Some(format!("{INSTALL_FAILED} version {release}: {error:#}"));
                    }
                }
            }
        }
        let checks = state.helper_checks.as_ref().expect("initialized checks");
        let alert = if let Some(error) = &checks.install_error {
            Some(error.clone())
        } else if checks.overdue(now)? {
            checks
                .error
                .as_ref()
                .map(|error| format!("{DESKTOP_OVERDUE}: {error}"))
        } else {
            None
        };
        let serialized = serde_json::to_vec(&state)?;
        if stored.as_deref() != Some(serialized.as_slice()) {
            crate::config::atomic_write(&path, &serialized)?;
        }
        Ok(alert)
    })();
    // Network failures are persisted and returned to the sync diagnostics, but only
    // two completed failed windows produce a remote alert. Sync itself continues.
    crate::notifications::update_for(settings, NOTIFICATION, |current| match &result {
        Err(error) => Some(format!("{MONITORING_FAILED} {error:#}")),
        Ok(Some(error)) => Some(error.clone()),
        Ok(None) => current
            .filter(|e| {
                !e.starts_with(DESKTOP_OVERDUE)
                    && !e.starts_with(MONITORING_FAILED)
                    && !e.starts_with(INSTALL_FAILED)
            })
            .map(str::to_owned),
    })?;
    result?;
    if let Some(error) = failure {
        anyhow::bail!("Desktop update check failed: {error}");
    }
    Ok(())
}

#[cfg(test)]
mod tests;
