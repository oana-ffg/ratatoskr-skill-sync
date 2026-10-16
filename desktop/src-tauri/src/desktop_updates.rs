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
struct InstallationGuard {
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

    fn idle_guard(&self, launch: bool) -> Result<Option<InstallationGuard>> {
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

    pub(crate) fn report_outcome(&self, error: Option<&str>) -> Result<()> {
        if let Some(settings) = &self.workspace {
            core::updates::report_outcome(settings, error, core::updates::UpdateKind::Desktop)?;
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
    install_update: impl FnOnce(U) -> Result<()>,
) -> Result<AutomaticOutcome> {
    let settings = context.settings();
    let mut outcome = AutomaticOutcome {
        checked: false,
        available: false,
        installed: false,
    };
    if !settings.automatic || (!launch && !core::updates::desktop_window(settings, now)?) {
        *pending = None;
        return Ok(outcome);
    }
    if pending.as_ref().is_some_and(|(saved, _)| saved != settings) {
        *pending = None;
    }
    let mut state = context.state()?;
    if (launch || !install) && state.due(settings, now, launch)? {
        state.record_attempt(&context.state_path, settings, now)?;
        *pending = None;
        *pending = check(settings)?.map(|update| (settings.clone(), update));
        state.record_available(&context.state_path, pending.is_some())?;
        outcome.checked = true;
    }
    if install && pending.is_some() {
        let Some(_idle) = context.idle_guard(launch)? else {
            outcome.available = true;
            return Ok(outcome);
        };
        if let Some((_, update)) = pending.take() {
            install_update(update)?;
            state.record_available(&context.state_path, false)?;
            outcome.installed = true;
        }
    }
    outcome.available = pending.is_some();
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
    install: bool,
    check: impl FnOnce(&UpdateSettings) -> Result<Option<U>>,
    version: impl FnOnce(&U) -> String,
    install_update: impl FnOnce(U) -> Result<()>,
) -> Result<DesktopUpdateResult> {
    match check(context.settings())? {
        Some(update) if install => {
            let _idle = context.idle_guard(false)?.ok_or_else(|| {
                anyhow::anyhow!("Apply, sync, or a schedule change is running; retry when idle")
            })?;
            install_update(update)?;
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
                Ok(())
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
            |_| Ok(()),
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
            |_| Ok(()),
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
            |_| Ok(()),
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
            |_| panic!("configured time wins over midnight default"),
            |_| unreachable!(),
        )
        .unwrap();
        assert!(!before.checked);
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
            |_| Ok(()),
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
            |_| Ok(()),
        )
        .unwrap();
        assert!(outcome.checked && outcome.installed);
    }
}
