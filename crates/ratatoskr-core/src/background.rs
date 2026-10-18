use crate::{
    Settings,
    config::{identifier, no_symlinks},
    jobs,
};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    io::Write,
    os::windows::io::AsRawHandle,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub struct RecordedSyncError(String);
impl std::fmt::Display for RecordedSyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for RecordedSyncError {}

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

struct Logs {
    stdout: fs::File,
    stderr: fs::File,
    old_stdout: windows_sys::Win32::Foundation::HANDLE,
    old_stderr: windows_sys::Win32::Foundation::HANDLE,
}

impl Logs {
    fn open(settings: &Settings) -> Result<Self> {
        use windows_sys::Win32::System::Console::{
            GetStdHandle, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
        };
        let directory = settings.state_dir.join(&settings.machine);
        no_symlinks(&directory)?;
        fs::create_dir_all(&directory)?;
        let open = |filename: &str| -> Result<fs::File> {
            let path = directory.join(filename);
            no_symlinks(&path)?;
            Ok(fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(path)?)
        };
        let logs = Self {
            stdout: open("sync.stdout.log")?,
            stderr: open("sync.stderr.log")?,
            old_stdout: unsafe { GetStdHandle(STD_OUTPUT_HANDLE) },
            old_stderr: unsafe { GetStdHandle(STD_ERROR_HANDLE) },
        };
        // The helper owns this process. Keep the file handles alive through all
        // core calls so eprintln!, lock-release failures and panic diagnostics
        // are durable too, even though the PE subsystem has no console.
        ensure!(
            unsafe { SetStdHandle(STD_OUTPUT_HANDLE, logs.stdout.as_raw_handle()) } != 0,
            "Redirect background stdout: {}",
            std::io::Error::last_os_error()
        );
        if unsafe { SetStdHandle(STD_ERROR_HANDLE, logs.stderr.as_raw_handle()) } == 0 {
            let error = std::io::Error::last_os_error();
            ensure!(
                unsafe { SetStdHandle(STD_OUTPUT_HANDLE, logs.old_stdout) } != 0,
                "Restore stdout after stderr redirection failed: {error}; {}",
                std::io::Error::last_os_error()
            );
            return Err(error.into());
        }
        Ok(logs)
    }

    fn close(mut self) -> Result<()> {
        use windows_sys::Win32::System::Console::{
            STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
        };
        // Attempt both restorations before checking either result.
        let stdout = unsafe { SetStdHandle(STD_OUTPUT_HANDLE, self.old_stdout) };
        let stdout_error = std::io::Error::last_os_error();
        let stderr = unsafe { SetStdHandle(STD_ERROR_HANDLE, self.old_stderr) };
        ensure!(
            stdout != 0 && stderr != 0,
            "Restore background output handles: stdout={stdout} ({stdout_error}), stderr={stderr} ({})",
            std::io::Error::last_os_error()
        );
        self.stdout.flush()?;
        self.stderr.flush()?;
        Ok(())
    }
}

/// Direct core entry point for the windowless Windows task. No console or CLI subprocess.
pub fn scheduled_sync(config: &Path) -> Result<()> {
    ensure!(
        config.is_absolute(),
        "Scheduled config path must be absolute"
    );
    let (diagnostics, bootstrap) = diagnostics(config)?;
    let mut logs = Logs::open(&diagnostics).context("Open background sync logs")?;
    let result = (|| {
        let settings = match crate::load(config) {
            Ok(settings) => settings,
            Err(error) => {
                let run = jobs::start(&diagnostics).context("Start configuration-error ledger")?;
                jobs::finish(&diagnostics, run, Some(&format!("{error:#}")))
                    .context("Finish configuration-error ledger")?;
                return Err(error);
            }
        };
        let report = crate::sync(&settings)?;
        writeln!(logs.stdout, "{}", serde_json::to_string_pretty(&report)?)?;
        ensure!(report.conflicts.is_empty(), "Sync completed with conflicts");
        Ok(())
    })();
    let error = result
        .as_ref()
        .err()
        .map(|error: &anyhow::Error| format!("{error:#}"));
    // Captures even load/lock failures, plus any stderr emitted by the core.
    let write = if let Some(error) = &error {
        writeln!(logs.stderr, "{error}")
    } else {
        Ok(())
    };
    let close = logs.close();
    if write.is_err() || close.is_err() {
        anyhow::bail!(
            "Background sync: {}; error-log write: {write:?}; log close: {close:?}",
            error.as_deref().unwrap_or("completed")
        );
    }
    if let Some(error) = error {
        return Err(RecordedSyncError(format!(
            "{error}; diagnostics: {}{}",
            diagnostics.state_dir.display(),
            if bootstrap {
                " (configuration could not be loaded)"
            } else {
                ""
            }
        ))
        .into());
    }
    Ok(())
}
