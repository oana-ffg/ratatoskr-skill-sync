#![cfg(windows)]

use anyhow::{Context, Result, ensure};
use ratatoskr_core::{Schedule, Settings, migrate_windows_schedule, save, tray::TrayPreferences};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn checked(command: &mut Command) -> Result<Output> {
    let output = command
        .output()
        .with_context(|| format!("Start {command:?}"))?;
    ensure!(
        output.status.success(),
        "{command:?}: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output)
}
fn registered(label: &str) -> Result<bool> {
    let output = checked(Command::new("schtasks").args(["/Query", "/FO", "CSV", "/NH"]))?;
    Ok(String::from_utf8_lossy(&output.stdout).lines().any(|line| {
        line.split(',')
            .next()
            .is_some_and(|name| name.trim_matches('"').trim_start_matches('\\') == label)
    }))
}
fn write_definition(schedule: &Schedule, utf8: bool) -> Result<()> {
    let bytes: Vec<u8> = if utf8 {
        schedule.definition.as_bytes().to_vec()
    } else {
        std::iter::once(0xfeff)
            .chain(schedule.definition.encode_utf16())
            .flat_map(u16::to_le_bytes)
            .collect()
    };
    fs::write(&schedule.path, bytes)?;
    Ok(())
}
fn cli(config: &Path, action: &str) -> Command {
    let executable = std::env::var_os("RATATOSKR_TEST_CLI")
        .unwrap_or_else(|| env!("CARGO_BIN_EXE_ratatoskr").into());
    let mut command = Command::new(executable);
    command
        .arg("--config")
        .arg(config)
        .args(["--json", "schedule", action]);
    command
}

#[test]
fn windows_migration_preserves_unowned_tasks_and_removes_enabled_and_disabled_owned_tasks()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let config = std::env::var_os("RATATOSKR_TEST_CONFIG")
        .map(std::path::PathBuf::from)
        .unwrap_or(temp.path().join("config Æ 🦀 & test.toml"));
    let config = config
        .parent()
        .context("Config parent missing")?
        .canonicalize()?
        .join(config.file_name().context("Config filename missing")?);
    let root = config.parent().unwrap();
    let source = root.join("source");
    fs::create_dir_all(&source)?;
    let settings = Settings {
        source,
        machine: "schedule-ci".into(),
        state_dir: root.join("state"),
        ..Default::default()
    };
    save(&config, &settings)?;
    // Derive the same label through an absent ownership error, without adding
    // a task-creation API to the product. SHA256 matches historical labels.
    let label_output = checked(Command::new("powershell.exe").args(["-NoProfile", "-NonInteractive", "-Command",
        "$hash=[Security.Cryptography.SHA256]::Create().ComputeHash([Text.Encoding]::UTF8.GetBytes($env:RATATOSKR_TEST_PATH)); 'io.github.oana-ffg.ratatoskr.'+([BitConverter]::ToString($hash).Replace('-','').ToLowerInvariant()).Substring(0,12); [Security.Principal.WindowsIdentity]::GetCurrent().User.Value"])
        .env("RATATOSKR_TEST_PATH", &config))?;
    let identity = String::from_utf8(label_output.stdout)?;
    let mut lines = identity.lines();
    let label = lines
        .next()
        .context("Task label missing")?
        .trim()
        .to_owned();
    let sid = lines.next().context("Current user SID missing")?.trim();
    ensure!(
        sid.starts_with("S-1-")
            && sid
                .chars()
                .all(|c| c.is_ascii_digit() || c == 'S' || c == '-'),
        "Invalid user SID"
    );
    let directory = settings.state_dir.join(&settings.machine);
    fs::create_dir_all(&directory)?;
    let record = directory.join("installed-schedule.json");
    let definition = format!(
        r#"<?xml version="1.0" encoding="UTF-16"?><Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><RegistrationInfo><Description>Ratatoskr CI harmless legacy fixture</Description></RegistrationInfo><Triggers><TimeTrigger><Repetition><Interval>PT120S</Interval><StopAtDurationEnd>false</StopAtDurationEnd></Repetition><StartBoundary>2099-01-01T00:00:00</StartBoundary><Enabled>true</Enabled></TimeTrigger></Triggers><Principals><Principal id="Author"><UserId>{sid}</UserId><LogonType>InteractiveToken</LogonType><RunLevel>LeastPrivilege</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><ExecutionTimeLimit>PT1M</ExecutionTimeLimit></Settings><Actions Context="Author"><Exec><Command>cmd.exe</Command><Arguments>/c exit 0</Arguments></Exec></Actions></Task>"#
    );
    let mut schedule = Schedule {
        label: label.clone(),
        path: directory.join("schedule.xml"),
        definition,
        os: "windows".into(),
    };
    let mut task_exists = false;
    let result = (|| -> Result<()> {
        let install = cli(&config, "install").output()?;
        ensure!(!install.status.success());
        ensure!(String::from_utf8_lossy(&install.stderr).contains("tray sync"));
        ensure!(
            !registered(&label)? && !record.exists(),
            "schedule install registered a task"
        );
        // Seed via the real registration harness, never via product code.
        write_definition(&schedule, false)?;
        checked(
            Command::new("schtasks")
                .args(["/Create", "/TN", &label, "/XML"])
                .arg(&schedule.path),
        )?;
        task_exists = true;
        let unowned = migrate_windows_schedule(&settings, &config).unwrap_err();
        ensure!(format!("{unowned:#}").contains("no ownership record"));
        ensure!(registered(&label)?, "Unowned task was removed");
        ratatoskr_core::tray::record_problem(&settings, Some(&format!("{unowned:#}")))?;
        ensure!(
            ratatoskr_core::job_status(&settings)?
                .background_error
                .is_some()
        );

        for (disabled, utf8) in [(false, false), (true, false), (false, true), (true, true)] {
            if !task_exists {
                schedule.definition = schedule
                    .definition
                    .replace("encoding=\"UTF-8\"", "encoding=\"UTF-16\"");
                write_definition(&schedule, false)?;
                checked(
                    Command::new("schtasks")
                        .args(["/Create", "/TN", &label, "/XML"])
                        .arg(&schedule.path),
                )?;
                task_exists = true;
            }
            if disabled {
                checked(Command::new("schtasks").args(["/Change", "/TN", &label, "/DISABLE"]))?;
            }
            if utf8 {
                schedule.definition = schedule
                    .definition
                    .replace("encoding=\"UTF-16\"", "encoding=\"UTF-8\"");
            }
            write_definition(&schedule, utf8)?;
            fs::write(&record, serde_json::to_vec(&schedule)?)?;
            TrayPreferences {
                paused: true,
                interval_seconds: 60,
            }
            .save(&settings)?;
            let original = fs::read(&schedule.path)?;
            let mut changed = original.clone();
            changed.push(b' ');
            fs::write(&schedule.path, &changed)?;
            ensure!(migrate_windows_schedule(&settings, &config).is_err());
            ensure!(
                registered(&label)? && fs::read(&schedule.path)? == changed,
                "Edited task was changed"
            );
            fs::write(&schedule.path, original)?;
            ensure!(migrate_windows_schedule(&settings, &config)?);
            task_exists = false;
            ensure!(!registered(&label)? && !schedule.path.exists() && !record.exists());
            let preferences = TrayPreferences::load(&settings)?;
            ensure!(
                !preferences.paused && preferences.interval_seconds == 120,
                "Migration did not enable tray sync with existing interval"
            );
            println!(
                "PASS: owned task removed; disabled={disabled}, legacy_utf8={utf8}; tray sync enabled"
            );
        }
        ensure!(
            !migrate_windows_schedule(&settings, &config)?,
            "Migration must be idempotent"
        );
        Ok(())
    })();
    if task_exists {
        let cleanup = checked(Command::new("schtasks").args(["/Delete", "/TN", &label, "/F"]));
        ensure!(
            cleanup.is_ok(),
            "Task cleanup failed: {cleanup:?}; test: {result:?}"
        );
    }
    result
}
