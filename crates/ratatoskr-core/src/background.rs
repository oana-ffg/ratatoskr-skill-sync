use crate::{
    Settings,
    config::{identifier, no_symlinks},
    jobs,
};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

fn diagnostics(config: &Path) -> Result<(Settings, bool)> {
    no_symlinks(config)?;
    // Parsing separately lets a missing checkout/invalid update setting still
    // reach the existing job ledger. Never use an unvalidated path or machine ID.
    let parsed = (|| -> Result<Settings> {
        let settings: Settings = toml::from_str(&fs::read_to_string(config)?)?;
        identifier(&settings.machine)?;
        ensure!(
            settings.state_dir.is_absolute(),
            "State directory must be absolute"
        );
        no_symlinks(&settings.state_dir)?;
        ensure!(
            !crate::config::overlaps(&settings.source, &settings.state_dir),
            "Source and state directories must not overlap"
        );
        Ok(settings)
    })();
    if let Ok(settings) = parsed {
        return Ok((settings, false));
    }
    // Malformed/unreadable configuration cannot tell us its state directory.
    // Keep a separate bootstrap ledger beside that exact config, never in CWD.
    let mut directory = config.as_os_str().to_owned();
    directory.push(".diagnostics");
    Ok((
        Settings {
            state_dir: PathBuf::from(directory),
            machine: "startup".into(),
            ..Settings::default()
        },
        true,
    ))
}

/// Direct core entry point for tray runs. Logs use owned file handles, never
/// process-wide stdout redirection inside a multithreaded desktop application.
/// None means the existing sync operation lock was busy; no run was started.
pub fn recorded_sync(config: &Path) -> Result<Option<crate::Plan>> {
    ensure!(
        config.is_absolute(),
        "Background config path must be absolute"
    );
    let (diagnostics, _) = diagnostics(config)?;
    let directory = diagnostics.state_dir.join(&diagnostics.machine);
    no_symlinks(&directory)?;
    fs::create_dir_all(&directory)?;
    let open = |filename: &str| -> Result<fs::File> {
        let path = directory.join(filename);
        no_symlinks(&path)?;
        if path.try_exists()? && fs::metadata(&path)?.len() > 262_144 {
            let bytes = fs::read(&path)?;
            crate::config::atomic_write(&path, &bytes[bytes.len().saturating_sub(65_536)..])?;
        }
        Ok(fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?)
    };
    let mut stdout = open("sync.stdout.log").context("Open background output log")?;
    let mut stderr = open("sync.stderr.log").context("Open background error log")?;
    let result = (|| {
        let settings = match crate::load(config) {
            Ok(settings) => settings,
            Err(error) => {
                let Some(_lock) = crate::config::try_operation_lock(&diagnostics, "sync.lock")?
                else {
                    return Ok(None);
                };
                let run = jobs::start(&diagnostics)?;
                jobs::finish(&diagnostics, run, Some(&format!("{error:#}")))?;
                return Err(error);
            }
        };
        crate::git::sync_in_process(&settings)
    })();
    match &result {
        Ok(Some(plan)) => writeln!(stdout, "{}", serde_json::to_string(plan)?)?,
        Ok(None) => (),
        Err(error) => writeln!(stderr, "{error:#}")?,
    }
    stdout.flush()?;
    stderr.flush()?;
    result
}

/// Configuration errors still have a safe bootstrap ledger beside the config.
pub fn record_background_problem(config: &Path, error: Option<&str>) -> Result<()> {
    let (diagnostics, _) = diagnostics(config)?;
    crate::tray::record_problem(&diagnostics, error)
}
pub fn notify_background(
    config: &Path,
    error: Option<&str>,
    send: impl FnOnce(&str) -> Result<()>,
) -> Result<()> {
    let (diagnostics, _) = diagnostics(config)?;
    let mut state = crate::tray::NotificationState::load(&diagnostics)?;
    let previous = state.clone();
    state.deliver(error, send)?;
    if state != previous {
        state.save(&diagnostics)?;
    }
    Ok(())
}
