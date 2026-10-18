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

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct NotificationState {
    error: Option<String>,
}
impl NotificationState {
    /// Commit only after successful delivery, so failed toasts stay retryable.
    pub fn deliver(
        &mut self,
        error: Option<&str>,
        send: impl FnOnce(&str) -> Result<()>,
    ) -> Result<()> {
        if self.error.as_deref() == error {
            return Ok(());
        }
        send(error.unwrap_or("Sync recovered"))?;
        self.error = error.map(str::to_owned);
        Ok(())
    }
}

pub fn record_problem(settings: &Settings, error: Option<&str>) -> Result<()> {
    if problem(settings)?.as_deref() == error {
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
pub(crate) fn problem(settings: &Settings) -> Result<Option<String>> {
    let path = settings
        .state_dir
        .join(&settings.machine)
        .join("tray-problem.json");
    no_symlinks(&path)?;
    Ok(if path.try_exists()? {
        serde_json::from_slice(&fs::read(path)?)?
    } else {
        None
    })
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
    fn notification_transitions_are_deduplicated_without_a_time_limit() {
        let mut state = NotificationState::default();
        let mut messages = vec![];
        for error in [
            Some("network"),
            Some("network"),
            None,
            None,
            Some("network"),
            Some("conflict"),
            Some("conflict"),
        ] {
            state
                .deliver(error, |message| {
                    messages.push(message.to_owned());
                    Ok(())
                })
                .unwrap();
        }
        assert_eq!(
            messages,
            ["network", "Sync recovered", "network", "conflict"]
        );
        let mut persisted: NotificationState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        persisted
            .deliver(Some("conflict"), |_| panic!("must survive restart"))
            .unwrap();
        assert!(
            persisted
                .deliver(None, |_| anyhow::bail!("toast unavailable"))
                .is_err()
        );
        persisted.deliver(None, |_| Ok(())).unwrap();
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
