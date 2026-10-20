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
    #[serde(default)]
    last_success: Option<chrono::DateTime<chrono::FixedOffset>>,
    #[serde(default)]
    monitoring_since: Option<chrono::DateTime<chrono::FixedOffset>>,
    #[serde(default)]
    catch_up_started: Option<chrono::DateTime<chrono::FixedOffset>>,
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
        Ok(missed || (launch && same_settings && self.pending))
    }

    pub fn record_attempt(
        &mut self,
        path: &std::path::Path,
        settings: &UpdateSettings,
        now: chrono::DateTime<chrono::FixedOffset>,
    ) -> Result<()> {
        if self.settings.as_ref() != Some(settings) {
            self.last_success = None;
            self.monitoring_since = Some(now);
        }
        self.catch_up_started = (!desktop_window(settings, now)?).then_some(now);
        self.last_check = Some(now);
        self.settings = Some(settings.clone());
        self.record_available(path, false)
    }

    /// Normal nightly installation stays within the configured hour. A missed
    /// window gets one persisted hour from its catch-up attempt, never a sliding retry.
    pub fn installation_allowed(
        &self,
        settings: &UpdateSettings,
        now: chrono::DateTime<chrono::FixedOffset>,
        launch: bool,
    ) -> Result<bool> {
        if !settings.automatic || self.settings.as_ref() != Some(settings) {
            return Ok(false);
        }
        Ok(launch
            || desktop_window(settings, now)?
            || self.catch_up_started.is_some_and(|started| {
                let elapsed = now.signed_duration_since(started);
                elapsed >= chrono::Duration::zero()
                    && elapsed < chrono::Duration::minutes(DESKTOP_WINDOW_MINUTES)
            }))
    }

    /// Old attempts are not proof of success. macOS preserves health across
    /// restarts for the helper watchdog; other platforms track resident sessions.
    pub fn observe(
        &mut self,
        path: &std::path::Path,
        settings: &UpdateSettings,
        now: chrono::DateTime<chrono::FixedOffset>,
        new_session: bool,
    ) -> Result<()> {
        let changed = self.settings.as_ref() != Some(settings);
        if changed {
            self.last_check = None;
            self.last_success = None;
            self.pending = false;
            self.catch_up_started = None;
        }
        if self.monitoring_since.is_none() || changed || (new_session && !cfg!(target_os = "macos"))
        {
            self.monitoring_since = Some(now);
            self.settings = Some(settings.clone());
            crate::config::atomic_write(path, &serde_json::to_vec(self)?)?;
        }
        Ok(())
    }

    /// Two completed nightly windows without a successful check are unhealthy.
    /// Keep the message stable so the existing 24-hour alert dedup policy applies.
    pub fn overdue(
        &self,
        settings: &UpdateSettings,
        now: chrono::DateTime<chrono::FixedOffset>,
    ) -> Result<bool> {
        if !settings.automatic || self.settings.as_ref() != Some(settings) {
            return Ok(false);
        }
        let Some(since) = self.last_success.max(self.monitoring_since) else {
            return Ok(false);
        };
        let slot = desktop_slot(settings, now)?;
        let completed =
            if now.naive_local() - slot < chrono::Duration::minutes(DESKTOP_WINDOW_MINUTES) {
                slot - chrono::Duration::days(1)
            } else {
                slot
            };
        // Stored local calendar slots survive DST offset changes without turning
        // yesterday's midnight check into a check on the day before yesterday.
        Ok(completed - desktop_slot(settings, since)? >= chrono::Duration::days(2))
    }

    pub fn record_success(
        &mut self,
        path: &std::path::Path,
        now: chrono::DateTime<chrono::FixedOffset>,
        available: bool,
    ) -> Result<()> {
        self.last_success = Some(now);
        self.record_available(path, available)
    }

    pub fn record_available(&mut self, path: &std::path::Path, available: bool) -> Result<()> {
        self.pending = available;
        crate::config::atomic_write(path, &serde_json::to_vec(self)?)
    }
}

pub const DESKTOP_OVERDUE: &str =
    "Desktop update overdue: two nightly windows passed without a successful check";
#[cfg(any(target_os = "macos", test))]
const MONITORING_FAILED: &str = "Desktop update monitoring failed:";

#[cfg(target_os = "macos")]
pub(crate) fn desktop_watchdog(settings: &crate::Settings) -> Result<()> {
    let executable = std::env::current_exe()?;
    if !executable
        .file_stem()
        .is_some_and(|name| name == "ratatoskr")
    {
        // In-process desktop/library calls do not own the helper watchdog.
        return Ok(());
    }
    // Match Tauri's app_data_dir: data_dir / bundle identifier. No installed
    // application, launch agent, or desktop-owned state is modified here.
    let data = directories::BaseDirs::new()
        .context("Find desktop app-data directory")?
        .data_dir()
        .join("io.github.oana-ffg.ratatoskr-skill-sync");
    let state = data.join("desktop-update.json");
    let grace = settings
        .state_dir
        .join(&settings.machine)
        .join("desktop-update-watchdog.json");
    if !bundled_helper_path(&executable) && !state.try_exists()? && !grace.try_exists()? {
        // A CLI-only installation has no desktop scheduler to monitor.
        return Ok(());
    }
    desktop_watchdog_with(settings, &state, chrono::Local::now().fixed_offset())
}

/// The helper owns only its grace ledger. Never write the desktop's scheduler
/// ledger: that could race a successful check or erase pending installation.
#[cfg(any(target_os = "macos", test))]
fn desktop_watchdog_with(
    settings: &crate::Settings,
    state_path: &std::path::Path,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> Result<()> {
    let mut result = Ok(false);
    crate::notifications::update_for(settings, "desktop-update-notification.json", |current| {
        // Re-read health while holding the delivery lock. A waiting helper must
        // not overwrite a desktop recovery using an earlier overdue snapshot.
        result = (|| -> Result<bool> {
            let grace = settings
                .state_dir
                .join(&settings.machine)
                .join("desktop-update-watchdog.json");
            let mut baseline = DesktopUpdateState::load(&grace)?;
            baseline.observe(&grace, &settings.updates, now, false)?;
            if !settings.updates.automatic {
                return Ok(false);
            }
            let mut state = DesktopUpdateState::load(state_path)?;
            // Old files contain attempts only. Start one persistent grace period,
            // rather than treating last_check as success or resetting every minute.
            if state.settings.as_ref() != Some(&settings.updates) {
                state = baseline;
            } else if state.monitoring_since.is_none() && state.last_success.is_none() {
                state.monitoring_since = baseline.monitoring_since;
            }
            state.overdue(&settings.updates, now)
        })();
        if let Err(error) = &result {
            return Some(format!("{MONITORING_FAILED} {error:#}"));
        }
        let specific = current.filter(|error| {
            !error.starts_with(DESKTOP_OVERDUE) && !error.starts_with(MONITORING_FAILED)
        });
        if let Some(error) = specific {
            // Retry undelivered causes, without replacing them with a generic warning.
            Some(error.to_owned())
        } else if matches!(result, Ok(true)) {
            Some(DESKTOP_OVERDUE.to_owned())
        } else {
            None
        }
    })?;
    result.map(|_| ())
}

#[derive(Default, Deserialize, Serialize)]
struct HelperState {
    last_check: Option<chrono::DateTime<chrono::FixedOffset>>,
}

pub fn helper_update(settings: &crate::Settings) -> Result<()> {
    let standalone = standalone_helper()?;
    helper_update_for(
        settings,
        standalone,
        chrono::Local::now().fixed_offset(),
        || {
            if let Some(release) = check(&settings.updates, env!("CARGO_PKG_VERSION"))? {
                install(&settings.updates, &release)?;
            }
            Ok(())
        },
    )
}

fn helper_update_for(
    settings: &crate::Settings,
    standalone: bool,
    now: chrono::DateTime<chrono::FixedOffset>,
    check: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if standalone {
        return helper_update_with(settings, now, check);
    }
    // This legacy error described ownership, not a broken helper. Bundled helpers
    // are replaced by the desktop updater and never attempt self-replacement.
    crate::notifications::update_for(settings, "helper-update-notification.json", |error| {
        error
            .filter(|error| !error.starts_with(BUNDLED_HELPER_ERROR))
            .map(str::to_owned)
    })
}

const BUNDLED_HELPER_ERROR: &str =
    "Helper update: Update the desktop app to replace its bundled helper";

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
    } else {
        crate::notifications::retry_for(settings, "helper-update-notification.json")?;
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

fn bundled_helper_path(path: &std::path::Path) -> bool {
    path.ancestors().any(|parent| {
        parent
            .extension()
            .is_some_and(|extension| extension == "app")
    })
}

pub fn standalone_helper() -> Result<bool> {
    let path = std::env::current_exe()?;
    Ok(path.file_stem().is_some_and(|x| x == "ratatoskr")
        && !bundled_helper_path(&path)
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
            ..Default::default()
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
        assert!(nightly_due(
            &settings,
            clock("2026-10-16T03:14:59+02:00"),
            None
        ));
        let now = clock("2026-10-16T03:15:00+02:00");
        assert!(!desktop_window(&settings, clock("2026-10-16T03:14:59+02:00")).unwrap());
        assert!(desktop_window(&settings, now).unwrap());
        assert!(!desktop_window(&settings, clock("2026-10-16T04:15:00+02:00")).unwrap());
        assert!(nightly_due(&settings, now, None));
        assert!(!helper_check_due(&settings, now, Some(now)).unwrap());
        assert!(
            helper_check_due(&settings, clock("2026-10-17T03:15:00+02:00"), Some(now)).unwrap()
        );
        assert!(nightly_due(
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
        assert!(state.due(&settings.updates, now, false).unwrap());
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
    fn resident_checks_catch_up_after_sleep_and_days_of_uptime() {
        let settings = UpdateSettings::default();
        let temp = tempfile::tempdir().unwrap();
        let path = temp
            .path()
            .canonicalize()
            .unwrap()
            .join("desktop-update.json");
        let mut state = DesktopUpdateState::default();
        let checked = clock("2026-10-18T00:01:00+02:00");
        state.record_attempt(&path, &settings, checked).unwrap();
        state.record_success(&path, checked, false).unwrap();
        for resume in ["2026-10-19T13:38:00+02:00", "2026-10-25T13:38:00+01:00"] {
            let mut resumed = DesktopUpdateState::load(&path).unwrap();
            let now = clock(resume);
            assert!(!desktop_window(&settings, now).unwrap());
            assert!(resumed.due(&settings, now, false).unwrap());
            resumed.record_attempt(&path, &settings, now).unwrap();
            assert!(resumed.installation_allowed(&settings, now, false).unwrap());
            assert!(!resumed.due(&settings, now, false).unwrap());
            let reopened = DesktopUpdateState::load(&path).unwrap();
            assert!(
                reopened
                    .installation_allowed(&settings, now + chrono::Duration::minutes(59), false)
                    .unwrap()
            );
            assert!(
                !reopened
                    .installation_allowed(&settings, now + chrono::Duration::hours(1), false)
                    .unwrap()
            );
        }
        let normal = clock("2026-10-26T00:01:00+01:00");
        state.record_attempt(&path, &settings, normal).unwrap();
        assert!(
            state
                .installation_allowed(&settings, clock("2026-10-26T00:59:59+01:00"), false)
                .unwrap()
        );
        assert!(
            !state
                .installation_allowed(&settings, clock("2026-10-26T01:00:00+01:00"), false)
                .unwrap()
        );
    }

    #[test]
    fn overdue_tracks_success_not_attempts_and_migrates_legacy_state_with_grace() {
        let settings = UpdateSettings::default();
        let temp = tempfile::tempdir().unwrap();
        let path = temp
            .path()
            .canonicalize()
            .unwrap()
            .join("desktop-update.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&serde_json::json!({
                "last_check": "2026-10-18T00:01:00+02:00", "settings": settings, "pending": false
            }))
            .unwrap(),
        )
        .unwrap();
        let mut state = DesktopUpdateState::load(&path).unwrap();
        let started = clock("2026-10-19T13:38:00+02:00");
        state.observe(&path, &settings, started, false).unwrap();
        assert!(!state.overdue(&settings, started).unwrap());
        for value in ["2026-10-20T13:38:00+02:00", "2026-10-21T00:59:59+02:00"] {
            assert!(!state.overdue(&settings, clock(value)).unwrap());
        }
        let overdue = clock("2026-10-21T01:00:00+02:00");
        state.record_attempt(&path, &settings, overdue).unwrap();
        assert!(
            DesktopUpdateState::load(&path)
                .unwrap()
                .overdue(&settings, overdue)
                .unwrap()
        );
        assert!(
            state
                .overdue(&settings, clock("2026-10-22T00:30:00+02:00"))
                .unwrap()
        );
        state.record_success(&path, overdue, false).unwrap();
        assert!(!state.overdue(&settings, overdue).unwrap());
        state
            .record_success(&path, clock("2026-10-24T00:01:00+02:00"), false)
            .unwrap();
        assert!(
            !state
                .overdue(&settings, clock("2026-10-25T13:38:00+01:00"))
                .unwrap()
        );
        assert!(
            state
                .overdue(&settings, clock("2026-10-26T13:38:00+01:00"))
                .unwrap()
        );
        let reopened = clock("2026-10-28T13:38:00+01:00");
        state.observe(&path, &settings, reopened, true).unwrap();
        assert_eq!(
            state.overdue(&settings, reopened).unwrap(),
            cfg!(target_os = "macos"),
            "macOS must preserve health while closed; other platforms retain resident-session accounting"
        );
        assert!(
            state
                .overdue(&settings, clock("2026-10-30T01:00:00+01:00"))
                .unwrap()
        );
        let disabled = UpdateSettings {
            automatic: false,
            ..settings.clone()
        };
        assert!(
            !state
                .overdue(&disabled, clock("2026-10-25T13:38:00+01:00"))
                .unwrap()
        );
        let changed = UpdateSettings {
            time: "23:30".into(),
            ..settings
        };
        state.observe(&path, &changed, overdue, false).unwrap();
        assert!(state.due(&changed, overdue, false).unwrap());
        assert!(!state.overdue(&changed, overdue).unwrap());
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

    fn alert_settings(temp: &tempfile::TempDir) -> crate::Settings {
        let root = dunce::canonicalize(temp.path()).unwrap();
        let settings = crate::Settings {
            source: root.join("source"),
            state_dir: root.join("state"),
            machine: "test".into(),
            ..Default::default()
        };
        std::fs::create_dir(&settings.source).unwrap();
        std::fs::create_dir(&settings.state_dir).unwrap();
        settings
    }

    fn enable_recorder(settings: &mut crate::Settings) -> std::path::PathBuf {
        let path = settings.state_dir.join("alerts.gitconfig");
        settings.alert_command = vec![
            "git".into(),
            "config".into(),
            "--file".into(),
            path.to_string_lossy().into_owned(),
            "--add".into(),
            "alerts.message".into(),
        ];
        path
    }
    fn alert_count(path: &std::path::Path) -> usize {
        if !path.exists() {
            return 0;
        }
        let output = crate::process::command("git")
            .args(["config", "--file"])
            .arg(path)
            .args(["--get-all", "alerts.message"])
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8(output.stdout).unwrap().lines().count()
    }

    #[test]
    fn skipped_helper_check_delivers_previously_unsent_error_and_recovers_once() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = alert_settings(&temp);
        let now = clock("2026-10-20T00:01:00+02:00");
        assert!(helper_update_with(&settings, now, || anyhow::bail!("offline")).is_err());
        let ledger = settings
            .state_dir
            .join("test/helper-update-notification.json");
        let unsent: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&ledger).unwrap()).unwrap();
        assert_eq!(unsent["sent_at"], 0);
        let alerts = enable_recorder(&mut settings);
        for _ in 0..2 {
            helper_update_with(&settings, now, || panic!("daily attempt already consumed"))
                .unwrap();
        }
        assert_eq!(alert_count(&alerts), 1);
        helper_update_with(&settings, clock("2026-10-21T00:01:00+02:00"), || Ok(())).unwrap();
        helper_update_with(
            &settings,
            clock("2026-10-21T00:02:00+02:00"),
            || unreachable!(),
        )
        .unwrap();
        assert_eq!(alert_count(&alerts), 2);
        assert!(
            crate::job_status(&settings)
                .unwrap()
                .helper_update_error
                .is_none()
        );
    }

    #[test]
    fn bundled_helpers_clear_obsolete_errors_without_attempting_self_update() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = alert_settings(&temp);
        let alerts = enable_recorder(&mut settings);
        report_outcome(&settings, Some(BUNDLED_HELPER_ERROR), UpdateKind::Helper).unwrap();
        assert_eq!(alert_count(&alerts), 1);
        for _ in 0..2 {
            helper_update_for(&settings, false, clock("2026-10-20T00:01:00+02:00"), || {
                panic!("bundled helpers must not check or install helper updates")
            })
            .unwrap();
        }
        assert!(
            crate::job_status(&settings)
                .unwrap()
                .helper_update_error
                .is_none()
        );
        assert_eq!(alert_count(&alerts), 2);
        report_outcome(
            &settings,
            Some("Helper update: unknown failure"),
            UpdateKind::Helper,
        )
        .unwrap();
        helper_update_for(
            &settings,
            false,
            clock("2026-10-20T00:01:00+02:00"),
            || unreachable!(),
        )
        .unwrap();
        assert_eq!(
            crate::job_status(&settings)
                .unwrap()
                .helper_update_error
                .as_deref(),
            Some("Helper update: unknown failure")
        );
        // An obsolete error never delivered does not generate a false recovery.
        let mut quiet = settings.clone();
        quiet.machine = "quiet".into();
        quiet.alert_command.clear();
        report_outcome(&quiet, Some(BUNDLED_HELPER_ERROR), UpdateKind::Helper).unwrap();
        quiet.alert_command = settings.alert_command.clone();
        helper_update_for(
            &quiet,
            false,
            clock("2026-10-20T00:01:00+02:00"),
            || unreachable!(),
        )
        .unwrap();
        assert_eq!(alert_count(&alerts), 3);
    }

    #[test]
    fn helper_watchdog_observes_stopped_desktop_deduplicates_and_recovers() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = alert_settings(&temp);
        let alerts = enable_recorder(&mut settings);
        let path = settings.state_dir.join("app/desktop-update.json");
        let first = clock("2026-10-18T00:01:00+02:00");
        let mut state = DesktopUpdateState::default();
        state
            .observe(&path, &settings.updates, first, false)
            .unwrap();
        state.record_success(&path, first, false).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        desktop_watchdog_with(&settings, &path, clock("2026-10-20T00:59:59+02:00")).unwrap();
        assert_eq!(alert_count(&alerts), 0);
        let overdue = clock("2026-10-20T01:00:00+02:00");
        for _ in 0..2 {
            desktop_watchdog_with(&settings, &path, overdue).unwrap();
        }
        assert_eq!(alert_count(&alerts), 1);
        assert_eq!(
            std::fs::read(&path).unwrap(),
            bytes,
            "helper must not modify desktop-owned state"
        );
        assert_eq!(
            crate::job_status(&settings)
                .unwrap()
                .desktop_update_error
                .as_deref(),
            Some(DESKTOP_OVERDUE)
        );
        // Existing desktop reporter and helper share delivery history.
        report_outcome(&settings, Some(DESKTOP_OVERDUE), UpdateKind::Desktop).unwrap();
        assert_eq!(alert_count(&alerts), 1);
        // Simulate a delivered alert older than 24 hours.
        let ledger = settings
            .state_dir
            .join("test/desktop-update-notification.json");
        let mut notification: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&ledger).unwrap()).unwrap();
        notification["sent_at"] = 1.into();
        std::fs::write(&ledger, serde_json::to_vec(&notification).unwrap()).unwrap();
        desktop_watchdog_with(&settings, &path, overdue).unwrap();
        assert_eq!(alert_count(&alerts), 2);
        state.record_success(&path, overdue, false).unwrap();
        for _ in 0..2 {
            desktop_watchdog_with(&settings, &path, overdue).unwrap();
        }
        assert_eq!(alert_count(&alerts), 3);
        assert!(
            crate::job_status(&settings)
                .unwrap()
                .desktop_update_error
                .is_none()
        );
    }

    #[test]
    fn helper_watchdog_missing_and_legacy_state_have_bounded_grace() {
        for legacy in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let settings = alert_settings(&temp);
            let path = settings.state_dir.join("desktop-update.json");
            if legacy {
                std::fs::write(&path, serde_json::to_vec(&serde_json::json!({
                    "last_check": "2026-10-01T00:01:00+02:00", "settings": settings.updates, "pending": false
                })).unwrap()).unwrap();
            }
            let first = clock("2026-10-20T12:00:00+02:00");
            desktop_watchdog_with(&settings, &path, first).unwrap();
            desktop_watchdog_with(&settings, &path, clock("2026-10-22T00:59:59+02:00")).unwrap();
            assert!(
                crate::job_status(&settings)
                    .unwrap()
                    .desktop_update_error
                    .is_none()
            );
            desktop_watchdog_with(&settings, &path, clock("2026-10-22T01:00:00+02:00")).unwrap();
            assert_eq!(
                crate::job_status(&settings)
                    .unwrap()
                    .desktop_update_error
                    .as_deref(),
                Some(DESKTOP_OVERDUE)
            );
        }
    }

    #[test]
    fn helper_watchdog_reports_unverifiable_state_retries_specific_errors_and_respects_opt_out() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = alert_settings(&temp);
        let path = settings.state_dir.join("desktop-update.json");
        let now = clock("2026-10-20T12:00:00+02:00");
        std::fs::write(&path, "broken").unwrap();
        assert!(desktop_watchdog_with(&settings, &path, now).is_err());
        assert!(
            crate::job_status(&settings)
                .unwrap()
                .desktop_update_error
                .unwrap()
                .starts_with(MONITORING_FAILED)
        );
        std::fs::remove_file(&path).unwrap();
        desktop_watchdog_with(&settings, &path, now).unwrap();
        report_outcome(
            &settings,
            Some("Desktop update: bad signature"),
            UpdateKind::Desktop,
        )
        .unwrap();
        let alerts = enable_recorder(&mut settings);
        for _ in 0..2 {
            desktop_watchdog_with(&settings, &path, clock("2026-10-23T12:00:00+02:00")).unwrap();
        }
        assert_eq!(alert_count(&alerts), 1);
        assert_eq!(
            crate::job_status(&settings)
                .unwrap()
                .desktop_update_error
                .as_deref(),
            Some("Desktop update: bad signature")
        );
        report_outcome(&settings, None, UpdateKind::Desktop).unwrap();
        settings.updates.automatic = false;
        std::fs::write(&path, "broken but automatic updates are opted out").unwrap();
        desktop_watchdog_with(&settings, &path, clock("2026-11-01T12:00:00+01:00")).unwrap();
        assert!(
            crate::job_status(&settings)
                .unwrap()
                .desktop_update_error
                .is_none()
        );
    }
}
