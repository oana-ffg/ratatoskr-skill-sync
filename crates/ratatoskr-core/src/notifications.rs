use crate::{
    SecretReference, Settings,
    config::{atomic_write, no_symlinks},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
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
    if !send || (settings.alert.is_none() && settings.alert_command.is_empty()) {
        return Ok(());
    }
    let message = error.map_or_else(
        || format!("Ratatoskr {}: operation recovered", settings.machine),
        |e| format!("Ratatoskr {}: {e}", settings.machine),
    );
    if let Some(alert) = &settings.alert {
        if let Err(error) = alert.deliver(&message) {
            state.error = Some(format!("{message}; alert delivery failed: {error:#}"));
            atomic_write(&path, &serde_json::to_vec(&state)?)?;
            return Err(error.context("Alert delivery failed"));
        }
    } else {
        let (program, args) = settings
            .alert_command
            .split_first()
            .context("No alert program configured")?;
        let output = crate::process::command(program)
            .args(args)
            .arg(message)
            .output()
            .context("Run configured alert command")?;
        ensure!(
            output.status.success(),
            "Alert delivery failed with {}",
            output.status
        );
    }
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AlertSettings {
    System,
    Pushover {
        #[serde(
            alias = "token_env",
            deserialize_with = "crate::secrets::required_reference"
        )]
        token: SecretReference,
        #[serde(
            alias = "user_env",
            deserialize_with = "crate::secrets::required_reference"
        )]
        user: SecretReference,
    },
    Ntfy {
        url: String,
        #[serde(
            default,
            alias = "token_env",
            deserialize_with = "crate::secrets::optional_reference",
            skip_serializing_if = "Option::is_none"
        )]
        token: Option<SecretReference>,
    },
}
impl AlertSettings {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::Pushover { token, user } => {
                token.validate()?;
                user.validate()?;
            }
            Self::Ntfy { url, token } => {
                let parsed = reqwest::Url::parse(url)?;
                ensure!(
                    parsed.scheme() == "https" && parsed.host_str().is_some(),
                    "ntfy requires an HTTPS topic URL"
                );
                ensure!(
                    parsed.username().is_empty() && parsed.password().is_none(),
                    "Store ntfy credentials as a secret reference, not in the URL"
                );
                if let Some(token) = token {
                    token.validate()?;
                }
            }
            _ => (),
        }
        Ok(())
    }
    fn deliver(&self, message: &str) -> Result<()> {
        self.validate()?;
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        match self {
            Self::Pushover { token, user } => {
                let token = token
                    .resolve()
                    .context("Resolve Pushover application token")?;
                let user = user.resolve().context("Resolve Pushover user key")?;
                let body: serde_json::Value = client
                    .post("https://api.pushover.net/1/messages.json")
                    .form(&[
                        ("token", token.as_str()),
                        ("user", user.as_str()),
                        ("message", message),
                    ])
                    .send()?
                    .error_for_status()?
                    .json()?;
                ensure!(body["status"] == 1, "Pushover rejected the alert");
            }
            Self::Ntfy { url, token } => {
                let mut request = client.post(url).body(message.to_owned());
                if let Some(token) = token {
                    request =
                        request.bearer_auth(token.resolve().context("Resolve ntfy bearer token")?);
                }
                request.send()?.error_for_status()?;
            }
            Self::System => {
                #[cfg(target_os="macos")]
                let result=crate::process::command("osascript").args(["-e","on run argv\ndisplay notification (item 1 of argv) with title \"Ratatoskr Skill Sync\"\nend run",message]).output()?;
                #[cfg(target_os = "linux")]
                let result = crate::process::command("notify-send")
                    .args(["Ratatoskr Skill Sync", message])
                    .output()?;
                #[cfg(target_os="windows")]
                let result=crate::process::command("powershell.exe").args(["-NoProfile","-NonInteractive","-Command",r#"$ErrorActionPreference='Stop'; [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType=WindowsRuntime] > $null; [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType=WindowsRuntime] > $null; $xml = New-Object Windows.Data.Xml.Dom.XmlDocument; $xml.LoadXml('<toast><visual><binding template="ToastText02"><text id="1">Ratatoskr Skill Sync</text><text id="2"></text></binding></visual></toast>'); $xml.GetElementsByTagName('text').Item(1).InnerText=$env:RATATOSKR_ALERT_MESSAGE; [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('Ratatoskr Skill Sync').Show([Windows.UI.Notifications.ToastNotification]::new($xml))"#]).env("RATATOSKR_ALERT_MESSAGE",message).output()?;
                ensure!(
                    result.status.success(),
                    "System notification failed: {}",
                    String::from_utf8_lossy(&result.stderr)
                );
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod reference_tests {
    use super::*;
    #[test]
    fn legacy_environment_settings_migrate_without_values() {
        let alert: AlertSettings = serde_json::from_value(serde_json::json!({"kind":"pushover", "token_env":"PUSH_TOKEN", "user_env":"PUSH_USER"})).unwrap();
        alert.validate().unwrap();
        let stored = serde_json::to_value(alert).unwrap();
        assert_eq!(
            stored["token"],
            serde_json::json!({"kind":"environment","variable":"PUSH_TOKEN"})
        );
        assert!(stored.get("token_env").is_none());
        let public: AlertSettings = serde_json::from_value(
            serde_json::json!({"kind":"ntfy","url":"https://ntfy.sh/ratatoskr","token_env":""}),
        )
        .unwrap();
        assert!(serde_json::to_value(public).unwrap().get("token").is_none());
    }
    #[test]
    fn failed_secret_resolution_is_visible_and_retryable() {
        let temp = tempfile::tempdir().unwrap();
        let settings = Settings {
            machine: "test".into(),
            state_dir: temp.path().canonicalize().unwrap(),
            alert: Some(AlertSettings::Pushover {
                token: SecretReference::Environment {
                    variable: "RATATOSKR_MISSING_ALERT_TEST".into(),
                },
                user: SecretReference::Environment {
                    variable: "RATATOSKR_MISSING_USER_TEST".into(),
                },
            }),
            ..Default::default()
        };
        for _ in 0..2 {
            assert!(outcome(&settings, Some("sync failed")).is_err());
            let error = current_error(&settings, "notification.json")
                .unwrap()
                .unwrap();
            assert!(
                error.contains("sync failed")
                    && error.contains("alert delivery failed")
                    && error.contains("variable unavailable")
            );
        }
        let state: State = serde_json::from_slice(
            &fs::read(settings.state_dir.join("test/notification.json")).unwrap(),
        )
        .unwrap();
        assert!(state.delivered_error.is_none());
    }
}
