use crate::{
    Settings,
    config::{atomic_write, no_symlinks},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Default, Deserialize, Serialize)]
struct State {
    #[serde(default)]
    version: u32,
    error: Option<String>,
    sent_at: u64,
    #[serde(default)]
    delivered_error: Option<String>,
}

pub(crate) fn outcome(settings: &Settings, error: Option<&str>) -> Result<()> {
    outcome_for(settings, error, "notification.json")
}

pub(crate) fn outcome_for(settings: &Settings, error: Option<&str>, filename: &str) -> Result<()> {
    let path = settings.state_dir.join(&settings.machine).join(filename);
    no_symlinks(&path)?;
    let mut state: State = if path.try_exists()? {
        serde_json::from_slice(&fs::read(&path)?)?
    } else {
        State::default()
    };
    if state.version == 0 {
        // v0.1 wrote a ledger only after delivery; preserve its 24-hour dedup window.
        if state.sent_at != 0 {
            state.delivered_error = state.error.clone();
        }
        state.version = 1;
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let send = match error {
        Some(error) => {
            state.delivered_error.as_deref() != Some(error)
                || now.saturating_sub(state.sent_at) >= 86400
        }
        None => state.delivered_error.is_some(),
    };
    state.error = error.map(str::to_owned);
    atomic_write(&path, &serde_json::to_vec(&state)?)?;
    if !send || settings.alert_command.is_empty() {
        return Ok(());
    }
    let message = error.map_or_else(
        || format!("Ratatoskr {}: operation recovered", settings.machine),
        |e| format!("Ratatoskr {}: {e}", settings.machine),
    );
    let (program, args) = settings
        .alert_command
        .split_first()
        .context("No alert program configured")?;
    let output = Command::new(program)
        .args(args)
        .arg(message)
        .output()
        .context("Run configured alert command")?;
    ensure!(
        output.status.success(),
        "Alert delivery failed with {}",
        output.status
    );
    state.error = error.map(str::to_owned);
    state.delivered_error = error.map(str::to_owned);
    state.sent_at = now;
    atomic_write(&path, &serde_json::to_vec(&state)?)
}

pub(crate) fn current_error(settings: &Settings, filename: &str) -> Result<Option<String>> {
    let path = settings.state_dir.join(&settings.machine).join(filename);
    no_symlinks(&path)?;
    if !path.try_exists()? {
        return Ok(None);
    }
    let state: State = serde_json::from_slice(&fs::read(path)?)?;
    Ok(state.error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_ledger_keeps_deduplication_and_failed_delivery_stays_retryable() {
        let temp = tempfile::tempdir().unwrap();
        let settings = Settings {
            state_dir: temp.path().canonicalize().unwrap(),
            machine: "test".into(),
            alert_command: vec!["ratatoskr-test-nonexistent-notifier".into()],
            ..Default::default()
        };
        let path = settings.state_dir.join("test/notification.json");
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        atomic_write(
            &path,
            &serde_json::to_vec(
                &serde_json::json!({ "error": "already delivered", "sent_at": now }),
            )
            .unwrap(),
        )
        .unwrap();
        outcome(&settings, Some("already delivered")).unwrap();
        assert!(outcome(&settings, Some("new failure")).is_err());
        assert_eq!(
            current_error(&settings, "notification.json")
                .unwrap()
                .as_deref(),
            Some("new failure")
        );
        assert!(
            outcome(&settings, Some("new failure")).is_err(),
            "Failed delivery must retry immediately rather than dedup the undelivered error"
        );
    }
}
