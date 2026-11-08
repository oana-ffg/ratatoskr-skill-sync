#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub(crate) use macos::*;
#[cfg(not(target_os = "macos"))]
mod resident;
#[cfg(not(target_os = "macos"))]
pub(crate) use resident::*;
