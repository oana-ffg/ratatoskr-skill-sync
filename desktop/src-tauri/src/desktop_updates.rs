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
mod tests {
    use super::*;
    use std::cell::Cell;

    fn clock(value: &str) -> chrono::DateTime<chrono::FixedOffset> {
        chrono::DateTime::parse_from_rfc3339(value).unwrap()
    }
    fn unconfigured(temp: &tempfile::TempDir) -> UpdateContext {
        let root = temp.path().canonicalize().unwrap();
        UpdateContext::load(&root.join("missing.toml"), &root.join("app")).unwrap()
    }

    #[test]
    fn unconfigured_launch_installs_and_persists_catch_up() {
        let temp = tempfile::tempdir().unwrap();
        let context = unconfigured(&temp);
        let installed = Cell::new(false);
        let now = clock("2026-10-16T09:00:00+02:00");
        let outcome = automatic_update_with(
            &context,
            &mut None,
            now,
            true,
            true,
            |settings| {
                assert_eq!(settings, &UpdateSettings::default());
                Ok(Some("0.2.1"))
            },
            |_| {
                installed.set(true);
                Ok(true)
            },
        )
        .unwrap();
        assert!(installed.get() && outcome.checked && outcome.installed);
        let reopened = unconfigured(&temp);
        let outcome = automatic_update_with::<()>(
            &reopened,
            &mut None,
            now,
            true,
            true,
            |_| panic!("must not repeat completed check"),
            |_| panic!("no pending install"),
        )
        .unwrap();
        assert!(!outcome.checked && !outcome.installed);
        assert!(!temp.path().join("missing.toml").exists());
    }

    #[test]
    fn unconfigured_nightly_checks_then_installs_and_retries_next_window() {
        let temp = tempfile::tempdir().unwrap();
        let context = unconfigured(&temp);
        let now = clock("2026-10-16T00:01:00+02:00");
        let mut pending = None;
        let outcome = automatic_update_with(
            &context,
            &mut pending,
            now,
            false,
            false,
            |_| Ok(Some("0.2.1")),
            |_| panic!("check cannot install"),
        )
        .unwrap();
        assert!(outcome.checked && outcome.available && !outcome.installed);
        let outcome = automatic_update_with(
            &context,
            &mut pending,
            now,
            true,
            false,
            |_| panic!("use checked update"),
            |_| Ok(true),
        )
        .unwrap();
        assert!(outcome.installed);
        let outcome = automatic_update_with(
            &context,
            &mut pending,
            now,
            false,
            false,
            |_| panic!("one check per window"),
            |_| Ok(true),
        )
        .unwrap();
        assert!(!outcome.checked);
        let outcome = automatic_update_with(
            &context,
            &mut pending,
            clock("2026-10-17T00:01:00+02:00"),
            false,
            false,
            |_| Ok(None),
            |_| Ok(true),
        )
        .unwrap();
        assert!(outcome.checked);
    }

    #[test]
    fn unconfigured_manual_check_and_install_use_official_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let context = unconfigured(&temp);
        for install in [false, true] {
            let installed = Cell::new(false);
            let result = manual_update_with(
                &context,
                clock("2026-10-16T09:00:00+02:00"),
                install,
                |settings| {
                    assert_eq!(settings, &UpdateSettings::default());
                    Ok(Some("0.2.1"))
                },
                |v| v.to_string(),
                |_| {
                    installed.set(true);
                    Ok(())
                },
            )
            .unwrap();
            assert_eq!(result.available, !install);
            assert_eq!(result.installed, install);
            assert_eq!(installed.get(), install);
            if install {
                assert!(
                    !context
                        .state()
                        .unwrap()
                        .due(context.settings(), clock("2026-10-16T09:00:00+02:00"), true)
                        .unwrap(),
                    "a completed manual install must not be repeated on launch"
                );
            }
        }
    }

    fn configured(temp: &tempfile::TempDir) -> (PathBuf, Settings) {
        let root = temp.path().canonicalize().unwrap();
        let config = root.join("config.toml");
        let settings = Settings {
            source: root.join("source"),
            state_dir: root.join("workspace-state"),
            machine: "test".into(),
            updates: UpdateSettings {
                endpoint: Some("https://example.com/custom.json".into()),
                public_key: Some(core::updates::OFFICIAL_PUBLIC_KEY.into()),
                automatic: false,
                time: "23:30".into(),
            },
            ..Default::default()
        };
        std::fs::create_dir(&settings.source).unwrap();
        core::save(&config, &settings).unwrap();
        (config, settings)
    }

    #[test]
    fn configured_overrides_win_and_invalid_configs_do_not_fall_back() {
        let temp = tempfile::tempdir().unwrap();
        let (config, settings) = configured(&temp);
        let context =
            UpdateContext::load(&config, &temp.path().canonicalize().unwrap().join("app")).unwrap();
        assert_eq!(context.settings(), &settings.updates);
        manual_update_with::<()>(
            &context,
            clock("2026-10-16T09:00:00+02:00"),
            false,
            |updates| {
                assert_eq!(updates, &settings.updates);
                Ok(None)
            },
            |_| unreachable!(),
            |_| unreachable!(),
        )
        .unwrap();
        let outcome = automatic_update_with::<()>(
            &context,
            &mut None,
            clock("2026-10-16T09:00:00+02:00"),
            true,
            true,
            |_| panic!("automatic disabled"),
            |_| unreachable!(),
        )
        .unwrap();
        assert!(!outcome.checked);
        std::fs::write(&config, "invalid toml").unwrap();
        assert!(UpdateContext::load(&config, temp.path()).is_err());
    }

    #[test]
    fn configured_legacy_attempt_is_preserved_then_saved_in_app_directory() {
        let temp = tempfile::tempdir().unwrap();
        let (config, mut settings) = configured(&temp);
        settings.updates.automatic = true;
        core::save(&config, &settings).unwrap();
        let legacy = settings.state_dir.join("test/desktop-update.json");
        let now = clock("2026-10-16T09:00:00+02:00");
        DesktopUpdateState::default()
            .record_attempt(&legacy, &settings.updates, now)
            .unwrap();
        let context =
            UpdateContext::load(&config, &temp.path().canonicalize().unwrap().join("app")).unwrap();
        assert!(
            !context
                .state()
                .unwrap()
                .due(context.settings(), now, true)
                .unwrap()
        );
        automatic_update_with::<()>(
            &context,
            &mut None,
            clock("2026-10-17T09:00:00+02:00"),
            true,
            true,
            |_| Ok(None),
            |_| unreachable!(),
        )
        .unwrap();
        assert!(context.state_path.exists());
    }

    #[test]
    fn unconfigured_errors_propagate_and_failed_checks_are_not_repeated() {
        let temp = tempfile::tempdir().unwrap();
        let context = unconfigured(&temp);
        let now = clock("2026-10-16T09:00:00+02:00");
        assert!(
            automatic_update_with::<()>(
                &context,
                &mut None,
                now,
                true,
                true,
                |_| anyhow::bail!("offline"),
                |_| unreachable!()
            )
            .is_err()
        );
        assert!(
            !context
                .state()
                .unwrap()
                .due(context.settings(), now, true)
                .unwrap()
        );
        assert!(
            manual_update_with::<()>(
                &context,
                clock("2026-10-16T09:00:00+02:00"),
                false,
                |_| anyhow::bail!("offline"),
                |_| unreachable!(),
                |_| unreachable!()
            )
            .is_err()
        );
        assert!(
            manual_update_with(
                &context,
                clock("2026-10-16T09:00:00+02:00"),
                true,
                |_| Ok(Some(())),
                |_| "0.2.1".into(),
                |_| anyhow::bail!("bad signature")
            )
            .is_err()
        );
        std::fs::write(&context.state_path, "corrupt").unwrap();
        assert!(context.state().is_err());
    }

    #[test]
    fn configured_nightly_honors_override_window_and_defers_while_locked() {
        let temp = tempfile::tempdir().unwrap();
        let (config, mut settings) = configured(&temp);
        settings.updates.automatic = true;
        core::save(&config, &settings).unwrap();
        let context =
            UpdateContext::load(&config, &temp.path().canonicalize().unwrap().join("app")).unwrap();
        let mut pending = None;
        let before = automatic_update_with::<()>(
            &context,
            &mut pending,
            clock("2026-10-16T00:45:00+02:00"),
            false,
            false,
            |_| Ok(None),
            |_| unreachable!(),
        )
        .unwrap();
        assert!(before.checked);
        let now = clock("2026-10-16T23:31:00+02:00");
        automatic_update_with(
            &context,
            &mut pending,
            now,
            false,
            false,
            |updates| {
                assert_eq!(updates, &settings.updates);
                Ok(Some(()))
            },
            |_| unreachable!(),
        )
        .unwrap();
        let lock = core::updates::idle_guard(&settings, false)
            .unwrap()
            .unwrap();
        let deferred = automatic_update_with(
            &context,
            &mut pending,
            now,
            true,
            false,
            |_| unreachable!(),
            |_| panic!("workspace busy"),
        )
        .unwrap();
        assert!(deferred.available && !deferred.installed);
        assert!(
            context
                .state()
                .unwrap()
                .due(&settings.updates, now, true)
                .unwrap()
        );
        assert!(
            manual_update_with(
                &context,
                now,
                true,
                |_| Ok(Some(())),
                |_| "0.2.1".into(),
                |_| panic!("workspace busy")
            )
            .is_err()
        );
        drop(lock);
        let installed = automatic_update_with(
            &context,
            &mut pending,
            now,
            true,
            false,
            |_| unreachable!(),
            |_| Ok(true),
        )
        .unwrap();
        assert!(installed.installed);
        assert!(
            !context
                .state()
                .unwrap()
                .due(&settings.updates, now, true)
                .unwrap()
        );
    }

    #[test]
    fn unconfigured_deferred_update_is_rechecked_on_launch() {
        let temp = tempfile::tempdir().unwrap();
        let context = unconfigured(&temp);
        automatic_update_with(
            &context,
            &mut None,
            clock("2026-10-16T00:01:00+02:00"),
            false,
            false,
            |_| Ok(Some(())),
            |_| unreachable!(),
        )
        .unwrap();
        let reopened = unconfigured(&temp);
        let outcome = automatic_update_with(
            &reopened,
            &mut None,
            clock("2026-10-16T09:00:00+02:00"),
            true,
            true,
            |_| Ok(Some(())),
            |_| Ok(true),
        )
        .unwrap();
        assert!(outcome.checked && outcome.installed);
    }
    #[test]
    fn resident_missed_window_catches_up_and_busy_install_does_not_extend_the_hour() {
        let temp = tempfile::tempdir().unwrap();
        let context = unconfigured(&temp);
        let mut pending = None;
        automatic_update_with(
            &context,
            &mut pending,
            clock("2026-10-18T00:01:00+02:00"),
            false,
            false,
            |_| Ok(None::<()>),
            |_| unreachable!(),
        )
        .unwrap();
        let resumed = clock("2026-10-19T13:38:00+02:00");
        let checked = automatic_update_with(
            &context,
            &mut pending,
            resumed,
            false,
            false,
            |_| Ok(Some(())),
            |_| unreachable!(),
        )
        .unwrap();
        assert!(checked.checked && checked.available);
        // An edit started during download. Retain the checked update for an idle retry.
        let busy = automatic_update_with(
            &context,
            &mut pending,
            resumed,
            true,
            false,
            |_| unreachable!(),
            |_| Ok(false),
        )
        .unwrap();
        assert!(busy.available && !busy.installed);
        let expired = automatic_update_with(
            &context,
            &mut pending,
            resumed + chrono::Duration::hours(1),
            true,
            false,
            |_| unreachable!(),
            |_| panic!("catch-up hour expired"),
        )
        .unwrap();
        assert!(expired.available && !expired.installed);
        let next = clock("2026-10-20T00:01:00+02:00");
        automatic_update_with(
            &context,
            &mut pending,
            next,
            false,
            false,
            |_| Ok(Some(())),
            |_| unreachable!(),
        )
        .unwrap();
        assert!(
            automatic_update_with(
                &context,
                &mut pending,
                next,
                true,
                false,
                |_| unreachable!(),
                |_| Ok(true)
            )
            .unwrap()
            .installed
        );
        let tomorrow = clock("2026-10-21T00:01:00+02:00");
        automatic_update_with(
            &context,
            &mut pending,
            tomorrow,
            false,
            false,
            |_| Ok(Some(())),
            |_| unreachable!(),
        )
        .unwrap();
        assert!(
            automatic_update_with(
                &context,
                &mut pending,
                tomorrow,
                true,
                false,
                |_| unreachable!(),
                |_| anyhow::bail!("invalid signature")
            )
            .is_err()
        );
        assert!(
            !automatic_update_with(
                &context,
                &mut pending,
                tomorrow,
                false,
                false,
                |_| panic!("failed install is not rechecked every minute"),
                |_| unreachable!()
            )
            .unwrap()
            .available
        );
    }

    #[test]
    fn manual_check_does_not_queue_automatic_installation_or_open_catch_up() {
        let temp = tempfile::tempdir().unwrap();
        let context = unconfigured(&temp);
        let mut pending = None;
        automatic_update_with(
            &context,
            &mut pending,
            clock("2026-10-19T00:01:00+02:00"),
            false,
            false,
            |_| Ok(Some(())),
            |_| unreachable!(),
        )
        .unwrap();
        // The manual command clears any old in-memory automatic update first.
        pending = None;
        let now = clock("2026-10-19T13:38:00+02:00");
        let result = manual_update_with(
            &context,
            now,
            false,
            |_| Ok(Some(())),
            |_| "0.2.9".into(),
            |_| panic!("manual check must not install"),
        )
        .unwrap();
        assert!(result.available && !result.installed);
        let state = context.state().unwrap();
        assert!(
            !state
                .installation_allowed(context.settings(), now, false)
                .unwrap()
        );
        assert!(!state.due(context.settings(), now, true).unwrap());
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&context.state_path).unwrap()).unwrap();
        assert!(saved["catch_up_started"].is_null());
        assert_eq!(saved["pending"], false);
        assert_eq!(saved["last_success"], now.to_rfc3339());
        for (install, launch) in [(false, false), (true, false), (true, true)] {
            let reopened = unconfigured(&temp);
            let outcome = automatic_update_with::<()>(
                &reopened,
                &mut pending,
                now,
                install,
                launch,
                |_| panic!("manual check must not queue a launch recheck"),
                |_| panic!("manual check must not queue an automatic install"),
            )
            .unwrap();
            assert!(!outcome.checked && !outcome.available && !outcome.installed);
        }
        assert!(
            context
                .state()
                .unwrap()
                .due(
                    context.settings(),
                    clock("2026-10-20T00:01:00+02:00"),
                    false
                )
                .unwrap(),
            "the next automatic window remains due"
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn overdue_failing_checks_keep_specific_error_and_one_alert_per_24_hours() {
        let temp = tempfile::tempdir().unwrap();
        let (config, mut settings) = configured(&temp);
        settings.updates.automatic = true;
        settings.updates.time = "00:00".into();
        // Git's --file cannot open Windows verbatim paths from std::fs::canonicalize.
        let alerts = dunce::canonicalize(temp.path())
            .unwrap()
            .join("alerts.gitconfig");
        // Git's argv-only config writer is a cross-platform, local alert recorder.
        settings.alert_command = vec![
            "git".into(),
            "config".into(),
            "--file".into(),
            alerts.display().to_string(),
            "--add".into(),
            "alerts.message".into(),
        ];
        core::save(&config, &settings).unwrap();
        let context =
            UpdateContext::load(&config, &temp.path().canonicalize().unwrap().join("app")).unwrap();
        let first = clock("2026-10-18T00:01:00+02:00");
        automatic_update_with::<()>(
            &context,
            &mut None,
            first,
            false,
            false,
            |_| Ok(None),
            |_| unreachable!(),
        )
        .unwrap();
        let now = clock("2026-10-20T13:38:00+02:00");
        assert!(
            context
                .state()
                .unwrap()
                .overdue(&settings.updates, now)
                .unwrap()
        );
        let fail_check = |time| {
            let result = automatic_update_with::<()>(
                &context,
                &mut None,
                time,
                false,
                false,
                |_| anyhow::bail!("release endpoint offline"),
                |_| unreachable!(),
            );
            let error = format!("Desktop update: {:#}", result.err().expect("check fails"));
            // The command layer reports the specific check result after policy returns.
            context.report_outcome(Some(&error)).unwrap();
            error
        };
        let error = fail_check(now);
        let read_alerts = || {
            let output = std::process::Command::new("git")
                .args(["config", "--file"])
                .arg(&alerts)
                .args(["--get-all", "alerts.message"])
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        let assert_idle = |time| {
            automatic_update_with::<()>(
                &context,
                &mut None,
                time,
                false,
                false,
                |_| panic!("one check per slot"),
                |_| unreachable!(),
            )
            .unwrap();
            assert_eq!(
                core::job_status(&settings)
                    .unwrap()
                    .desktop_update_error
                    .as_deref(),
                Some(error.as_str())
            );
        };
        for minute in 1..=60 {
            assert_idle(now + chrono::Duration::minutes(minute));
        }
        assert_eq!(read_alerts(), [format!("Ratatoskr test: {error}")]);
        // The transport clock has not advanced 24 hours, even though a new slot is due.
        let tomorrow = now + chrono::Duration::days(1);
        assert_eq!(fail_check(tomorrow), error);
        assert_idle(tomorrow + chrono::Duration::minutes(1));
        assert_eq!(read_alerts().len(), 1);
        // Age the real transport ledger to the reminder boundary without sleeping a day.
        let notification = settings
            .state_dir
            .join("test/desktop-update-notification.json");
        let mut ledger: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&notification).unwrap()).unwrap();
        let sent_at = ledger["sent_at"].as_u64().unwrap();
        ledger["sent_at"] = (sent_at - 86400).into();
        std::fs::write(&notification, serde_json::to_vec(&ledger).unwrap()).unwrap();
        let next_day = tomorrow + chrono::Duration::days(1);
        assert_eq!(fail_check(next_day), error);
        assert_idle(next_day + chrono::Duration::minutes(1));
        assert_eq!(read_alerts(), vec![format!("Ratatoskr test: {error}"); 2]);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn overdue_is_visible_in_job_status_uses_alert_dedup_and_recovers_only_on_success() {
        let temp = tempfile::tempdir().unwrap();
        let (config, mut settings) = configured(&temp);
        settings.updates.automatic = true;
        settings.updates.time = "00:00".into();
        core::save(&config, &settings).unwrap();
        let context =
            UpdateContext::load(&config, &temp.path().canonicalize().unwrap().join("app")).unwrap();
        let first = clock("2026-10-18T00:01:00+02:00");
        let mut state = DesktopUpdateState::default();
        state
            .observe(&context.state_path, &settings.updates, first, false)
            .unwrap();
        state
            .record_attempt(&context.state_path, &settings.updates, first)
            .unwrap();
        state
            .record_success(&context.state_path, first, false)
            .unwrap();
        let now = clock("2026-10-20T01:01:00+02:00");
        // Simulate an attempted check that never succeeded. No new attempt this slot.
        state
            .record_attempt(&context.state_path, &settings.updates, now)
            .unwrap();
        for _ in 0..2 {
            automatic_update_with::<()>(
                &context,
                &mut None,
                now,
                false,
                false,
                |_| panic!("already attempted"),
                |_| unreachable!(),
            )
            .unwrap();
            assert!(
                core::job_status(&settings)
                    .unwrap()
                    .desktop_update_error
                    .unwrap()
                    .contains("overdue")
            );
        }
        // Failed delivery must remain visible and retryable through the existing transport.
        settings.alert_command = vec!["ratatoskr-nonexistent-updater-test-notifier".into()];
        core::save(&config, &settings).unwrap();
        let alerting =
            UpdateContext::load(&config, &temp.path().canonicalize().unwrap().join("app")).unwrap();
        let notification = settings
            .state_dir
            .join("test/desktop-update-notification.json");
        let message = core::job_status(&settings)
            .unwrap()
            .desktop_update_error
            .unwrap();
        let sent_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        std::fs::write(
            &notification,
            serde_json::to_vec(&serde_json::json!({
                "version": 1, "error": message, "delivered_error": message, "sent_at": sent_at
            }))
            .unwrap(),
        )
        .unwrap();
        automatic_update_with::<()>(
            &alerting,
            &mut None,
            now,
            false,
            false,
            |_| unreachable!(),
            |_| unreachable!(),
        )
        .expect("delivered overdue errors dedup even with an unavailable notifier");
        // A failed/undelivered alert is immediately retryable.
        std::fs::write(
            &notification,
            serde_json::to_vec(&serde_json::json!({
                "version": 1, "error": message, "delivered_error": null, "sent_at": 0
            }))
            .unwrap(),
        )
        .unwrap();
        for _ in 0..2 {
            assert!(
                automatic_update_with::<()>(
                    &alerting,
                    &mut None,
                    now,
                    false,
                    false,
                    |_| unreachable!(),
                    |_| unreachable!()
                )
                .is_err()
            );
        }
        assert!(
            core::job_status(&settings)
                .unwrap()
                .desktop_update_error
                .is_some()
        );
        let checked_despite_alert = Cell::new(false);
        let recovered = automatic_update_with::<()>(
            &alerting,
            &mut None,
            clock("2026-10-21T00:01:00+02:00"),
            false,
            false,
            |_| {
                checked_despite_alert.set(true);
                Ok(None)
            },
            |_| unreachable!(),
        )
        .expect("successful catch-up does not send a stale overdue alert");
        assert!(recovered.checked && checked_despite_alert.get());
        // Production's command layer reports actual successful checks as recovery.
        let recovered = automatic_update_with::<()>(
            &context,
            &mut None,
            clock("2026-10-22T00:01:00+02:00"),
            false,
            false,
            |_| Ok(None),
            |_| unreachable!(),
        )
        .unwrap();
        assert!(recovered.checked);
        context.report_outcome(None).unwrap();
        assert!(
            core::job_status(&settings)
                .unwrap()
                .desktop_update_error
                .is_none()
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_desktop_leaves_overdue_delivery_to_helper_and_success_clears_bundled_errors() {
        let temp = tempfile::tempdir().unwrap();
        let (config, mut settings) = configured(&temp);
        settings.updates.automatic = true;
        settings.updates.time = "00:00".into();
        core::save(&config, &settings).unwrap();
        let context = UpdateContext::load(
            &config,
            &dunce::canonicalize(temp.path()).unwrap().join("app"),
        )
        .unwrap();
        let mut state = DesktopUpdateState::default();
        let first = clock("2026-10-18T00:01:00+02:00");
        state
            .observe(&context.state_path, &settings.updates, first, false)
            .unwrap();
        state
            .record_success(&context.state_path, first, false)
            .unwrap();
        let overdue = clock("2026-10-20T01:01:00+02:00");
        state
            .record_attempt(&context.state_path, &settings.updates, overdue)
            .unwrap();
        automatic_update_with::<()>(
            &context,
            &mut None,
            overdue,
            false,
            false,
            |_| panic!("already attempted"),
            |_| unreachable!(),
        )
        .unwrap();
        assert!(
            core::job_status(&settings)
                .unwrap()
                .desktop_update_error
                .is_none()
        );
        core::updates::report_outcome(
            &settings,
            Some(core::updates::DESKTOP_OVERDUE),
            core::updates::UpdateKind::Desktop,
        )
        .unwrap();
        core::updates::report_outcome(
            &settings,
            Some("Helper update: old standalone failure"),
            core::updates::UpdateKind::Helper,
        )
        .unwrap();
        for _ in 0..2 {
            automatic_update_with::<()>(
                &context,
                &mut None,
                overdue,
                false,
                false,
                |_| unreachable!(),
                |_| unreachable!(),
            )
            .unwrap();
        }
        assert_eq!(
            core::job_status(&settings)
                .unwrap()
                .desktop_update_error
                .as_deref(),
            Some(core::updates::DESKTOP_OVERDUE)
        );
        let checked = automatic_update_with::<()>(
            &context,
            &mut None,
            clock("2026-10-21T00:01:00+02:00"),
            false,
            false,
            |_| Ok(None),
            |_| unreachable!(),
        )
        .unwrap();
        assert!(checked.checked);
        context.report_outcome(None).unwrap();
        let jobs = core::job_status(&settings).unwrap();
        assert!(jobs.desktop_update_error.is_none() && jobs.helper_update_error.is_none());
    }
}
