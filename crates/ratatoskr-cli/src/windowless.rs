#![windows_subsystem = "windows"]

#[cfg(not(windows))]
compile_error!("ratatoskrw is a Windows-only scheduled-sync helper");

#[cfg(windows)]
#[path = "windowless/fatal.rs"]
mod fatal;

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
        // scheduled_sync persists normal run failures. Fatal reporting must
        // remain non-interactive even if every diagnostics channel is broken.
        if !error.is::<ratatoskr_core::RecordedSyncError>()
            && let Err(reporting) = fatal::report(&format!("Ratatoskr background sync: {error:#}"))
        {
            // Useful when launched with inherited/redirected stderr. The
            // exit code still records failure when no channel is writable.
            eprintln!(
                "Ratatoskr background sync: {error:#}; fatal reporting failed: {reporting:#}"
            );
        }
        std::process::exit(2);
    }
}
