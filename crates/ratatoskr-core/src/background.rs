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
                let mut run = jobs::start(&diagnostics)?;
                let problem = crate::tray::Problem::from_error("Sync", &error);
                run.problem_identity = Some(problem.identity.clone());
                jobs::finish(&diagnostics, run, Some(&problem.message))?;
                return Err(problem.into());
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
pub fn record_background_problem(
    config: &Path,
    error: Option<&crate::tray::Problem>,
) -> Result<()> {
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
#[derive(Default, serde::Deserialize, serde::Serialize)]
struct NotificationLedger {
    problems: std::collections::BTreeMap<NotificationKind, crate::tray::NotificationState>,
    last_problem_toast: Option<u64>,
}
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
        outcomes: &[(NotificationKind, Option<&crate::tray::Problem>)],
        send: impl FnMut(NotificationKind, &str, bool) -> Result<()>,
    ) -> Result<()> {
        let settings = diagnostics(config)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        self.outcome_with(&settings, now, outcomes, send, |state| {
            store_notifications(&settings, state).context("Persist tray notification ledger")
        })
    }
    fn outcome_with(
        &mut self,
        settings: &Settings,
        now: u64,
        outcomes: &[(NotificationKind, Option<&crate::tray::Problem>)],
        mut send: impl FnMut(NotificationKind, &str, bool) -> Result<()>,
        persist: impl FnOnce(&NotificationLedger) -> Result<()>,
    ) -> Result<()> {
        let context = settings.state_dir.join(&settings.machine);
        if self.context.as_ref() != Some(&context) {
            self.context = Some(context);
            self.state = NotificationLedger::default();
            self.dirty = false;
            // Return the read error, then keep a memory ledger so that error can
            // still be notified once even if the persistent ledger is broken.
            self.state = load_notifications(settings).context("Read tray notification ledger")?;
        }
        let mut errors = Vec::new();
        for (kind, error) in outcomes {
            let previous = self.state.problems.get(kind).cloned().unwrap_or_default();
            let mut next = previous.clone();
            let allow_problem = self
                .state
                .last_problem_toast
                .is_none_or(|last| now.saturating_sub(last) >= 3600);
            match next.deliver(*error, allow_problem, |message| {
                send(*kind, message, error.is_none())
            }) {
                Ok(true) => {
                    self.state.last_problem_toast = Some(now);
                    self.dirty = true;
                }
                Ok(false) => (),
                Err(error) => errors.push(format!("{} toast failed: {error:#}", kind.label())),
            }
            if next != previous {
                self.state.problems.insert(*kind, next);
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
    use crate::tray::Problem;
    fn settings(root: &Path) -> Settings {
        Settings {
            state_dir: root.canonicalize().unwrap(),
            machine: "test".into(),
            ..Default::default()
        }
    }
    #[test]
    fn ten_distinct_failures_in_an_hour_send_one_problem_toast_and_one_recovery() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let mut controller = BackgroundNotifications::default();
        let mut messages = vec![];
        for (index, context) in [
            "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine", "Ten",
        ]
        .into_iter()
        .enumerate()
        {
            let problem = Problem::message(context, format!("failure {index}"));
            controller
                .outcome_with(
                    &settings,
                    1000 + index as u64 * 60,
                    &[(NotificationKind::Sync, Some(&problem))],
                    |_, message, recovered| {
                        messages.push((message.to_owned(), recovered));
                        Ok(())
                    },
                    |state| store_notifications(&settings, state),
                )
                .unwrap();
        }
        assert_eq!(messages.len(), 1);
        // Restart cannot reset the shared one-hour cap, even for another source.
        let problem = Problem::message("Update", "new problem after restart");
        let mut controller = BackgroundNotifications::default();
        controller
            .outcome_with(
                &settings,
                2000,
                &[(NotificationKind::DesktopUpdate, Some(&problem))],
                |_, _, _| panic!("global cap survives restart"),
                |state| store_notifications(&settings, state),
            )
            .unwrap();
        controller
            .outcome_with(
                &settings,
                2001,
                &[(NotificationKind::Sync, None)],
                |_, message, recovered| {
                    messages.push((message.to_owned(), recovered));
                    Ok(())
                },
                |state| store_notifications(&settings, state),
            )
            .unwrap();
        assert_eq!(messages.len(), 2);
        assert!(messages[1].1);
        controller
            .outcome_with(
                &settings,
                2002,
                &[(NotificationKind::Sync, None)],
                |_, _, _| panic!("one recovery"),
                |state| store_notifications(&settings, state),
            )
            .unwrap();
        controller
            .outcome_with(
                &settings,
                4599,
                &[(NotificationKind::DesktopUpdate, Some(&problem))],
                |_, _, _| panic!("not one hour yet"),
                |state| store_notifications(&settings, state),
            )
            .unwrap();
        controller
            .outcome_with(
                &settings,
                4600,
                &[(NotificationKind::DesktopUpdate, Some(&problem))],
                |_, _, recovered| {
                    assert!(!recovered);
                    Ok(())
                },
                |state| store_notifications(&settings, state),
            )
            .unwrap();
        controller
            .outcome_with(
                &settings,
                9000,
                &[(NotificationKind::DesktopUpdate, Some(&problem))],
                |_, _, _| panic!("persistent same problem never repeats"),
                |state| store_notifications(&settings, state),
            )
            .unwrap();
    }
    #[test]
    fn volatile_pids_and_timings_do_not_produce_new_notifications_after_the_cap_expires() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let mut controller = BackgroundNotifications::default();
        let mut messages = vec![];
        for (now, pid, timing) in [(1, 123, 12), (61, 456, 30), (7201, 987, 999)] {
            let error = anyhow::Error::new(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                format!("lock held by PID {pid} for {timing}ms at /tmp/{pid}"),
            ));
            let problem = Problem::from_error("Sync", &error);
            controller
                .outcome_with(
                    &settings,
                    now,
                    &[(NotificationKind::Sync, Some(&problem))],
                    |_, message, _| {
                        messages.push(message.to_owned());
                        Ok(())
                    },
                    |state| store_notifications(&settings, state),
                )
                .unwrap();
        }
        assert_eq!(messages.len(), 1);
    }
    #[test]
    fn failed_ledger_writes_keep_the_cap_and_dedupe_in_memory() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let mut controller = BackgroundNotifications::default();
        let first = Problem::message("Sync", "offline");
        let second = Problem::message("Update", "update unavailable");
        let mut messages = vec![];
        for (kind, problem) in [
            (NotificationKind::Sync, Some(&first)),
            (NotificationKind::Sync, Some(&first)),
            (NotificationKind::DesktopUpdate, Some(&second)),
            (NotificationKind::Sync, None),
            (NotificationKind::Sync, None),
        ] {
            assert!(
                controller
                    .outcome_with(
                        &settings,
                        1000,
                        &[(kind, problem)],
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
        controller
            .outcome_with(
                &settings,
                1001,
                &[(NotificationKind::Sync, None)],
                |_, _, _| panic!("already delivered"),
                |state| store_notifications(&settings, state),
            )
            .unwrap();
        BackgroundNotifications::default()
            .outcome_with(
                &settings,
                1002,
                &[(NotificationKind::DesktopUpdate, Some(&second))],
                |_, _, _| panic!("persisted cap"),
                |state| store_notifications(&settings, state),
            )
            .unwrap();
    }
    #[test]
    fn failed_delivery_does_not_consume_the_cap_or_block_saving_a_prior_delivery() {
        let temp = tempfile::tempdir().unwrap();
        let settings = settings(temp.path());
        let mut controller = BackgroundNotifications::default();
        let problem = Problem::message("Sync", "offline");
        assert!(
            controller
                .outcome_with(
                    &settings,
                    1000,
                    &[(NotificationKind::Sync, Some(&problem))],
                    |_, _, _| anyhow::bail!("toast unavailable"),
                    |_| panic!("nothing delivered")
                )
                .is_err()
        );
        assert!(
            controller
                .outcome_with(
                    &settings,
                    1001,
                    &[(NotificationKind::Sync, Some(&problem))],
                    |_, _, _| Ok(()),
                    |_| anyhow::bail!("ledger unavailable")
                )
                .is_err()
        );
        let mut stored = false;
        assert!(
            controller
                .outcome_with(
                    &settings,
                    1002,
                    &[(NotificationKind::Sync, None)],
                    |_, _, _| anyhow::bail!("recovery unavailable"),
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
                1003,
                &[(NotificationKind::Sync, Some(&problem))],
                |_, _, _| panic!("delivered error persisted"),
                |_| panic!("nothing changed"),
            )
            .unwrap();
    }
}
