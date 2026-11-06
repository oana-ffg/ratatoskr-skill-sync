mod desktop;
use anyhow::{Context, Result, ensure};
use base64::Engine;
pub use desktop::{
    DesktopInstance, DesktopRequestOutcome, DesktopUpdateRequest, DesktopUpdateResponse,
    QueuedDesktopUpdate, UpdateGuard, answer_desktop_request, claim_desktop_instance,
    desktop_request_status, finish_desktop_request, idle_guard, pending_desktop_request,
    queue_desktop_update, request_desktop_update, running_desktop, start_desktop_hidden,
};
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

/// Storage identity stays stable when the macOS publisher/bundle identity changes.
/// The desktop and helper must read the same ledger, including during an update.
pub fn desktop_data_dir() -> Result<std::path::PathBuf> {
    Ok(directories::BaseDirs::new()
        .context("Find desktop app-data directory")?
        .data_dir()
        .join("io.github.oana-ffg.ratatoskr-skill-sync"))
}
#[cfg(any(target_os = "macos", test))]
const MONITORING_FAILED: &str = "Desktop update monitoring failed:";

#[cfg(target_os = "macos")]
pub(crate) fn desktop_watchdog(settings: &crate::Settings) -> Result<()> {
    let executable = std::env::current_exe()?;
    if executable
        .file_stem()
        .is_none_or(|name| name != "ratatoskr")
    {
        // In-process desktop/library calls do not own the helper watchdog.
        return Ok(());
    }
    // No installed application, launch agent, or desktop-owned state is modified.
    let data = desktop_data_dir()?;
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

#[cfg(test)]
mod tests;
