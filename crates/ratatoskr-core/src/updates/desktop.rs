//! Desktop instance ownership, installation locks and CLI update requests.
use super::*;

/// Holds the same locks as CLI sync/apply/schedule operations through desktop installation.
pub struct UpdateGuard {
    locks: Vec<std::fs::File>,
}

impl Drop for UpdateGuard {
    fn drop(&mut self) {
        for file in &self.locks {
            // Explicit unlock also releases locks retained by a briefly inherited file handle.
            if let Err(error) = fs2::FileExt::unlock(file) {
                eprintln!("Ratatoskr: release desktop update lock: {error}");
            }
        }
    }
}

/// Startup waits for helper work to finish; an interactive nightly tick defers instead.
pub fn idle_guard(settings: &crate::Settings, wait: bool) -> Result<Option<UpdateGuard>> {
    let mut guard = UpdateGuard { locks: Vec::new() };
    for name in ["sync.lock", "apply.lock", "schedule.lock"] {
        let file = crate::config::state_lock(settings, name)?;
        let result = if wait {
            fs2::FileExt::lock_exclusive(&file)
        } else {
            fs2::FileExt::try_lock_exclusive(&file)
        };
        match result {
            Ok(()) => guard.locks.push(file),
            Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
                return Ok(None);
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(Some(guard))
}

/// An explicit install request queued by the CLI and carried out by the running desktop app,
/// so agents never need to drive the app's UI. Only the CLI writes the request file and
/// only the desktop writes the response file; each response names the request it answers.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct DesktopUpdateRequest {
    pub id: String,
    pub requested_at: chrono::DateTime<chrono::FixedOffset>,
    /// The desktop executable that must carry out the request; other copies ignore it.
    pub app: std::path::PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct DesktopUpdateResponse {
    pub id: String,
    pub finished_at: chrono::DateTime<chrono::FixedOffset>,
    pub outcome: DesktopRequestOutcome,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "result", rename_all = "kebab-case")]
pub enum DesktopRequestOutcome {
    /// No newer signed release exists.
    Current,
    /// The installer is running; Windows exits the app here and relaunches it afterwards.
    Installing {
        version: String,
    },
    /// The restarted app runs this version.
    Installed {
        version: String,
    },
    Failed {
        error: String,
    },
}

const REQUEST_FILE: &str = "desktop-update-request.json";
const RESPONSE_FILE: &str = "desktop-update-response.json";
const INSTANCE_LOCK: &str = "desktop-instance.lock";
const INSTANCE_FILE: &str = "desktop-instance.json";

fn same_app(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

fn instance_lock(data: &std::path::Path) -> Result<std::fs::File> {
    std::fs::create_dir_all(data)?;
    let path = data.join(INSTANCE_LOCK);
    crate::config::no_symlinks(&path)?;
    Ok(std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?)
}

/// Held by the running desktop app for its lifetime. The CLI uses it to find out whether,
/// and from where, the app is running, without asking the OS to launch or reopen it.
pub struct DesktopInstance {
    lock: std::fs::File,
}

impl Drop for DesktopInstance {
    fn drop(&mut self) {
        if let Err(error) = fs2::FileExt::unlock(&self.lock) {
            eprintln!("Ratatoskr: release desktop instance lock: {error}");
        }
    }
}

/// Claim the instance for `app`; `None` while another desktop process holds it.
pub fn claim_desktop_instance(
    data: &std::path::Path,
    app: &std::path::Path,
) -> Result<Option<DesktopInstance>> {
    let lock = instance_lock(data)?;
    match fs2::FileExt::try_lock_exclusive(&lock) {
        Ok(()) => {
            // A separate file: Windows locks block other processes from reading the lock file.
            crate::config::atomic_write(&data.join(INSTANCE_FILE), &serde_json::to_vec(app)?)?;
            Ok(Some(DesktopInstance { lock }))
        }
        Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
            Ok(None)
        }
        Err(error) => Err(error).context("Claim the desktop instance"),
    }
}

/// The executable of the running desktop app, if one holds the instance lock.
pub fn running_desktop(data: &std::path::Path) -> Result<Option<std::path::PathBuf>> {
    let lock = instance_lock(data)?;
    match fs2::FileExt::try_lock_exclusive(&lock) {
        Ok(()) => {
            fs2::FileExt::unlock(&lock)?;
            Ok(None)
        }
        Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
            let app: Option<std::path::PathBuf> = read_json(&data.join(INSTANCE_FILE))?;
            Ok(Some(app.context(
                "A desktop app is starting but has not recorded its path; retry in a moment",
            )?))
        }
        Err(error) => Err(error).context("Check for a running desktop app"),
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &std::path::Path) -> Result<Option<T>> {
    crate::config::no_symlinks(path)?;
    if !path.try_exists()? {
        return Ok(None);
    }
    let bytes = std::fs::read(path)?;
    serde_json::from_slice(&bytes)
        .map(Some)
        .with_context(|| format!("Read {}", path.display()))
}

pub fn request_desktop_update(
    data: &std::path::Path,
    app: &std::path::Path,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> Result<DesktopUpdateRequest> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let request = DesktopUpdateRequest {
        id: format!("{nanos}-{}", std::process::id()),
        requested_at: now,
        app: app.to_path_buf(),
    };
    std::fs::create_dir_all(data)?;
    crate::config::atomic_write(
        &data.join(REQUEST_FILE),
        &serde_json::to_vec_pretty(&request)?,
    )?;
    Ok(request)
}

pub struct QueuedDesktopUpdate {
    pub request: DesktopUpdateRequest,
    /// The app was not running, so this call started it hidden.
    pub launched: bool,
}

/// Queue an install for the desktop app beside this CLI and make sure it runs, hidden,
/// to carry it out. The app is started directly rather than through the OS launcher,
/// so no window opens and the app is neither activated nor reopened.
pub fn queue_desktop_update() -> Result<QueuedDesktopUpdate> {
    let desktop = desktop_beside(&std::env::current_exe()?)?;
    let data = desktop_data_dir()?;
    let running = running_desktop(&data)?;
    if let Some(running) = &running {
        ensure!(
            same_app(running, &desktop),
            "Another copy of Ratatoskr is running from {}. Quit it, then request the update again.",
            running.display()
        );
    }
    let request = request_desktop_update(&data, &desktop, chrono::Local::now().fixed_offset())?;
    let launched = running.is_none();
    if launched {
        start_desktop_hidden(&desktop)?;
    }
    Ok(QueuedDesktopUpdate { request, launched })
}

fn desktop_beside(cli: &std::path::Path) -> Result<std::path::PathBuf> {
    let desktop = cli.with_file_name(if cfg!(windows) {
        "ratatoskr-desktop.exe"
    } else {
        "ratatoskr-desktop"
    });
    ensure!(
        (cfg!(windows) || bundled_helper_path(cli)) && desktop.try_exists()?,
        "No Ratatoskr desktop app beside this CLI ({}). A standalone CLI updates itself with `ratatoskr update --install`.",
        cli.display()
    );
    Ok(desktop)
}

/// Start the desktop executable directly with `--background`, detached from the caller.
pub fn start_desktop_hidden(desktop: &std::path::Path) -> Result<()> {
    let mut command = std::process::Command::new(desktop);
    command
        .arg("--background")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        command.creation_flags(DETACHED_PROCESS);
    }
    command.spawn().context("Start the desktop app")?;
    Ok(())
}

/// The latest request and the desktop's answer to it, if any.
pub fn desktop_request_status(
    data: &std::path::Path,
) -> Result<(Option<DesktopUpdateRequest>, Option<DesktopUpdateResponse>)> {
    let request: Option<DesktopUpdateRequest> = read_json(&data.join(REQUEST_FILE))?;
    let response: Option<DesktopUpdateResponse> = read_json(&data.join(RESPONSE_FILE))?;
    let response = response.filter(|r| request.as_ref().is_some_and(|q| q.id == r.id));
    Ok((request, response))
}

/// A request for `app` still waiting to be installed or declined.
pub fn pending_desktop_request(
    data: &std::path::Path,
    app: &std::path::Path,
) -> Result<Option<DesktopUpdateRequest>> {
    let (request, response) = desktop_request_status(data)?;
    Ok(request.filter(|r| response.is_none() && same_app(&r.app, app)))
}

pub fn answer_desktop_request(
    data: &std::path::Path,
    request: &DesktopUpdateRequest,
    now: chrono::DateTime<chrono::FixedOffset>,
    outcome: DesktopRequestOutcome,
) -> Result<()> {
    let response = DesktopUpdateResponse {
        id: request.id.clone(),
        finished_at: now,
        outcome,
    };
    crate::config::atomic_write(
        &data.join(RESPONSE_FILE),
        &serde_json::to_vec_pretty(&response)?,
    )
}

/// Run at desktop startup: settle an install that ended with `app` restarting. Another
/// copy's request is left untouched. Returns the settled outcome for reporting.
pub fn finish_desktop_request(
    data: &std::path::Path,
    app: &std::path::Path,
    running: &str,
    now: chrono::DateTime<chrono::FixedOffset>,
) -> Result<Option<DesktopRequestOutcome>> {
    let (Some(request), Some(response)) = desktop_request_status(data)? else {
        return Ok(None);
    };
    if !same_app(&request.app, app) {
        return Ok(None);
    }
    let DesktopRequestOutcome::Installing { version } = response.outcome else {
        return Ok(None);
    };
    let outcome = if version == running {
        DesktopRequestOutcome::Installed { version }
    } else {
        DesktopRequestOutcome::Failed {
            error: format!("Installer for {version} finished, but the app still runs {running}"),
        }
    };
    answer_desktop_request(data, &request, now, outcome.clone())?;
    Ok(Some(outcome))
}

#[cfg(test)]
mod tests;
