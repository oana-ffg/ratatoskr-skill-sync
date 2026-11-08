//! CI fixture: launch the real desktop from a short-lived launchd job.
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let desktop = args
        .next()
        .ok_or_else(|| anyhow::anyhow!("Desktop executable required"))?;
    ratatoskr_core::updates::start_desktop_hidden(std::path::Path::new(&desktop))?;
    if let Some(marker) = args.next() {
        std::fs::write(marker, "parent finished")?;
    }
    Ok(())
}
