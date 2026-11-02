//! A detached supervisor owns the installer after the desktop process exits.
//! PowerShell ships with supported Windows versions and does not lock either
//! executable being replaced. Each transaction keeps its artifacts for diagnosis,
//! and only the most recent ones survive, because each holds a full set of binaries.
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
const TRANSACTION_PREFIX: &str = "windows-install-";
/// Counted before a new transaction starts, which brings the total to three.
const RETAINED_PRIOR_TRANSACTIONS: usize = 2;

#[derive(Serialize)]
struct Plan<'a> {
    parent: u32,
    desktop: &'a Path,
    data: &'a Path,
    app_id: &'a str,
    version: &'a str,
    sha256: String,
}

#[derive(Deserialize)]
struct Outcome {
    version: String,
    error: Option<String>,
}

/// An install failure survives successful manifest checks and ordinary syncs. It
/// ends once this app runs the failed version or newer, whether a later supervised
/// installation or the documented manual setup installed it.
pub fn failure(data: &Path) -> Result<Option<String>> {
    failure_while_running(data, env!("CARGO_PKG_VERSION"))
}

fn failure_while_running(data: &Path, running: &str) -> Result<Option<String>> {
    let path = data.join(RESULT);
    crate::config::no_symlinks(&path)?;
    if !path.try_exists()? {
        return Ok(None);
    }
    let outcome: Outcome = serde_json::from_slice(&fs::read(path)?)?;
    let resolved = semver::Version::parse(running)? >= semver::Version::parse(&outcome.version)?;
    Ok(if resolved { None } else { outcome.error })
}

/// Oldest first removal, so a persistent failure keeps its latest evidence. A folder
/// that cannot be removed stops the update visibly instead of growing without limit.
fn prune_transactions(data: &Path) -> Result<()> {
    let mut transactions = Vec::new();
    for entry in fs::read_dir(data)? {
        let entry = entry?;
        let is_transaction = entry.file_type()?.is_dir()
            && entry
                .file_name()
                .to_string_lossy()
                .starts_with(TRANSACTION_PREFIX);
        if is_transaction {
            transactions.push((entry.metadata()?.modified()?, entry.path()));
        }
    }
    transactions.sort();
    let excess = transactions
        .len()
        .saturating_sub(RETAINED_PRIOR_TRANSACTIONS);
    for (_, path) in transactions.into_iter().take(excess) {
        fs::remove_dir_all(&path)
            .with_context(|| format!("Remove old update evidence {}", path.display()))?;
    }
    Ok(())
}

/// Bytes must have passed the signed updater's download verification first.
/// Returns only once the supervisor has opened the parent's process handle.
/// The caller must then exit, rather than launch another desktop instance.
pub fn supervise(
    desktop: &Path,
    data: &Path,
    app_id: &str,
    version: &str,
    bytes: &[u8],
) -> Result<()> {
    prune_transactions(data)?;
    let directory = tempfile::Builder::new()
        .prefix(TRANSACTION_PREFIX)
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
            app_id,
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
        let running = |version| failure_while_running(&data, version);
        assert!(running("0.2.20").unwrap().is_none());
        fs::write(
            data.join(RESULT),
            br#"{"version":"0.2.21","error":"Installer timed out"}"#,
        )
        .unwrap();
        assert_eq!(
            running("0.2.20").unwrap().as_deref(),
            Some("Installer timed out")
        );
        fs::write(data.join(RESULT), br#"{"version":"0.2.21","error":null}"#).unwrap();
        assert!(running("0.2.20").unwrap().is_none());
        fs::write(data.join(RESULT), b"broken").unwrap();
        assert!(running("0.2.20").is_err());
    }

    #[test]
    fn manually_installing_the_failed_version_or_newer_ends_the_failure() {
        let temp = tempfile::tempdir().unwrap();
        let data = dunce::canonicalize(temp.path()).unwrap();
        fs::write(
            data.join(RESULT),
            br#"{"version":"0.2.21","error":"Installer timed out"}"#,
        )
        .unwrap();
        for version in ["0.2.21", "0.2.22", "0.3.0"] {
            assert!(
                failure_while_running(&data, version).unwrap().is_none(),
                "{version}"
            );
        }
    }

    #[test]
    fn only_the_newest_transactions_survive_and_nothing_else_is_touched() {
        let temp = tempfile::tempdir().unwrap();
        let data = dunce::canonicalize(temp.path()).unwrap();
        for name in ["oldest", "older", "newer", "newest"] {
            let transaction = data.join(format!("{TRANSACTION_PREFIX}{name}"));
            fs::create_dir(&transaction).unwrap();
            fs::write(transaction.join("installer.exe"), b"bytes").unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
        fs::write(data.join(RESULT), br#"{"version":"0.2.21","error":null}"#).unwrap();
        fs::create_dir(data.join("unrelated")).unwrap();

        prune_transactions(&data).unwrap();

        let mut remaining: Vec<_> = fs::read_dir(&data)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        remaining.sort();
        assert_eq!(
            remaining,
            [
                "unrelated",
                "windows-install-newer",
                "windows-install-newest",
                RESULT,
            ]
        );
    }
}
