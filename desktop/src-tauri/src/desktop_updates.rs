use anyhow::Result;
use ratatoskr_core::{
    self as core, Settings,
    updates::{DesktopUpdateState, UpdateSettings},
};
use serde::Serialize;
use std::path::{Path, PathBuf};

/// Only an absent config selects defaults. Invalid or unreadable configs remain errors.
pub(crate) struct UpdateContext {
    workspace: Option<Settings>,
    updates: UpdateSettings,
    state_path: PathBuf,
}

/// A configured install holds workspace locks; an unconfigured app has no work to lock.
pub(crate) struct InstallationGuard {
    _workspace: Option<core::updates::UpdateGuard>,
}

impl UpdateContext {
    pub(crate) fn load(config: &Path, app_data: &Path) -> Result<Self> {
        let workspace = if config.try_exists()? {
            Some(core::load(config)?)
        } else {
            None
        };
        let updates = workspace
            .as_ref()
            .map(|s| s.updates.clone())
            .unwrap_or_default();
        Ok(Self {
            workspace,
            updates,
            state_path: app_data.join("desktop-update.json"),
        })
    }

    pub(crate) fn settings(&self) -> &UpdateSettings {
        &self.updates
    }

    fn state(&self) -> Result<DesktopUpdateState> {
        let state = DesktopUpdateState::load(&self.state_path)?;
        if !self.state_path.try_exists()?
            && let Some(settings) = &self.workspace
        {
            // Read the old workspace ledger once; subsequent writes use the app directory.
            return DesktopUpdateState::load(
                &settings
                    .state_dir
                    .join(&settings.machine)
                    .join("desktop-update.json"),
            );
        }
        Ok(state)
    }

    /// An explicit request's check counts toward update health like a manual check,
    /// without consuming an automatic slot or opening a catch-up window.
    pub(crate) fn record_explicit_check(
        &self,
        now: chrono::DateTime<chrono::FixedOffset>,
    ) -> Result<()> {
        let mut state = self.state()?;
        state.observe(&self.state_path, self.settings(), now, false)?;
        state.record_success(&self.state_path, now, false)
    }

    pub(crate) fn installation_allowed(
        &self,
        now: chrono::DateTime<chrono::FixedOffset>,
        launch: bool,
    ) -> Result<bool> {
        self.state()?
            .installation_allowed(self.settings(), now, launch)
    }

    pub(crate) fn idle_guard(&self, launch: bool) -> Result<Option<InstallationGuard>> {
        match &self.workspace {
            Some(settings) => {
                Ok(
                    core::updates::idle_guard(settings, launch)?.map(|guard| InstallationGuard {
                        _workspace: Some(guard),
                    }),
                )
            }
            None => Ok(Some(InstallationGuard { _workspace: None })),
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn report_overdue(&self) -> Result<()> {
        const OVERDUE: &str = core::updates::DESKTOP_OVERDUE;
        if let Some(settings) = &self.workspace
            && core::job_status(settings)?
                .desktop_update_error
                .is_some_and(|error| !error.starts_with(OVERDUE))
        {
            // Preserve the specific failure and its delivery/deduplication history.
            return Ok(());
        }
        self.report_outcome(Some(OVERDUE))
    }

    pub(crate) fn report_outcome(&self, error: Option<&str>) -> Result<()> {
        #[cfg(windows)]
        let installation_error = core::windows_update::failure(
            self.state_path.parent().expect("update state has a parent"),
        )?;
        #[cfg(windows)]
        let error = error.or(installation_error.as_deref());
        if let Some(settings) = &self.workspace {
            // The resident Windows controller owns native transition toasts.
            // Keep the updater ledger and optional remote/custom transports.
            #[cfg(windows)]
            let settings = &{
                let mut alerts = settings.clone();
                if matches!(alerts.alert, Some(core::AlertSettings::System)) {
                    alerts.alert = None;
                    alerts.alert_command.clear();
                }
                alerts
            };
            core::updates::report_outcome(settings, error, core::updates::UpdateKind::Desktop)?;
            if error.is_none() {
                // A successful desktop check owns the bundled helper's update health.
                core::updates::report_outcome(settings, None, core::updates::UpdateKind::Helper)?;
            }
        }
        // Without a workspace there is no configured alert transport; the command returns
        // the original error to the desktop's visible status message.
        Ok(())
    }
}

pub(crate) struct AutomaticOutcome {
    pub checked: bool,
    pub available: bool,
    pub installed: bool,
}

/// Shared launch/nightly policy, with transport injected so tests exercise real persistence.
pub(crate) fn automatic_update_with<U>(
    context: &UpdateContext,
    pending: &mut Option<(UpdateSettings, U)>,
    now: chrono::DateTime<chrono::FixedOffset>,
    install: bool,
    launch: bool,
    check: impl FnOnce(&UpdateSettings) -> Result<Option<U>>,
    install_update: impl FnOnce(&U) -> Result<bool>,
) -> Result<AutomaticOutcome> {
    let settings = context.settings();
    let mut outcome = AutomaticOutcome {
        checked: false,
        available: false,
        installed: false,
    };
    let mut state = context.state()?;
    state.observe(&context.state_path, settings, now, launch)?;
    if !settings.automatic {
        *pending = None;
        return Ok(outcome);
    }
    if pending.as_ref().is_some_and(|(saved, _)| saved != settings) {
        *pending = None;
    }
    let outcome = (|| -> Result<AutomaticOutcome> {
        if (launch || !install) && state.due(settings, now, launch)? {
            state.record_attempt(&context.state_path, settings, now)?;
            *pending = None;
            *pending = check(settings)?.map(|update| (settings.clone(), update));
            state.record_success(&context.state_path, now, pending.is_some())?;
            outcome.checked = true;
        }
        if install && pending.is_some() && state.installation_allowed(settings, now, launch)? {
            let Some(_idle) = context.idle_guard(launch)? else {
                outcome.available = true;
                return Ok(outcome);
            };
            if let Some((_, update)) = pending.as_ref() {
                match install_update(update) {
                    Ok(true) => {
                        *pending = None;
                        state.record_available(&context.state_path, false)?;
                        outcome.installed = true;
                    }
                    Ok(false) => (), // Busy: retain the checked update for an idle retry.
                    Err(error) => {
                        // Real failures wait until the next nightly attempt or launch.
                        // Persisted pending state still permits launch-time recovery.
                        *pending = None;
                        return Err(error);
                    }
                }
            }
        }
        outcome.available = pending.is_some();
        Ok(outcome)
    })()?;
    // A failed check is reported by the command layer with its specific cause.
    // Successful work recovers there too; only an idle, overdue tick needs a watchdog alert.
    #[cfg(not(target_os = "macos"))]
    if !outcome.checked && !outcome.installed && state.overdue(settings, now)? {
        context.report_overdue()?;
    }
    Ok(outcome)
}

#[derive(Serialize)]
pub(crate) struct DesktopUpdateResult {
    message: String,
    available: bool,
    installed: bool,
}

pub(crate) fn manual_update_with<U>(
    context: &UpdateContext,
    now: chrono::DateTime<chrono::FixedOffset>,
    install: bool,
    check: impl FnOnce(&UpdateSettings) -> Result<Option<U>>,
    version: impl FnOnce(&U) -> String,
    install_update: impl FnOnce(U) -> Result<()>,
) -> Result<DesktopUpdateResult> {
    let update = check(context.settings())?;
    let mut state = context.state()?;
    state.observe(&context.state_path, context.settings(), now, false)?;
    // A manual check contributes to health without consuming an automatic slot
    // or queuing installation. Explicit installs retain attempt/recovery state.
    if install {
        state.record_attempt(&context.state_path, context.settings(), now)?;
    }
    state.record_success(&context.state_path, now, install && update.is_some())?;
    match update {
        Some(update) if install => {
            let _idle = context.idle_guard(false)?.ok_or_else(|| {
                anyhow::anyhow!("Apply, sync, or a schedule change is running; retry when idle")
            })?;
            install_update(update)?;
            state.record_available(&context.state_path, false)?;
            Ok(DesktopUpdateResult {
                message: "Signed desktop update installed. Restart to use it.".into(),
                available: false,
                installed: true,
            })
        }
        Some(update) => Ok(DesktopUpdateResult {
            message: format!("Desktop update available: {}", version(&update)),
            available: true,
            installed: false,
        }),
        None => Ok(DesktopUpdateResult {
            message: "No newer desktop release available.".into(),
            available: false,
            installed: false,
        }),
    }
}

#[cfg(test)]
mod tests;
