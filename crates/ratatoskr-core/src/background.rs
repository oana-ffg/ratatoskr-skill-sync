use crate::{
    Settings,
    config::{identifier, no_symlinks},
    jobs,
};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

fn diagnostics(config: &Path) -> Result<Settings> {
    ensure!(
        config.is_absolute(),
        "Background config path must be absolute"
    );
    no_symlinks(config)?;
    // Parsing separately lets a missing checkout/invalid update setting still
    // reach the existing job ledger. Never use an unvalidated path or machine ID.
    let parsed = (|| -> Result<Settings> {
        let settings: Settings = toml::from_str(&fs::read_to_string(config)?)?;
        identifier(&settings.machine)?;
        ensure!(
            settings.state_dir.is_absolute(),
            "State directory must be absolute"
        );
        no_symlinks(&settings.state_dir)?;
        ensure!(
            !crate::config::overlaps(&settings.source, &settings.state_dir),
            "Source and state directories must not overlap"
        );
        Ok(settings)
    })();
    if let Ok(settings) = parsed {
        return Ok(settings);
    }
    // Malformed/unreadable configuration cannot tell us its state directory.
    // Keep a separate bootstrap ledger beside that exact config, never in CWD.
    let mut directory = config.as_os_str().to_owned();
    directory.push(".diagnostics");
    Ok(Settings {
        state_dir: PathBuf::from(directory),
        machine: "startup".into(),
        ..Settings::default()
    })
}

/// Direct core entry point for tray runs. Logs use owned file handles, never
/// process-wide stdout redirection inside a multithreaded desktop application.
/// None means the existing sync operation lock was busy; no run was started.
pub fn recorded_sync(config: &Path) -> Result<Option<crate::Plan>> {
    let diagnostics = diagnostics(config)?;
    let directory = diagnostics.state_dir.join(&diagnostics.machine);
    no_symlinks(&directory)?;
    fs::create_dir_all(&directory)?;
    let open = |filename: &str| -> Result<fs::File> {
        let path = directory.join(filename);
        no_symlinks(&path)?;
        if path.try_exists()? && fs::metadata(&path)?.len() > 262_144 {
            let bytes = fs::read(&path)?;
            crate::config::atomic_write(&path, &bytes[bytes.len().saturating_sub(65_536)..])?;
        }
        Ok(fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?)
    };
    let mut stdout = open("sync.stdout.log").context("Open background output log")?;
    let mut stderr = open("sync.stderr.log").context("Open background error log")?;
    let result = (|| {
        let settings = match crate::load(config) {
            Ok(settings) => settings,
            Err(error) => {
                let Some(_lock) = crate::config::try_operation_lock(&diagnostics, "sync.lock")?
                else {
                    return Ok(None);
                };
                let run = jobs::start(&diagnostics)?;
                jobs::finish(&diagnostics, run, Some(&format!("{error:#}")))?;
                return Err(error);
            }
        };
        crate::git::sync_in_process(&settings)
    })();
    match &result {
        Ok(Some(plan)) => writeln!(stdout, "{}", serde_json::to_string(plan)?)?,
        Ok(None) => (),
        Err(error) => writeln!(stderr, "{error:#}")?,
    }
    stdout.flush()?;
    stderr.flush()?;
    result
}

/// Configuration errors still have a safe bootstrap ledger beside the config.
pub fn record_background_problem(config: &Path, error: Option<&str>) -> Result<()> {
    let diagnostics = diagnostics(config)?;
    crate::tray::record_problem(&diagnostics, error)
}
/// Independent jobs keep independent notification identities and recoveries.
#[derive(
    Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, serde::Deserialize, serde::Serialize,
)]
#[serde(rename_all = "kebab-case")]
pub enum NotificationKind {
    Sync,
    Migration,
    DesktopUpdate,
    Notifications,
    Login,
    Restart,
    Preferences,
}
impl NotificationKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Sync => "Sync",
            Self::Migration => "Task migration",
            Self::DesktopUpdate => "Desktop update",
            Self::Notifications => "Notifications",
            Self::Login => "Open at login",
            Self::Restart => "Restart",
            Self::Preferences => "Sync settings",
        }
    }
}
type NotificationLedger =
    std::collections::BTreeMap<NotificationKind, crate::tray::NotificationState>;
fn notification_path(settings: &Settings) -> PathBuf {
    settings
        .state_dir
        .join(&settings.machine)
        .join("tray-notification.json")
}
fn load_notifications(settings: &Settings) -> Result<NotificationLedger> {
    let path = notification_path(settings);
    no_symlinks(&path)?;
    Ok(if path.try_exists()? {
        serde_json::from_slice(&fs::read(path)?)?
    } else {
        NotificationLedger::default()
    })
}
fn store_notifications(settings: &Settings, state: &NotificationLedger) -> Result<()> {
    crate::config::atomic_write(&notification_path(settings), &serde_json::to_vec(state)?)
}

/// Delivery remains deduplicated in memory while failed ledger writes retry.
#[derive(Default)]
pub struct BackgroundNotifications {
    context: Option<PathBuf>,
    state: NotificationLedger,
    dirty: bool,
}
impl BackgroundNotifications {
    pub fn outcome(
        &mut self,
        config: &Path,
        outcomes: &[(NotificationKind, Option<&str>)],
        send: impl FnMut(NotificationKind, &str, bool) -> Result<()>,
    ) -> Result<()> {
        let settings = diagnostics(config)?;
        self.outcome_with(&settings, outcomes, send, |state| {
            store_notifications(&settings, state).context("Persist tray notification ledger")
        })
    }
    fn outcome_with(
        &mut self,
        settings: &Settings,
        outcomes: &[(NotificationKind, Option<&str>)],
        mut send: impl FnMut(NotificationKind, &str, bool) -> Result<()>,
        persist: impl FnOnce(&NotificationLedger) -> Result<()>,
    ) -> Result<()> {
        let context = settings.state_dir.join(&settings.machine);
        if self.context.as_ref() != Some(&context) {
            self.context = Some(context);
            self.state.clear();
            self.dirty = false;
            // Return the read error, then keep a memory ledger so that error can
            // still be notified once even if the persistent ledger is broken.
            self.state = load_notifications(settings).context("Read tray notification ledger")?;
        }
        let mut errors = Vec::new();
        for (kind, error) in outcomes {
            let previous = self.state.get(kind).cloned().unwrap_or_default();
            let mut next = previous.clone();
            if let Err(error) =
                next.deliver(*error, |message| send(*kind, message, error.is_none()))
            {
                errors.push(format!("{} toast failed: {error:#}", kind.label()));
            }
            if next != previous {
                self.state.insert(*kind, next);
                self.dirty = true;
            }
        }
        // A failed new toast must not prevent saving earlier delivered ones.
        if self.dirty {
            match persist(&self.state) {
                Ok(()) => self.dirty = false,
                Err(error) => errors.push(format!("{error:#}")),
            }
        }
        ensure!(errors.is_empty(), "{}", errors.join("; "));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_ledger_writes_retry_without_repeating_error_or_recovery_toasts() {
        let temp = tempfile::tempdir().unwrap();
        let settings = Settings {
            state_dir: temp.path().canonicalize().unwrap(),
            machine: "test".into(),
            ..Default::default()
        };
        let mut controller = BackgroundNotifications::default();
        let mut messages = vec![];
        for error in [
            Some("offline"),
            Some("offline"),
            Some("offline"),
            None,
            None,
            None,
        ] {
            assert!(
                controller
                    .outcome_with(
                        &settings,
                        &[(NotificationKind::Sync, error)],
                        |_, message, _| {
                            messages.push(message.to_owned());
                            Ok(())
                        },
                        |_| anyhow::bail!("ledger is read-only")
                    )
                    .is_err()
            );
        }
        assert_eq!(messages, ["offline", "Sync recovered"]);
        let mut retried = false;
        controller
            .outcome_with(
                &settings,
                &[(NotificationKind::Sync, None)],
                |_, _, _| panic!("already delivered"),
                |state| {
                    retried = true;
                    store_notifications(&settings, state)
                },
            )
            .unwrap();
        assert!(retried);
        BackgroundNotifications::default()
            .outcome_with(
                &settings,
                &[(NotificationKind::Sync, None)],
                |_, _, _| panic!("saved recovery must survive restart"),
                |_| panic!("no pending save"),
            )
            .unwrap();
    }
    #[test]
    fn pending_delivery_record_is_saved_even_if_the_next_toast_fails() {
        let temp = tempfile::tempdir().unwrap();
        let settings = Settings {
            state_dir: temp.path().canonicalize().unwrap(),
            machine: "test".into(),
            ..Default::default()
        };
        let mut controller = BackgroundNotifications::default();
        assert!(
            controller
                .outcome_with(
                    &settings,
                    &[(NotificationKind::Sync, Some("offline"))],
                    |_, _, _| Ok(()),
                    |_| anyhow::bail!("read-only ledger")
                )
                .is_err()
        );
        let mut stored = false;
        assert!(
            controller
                .outcome_with(
                    &settings,
                    &[(NotificationKind::Sync, Some("conflict"))],
                    |_, _, _| anyhow::bail!("toast unavailable"),
                    |state| {
                        stored = true;
                        store_notifications(&settings, state)
                    }
                )
                .is_err()
        );
        assert!(stored);
        BackgroundNotifications::default()
            .outcome_with(
                &settings,
                &[(NotificationKind::Sync, Some("offline"))],
                |_, _, _| panic!("earlier delivered toast must be persisted"),
                |_| panic!("nothing to save"),
            )
            .unwrap();
    }
    #[test]
    fn sync_recovery_does_not_repeat_or_clear_an_existing_update_error() {
        let temp = tempfile::tempdir().unwrap();
        let settings = Settings {
            state_dir: temp.path().canonicalize().unwrap(),
            machine: "test".into(),
            ..Default::default()
        };
        let mut controller = BackgroundNotifications::default();
        let mut messages = vec![];
        for sync in [Some("offline"), Some("offline"), None, None] {
            controller
                .outcome_with(
                    &settings,
                    &[
                        (NotificationKind::Sync, sync),
                        (NotificationKind::DesktopUpdate, Some("update unavailable")),
                    ],
                    |kind, message, recovered| {
                        messages.push((kind, message.to_owned(), recovered));
                        Ok(())
                    },
                    |state| store_notifications(&settings, state),
                )
                .unwrap();
        }
        assert_eq!(
            messages,
            [
                (NotificationKind::Sync, "offline".into(), false),
                (
                    NotificationKind::DesktopUpdate,
                    "update unavailable".into(),
                    false
                ),
                (NotificationKind::Sync, "Sync recovered".into(), true),
            ]
        );
    }
}
