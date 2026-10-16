use anyhow::{Context, Result, ensure};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    time::Duration,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateSettings {
    pub endpoint: String,
    pub public_key: String,
    #[serde(default)]
    pub automatic: bool,
}

impl UpdateSettings {
    pub fn tauri_public_key(&self) -> Result<String> {
        self.validate()?;
        if minisign_verify::PublicKey::from_base64(self.public_key.trim()).is_ok() {
            Ok(base64::engine::general_purpose::STANDARD.encode(format!(
                "untrusted comment: Ratatoskr update public key\n{}\n",
                self.public_key.trim()
            )))
        } else {
            Ok(self.public_key.trim().into())
        }
    }

    pub fn validate(&self) -> Result<()> {
        let endpoint = reqwest::Url::parse(&self.endpoint)?;
        ensure!(
            endpoint.scheme() == "https"
                && endpoint.username().is_empty()
                && endpoint.password().is_none(),
            "Update endpoint must use HTTPS without embedded credentials"
        );
        key(&self.public_key)?;
        Ok(())
    }
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
    let response = client()?.get(&settings.endpoint).send()?;
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
    verify(&settings.public_key, &bytes, &asset.signature)?;
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
