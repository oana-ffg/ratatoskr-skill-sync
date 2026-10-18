#![cfg(windows)]

use anyhow::{Context, Result, bail, ensure};
use ratatoskr_core::{Schedule, Settings, save, schedule_plan};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn checked(command: &mut Command) -> Result<Output> {
    let output = command
        .output()
        .with_context(|| format!("Start {command:?}"))?;
    ensure!(
        output.status.success(),
        "{command:?} exited with {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output)
}

fn cli(config: &Path, action: &str) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ratatoskr"));
    command.args(["--config"]).arg(config);
    command.args(["--json", "schedule", action]);
    command
}

// Each plan gets a fresh wall-clock StartBoundary. Only that value may differ
// between separate CLI plan/install calls; every other XML character must match.
fn without_start_boundary(definition: &str) -> Result<(&str, &str)> {
    let (before, rest) = definition
        .split_once("<StartBoundary>")
        .context("Missing StartBoundary")?;
    let (_, after) = rest
        .split_once("</StartBoundary>")
        .context("Unclosed StartBoundary")?;
    Ok((before, after))
}

#[test]
fn windows_schedule_cli_registers_queries_and_removes_real_task() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    let source = root.join("source");
    fs::create_dir(&source)?;
    let settings = Settings {
        source,
        machine: "schedule-ci".into(),
        state_dir: root.join("state"),
        ..Settings::default()
    };
    let config = root.join("config Æ 🦀 & test.toml");
    save(&config, &settings)?;
    // where.exe only searches for files. Even if the immediate time trigger
    // fires, it cannot run Ratatoskr or sync a real configuration.
    let helper = PathBuf::from(std::env::var_os("SystemRoot").context("Missing SystemRoot")?)
        .join("System32/where.exe")
        .canonicalize()?;
    let planned = schedule_plan(&settings, &config, &helper, 60)?;
    let mut needs_cleanup = false;
    let result = (|| -> Result<()> {
        let preview = checked(cli(&config, "plan").arg("--helper").arg(&helper))?;
        let preview = String::from_utf8(preview.stdout)?;
        ensure!(!planned.path.exists(), "Preview wrote a schedule file");
        ensure!(preview.starts_with("<?xml version=\"1.0\" encoding=\"UTF-16\"?>"));

        // Clean up even if installation fails after registering the task.
        needs_cleanup = true;
        checked(
            cli(&config, "install")
                .arg("--helper")
                .arg(&helper)
                .args(["--seconds", "60"]),
        )?;
        let record = settings
            .state_dir
            .join(&settings.machine)
            .join("installed-schedule.json");
        let installed: Schedule = serde_json::from_slice(&fs::read(&record)?)?;
        ensure!(installed.label == planned.label);
        let bytes = fs::read(&installed.path)?;
        ensure!(bytes.starts_with(&[0xff, 0xfe]), "Missing UTF-16LE BOM");
        let (pairs, remainder) = bytes[2..].as_chunks::<2>();
        ensure!(remainder.is_empty(), "Truncated UTF-16 code unit");
        let units: Vec<_> = pairs.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
        let definition = String::from_utf16(&units)?;
        ensure!(
            definition == installed.definition,
            "Installed bytes differ from recorded XML"
        );
        ensure!(definition.contains("config Æ 🦀 &amp; test.toml"));
        ensure!(
            !definition.contains(r"\\?\"),
            "Verbatim paths leaked into task actions"
        );
        ensure!(definition.contains("<LogonType>InteractiveToken</LogonType>"));
        ensure!(definition.contains("<RunLevel>LeastPrivilege</RunLevel>"));
        ensure!(definition.contains("<Actions Context=\"Author\">"));
        ensure!(
            without_start_boundary(preview.trim_end())? == without_start_boundary(&definition)?,
            "CLI preview differs from installed XML"
        );

        // Exercise the exact /Create /XML /TN /F import used for regression
        // coverage, using the actual file written by schedule install.
        checked(
            Command::new("schtasks")
                .args(["/Create", "/XML"])
                .arg(&installed.path)
                .args(["/TN", &installed.label, "/F"]),
        )?;
        checked(Command::new("schtasks").args(["/Query", "/TN", &installed.label, "/XML"]))?;

        let mut edited = bytes.clone();
        edited.extend_from_slice(&[b' ', 0]);
        fs::write(&installed.path, edited)?;
        let refused = cli(&config, "remove").output()?;
        ensure!(!refused.status.success(), "Removal accepted edited XML");
        ensure!(String::from_utf8_lossy(&refused.stderr).contains("Schedule definition changed"));
        checked(Command::new("schtasks").args(["/Query", "/TN", &installed.label]))?;
        fs::write(&installed.path, bytes)?;

        // The real remove path must read the UTF-16 file, check ownership and
        // successfully invoke schtasks /Delete /TN <name> /F.
        checked(&mut cli(&config, "remove"))?;
        needs_cleanup = false;
        ensure!(!installed.path.exists() && !record.exists());
        let deleted = Command::new("schtasks")
            .args(["/Query", "/TN", &installed.label])
            .output()?;
        ensure!(!deleted.status.success(), "Task still exists after removal");
        Ok(())
    })();
    if needs_cleanup {
        let cleanup =
            checked(Command::new("schtasks").args(["/Delete", "/TN", &planned.label, "/F"]));
        if let Err(error) = cleanup {
            bail!("Task cleanup failed: {error:#}; test result: {result:?}");
        }
    }
    result
}
