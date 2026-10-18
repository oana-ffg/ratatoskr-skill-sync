use anyhow::{Context, Result, bail, ensure};
use std::{fs, io::Write, path::PathBuf, ptr};
use windows_sys::Win32::System::EventLog::{
    DeregisterEventSource, EVENTLOG_ERROR_TYPE, RegisterEventSourceW, ReportEventW,
};

fn event_log(message: &str) -> Result<()> {
    let source: Vec<u16> = "Ratatoskr Skill Sync\0".encode_utf16().collect();
    // Preserve embedded NULs visibly rather than letting the API truncate them.
    let text: Vec<u16> = message
        .replace('\0', "\\0")
        .encode_utf16()
        .chain([0])
        .collect();
    // No registry registration or administrator rights are needed. An unknown
    // source is recorded in Application, including these insertion strings.
    let handle = unsafe { RegisterEventSourceW(ptr::null(), source.as_ptr()) };
    ensure!(
        !handle.is_null(),
        "Register event source: {}",
        std::io::Error::last_os_error()
    );
    let strings = [text.as_ptr()];
    // All pointers stay valid through ReportEventW. Always release the handle,
    // retaining the reporting error before the cleanup can change GetLastError.
    let written = unsafe {
        ReportEventW(
            handle,
            EVENTLOG_ERROR_TYPE,
            0,
            1,
            ptr::null_mut(),
            1,
            0,
            strings.as_ptr(),
            ptr::null(),
        )
    };
    let write_error = std::io::Error::last_os_error();
    let closed = unsafe { DeregisterEventSource(handle) };
    let close_error = std::io::Error::last_os_error();
    ensure!(
        written != 0 && closed != 0,
        "Application event log: write={written} ({write_error}); release={closed} ({close_error})"
    );
    Ok(())
}

fn fallback_log(message: &str) -> Result<()> {
    let local =
        PathBuf::from(std::env::var_os("LOCALAPPDATA").context("LOCALAPPDATA is unavailable")?);
    ensure!(local.is_absolute(), "LOCALAPPDATA must be absolute");
    let directory = local.join("ratatoskr-skill-sync");
    fs::create_dir_all(&directory).context("Create fatal log directory")?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("background-fatal.log"))
        .context("Open fatal log")?;
    // JSON keeps the full error on one timestamped line, including newlines.
    writeln!(
        file,
        "{} {}",
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        serde_json::to_string(message)?
    )?;
    file.sync_all().context("Persist fatal log")
}

/// Attempt both durable channels. Record a channel failure in the surviving
/// channel; if neither can persist the error, return it to the nonzero exit path.
pub(super) fn report(message: &str) -> Result<()> {
    let event = event_log(message);
    let details = match &event {
        Ok(()) => message.to_owned(),
        Err(error) => format!("{message}; event reporting failed: {error:#}"),
    };
    let fallback = fallback_log(&details);
    match (event, fallback) {
        (_, Ok(())) => Ok(()), // Event failure, if any, is in the durable line.
        (Ok(()), Err(error)) => event_log(&format!("{message}; fallback log failed: {error:#}")),
        (Err(event), Err(fallback)) => {
            bail!("Event Log failed: {event:#}; fallback log failed: {fallback:#}")
        }
    }
}
