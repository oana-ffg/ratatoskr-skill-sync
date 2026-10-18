#![windows_subsystem = "windows"]

#[cfg(not(windows))]
compile_error!("ratatoskrw is a Windows-only scheduled-sync helper");

#[cfg(windows)]
fn main() {
    use clap::{Parser, Subcommand};
    use std::path::PathBuf;

    #[derive(Parser)]
    #[command(name = "ratatoskrw", version, about = "Ratatoskr background sync")]
    struct Cli {
        #[arg(long, global = true)]
        config: Option<PathBuf>,
        #[command(subcommand)]
        command: Action,
    }
    #[derive(Subcommand)]
    enum Action {
        Sync,
    }

    // try_parse avoids clap's process exit before failures can be persisted.
    let result = (|| -> anyhow::Result<()> {
        let cli = match Cli::try_parse() {
            Ok(cli) => cli,
            Err(error)
                if matches!(
                    error.kind(),
                    clap::error::ErrorKind::DisplayVersion | clap::error::ErrorKind::DisplayHelp
                ) =>
            {
                use std::io::Write;
                std::io::stdout().write_all(error.to_string().as_bytes())?;
                return Ok(());
            }
            Err(error) => return Err(error.into()),
        };
        let config = cli
            .config
            .map(Ok)
            .unwrap_or_else(ratatoskr_core::default_config_path)?;
        match cli.command {
            Action::Sync => ratatoskr_core::scheduled_sync(&config),
        }
    })();
    if let Err(error) = result {
        // scheduled_sync persists run failures. If it cannot even write its
        // diagnostics, show that failure instead of losing it without stderr.
        if !error.is::<ratatoskr_core::RecordedSyncError>() {
            let text: Vec<u16> = format!("Ratatoskr background sync: {error:#}\0")
                .encode_utf16()
                .collect();
            let title: Vec<u16> = "Ratatoskr Skill Sync\0".encode_utf16().collect();
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::MessageBoxW(
                    std::ptr::null_mut(),
                    text.as_ptr(),
                    title.as_ptr(),
                    windows_sys::Win32::UI::WindowsAndMessaging::MB_OK
                        | windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONERROR,
                );
            }
        }
        std::process::exit(2);
    }
}
