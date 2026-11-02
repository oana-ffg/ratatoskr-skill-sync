//! A detached supervisor owns the installer after the desktop process exits.
//! PowerShell ships with supported Windows versions and does not lock either
//! executable being replaced. Keep all transaction artifacts for diagnosis.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::Path,
    process::Stdio,
    time::{Duration, Instant},
};

const SCRIPT: &str = include_str!("windows_update.ps1");
const RESULT: &str = "windows-install-result.json";

#[derive(Serialize)]
struct Plan<'a> {
    parent: u32,
    desktop: &'a Path,
    data: &'a Path,
    version: &'a str,
    sha256: String,
}

#[derive(Deserialize)]
struct Outcome {
    error: Option<String>,
}

/// An install failure survives successful manifest checks and ordinary syncs.
/// Only a later supervised installation replaces this outcome with success.
pub fn failure(data: &Path) -> Result<Option<String>> {
    let path = data.join(RESULT);
    crate::config::no_symlinks(&path)?;
    if !path.try_exists()? {
        return Ok(None);
    }
    Ok(serde_json::from_slice::<Outcome>(&fs::read(path)?)?.error)
}

/// Bytes must have passed the signed updater's download verification first.
/// Returns only once the supervisor has opened the parent's process handle.
/// The caller must then exit, rather than launch another desktop instance.
pub fn supervise(desktop: &Path, data: &Path, version: &str, bytes: &[u8]) -> Result<()> {
    let directory = tempfile::Builder::new()
        .prefix("windows-install-")
        .tempdir_in(data)?;
    let root = directory.path();
    fs::create_dir(root.join("backup"))?;
    for name in ["ratatoskr-desktop.exe", "ratatoskr.exe"] {
        fs::copy(desktop.with_file_name(name), root.join("backup").join(name))
            .with_context(|| format!("Back up {name} before update"))?;
    }
    fs::write(root.join("installer.exe"), bytes)?;
    fs::write(root.join("supervise.ps1"), SCRIPT)?;
    fs::write(
        root.join("plan.json"),
        serde_json::to_vec(&Plan {
            parent: std::process::id(),
            desktop,
            data,
            version,
            sha256: format!("{:x}", Sha256::digest(bytes)),
        })?,
    )?;
    // A failed install must not immediately loop on launch-time pending state.
    let state = data.join("desktop-update.json");
    crate::updates::DesktopUpdateState::load(&state)?.record_available(&state, false)?;
    let log = fs::File::create(root.join("supervisor.log"))?;
    // Retain logs and backups even if starting the supervisor fails.
    let root = directory.keep();
    let mut child = crate::process::command("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(root.join("supervise.ps1"))
        .arg(root.join("plan.json"))
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log)
        .spawn()
        .context("Start Windows update supervisor")?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if root.join("ready").try_exists()? {
            return Ok(());
        }
        if let Some(status) = child.try_wait()? {
            bail!(
                "Windows update supervisor exited ({status}); see {}",
                root.display()
            );
        }
        if Instant::now() >= deadline {
            child
                .kill()
                .context("Stop unready Windows update supervisor")?;
            child.wait()?;
            bail!(
                "Windows update supervisor did not become ready; see {}",
                root.display()
            );
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_install_is_not_a_successful_update_check() {
        let temp = tempfile::tempdir().unwrap();
        let data = dunce::canonicalize(temp.path()).unwrap();
        assert!(failure(&data).unwrap().is_none());
        fs::write(data.join(RESULT), br#"{"error":"Installer timed out"}"#).unwrap();
        assert_eq!(
            failure(&data).unwrap().as_deref(),
            Some("Installer timed out")
        );
        fs::write(data.join(RESULT), br#"{"error":null}"#).unwrap();
        assert!(failure(&data).unwrap().is_none());
        fs::write(data.join(RESULT), b"broken").unwrap();
        assert!(failure(&data).is_err());
    }
}
