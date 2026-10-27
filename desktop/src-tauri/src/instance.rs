//! One desktop process per installation. The CLI reads the same lock to find the running
//! app, so it never has to ask the OS launcher, which would activate or reopen it.
use std::sync::Mutex;

static INSTANCE: Mutex<Option<ratatoskr_core::updates::DesktopInstance>> = Mutex::new(None);

/// Claim the instance. `false` means another desktop process holds it.
pub(crate) fn claim() -> anyhow::Result<bool> {
    let data = ratatoskr_core::updates::desktop_data_dir()?;
    let exe = std::env::current_exe()?;
    // macOS: a restarting predecessor may still be exiting; give it a moment. Windows
    // forwards a second launch at once through the single-instance plugin.
    let attempts = if cfg!(windows) { 1 } else { 50 };
    for attempt in 1..=attempts {
        if let Some(instance) = ratatoskr_core::updates::claim_desktop_instance(&data, &exe)? {
            *INSTANCE
                .lock()
                .map_err(|e| anyhow::anyhow!(e.to_string()))? = Some(instance);
            return Ok(true);
        }
        if attempt < attempts {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }
    Ok(false)
}

/// Release before relaunching so the successor can claim the instance at once.
pub(crate) fn release() {
    match INSTANCE.lock() {
        Ok(mut instance) => drop(instance.take()),
        Err(error) => eprintln!("Ratatoskr: release desktop instance: {error}"),
    }
}
