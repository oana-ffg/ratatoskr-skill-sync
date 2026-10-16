use anyhow::{Context, Result, ensure};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    time::Duration,
};

pub const OFFICIAL_ENDPOINT: &str =
    "https://github.com/oana-ffg/ratatoskr-skill-sync/releases/latest/download/latest.json";
pub const OFFICIAL_PUBLIC_KEY: &str = include_str!("update-public-key.txt");
pub const DESKTOP_WINDOW_MINUTES: i64 = 60;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct UpdateSettings {
    /// Explicit fork overrides; absent fields retain the compiled official trust root.
    pub endpoint: Option<String>,
    pub public_key: Option<String>,
    pub automatic: bool,
    pub time: String,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            endpoint: None,
            public_key: None,
            automatic: true,
            time: "00:00".into(),
        }
    }
}

impl UpdateSettings {
    pub fn endpoint(&self) -> &str {
        self.endpoint.as_deref().unwrap_or(OFFICIAL_ENDPOINT)
    }
    pub fn public_key(&self) -> &str {
        self.public_key
            .as_deref()
            .unwrap_or(OFFICIAL_PUBLIC_KEY)
            .trim()
    }
    pub fn override_active(&self) -> bool {
        self.endpoint.is_some() || self.public_key.is_some()
    }
    pub fn scheduled_time(&self) -> Result<chrono::NaiveTime> {
        ensure!(
            self.time.len() == 5,
            "Update time must use HH:MM local time"
        );
        let time = chrono::NaiveTime::parse_from_str(&self.time, "%H:%M")?;
        ensure!(
            time.format("%H:%M").to_string() == self.time,
            "Update time must use HH:MM local time"
        );
        Ok(time)
    }
    pub fn tauri_public_key(&self) -> Result<String> {
        self.validate()?;
        if minisign_verify::PublicKey::from_base64(self.public_key()).is_ok() {
            Ok(base64::engine::general_purpose::STANDARD.encode(format!(
                "untrusted comment: Ratatoskr update public key\n{}\n",
                self.public_key()
            )))
        } else {
            Ok(self.public_key().into())
        }
    }
    pub fn validate(&self) -> Result<()> {
        let endpoint = reqwest::Url::parse(self.endpoint())?;
        ensure!(
            endpoint.scheme() == "https"
                && endpoint.username().is_empty()
                && endpoint.password().is_none(),
            "Update endpoint must use HTTPS without embedded credentials"
        );
        key(self.public_key())?;
        self.scheduled_time()?;
        Ok(())
    }
}

/// The caller injects a local clock. Calendar-day accounting avoids minute-by-minute requests.
pub fn check_due(
    settings: &UpdateSettings,
    now: chrono::DateTime<chrono::FixedOffset>,
    last: Option<chrono::DateTime<chrono::FixedOffset>>,
    desktop: bool,
) -> Result<bool> {
    if desktop {
        let slot = desktop_slot(settings, now)?;
        let already_checked = last
            .map(|last| desktop_slot(settings, last))
            .transpose()?
            .is_some_and(|last| last >= slot);
        return Ok(!already_checked && desktop_window(settings, now)?);
    }
    let already_checked =
        last.is_some_and(|last| last.with_timezone(now.offset()).date_naive() >= now.date_naive());
    Ok(settings.automatic && !already_checked && now.time() >= settings.scheduled_time()?)
}

fn desktop_slot(
    settings: &UpdateSettings,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> Result<chrono::NaiveDateTime> {
    let today = now.date_naive().and_time(settings.scheduled_time()?);
    Ok(if now.naive_local() < today {
        today - chrono::Duration::days(1)
    } else {
        today
    })
}

pub fn desktop_window(
    settings: &UpdateSettings,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> Result<bool> {
    let elapsed = now.naive_local() - desktop_slot(settings, now)?;
    Ok(settings.automatic && elapsed < chrono::Duration::minutes(DESKTOP_WINDOW_MINUTES))
}

#[derive(Default, Deserialize, Serialize)]
struct HelperState {
    last_check: Option<chrono::DateTime<chrono::FixedOffset>>,
}

pub fn helper_update(settings: &crate::Settings) -> Result<()> {
    if !standalone_helper()? {
        return Ok(());
    }
    helper_update_with(settings, chrono::Local::now().fixed_offset(), || {
        if let Some(release) = check(&settings.updates, env!("CARGO_PKG_VERSION"))? {
            install(&settings.updates, &release)?;
        }
        Ok(())
    })
}

fn helper_update_with(
    settings: &crate::Settings,
    now: chrono::DateTime<chrono::FixedOffset>,
    check: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if !settings.updates.automatic {
        return Ok(());
    }
    let path = settings
        .state_dir
        .join(&settings.machine)
        .join("helper-update.json");
    crate::config::no_symlinks(&path)?;
    let state: HelperState = if path.try_exists()? {
        serde_json::from_slice(&std::fs::read(&path)?)?
    } else {
        HelperState::default()
    };
    if check_due(&settings.updates, now, state.last_check, false)? {
        // Record attempts before networking, including failures, to cap requests at one per day.
        crate::config::atomic_write(
            &path,
            &serde_json::to_vec(&HelperState {
                last_check: Some(now),
            })?,
        )?;
        check().context("Helper update failed")?;
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Release {
    pub version: String,
    #[serde(default)]
    pub notes: String,
    pub platforms: BTreeMap<String, Asset>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Asset {
    pub url: String,
    pub signature: String,
}

fn key(value: &str) -> Result<minisign_verify::PublicKey> {
    if let Ok(key) = minisign_verify::PublicKey::from_base64(value.trim()) {
        return Ok(key);
    }
    let decoded = base64::engine::general_purpose::STANDARD.decode(value.trim())?;
    let text = std::str::from_utf8(&decoded)?;
    let line = text
        .lines()
        .find(|line| line.starts_with("RW"))
        .context("No Minisign public key found")?;
    Ok(minisign_verify::PublicKey::from_base64(line)?)
}

fn client() -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(concat!("ratatoskr-skill-sync/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(60))
        .https_only(true)
        .build()?)
}

pub fn check(settings: &UpdateSettings, current: &str) -> Result<Option<Release>> {
    settings.validate()?;
    let response = client()?.get(settings.endpoint()).send()?;
    if response.status() == reqwest::StatusCode::NOT_FOUND
        || response.status() == reqwest::StatusCode::NO_CONTENT
    {
        return Ok(None);
    }
    let mut bytes = Vec::new();
    response
        .error_for_status()?
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= 1024 * 1024, "Update manifest is too large");
    let release: Release = serde_json::from_slice(&bytes)?;
    if semver::Version::parse(release.version.trim_start_matches('v'))?
        > semver::Version::parse(current.trim_start_matches('v'))?
    {
        Ok(Some(release))
    } else {
        Ok(None)
    }
}

pub fn platform() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    format!("{os}-{}-cli", std::env::consts::ARCH)
}

pub fn verify(public_key: &str, bytes: &[u8], signature: &str) -> Result<()> {
    let signature = base64::engine::general_purpose::STANDARD
        .decode(signature.trim())
        .ok();
    let signature = signature
        .as_deref()
        .map(std::str::from_utf8)
        .transpose()?
        .unwrap_or("");
    let signature = minisign_verify::Signature::decode(signature)?;
    key(public_key)?.verify(bytes, &signature, false)?;
    Ok(())
}

pub fn install(settings: &UpdateSettings, release: &Release) -> Result<()> {
    settings.validate()?;
    ensure!(
        standalone_helper()?,
        "Update the desktop app to replace its bundled helper"
    );
    ensure!(
        semver::Version::parse(release.version.trim_start_matches('v'))?
            > semver::Version::parse(env!("CARGO_PKG_VERSION"))?,
        "Refusing an old update"
    );
    let asset = release
        .platforms
        .get(&platform())
        .context("Release has no helper binary for this platform")?;
    let mut bytes = Vec::new();
    client()?
        .get(&asset.url)
        .send()?
        .error_for_status()?
        .take(128 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= 128 * 1024 * 1024,
        "Helper update is too large"
    );
    verify(settings.public_key(), &bytes, &asset.signature)?;
    let mut file = tempfile::Builder::new()
        .suffix(if cfg!(windows) { ".exe" } else { "" })
        .tempfile()?;
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(file.path(), std::fs::Permissions::from_mode(0o755))?;
    }
    let file = file.into_temp_path();
    let version = std::process::Command::new(&file)
        .arg("--version")
        .output()?;
    ensure!(
        version.status.success(),
        "Signed update failed its version check"
    );
    let actual = String::from_utf8(version.stdout)?;
    ensure!(
        actual.trim() == format!("ratatoskr {}", release.version.trim_start_matches('v')),
        "Signed binary version does not match manifest; refusing update"
    );
    self_replace::self_replace(&file)?;
    Ok(())
}

pub fn standalone_helper() -> Result<bool> {
    let path = std::env::current_exe()?;
    Ok(path.file_stem().is_some_and(|x| x == "ratatoskr")
        && !path
            .ancestors()
            .any(|p| p.extension().is_some_and(|x| x == "app"))
        && !path.with_file_name("ratatoskr-desktop.exe").try_exists()?)
}

pub enum UpdateKind {
    Desktop,
    Helper,
}

/// Unrelated sync successes must not clear an update alert.
pub fn report_outcome(
    settings: &crate::Settings,
    error: Option<&str>,
    kind: UpdateKind,
) -> Result<()> {
    let filename = match kind {
        UpdateKind::Desktop => "desktop-update-notification.json",
        UpdateKind::Helper => "helper-update-notification.json",
    };
    crate::notifications::outcome_for(settings, error, filename)
}

/// Holds the same locks as CLI sync/apply/schedule operations through desktop installation.
pub struct UpdateGuard {
    _locks: Vec<std::fs::File>,
}

pub fn try_idle_guard(settings: &crate::Settings) -> Result<Option<UpdateGuard>> {
    let mut locks = Vec::new();
    for name in ["sync.lock", "apply.lock", "schedule.lock"] {
        let file = crate::config::state_lock(settings, name)?;
        match fs2::FileExt::try_lock_exclusive(&file) {
            Ok(()) => locks.push(file),
            Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(Some(UpdateGuard { _locks: locks }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn desktop_install_defers_for_other_process_operations() {
        let temp = tempfile::tempdir().unwrap();
        let settings = crate::Settings {
            state_dir: temp.path().canonicalize().unwrap(),
            machine: "test".into(),
            ..Default::default()
        };
        for name in ["sync.lock", "apply.lock", "schedule.lock"] {
            let busy = crate::config::state_lock(&settings, name).unwrap();
            fs2::FileExt::try_lock_exclusive(&busy).unwrap();
            assert!(try_idle_guard(&settings).unwrap().is_none());
            drop(busy);
            let guard = try_idle_guard(&settings).unwrap().unwrap();
            let file = crate::config::state_lock(&settings, name).unwrap();
            assert!(fs2::FileExt::try_lock_exclusive(&file).is_err());
            drop(guard);
            assert!(fs2::FileExt::try_lock_exclusive(&file).is_ok());
        }
    }
    fn clock(value: &str) -> chrono::DateTime<chrono::FixedOffset> {
        chrono::DateTime::parse_from_rfc3339(value).unwrap()
    }
    #[test]
    fn official_defaults_and_explicit_overrides() {
        let defaults: UpdateSettings = toml::from_str("").unwrap();
        defaults.validate().unwrap();
        verify(
            OFFICIAL_PUBLIC_KEY,
            include_bytes!("../tests/fixtures/update.txt"),
            include_str!("../tests/fixtures/official-update.txt.sig"),
        )
        .unwrap();
        assert_eq!(defaults.endpoint(), OFFICIAL_ENDPOINT);
        assert_eq!(defaults.public_key(), OFFICIAL_PUBLIC_KEY.trim());
        assert!(!defaults.override_active());
        assert!(defaults.automatic);
        assert_eq!(defaults.time, "00:00");
        let settings: crate::Settings =
            toml::from_str("source='/content'\nmachine='test'\nstate_dir='/state'").unwrap();
        assert_eq!(settings.deploy_branch, "main");
        assert_eq!(settings.updates, defaults);
        let overridden: UpdateSettings =
            toml::from_str("endpoint='https://example.com/fork.json'").unwrap();
        assert_eq!(overridden.endpoint(), "https://example.com/fork.json");
        assert_eq!(overridden.public_key(), defaults.public_key());
        assert!(overridden.override_active());
        let key_only = UpdateSettings {
            public_key: Some(include_str!("../tests/fixtures/update.pub").trim().into()),
            ..Default::default()
        };
        key_only.validate().unwrap();
        assert_eq!(key_only.endpoint(), OFFICIAL_ENDPOINT);
        assert!(key_only.override_active());
        assert_ne!(key_only.public_key(), defaults.public_key());
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../../desktop/src-tauri/tauri.conf.json"))
                .unwrap();
        assert_eq!(
            config["plugins"]["updater"]["pubkey"],
            defaults.public_key()
        );
        assert_eq!(
            config["plugins"]["updater"]["endpoints"][0],
            OFFICIAL_ENDPOINT
        );
        assert!(
            UpdateSettings {
                endpoint: Some(String::new()),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            UpdateSettings {
                public_key: Some(String::new()),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    #[test]
    fn local_clock_controls_daily_checks_and_bounded_desktop_window() {
        let settings = UpdateSettings {
            time: "03:15".into(),
            ..Default::default()
        };
        assert!(!check_due(&settings, clock("2026-10-16T03:14:59+02:00"), None, true).unwrap());
        let now = clock("2026-10-16T03:15:00+02:00");
        assert!(check_due(&settings, now, None, true).unwrap());
        assert!(!check_due(&settings, now, Some(now), false).unwrap());
        assert!(
            check_due(
                &settings,
                clock("2026-10-17T03:15:00+02:00"),
                Some(now),
                false
            )
            .unwrap()
        );
        assert!(!check_due(&settings, clock("2026-10-16T04:15:00+02:00"), None, true).unwrap());
        assert!(check_due(&settings, clock("2026-10-16T04:15:00+02:00"), None, false).unwrap());
        assert!(
            !check_due(
                &UpdateSettings {
                    automatic: false,
                    ..settings
                },
                now,
                None,
                false
            )
            .unwrap()
        );
        assert!(
            UpdateSettings {
                time: "25:00".into(),
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    #[test]
    fn desktop_window_spans_midnight_and_handles_local_offset_changes() {
        let settings = UpdateSettings {
            time: "23:30".into(),
            ..Default::default()
        };
        let checked = clock("2026-10-16T23:30:00+02:00");
        let after_midnight = clock("2026-10-17T00:15:00+02:00");
        assert!(desktop_window(&settings, after_midnight).unwrap());
        assert!(!check_due(&settings, after_midnight, Some(checked), true).unwrap());
        assert!(!desktop_window(&settings, clock("2026-10-17T00:30:00+02:00")).unwrap());
        let repeated_hour = UpdateSettings {
            time: "02:15".into(),
            ..Default::default()
        };
        assert!(
            !check_due(
                &repeated_hour,
                clock("2026-10-25T02:30:00+01:00"),
                Some(clock("2026-10-25T02:15:00+02:00")),
                true
            )
            .unwrap()
        );
        let midnight = UpdateSettings::default();
        assert!(
            check_due(
                &midnight,
                clock("2026-10-26T00:00:00+01:00"),
                Some(clock("2026-10-25T00:00:00+02:00")),
                false
            )
            .unwrap()
        );
    }
    #[test]
    fn helper_persists_attempts_including_failures_and_retries_next_day() {
        let temp = tempfile::tempdir().unwrap();
        let settings = crate::Settings {
            state_dir: temp.path().canonicalize().unwrap(),
            machine: "test".into(),
            ..Default::default()
        };
        let now = clock("2026-10-16T00:00:00+02:00");
        assert!(helper_update_with(&settings, now, || anyhow::bail!("offline")).is_err());
        helper_update_with(&settings, clock("2026-10-16T23:59:00+02:00"), || {
            panic!("must not check twice")
        })
        .unwrap();
        let mut checked = false;
        helper_update_with(&settings, clock("2026-10-17T00:00:00+02:00"), || {
            checked = true;
            Ok(())
        })
        .unwrap();
        assert!(checked);
        let path = settings.state_dir.join("test/helper-update.json");
        std::fs::write(path, "corrupt").unwrap();
        assert!(
            helper_update_with(&settings, now, || panic!("invalid state must fail closed"))
                .is_err()
        );
    }
}
