//! Persistent tray preferences, diagnostics and notification transitions.
use crate::{
    Settings,
    config::{atomic_write, no_symlinks},
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TrayPreferences {
    pub paused: bool,
    pub interval_seconds: u32,
}
impl Default for TrayPreferences {
    fn default() -> Self {
        Self {
            paused: false,
            interval_seconds: 60,
        }
    }
}
impl TrayPreferences {
    pub fn load(settings: &Settings) -> Result<Self> {
        let path = settings
            .state_dir
            .join(&settings.machine)
            .join("tray-preferences.json");
        no_symlinks(&path)?;
        let preferences: Self = if path.try_exists()? {
            serde_json::from_slice(&fs::read(path)?)?
        } else {
            Self::default()
        };
        preferences.validate()?;
        Ok(preferences)
    }
    pub fn save(&self, settings: &Settings) -> Result<()> {
        self.validate()?;
        atomic_write(
            &settings
                .state_dir
                .join(&settings.machine)
                .join("tray-preferences.json"),
            &serde_json::to_vec(self)?,
        )
    }
    fn validate(&self) -> Result<()> {
        ensure!(
            (60..=86400).contains(&self.interval_seconds),
            "Sync interval must be 60–86400 seconds"
        );
        Ok(())
    }
}

/// A second guard prevents tray timer/manual requests from queuing overlapping
/// work. Core sync.lock still arbitrates against CLI and updater operations.
#[derive(Default)]
pub struct RunGate(AtomicBool);
pub struct RunPermit<'a>(&'a RunGate);
impl RunGate {
    pub fn enter(&self, paused: bool) -> Option<RunPermit<'_>> {
        if paused
            || self
                .0
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
        {
            return None;
        }
        Some(RunPermit(self))
    }
}
impl Drop for RunPermit<'_> {
    fn drop(&mut self) {
        self.0.0.store(false, Ordering::Release);
    }
}

/// Stable classification is separate from diagnostics; remote output, paths,
/// process IDs and timings are never parsed or used as notification identities.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProblemIdentity {
    context: String,
    kind: ProblemKind,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum ProblemKind {
    Io(String),
    Git(String),
    Http(u16),
    Timeout,
    Network,
    Configuration,
    InvalidData,
    Other,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Problem {
    pub identity: ProblemIdentity,
    pub message: String,
}
impl Problem {
    /// Use only a static operation label as context, never an error's Display.
    pub fn from_error(context: &'static str, error: &anyhow::Error) -> Self {
        if let Some(problem) = error.downcast_ref::<Self>() {
            return problem.clone();
        }
        let kind = if let Some(error) = error.downcast_ref::<crate::git::GitFailure>() {
            ProblemKind::Git(error.operation.clone())
        } else if let Some(error) = error.downcast_ref::<reqwest::Error>() {
            if error.is_timeout() {
                ProblemKind::Timeout
            } else if let Some(status) = error.status() {
                ProblemKind::Http(status.as_u16())
            } else {
                ProblemKind::Network
            }
        } else if let Some(error) = error.downcast_ref::<std::io::Error>() {
            ProblemKind::Io(format!("{:?}", error.kind()))
        } else if error.is::<toml::de::Error>() {
            ProblemKind::Configuration
        } else if error.is::<serde_json::Error>() || error.is::<std::string::FromUtf8Error>() {
            ProblemKind::InvalidData
        } else {
            ProblemKind::Other
        };
        Self {
            identity: ProblemIdentity {
                context: context.into(),
                kind,
            },
            message: format!("{error:#}"),
        }
    }
    /// String-only upstream ledgers retain a stable operation category.
    pub fn message(context: &'static str, message: impl Into<String>) -> Self {
        Self {
            identity: ProblemIdentity {
                context: context.into(),
                kind: ProblemKind::Other,
            },
            message: message.into(),
        }
    }
}
impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for Problem {}
impl std::ops::Deref for Problem {
    type Target = str;
    fn deref(&self) -> &str {
        &self.message
    }
}
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct NotificationState {
    identity: Option<ProblemIdentity>,
}
impl NotificationState {
    /// Suppressed new problems stay pending; a successful delivery commits the
    /// identity. Recovery is sent only for a problem that was actually notified.
    pub fn deliver(
        &mut self,
        problem: Option<&Problem>,
        allow_problem: bool,
        send: impl FnOnce(&str) -> Result<()>,
    ) -> Result<bool> {
        let identity = problem.map(|problem| &problem.identity);
        if self.identity.as_ref() == identity || (problem.is_some() && !allow_problem) {
            return Ok(false);
        }
        send(problem.map_or("Sync recovered", |problem| &problem.message))?;
        self.identity = identity.cloned();
        Ok(problem.is_some())
    }
}

pub fn record_problem(settings: &Settings, error: Option<&Problem>) -> Result<()> {
    if problem(settings)?.as_ref() == error {
        return Ok(());
    }
    atomic_write(
        &settings
            .state_dir
            .join(&settings.machine)
            .join("tray-problem.json"),
        &serde_json::to_vec(&error)?,
    )
}
pub(crate) fn problem(settings: &Settings) -> Result<Option<Problem>> {
    let path = settings
        .state_dir
        .join(&settings.machine)
        .join("tray-problem.json");
    no_symlinks(&path)?;
    Ok(if path.try_exists()? {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum StoredProblem {
            Structured(Problem),
            Legacy(String),
        }
        let stored: Option<StoredProblem> = serde_json::from_slice(&fs::read(path)?)?;
        stored.map(|problem| match problem {
            StoredProblem::Structured(problem) => problem,
            StoredProblem::Legacy(message) => Problem::message("Tray status", message),
        })
    } else {
        None
    })
}

#[derive(Deserialize, Serialize)]
struct MigrationRecord {
    completed_at: u64,
    open_at_login: bool,
}
#[cfg(windows)]
pub(crate) fn record_migration(settings: &Settings) -> Result<()> {
    let record = MigrationRecord {
        completed_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs(),
        open_at_login: true,
    };
    atomic_write(
        &settings
            .state_dir
            .join(&settings.machine)
            .join("tray-migration.json"),
        &serde_json::to_vec(&record)?,
    )
}
pub(crate) fn migration_log(settings: &Settings) -> Result<Option<String>> {
    let path = settings
        .state_dir
        .join(&settings.machine)
        .join("tray-migration.json");
    no_symlinks(&path)?;
    if !path.try_exists()? {
        return Ok(None);
    }
    let record: MigrationRecord = serde_json::from_slice(&fs::read(path)?)?;
    Ok(Some(format!(
        "{}: Legacy task migrated; Open Ratatoskr at login {} (--background); tray sync enabled\n",
        record.completed_at,
        if record.open_at_login {
            "enabled"
        } else {
            "disabled"
        }
    )))
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct LastSuccess {
    pub time: Option<u64>,
    pub skills: usize,
}
impl LastSuccess {
    pub fn load(settings: &Settings) -> Result<Self> {
        let path = settings
            .state_dir
            .join(&settings.machine)
            .join("tray-success.json");
        no_symlinks(&path)?;
        Ok(if path.try_exists()? {
            serde_json::from_slice(&fs::read(path)?)?
        } else {
            Self::default()
        })
    }
    pub fn save(&self, settings: &Settings) -> Result<()> {
        atomic_write(
            &settings
                .state_dir
                .join(&settings.machine)
                .join("tray-success.json"),
            &serde_json::to_vec(self)?,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn notification_identity_ignores_volatile_diagnostics_and_survives_restart() {
        let first = Problem::from_error(
            "Acquire sync lock",
            &anyhow::Error::new(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "PID 123 on host A after 12 ms",
            )),
        );
        let second = Problem::from_error(
            "Acquire sync lock",
            &anyhow::Error::new(std::io::Error::new(
                std::io::ErrorKind::WouldBlock,
                "PID 987 on host B after 89 ms",
            )),
        );
        assert_eq!(first.identity, second.identity);
        assert_ne!(first.message, second.message);
        let mut state = NotificationState::default();
        let mut messages = vec![];
        state
            .deliver(Some(&first), true, |message| {
                messages.push(message.to_owned());
                Ok(())
            })
            .unwrap();
        let mut state: NotificationState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        state
            .deliver(Some(&second), true, |_| {
                panic!("same problem must stay deduplicated")
            })
            .unwrap();
        assert!(
            state
                .deliver(None, true, |_| anyhow::bail!("toast unavailable"))
                .is_err()
        );
        state
            .deliver(None, true, |message| {
                messages.push(message.to_owned());
                Ok(())
            })
            .unwrap();
        state
            .deliver(None, true, |_| panic!("only one recovery"))
            .unwrap();
        assert_eq!(messages, [first.message, "Sync recovered".into()]);
    }
    #[test]
    fn diagnostic_ledger_updates_details_without_changing_problem_identity() {
        let temp = tempfile::tempdir().unwrap();
        let settings = Settings {
            state_dir: temp.path().canonicalize().unwrap(),
            machine: "test".into(),
            ..Default::default()
        };
        let first = Problem::message("Sync", "PID 1 timed out after 10 ms");
        let second = Problem::message("Sync", "PID 2 timed out after 50 ms");
        record_problem(&settings, Some(&first)).unwrap();
        record_problem(&settings, Some(&second)).unwrap();
        let saved = problem(&settings).unwrap().unwrap();
        assert_eq!(saved.identity, first.identity);
        assert_eq!(saved.message, second.message);
    }
    #[test]
    fn pause_resume_and_interval_survive_restart() {
        let temp = tempfile::tempdir().unwrap();
        let settings = Settings {
            state_dir: temp.path().canonicalize().unwrap(),
            machine: "test".into(),
            ..Default::default()
        };
        let mut state = TrayPreferences::load(&settings).unwrap();
        assert!(!state.paused);
        assert_eq!(state.interval_seconds, 60);
        state.paused = true;
        state.save(&settings).unwrap();
        assert!(TrayPreferences::load(&settings).unwrap().paused);
        state.paused = false;
        state.save(&settings).unwrap();
        assert!(!TrayPreferences::load(&settings).unwrap().paused);
        state.interval_seconds = 1;
        assert!(state.save(&settings).is_err());
    }
    #[test]
    fn runs_never_overlap_and_paused_runs_do_not_start() {
        let gate = RunGate::default();
        assert!(gate.enter(true).is_none());
        let run = gate.enter(false).unwrap();
        std::thread::scope(|scope| {
            let gate = &gate;
            scope
                .spawn(move || assert!(gate.enter(false).is_none()))
                .join()
                .unwrap();
        });
        drop(run);
        assert!(gate.enter(false).is_some());
    }
    #[test]
    fn existing_core_lock_defers_in_process_sync_without_overwriting_ledger() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let settings = Settings {
            source: root.join("source"),
            state_dir: root.join("state"),
            machine: "test".into(),
            ..Default::default()
        };
        fs::create_dir(&settings.source).unwrap();
        let run = crate::jobs::start(&settings).unwrap();
        let lock = crate::config::operation_lock(&settings, "sync.lock").unwrap();
        assert!(crate::git::sync_in_process(&settings).unwrap().is_none());
        assert_eq!(
            crate::job_status(&settings)
                .unwrap()
                .last_run
                .unwrap()
                .started_at,
            run.started_at
        );
        drop(lock);
        assert!(crate::git::sync_in_process(&settings).is_err());
        assert!(
            crate::job_status(&settings)
                .unwrap()
                .last_run
                .unwrap()
                .finished_at
                .is_some()
        );
    }
}

/// Carry visibility across the installer's own relaunch, which can retain stale
/// command-line arguments from an earlier login launch.
pub fn save_restart_mode(path: &std::path::Path, hidden: bool) -> Result<()> {
    atomic_write(path, &serde_json::to_vec(&hidden)?)
}
pub fn clear_restart_mode(path: &std::path::Path) -> Result<()> {
    no_symlinks(path)?;
    if path.try_exists()? {
        fs::remove_file(path)?;
    }
    Ok(())
}
/// The launch visibility `consume_restart_mode` will choose, without consuming the intent.
pub fn peek_restart_mode(path: &std::path::Path, background_argument: bool) -> Result<bool> {
    no_symlinks(path)?;
    if !path.try_exists()? {
        return Ok(background_argument);
    }
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}
pub fn consume_restart_mode(path: &std::path::Path, background_argument: bool) -> Result<bool> {
    no_symlinks(path)?;
    if !path.try_exists()? {
        return Ok(background_argument);
    }
    let hidden = serde_json::from_slice(&fs::read(path)?)?;
    fs::remove_file(path)?;
    Ok(hidden)
}

#[cfg(test)]
mod restart_tests {
    use super::*;
    #[test]
    fn hidden_restart_and_foreground_reopen_override_old_arguments_once() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap().join("app/restart.json");
        assert!(!consume_restart_mode(&path, false).unwrap());
        assert!(consume_restart_mode(&path, true).unwrap());
        save_restart_mode(&path, true).unwrap();
        // Peeking (to decide launch activation) leaves the one-shot intent in place.
        assert!(peek_restart_mode(&path, false).unwrap());
        assert!(consume_restart_mode(&path, false).unwrap());
        assert!(!path.exists());
        save_restart_mode(&path, false).unwrap();
        assert!(!consume_restart_mode(&path, true).unwrap());
        assert!(consume_restart_mode(&path, true).unwrap());
        save_restart_mode(&path, true).unwrap();
        clear_restart_mode(&path).unwrap();
        assert!(!consume_restart_mode(&path, false).unwrap());
        save_restart_mode(&path, true).unwrap();
        fs::write(&path, "invalid").unwrap();
        assert!(consume_restart_mode(&path, false).is_err());
        assert!(path.exists(), "Invalid intent must remain inspectable");
    }
}
