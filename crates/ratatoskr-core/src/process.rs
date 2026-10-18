use std::process::Command;

/// Captured child processes must never create a console in the background helper.
/// This flag is scoped to each child, so the interactive CLI remains a console app.
pub(crate) fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut command = command;
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
        command
    }
    #[cfg(not(windows))]
    command
}
