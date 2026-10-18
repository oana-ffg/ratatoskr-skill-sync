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
fn helper_check_due(
    settings: &UpdateSettings,
    now: chrono::DateTime<chrono::FixedOffset>,
    last: Option<chrono::DateTime<chrono::FixedOffset>>,
) -> Result<bool> {
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

/// Persist attempts across launches, including an available update deferred by busy work.
#[derive(Default, Deserialize, Serialize)]
pub struct DesktopUpdateState {
    last_check: Option<chrono::DateTime<chrono::FixedOffset>>,
    settings: Option<UpdateSettings>,
    pending: bool,
}

impl DesktopUpdateState {
    pub fn load(path: &std::path::Path) -> Result<Self> {
        crate::config::no_symlinks(path)?;
        if path.try_exists()? {
            Ok(serde_json::from_slice(&std::fs::read(path)?)?)
        } else {
            Ok(Self::default())
        }
    }

    pub fn due(
        &self,
        settings: &UpdateSettings,
        now: chrono::DateTime<chrono::FixedOffset>,
        launch: bool,
    ) -> Result<bool> {
        if !settings.automatic {
            return Ok(false);
        }
        let same_settings = self.settings.as_ref() == Some(settings);
        let last_slot = self
            .last_check
            .filter(|_| same_settings)
            .map(|last| desktop_slot(settings, last))
            .transpose()?;
        let slot = desktop_slot(settings, now)?;
        let missed = last_slot.is_none_or(|last| last < slot);
        Ok((missed || (launch && same_settings && self.pending))
            && (launch || desktop_window(settings, now)?))
    }

    pub fn record_attempt(
        &mut self,
        path: &std::path::Path,
        settings: &UpdateSettings,
        now: chrono::DateTime<chrono::FixedOffset>,
    ) -> Result<()> {
        self.last_check = Some(now);
        self.settings = Some(settings.clone());
        self.record_available(path, false)
    }

    pub fn record_available(&mut self, path: &std::path::Path, available: bool) -> Result<()> {
        self.pending = available;
        crate::config::atomic_write(path, &serde_json::to_vec(self)?)
    }
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

pub(crate) fn helper_update_with(
    settings: &crate::Settings,
    now: chrono::DateTime<chrono::FixedOffset>,
    check: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let result = (|| -> Result<bool> {
        if !settings.updates.automatic {
            return Ok(false);
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
        if helper_check_due(&settings.updates, now, state.last_check)? {
            // Record attempts before networking, including failures, to cap requests at one per day.
            crate::config::atomic_write(
                &path,
                &serde_json::to_vec(&HelperState {
                    last_check: Some(now),
                })?,
            )?;
            check()?;
            return Ok(true);
        }
        Ok(false)
    })();
    // A skipped check is not a recovery. Only actual attempts can clear an update alert.
    if !matches!(result, Ok(false)) {
        let error = result
            .as_ref()
            .err()
            .map(|e| format!("Helper update: {e:#}"));
        report_outcome(settings, error.as_deref(), UpdateKind::Helper).map_err(|notification| {
            anyhow::anyhow!(
                "{}; alert delivery failed: {notification:#}",
                error.as_deref().unwrap_or("Helper update completed")
            )
        })?;
    }
    result.map(|_| ())
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
    let file = download_helper(settings, asset, &release.version, "ratatoskr")?;
    #[cfg(windows)]
    {
        let asset = release
            .platforms
            .get(&format!("windows-{}-windowless", std::env::consts::ARCH))
            .context("Release has no windowless helper; refusing to update only one binary")?;
        let windowless = download_helper(settings, asset, &release.version, "ratatoskrw")?;
        install_windows_pair(&file, &windowless)?;
    }
    #[cfg(not(windows))]
    self_replace::self_replace(&file)?;
    Ok(())
}

fn download_helper(
    settings: &UpdateSettings,
    asset: &Asset,
    version: &str,
    name: &str,
) -> Result<tempfile::TempPath> {
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
    let version_output = crate::process::command(&file).arg("--version").output()?;
    ensure!(
        version_output.status.success(),
        "Signed update failed its version check"
    );
    let actual = String::from_utf8(version_output.stdout)?;
    ensure!(
        actual.trim() == format!("{name} {}", version.trim_start_matches('v')),
        "Signed binary version does not match manifest; refusing update"
    );
    Ok(file)
}

#[cfg(windows)]
fn install_windows_pair(cli: &std::path::Path, windowless: &std::path::Path) -> Result<()> {
    install_windows_pair_at(&std::env::current_exe()?, cli, windowless, |file| {
        self_replace::self_replace(file).map_err(Into::into)
    })
}

#[cfg(windows)]
fn install_windows_pair_at(
    executable: &std::path::Path,
    cli: &std::path::Path,
    windowless: &std::path::Path,
    replace_current: impl FnOnce(&std::path::Path) -> Result<()>,
) -> Result<()> {
    use crate::config::{atomic_write, no_symlinks};
    let directory = executable
        .parent()
        .context("Helper has no parent directory")?;
    let lock_path = directory.join(".ratatoskr-update.lock");
    no_symlinks(&lock_path)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    fs2::FileExt::try_lock_exclusive(&lock).context("Another helper update is in progress")?;
    let running_windowless = executable
        .file_stem()
        .is_some_and(|name| name == "ratatoskrw");
    let (current, companion, companion_name) = if running_windowless {
        (windowless, cli, "ratatoskr.exe")
    } else {
        (cli, windowless, "ratatoskrw.exe")
    };
    let destination = directory.join(companion_name);
    no_symlinks(&destination)?;
    no_symlinks(executable)?;
    let previous_current = std::fs::read(executable)?;
    let previous = if destination.try_exists()? {
        Some(std::fs::read(&destination)?)
    } else {
        None
    };
    // Stage/verify both downloads first. A busy companion fails here, before
    // changing the running executable, and the caller records the update error.
    atomic_write(&destination, &std::fs::read(companion)?).context(
        "Replace companion helper (close other Ratatoskr CLI processes and retry if busy)",
    )?;
    if let Err(error) = replace_current(current) {
        // self_replace can fail after relocating the running image. Restore
        // both paths, and attempt both restorations even if one fails.
        let current_restore = restore_helper(executable, Some(&previous_current));
        let companion_restore = restore_helper(&destination, previous.as_deref());
        if current_restore.is_err() || companion_restore.is_err() {
            anyhow::bail!(
                "Self-update failed: {error:#}; current rollback: {current_restore:?}; companion rollback: {companion_restore:?}"
            );
        }
        return Err(error);
    }
    Ok(())
}

#[cfg(windows)]
fn restore_helper(path: &std::path::Path, previous: Option<&[u8]>) -> Result<()> {
    crate::config::no_symlinks(path)?;
    let existing = if path.try_exists()? {
        Some(std::fs::read(path)?)
    } else {
        None
    };
    if existing.as_deref() == previous {
        return Ok(());
    }
    if let Some(previous) = previous {
        crate::config::atomic_write(path, previous)?;
    } else {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

pub fn standalone_helper() -> Result<bool> {
    let path = std::env::current_exe()?;
    Ok(path
        .file_stem()
        .is_some_and(|x| x == "ratatoskr" || (cfg!(windows) && x == "ratatoskrw"))
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
    locks: Vec<std::fs::File>,
}

impl Drop for UpdateGuard {
    fn drop(&mut self) {
        for file in &self.locks {
            // Explicit unlock also releases locks retained by a briefly inherited file handle.
            if let Err(error) = fs2::FileExt::unlock(file) {
                eprintln!("Ratatoskr: release desktop update lock: {error}");
            }
        }
    }
}

/// Startup waits for helper work to finish; an interactive nightly tick defers instead.
pub fn idle_guard(settings: &crate::Settings, wait: bool) -> Result<Option<UpdateGuard>> {
    let mut guard = UpdateGuard { locks: Vec::new() };
    for name in ["sync.lock", "apply.lock", "schedule.lock"] {
        let file = crate::config::state_lock(settings, name)?;
        let result = if wait {
            fs2::FileExt::lock_exclusive(&file)
        } else {
            fs2::FileExt::try_lock_exclusive(&file)
        };
        match result {
            Ok(()) => guard.locks.push(file),
            Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(Some(guard))
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
            assert!(idle_guard(&settings, false).unwrap().is_none());
            fs2::FileExt::unlock(&busy).unwrap();
            drop(busy);
            let guard = idle_guard(&settings, false).unwrap().unwrap();
            let file = crate::config::state_lock(&settings, name).unwrap();
            assert!(fs2::FileExt::try_lock_exclusive(&file).is_err());
            let inherited = guard.locks[0].try_clone().unwrap();
            drop(guard);
            assert!(fs2::FileExt::try_lock_exclusive(&file).is_ok());
            fs2::FileExt::unlock(&file).unwrap();
            drop(inherited);
        }
    }
    #[test]
    fn startup_install_waits_for_helper_work_to_finish() {
        let temp = tempfile::tempdir().unwrap();
        let settings = crate::Settings {
            state_dir: temp.path().canonicalize().unwrap(),
            machine: "test".into(),
            ..Default::default()
        };
        let busy = crate::config::state_lock(&settings, "sync.lock").unwrap();
        fs2::FileExt::try_lock_exclusive(&busy).unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (acquired_tx, acquired_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let _guard = idle_guard(&settings, true).unwrap().unwrap();
            acquired_tx.send(()).unwrap();
        });
        started_rx.recv().unwrap();
        assert!(matches!(
            acquired_rx.recv_timeout(Duration::from_millis(50)),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
        ));
        fs2::FileExt::unlock(&busy).unwrap();
        acquired_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        worker.join().unwrap();
    }

    fn clock(value: &str) -> chrono::DateTime<chrono::FixedOffset> {
        chrono::DateTime::parse_from_rfc3339(value).unwrap()
    }
    fn nightly_due(
        settings: &UpdateSettings,
        now: chrono::DateTime<chrono::FixedOffset>,
        last_check: Option<chrono::DateTime<chrono::FixedOffset>>,
    ) -> bool {
        DesktopUpdateState {
            last_check,
            settings: Some(settings.clone()),
            pending: false,
        }
        .due(settings, now, false)
        .unwrap()
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
        assert!(!nightly_due(
            &settings,
            clock("2026-10-16T03:14:59+02:00"),
            None
        ));
        let now = clock("2026-10-16T03:15:00+02:00");
        assert!(nightly_due(&settings, now, None));
        assert!(!helper_check_due(&settings, now, Some(now)).unwrap());
        assert!(
            helper_check_due(&settings, clock("2026-10-17T03:15:00+02:00"), Some(now)).unwrap()
        );
        assert!(!nightly_due(
            &settings,
            clock("2026-10-16T04:15:00+02:00"),
            None
        ));
        assert!(helper_check_due(&settings, clock("2026-10-16T04:15:00+02:00"), None).unwrap());
        assert!(
            !helper_check_due(
                &UpdateSettings {
                    automatic: false,
                    ..settings
                },
                now,
                None
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
        assert!(!nightly_due(&settings, after_midnight, Some(checked)));
        assert!(!desktop_window(&settings, clock("2026-10-17T00:30:00+02:00")).unwrap());
        let repeated_hour = UpdateSettings {
            time: "02:15".into(),
            ..Default::default()
        };
        assert!(!nightly_due(
            &repeated_hour,
            clock("2026-10-25T02:30:00+01:00"),
            Some(clock("2026-10-25T02:15:00+02:00"))
        ));
        let midnight = UpdateSettings::default();
        assert!(
            helper_check_due(
                &midnight,
                clock("2026-10-26T00:00:00+01:00"),
                Some(clock("2026-10-25T00:00:00+02:00"))
            )
            .unwrap()
        );
    }
    #[test]
    fn desktop_launch_catches_up_missed_windows_and_persists_decisions() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = crate::Settings {
            state_dir: temp.path().canonicalize().unwrap(),
            machine: "test".into(),
            ..Default::default()
        };
        let now = clock("2026-10-16T09:00:00+02:00");
        let path = settings.state_dir.join("test/desktop-update.json");
        let mut state = DesktopUpdateState::load(&path).unwrap();
        assert!(state.due(&settings.updates, now, true).unwrap());
        assert!(!state.due(&settings.updates, now, false).unwrap());
        state.record_attempt(&path, &settings.updates, now).unwrap();
        let mut state = DesktopUpdateState::load(&path).unwrap();
        assert!(!state.due(&settings.updates, now, true).unwrap());
        assert!(
            state
                .due(&settings.updates, clock("2026-10-17T00:00:00+02:00"), false)
                .unwrap()
        );
        assert!(
            state
                .due(&settings.updates, clock("2026-10-17T09:00:00+02:00"), true)
                .unwrap()
        );
        state.record_available(&path, true).unwrap();
        let mut state = DesktopUpdateState::load(&path).unwrap();
        assert!(
            state.due(&settings.updates, now, true).unwrap(),
            "deferred installation resumes on launch"
        );
        assert!(!state.due(&settings.updates, now, false).unwrap());
        state.record_available(&path, false).unwrap();
        assert!(
            !DesktopUpdateState::load(&path)
                .unwrap()
                .due(&settings.updates, now, true)
                .unwrap()
        );
        settings.updates.time = "23:30".into();
        assert!(
            state.due(&settings.updates, now, true).unwrap(),
            "changed settings get a new check"
        );
        state
            .record_attempt(&path, &settings.updates, clock("2026-10-15T23:45:00+02:00"))
            .unwrap();
        assert!(
            !state
                .due(&settings.updates, clock("2026-10-16T00:15:00+02:00"), true)
                .unwrap()
        );
        assert!(
            state
                .due(&settings.updates, clock("2026-10-17T09:00:00+02:00"), true)
                .unwrap()
        );
        settings.updates.automatic = false;
        assert!(!state.due(&settings.updates, now, true).unwrap());
        settings.updates.automatic = true;
        settings.updates.time = "invalid".into();
        assert!(state.due(&settings.updates, now, true).is_err());
        std::fs::write(
            settings.state_dir.join("test/desktop-update.json"),
            "corrupt",
        )
        .unwrap();
        assert!(DesktopUpdateState::load(&path).is_err());
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

#[cfg(all(test, windows))]
mod windows_pair_tests {
    use super::*;
    #[test]
    fn either_helper_updates_both_and_failed_self_replacement_restores_both() -> Result<()> {
        for running_name in ["ratatoskr.exe", "ratatoskrw.exe"] {
            let temp = tempfile::tempdir()?;
            let root = temp.path().canonicalize()?;
            let cli = root.join("new-cli.exe");
            let windowless = root.join("new-windowless.exe");
            std::fs::write(&cli, b"new console")?;
            std::fs::write(&windowless, b"new GUI")?;
            let installed_cli = root.join("ratatoskr.exe");
            let installed_windowless = root.join("ratatoskrw.exe");
            std::fs::write(&installed_cli, b"old console")?;
            std::fs::write(&installed_windowless, b"old GUI")?;
            let current = root.join(running_name);
            let error = install_windows_pair_at(&current, &cli, &windowless, |_| {
                std::fs::remove_file(&current)?;
                anyhow::bail!("simulated self-replace failure")
            });
            assert!(format!("{:#}", error.unwrap_err()).contains("simulated self-replace failure"));
            assert_eq!(std::fs::read(&installed_cli)?, b"old console");
            assert_eq!(std::fs::read(&installed_windowless)?, b"old GUI");
            install_windows_pair_at(&current, &cli, &windowless, |staged| {
                crate::config::atomic_write(&current, &std::fs::read(staged)?)
            })?;
            assert_eq!(std::fs::read(&installed_cli)?, b"new console");
            assert_eq!(std::fs::read(&installed_windowless)?, b"new GUI");
        }
        Ok(())
    }
}
