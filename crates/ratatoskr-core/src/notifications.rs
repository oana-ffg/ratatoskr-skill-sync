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
    error: Option<String>,
    sent_at: u64,
}

pub(crate) fn outcome(settings: &Settings, error: Option<&str>) -> Result<()> {
    if settings.alert_command.is_empty() {
        return Ok(());
    }
    let path = settings
        .state_dir
        .join(&settings.machine)
        .join("notification.json");
    no_symlinks(&path)?;
    let mut state: State = if path.try_exists()? {
        serde_json::from_slice(&fs::read(&path)?)?
    } else {
        State::default()
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
    let send = match error {
        Some(error) => {
            state.error.as_deref() != Some(error) || now.saturating_sub(state.sent_at) >= 86400
        }
        None => state.error.is_some(),
    };
    if !send {
        return Ok(());
    }
    let message = error.map_or_else(
        || format!("Ratatoskr {}: sync recovered", settings.machine),
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
    state.sent_at = now;
    atomic_write(&path, &serde_json::to_vec(&state)?)
}
