use crate::{
    Settings,
    config::{atomic_write, no_symlinks},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

#[derive(Clone, Default, Deserialize, Serialize)]
pub struct JobRun {
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct JobStatus {
    pub last_run: Option<JobRun>,
    pub current_error: Option<String>,
    pub helper_update_error: Option<String>,
    pub desktop_update_error: Option<String>,
    pub background_error: Option<String>,
    pub log: String,
}

fn now() -> Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs())
}

pub(crate) fn start(settings: &Settings) -> Result<JobRun> {
    let run = JobRun {
        started_at: now()?,
        ..JobRun::default()
    };
    store(settings, &run)?;
    Ok(run)
}

fn store(settings: &Settings, run: &JobRun) -> Result<()> {
    atomic_write(
        &settings
            .state_dir
            .join(&settings.machine)
            .join("last-sync.json"),
        &serde_json::to_vec(run)?,
    )
}

pub(crate) fn finish(settings: &Settings, mut run: JobRun, error: Option<&str>) -> Result<()> {
    run.finished_at = Some(now()?);
    run.error = error.map(str::to_owned);
    store(settings, &run)?;
    let path = settings
        .state_dir
        .join(&settings.machine)
        .join("sync-history.json");
    no_symlinks(&path)?;
    let mut history: Vec<JobRun> = if path.try_exists()? {
        serde_json::from_slice(&fs::read(&path)?)?
    } else {
        vec![]
    };
    history.push(run);
    if history.len() > 64 {
        history.drain(..history.len() - 64);
    }
    atomic_write(&path, &serde_json::to_vec(&history)?)
}

fn read_tail(path: &Path) -> Result<String> {
    no_symlinks(path)?;
    if !path.try_exists()? {
        return Ok(String::new());
    }
    let mut file = fs::File::open(path)?;
    let length = file.metadata()?.len();
    file.seek(SeekFrom::Start(length.saturating_sub(65_536)))?;
    let mut bytes = Vec::new();
    file.take(65_536).read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

pub fn job_status(settings: &Settings) -> Result<JobStatus> {
    settings.validate()?;
    let state = settings.state_dir.join(&settings.machine);
    let path = state.join("last-sync.json");
    no_symlinks(&path)?;
    let last_run = if path.try_exists()? {
        Some(serde_json::from_slice(&fs::read(path)?)?)
    } else {
        None
    };
    let history = state.join("sync-history.json");
    no_symlinks(&history)?;
    let mut log = if history.try_exists()? {
        let runs: Vec<JobRun> = serde_json::from_slice(&fs::read(history)?)?;
        runs.iter()
            .map(|r| {
                format!(
                    "{}: {}\n",
                    r.started_at,
                    r.error.as_deref().unwrap_or("Sync succeeded")
                )
            })
            .collect()
    } else {
        String::new()
    };
    for filename in ["sync.stdout.log", "sync.stderr.log"] {
        let tail = read_tail(&state.join(filename))?;
        if !tail.is_empty() {
            log.push_str(&format!("{filename} (last 64 KiB)\n{tail}\n"));
        }
    }
    Ok(JobStatus {
        background_error: crate::tray::problem(settings)?,
        last_run,
        current_error: crate::notifications::current_error(settings, "notification.json")?,
        helper_update_error: crate::notifications::current_error(
            settings,
            "helper-update-notification.json",
        )?,
        desktop_update_error: crate::notifications::current_error(
            settings,
            "desktop-update-notification.json",
        )?,
        log,
    })
}

/// Bundled sibling first, then a standalone CLI on PATH. Missing helpers are explicit errors.
pub fn detect_helper(executable: &Path, search_path: Option<&std::ffi::OsStr>) -> Result<PathBuf> {
    let filename = if cfg!(windows) {
        "ratatoskr.exe"
    } else {
        "ratatoskr"
    };
    let mut candidates = vec![executable.with_file_name(filename)];
    if let Some(paths) = search_path {
        candidates.extend(
            std::env::split_paths(paths)
                .filter(|p| p.is_absolute())
                .map(|p| p.join(filename)),
        );
    }
    for candidate in candidates {
        if !candidate.try_exists()? {
            continue;
        }
        let metadata = fs::metadata(&candidate)
            .with_context(|| format!("Inspect helper {}", candidate.display()))?;
        if !metadata.is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                continue;
            }
        }
        let path = candidate.canonicalize()?;
        ensure!(path.is_absolute(), "Helper path must be absolute");
        return Ok(path);
    }
    anyhow::bail!(
        "No bundled or standalone ratatoskr helper found. Install the CLI or use a packaged desktop build."
    )
}
