#![cfg(all(windows, feature = "windows-helper"))]

use anyhow::{Context, Result, bail, ensure};
use ratatoskr_core::{Schedule, Settings, save, schedule_plan};
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
        "{command:?} exited with {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output)
}

fn query_definition(label: &str) -> Result<String> {
    let output = checked(Command::new("schtasks").args(["/Query", "/TN", label, "/XML"]))?;
    let bytes = output.stdout;
    if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[b'<', 0]) {
        let start = if bytes.starts_with(&[0xff, 0xfe]) {
            2
        } else {
            0
        };
        let (pairs, remainder) = bytes[start..].as_chunks::<2>();
        ensure!(remainder.is_empty(), "Truncated scheduler XML output");
        Ok(String::from_utf16(
            &pairs
                .iter()
                .map(|pair| u16::from_le_bytes(*pair))
                .collect::<Vec<_>>(),
        )?)
    } else {
        // schtasks can also use the active console code page. Action executable
        // names are ASCII, independent of the encoding of the Unicode fixture.
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
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
    // Both binaries run against this disposable non-Git checkout only. No
    // update/pull/apply can happen, even if the immediate trigger fires.
    let helper = root.join("ratatoskr.exe");
    let windowless = root.join("ratatoskrw.exe");
    fs::copy(env!("CARGO_BIN_EXE_ratatoskr"), &helper)?;
    fs::copy(env!("CARGO_BIN_EXE_ratatoskrw"), &windowless)?;
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
        ensure!(definition.contains("ratatoskrw.exe</Command>"));
        ensure!(query_definition(&installed.label)?.contains("ratatoskrw.exe"));
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
        fs::write(&installed.path, &edited)?;
        let refused_install = cli(&config, "install")
            .arg("--helper")
            .arg(&helper)
            .output()?;
        ensure!(
            !refused_install.status.success(),
            "Replacement accepted edited XML"
        );
        ensure!(
            fs::read(&installed.path)? == edited,
            "Refusal changed user-edited bytes"
        );
        let refused = cli(&config, "remove").output()?;
        ensure!(!refused.status.success(), "Removal accepted edited XML");
        ensure!(String::from_utf8_lossy(&refused.stderr).contains("Schedule definition changed"));
        checked(Command::new("schtasks").args(["/Query", "/TN", &installed.label]))?;
        fs::write(&installed.path, bytes)?;

        // Seed a real legacy task/ownership record using the console action.
        // Test migration from both old UTF-8 and current UTF-16 definitions.
        for legacy_utf8 in [false, true] {
            let mut legacy = installed.clone();
            legacy.definition = legacy.definition.replace("ratatoskrw.exe", "ratatoskr.exe");
            let utf16: Vec<u8> = std::iter::once(0xfeff)
                .chain(legacy.definition.encode_utf16())
                .flat_map(u16::to_le_bytes)
                .collect();
            fs::write(&legacy.path, &utf16)?;
            checked(
                Command::new("schtasks")
                    .args(["/Create", "/XML"])
                    .arg(&legacy.path)
                    .args(["/TN", &legacy.label, "/F"]),
            )?;
            if legacy_utf8 {
                legacy.definition = legacy
                    .definition
                    .replace("encoding=\"UTF-16\"", "encoding=\"UTF-8\"");
                fs::write(&legacy.path, legacy.definition.as_bytes())?;
            }
            fs::write(&record, serde_json::to_vec(&legacy)?)?;
            checked(cli(&config, "install").arg("--helper").arg(&helper))?;
            let replaced: Schedule = serde_json::from_slice(&fs::read(&record)?)?;
            ensure!(replaced.label == legacy.label && replaced.path == legacy.path);
            ensure!(replaced.definition.contains("ratatoskrw.exe</Command>"));
            ensure!(query_definition(&legacy.label)?.contains("ratatoskrw.exe"));
        }

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
