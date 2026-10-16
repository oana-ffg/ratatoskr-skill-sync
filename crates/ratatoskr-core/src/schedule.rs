use crate::{
    Settings,
    config::{atomic_write, no_symlinks},
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Deserialize, Serialize)]
pub struct Schedule {
    pub label: String,
    pub path: PathBuf,
    pub definition: String,
    pub os: String,
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub fn schedule_plan(
    settings: &Settings,
    config: &Path,
    helper: &Path,
    seconds: u32,
) -> Result<Schedule> {
    settings.validate()?;
    no_symlinks(config)?;
    no_symlinks(helper)?;
    ensure!(
        config.is_absolute() && helper.is_absolute() && helper.is_file(),
        "Use an absolute config path and an installed helper executable"
    );
    ensure!(
        (60..=86400).contains(&seconds),
        "Schedule interval must be 60–86400 seconds"
    );
    let id = format!("{:x}", Sha256::digest(config.to_string_lossy().as_bytes()));
    let label = format!("io.github.oana-ffg.ratatoskr.{}", &id[..12]);
    let state = settings.state_dir.join(&settings.machine);
    let arguments = xml(&format!("--config \"{}\" sync", config.to_string_lossy()));
    let config = xml(&config.to_string_lossy());
    let helper = xml(&helper.to_string_lossy());
    let os = std::env::consts::OS;
    let (path, definition) = match os {
        "macos" => {
            let home = directories::BaseDirs::new().context("Home directory unavailable")?;
            let path = home
                .home_dir()
                .join("Library/LaunchAgents")
                .join(format!("{label}.plist"));
            let stdout = xml(&state.join("sync.stdout.log").to_string_lossy());
            let stderr = xml(&state.join("sync.stderr.log").to_string_lossy());
            let env_path =
                xml(&std::env::var("PATH")
                    .unwrap_or_else(|_| "/usr/bin:/bin:/usr/sbin:/sbin".into()));
            (
                path,
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{label}</string><key>ProgramArguments</key><array><string>{helper}</string><string>--config</string><string>{config}</string><string>sync</string></array><key>RunAtLoad</key><true/><key>StartInterval</key><integer>{seconds}</integer><key>EnvironmentVariables</key><dict><key>PATH</key><string>{env_path}</string></dict><key>StandardOutPath</key><string>{stdout}</string><key>StandardErrorPath</key><string>{stderr}</string></dict></plist>\n"
                ),
            )
        }
        "windows" => {
            let start = chrono::Local::now().format("%Y-%m-%dT%H:%M:%S");
            (
                state.join("schedule.xml"),
                format!(
                    "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Task version=\"1.2\" xmlns=\"http://schemas.microsoft.com/windows/2004/02/mit/task\"><RegistrationInfo><URI>\\{label}</URI><Description>Ratatoskr Skill Sync</Description></RegistrationInfo><Triggers><TimeTrigger><Repetition><Interval>PT{seconds}S</Interval><StopAtDurationEnd>false</StopAtDurationEnd></Repetition><StartBoundary>{start}</StartBoundary><Enabled>true</Enabled></TimeTrigger><LogonTrigger><Enabled>true</Enabled></LogonTrigger></Triggers><Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><StartWhenAvailable>true</StartWhenAvailable><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT30M</ExecutionTimeLimit></Settings><Actions><Exec><Command>{helper}</Command><Arguments>{arguments}</Arguments></Exec></Actions></Task>"
                ),
            )
        }
        _ => bail!("Scheduler installation is supported on macOS and Windows"),
    };
    no_symlinks(&path)?;
    Ok(Schedule {
        label,
        path,
        definition,
        os: os.into(),
    })
}

fn run(command: &str, args: &[&str]) -> Result<()> {
    let output = Command::new(command).args(args).output()?;
    ensure!(
        output.status.success(),
        "{command}: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}

fn record_path(settings: &Settings) -> PathBuf {
    settings
        .state_dir
        .join(&settings.machine)
        .join("installed-schedule.json")
}

pub fn install_schedule(settings: &Settings, schedule: &Schedule) -> Result<()> {
    settings.validate()?;
    let _lock = crate::config::operation_lock(settings, "schedule.lock")?;
    ensure!(
        schedule.os == std::env::consts::OS,
        "Schedule belongs to another OS"
    );
    no_symlinks(&schedule.path)?;
    ensure!(
        !schedule.path.try_exists()? && !record_path(settings).try_exists()?,
        "A schedule already exists; remove the owned schedule before replacing it"
    );
    fs::create_dir_all(settings.state_dir.join(&settings.machine))?;
    atomic_write(&schedule.path, schedule.definition.as_bytes())?;
    let result = if schedule.os == "macos" {
        let uid = Command::new("id").arg("-u").output()?;
        ensure!(uid.status.success(), "Cannot determine login UID");
        run(
            "launchctl",
            &[
                "bootstrap",
                &format!("gui/{}", String::from_utf8(uid.stdout)?.trim()),
                &schedule.path.to_string_lossy(),
            ],
        )
    } else {
        run(
            "schtasks",
            &[
                "/Create",
                "/TN",
                &schedule.label,
                "/XML",
                &schedule.path.to_string_lossy(),
            ],
        )
    };
    if let Err(error) = result {
        fs::remove_file(&schedule.path)?;
        return Err(error);
    }
    atomic_write(
        &record_path(settings),
        &serde_json::to_vec_pretty(schedule)?,
    )?;
    Ok(())
}

pub fn remove_schedule(settings: &Settings) -> Result<()> {
    settings.validate()?;
    let _lock = crate::config::operation_lock(settings, "schedule.lock")?;
    let record = record_path(settings);
    no_symlinks(&record)?;
    let schedule: Schedule = serde_json::from_slice(&fs::read(&record)?)?;
    no_symlinks(&schedule.path)?;
    ensure!(
        fs::read_to_string(&schedule.path)? == schedule.definition,
        "Schedule definition changed; preserving it for manual review"
    );
    if schedule.os == "macos" {
        let uid = Command::new("id").arg("-u").output()?;
        ensure!(uid.status.success(), "Cannot determine login UID");
        run(
            "launchctl",
            &[
                "bootout",
                &format!(
                    "gui/{}/{}",
                    String::from_utf8(uid.stdout)?.trim(),
                    schedule.label
                ),
            ],
        )?;
    } else {
        run("schtasks", &["/Delete", "/TN", &schedule.label, "/F"])?;
    }
    fs::remove_file(schedule.path)?;
    fs::remove_file(record)?;
    Ok(())
}
