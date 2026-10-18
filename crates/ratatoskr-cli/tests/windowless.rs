#![cfg(all(windows, feature = "windows-helper"))]

use anyhow::{Result, ensure};
use std::{fs, path::Path, process::Command};

fn subsystem(path: &Path) -> Result<u16> {
    let bytes = fs::read(path)?;
    ensure!(&bytes[..2] == b"MZ");
    let offset = u32::from_le_bytes(bytes[0x3c..0x40].try_into()?) as usize;
    ensure!(&bytes[offset..offset + 4] == b"PE\0\0");
    // Subsystem is at offset 68 in both PE32 and PE32+ optional headers.
    Ok(u16::from_le_bytes(
        bytes[offset + 24 + 68..offset + 24 + 70].try_into()?,
    ))
}

#[test]
fn shipped_helpers_have_gui_and_console_pe_subsystems() -> Result<()> {
    ensure!(subsystem(Path::new(env!("CARGO_BIN_EXE_ratatoskrw")))? == 2);
    ensure!(subsystem(Path::new(env!("CARGO_BIN_EXE_ratatoskr")))? == 3);
    let version = Command::new(env!("CARGO_BIN_EXE_ratatoskrw"))
        .arg("--version")
        .output()?;
    ensure!(version.status.success());
    ensure!(
        String::from_utf8(version.stdout)?.trim()
            == concat!("ratatoskrw ", env!("CARGO_PKG_VERSION"))
    );
    Ok(())
}

#[test]
fn windowless_bad_config_is_a_recorded_failure_without_a_console() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    let config = root.join("config.toml");
    let state = root.join("state");
    // Parseable but invalid: missing source. This used to fail before any ledger.
    fs::write(
        &config,
        format!(
            "source = {}\nmachine = 'test'\nstate_dir = {}\n",
            serde_json::to_string(&root.join("missing-source"))?,
            serde_json::to_string(&state)?
        ),
    )?;
    let output = Command::new(env!("CARGO_BIN_EXE_ratatoskrw"))
        .arg("--config")
        .arg(&config)
        .arg("sync")
        .output()?;
    ensure!(output.status.code() == Some(2));
    let run: serde_json::Value =
        serde_json::from_slice(&fs::read(state.join("test/last-sync.json"))?)?;
    ensure!(run["finished_at"].is_number());
    ensure!(
        run["error"]
            .as_str()
            .unwrap()
            .contains("Source checkout does not exist")
    );
    ensure!(
        fs::read_to_string(state.join("test/sync.stderr.log"))?
            .contains("Source checkout does not exist")
    );
    // Even an unparseable config has a durable bootstrap ledger/log, beside it.
    fs::write(&config, "not valid TOML [")?;
    let output = Command::new(env!("CARGO_BIN_EXE_ratatoskrw"))
        .arg("--config")
        .arg(&config)
        .arg("sync")
        .output()?;
    ensure!(output.status.code() == Some(2));
    let startup = root.join("config.toml.diagnostics/startup");
    let run: serde_json::Value =
        serde_json::from_slice(&fs::read(startup.join("last-sync.json"))?)?;
    ensure!(run["finished_at"].is_number() && run["error"].is_string());
    ensure!(!fs::read_to_string(startup.join("sync.stderr.log"))?.is_empty());
    Ok(())
}

fn git(root: &Path, arguments: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(arguments)
        .output()?;
    ensure!(
        output.status.success(),
        "git {arguments:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

#[test]
fn windowless_success_and_sync_errors_reach_job_status_and_logs() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    let source = root.join("content");
    fs::create_dir_all(source.join("machines"))?;
    fs::write(
        source.join("harnesses.toml"),
        "[codex]\nskills_dir='skills'\nglobal_file='AGENTS.md'\n[codex.roots]\nwindows='{{af:home}}/.codex'\n",
    )?;
    fs::write(
        source.join("machines/test.toml"),
        "os='windows'\nmode='staging'\nharnesses=['codex']\ncapabilities=[]\n",
    )?;
    fs::create_dir_all(source.join("skills/check"))?;
    let skill =
        "---\nname: check\ndescription: Verify background synchronization\n---\nFirst revision.\n";
    fs::write(source.join("skills/check/SKILL.md"), skill)?;
    fs::write(source.join("skills/check/skill.toml"), "")?;
    git(&root, &["init", "--bare", "remote.git"])?;
    git(&source, &["init", "-b", "main"])?;
    git(&source, &["config", "user.email", "ci@ratatoskr.invalid"])?;
    git(&source, &["config", "user.name", "Ratatoskr CI"])?;
    git(&source, &["add", "."])?;
    git(&source, &["commit", "-m", "fixture"])?;
    git(&source, &["remote", "add", "origin", "../remote.git"])?;
    git(&source, &["push", "-u", "origin", "main"])?;
    let settings = ratatoskr_core::Settings {
        source: source.clone(),
        machine: "test".into(),
        state_dir: root.join("state"),
        updates: ratatoskr_core::updates::UpdateSettings {
            automatic: false,
            ..Default::default()
        },
        ..Default::default()
    };
    let config = root.join("config.toml");
    ratatoskr_core::save(&config, &settings)?;
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_ratatoskrw"))
            .arg("--config")
            .arg(&config)
            .arg("sync")
            .output()
    };
    let success = run()?;
    ensure!(
        success.status.success(),
        "{}",
        fs::read_to_string(settings.state_dir.join("test/sync.stderr.log"))?
    );
    let status = ratatoskr_core::job_status(&settings)?;
    ensure!(status.last_run.unwrap().error.is_none());
    ensure!(status.log.contains("Sync succeeded"));
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(settings.state_dir.join("test/sync.stdout.log"))?)?;
    ensure!(report["conflicts"].as_array().unwrap().is_empty());
    let installed = settings
        .state_dir
        .join("test/staging/codex/skills/check/SKILL.md");
    ensure!(fs::read_to_string(&installed)?.contains("First revision"));
    fs::write(
        source.join("skills/check/SKILL.md"),
        skill.replace("First revision", "Second revision"),
    )?;
    git(&source, &["add", "."])?;
    git(&source, &["commit", "-m", "next fixture revision"])?;
    git(&source, &["push"])?;
    // A malformed update-attempt ledger fails before networking. Content sync
    // must still run and persist its independent update error without stderr.
    let mut update_settings = settings.clone();
    update_settings.updates.automatic = true;
    ratatoskr_core::save(&config, &update_settings)?;
    fs::write(
        settings.state_dir.join("test/helper-update.json"),
        "invalid JSON",
    )?;
    ensure!(run()?.status.code() == Some(2));
    let status = ratatoskr_core::job_status(&settings)?;
    ensure!(status.helper_update_error.is_some());
    ensure!(fs::read_to_string(&installed)?.contains("Second revision"));
    ensure!(
        status
            .last_run
            .unwrap()
            .error
            .unwrap()
            .contains("Helper update failed")
    );
    ensure!(
        status.current_error.is_none(),
        "A helper-update error became a sync error"
    );
    ensure!(status.log.contains("Helper update failed"));
    ratatoskr_core::save(&config, &settings)?;
    fs::write(source.join("dirty.txt"), "uncommitted")?;
    ensure!(run()?.status.code() == Some(2));
    let status = ratatoskr_core::job_status(&settings)?;
    ensure!(
        status
            .last_run
            .unwrap()
            .error
            .unwrap()
            .contains("uncommitted")
    );
    ensure!(status.current_error.unwrap().contains("uncommitted"));
    ensure!(status.log.contains("sync.stderr.log") && status.log.contains("uncommitted"));
    Ok(())
}
