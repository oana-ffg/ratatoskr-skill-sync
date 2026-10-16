use anyhow::{Context, Result, bail, ensure};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub source: PathBuf,
    pub machine: String,
    pub state_dir: PathBuf,
    #[serde(default)]
    pub projects: Vec<PathBuf>,
    #[serde(default)]
    pub alert_command: Vec<String>,
    #[serde(default)]
    pub updates: crate::updates::UpdateSettings,
    #[serde(default = "default_deploy_branch")]
    pub deploy_branch: String,
}

fn default_deploy_branch() -> String {
    "main".into()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            source: PathBuf::new(),
            machine: String::new(),
            state_dir: PathBuf::new(),
            projects: vec![],
            alert_command: vec![],
            updates: Default::default(),
            deploy_branch: default_deploy_branch(),
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.source.is_absolute() && self.state_dir.is_absolute(),
            "Source and state paths must be absolute"
        );
        identifier(&self.machine)?;
        no_symlinks(&self.source)?;
        no_symlinks(&self.state_dir)?;
        ensure!(self.source.is_dir(), "Source checkout does not exist");
        ensure!(
            !overlaps(&self.source, &self.state_dir),
            "Source and state directories must not overlap"
        );
        for project in &self.projects {
            ensure!(project.is_absolute(), "Project paths must be absolute");
        }
        ensure!(
            !self.deploy_branch.is_empty(),
            "Deploy branch must not be empty"
        );
        self.updates.validate()?;
        Ok(())
    }
}

pub fn default_config_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("RATATOSKR_CONFIG") {
        let path = PathBuf::from(path);
        ensure!(
            path.is_absolute(),
            "RATATOSKR_CONFIG must be an absolute path"
        );
        return Ok(path);
    }
    Ok(ProjectDirs::from("", "", "ratatoskr-skill-sync")
        .context("Cannot find the current user's configuration directory")?
        .config_dir()
        .join("config.toml"))
}

pub fn load(path: &Path) -> Result<Settings> {
    let settings: Settings = toml::from_str(
        &fs::read_to_string(path)
            .with_context(|| format!("Read configuration {}", path.display()))?,
    )?;
    no_symlinks(path)?;
    settings.validate()?;
    Ok(settings)
}

pub fn initialize(path: &Path, source: &Path, machine: &str) -> Result<Settings> {
    ensure!(
        !path.try_exists()?,
        "Configuration already exists: {}",
        path.display()
    );
    identifier(machine)?;
    let source = source
        .canonicalize()
        .context("Source checkout does not exist")?;
    ensure!(
        source
            .join("machines")
            .join(format!("{machine}.toml"))
            .is_file(),
        "Source has no profile for machine {machine}"
    );
    let state_dir = ProjectDirs::from("", "", "ratatoskr-skill-sync")
        .context("Cannot find the current user's data directory")?
        .data_local_dir()
        .join("state");
    let settings = Settings {
        source,
        machine: machine.into(),
        state_dir,
        projects: vec![],
        ..Settings::default()
    };
    crate::plan(&settings)?;
    save(path, &settings)?;
    Ok(settings)
}

pub fn save(path: &Path, settings: &Settings) -> Result<()> {
    settings.validate()?;
    no_symlinks(path)?;
    atomic_write(path, toml::to_string_pretty(settings)?.as_bytes())
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    no_symlinks(path)?;
    let parent = path.parent().context("File has no parent directory")?;
    fs::create_dir_all(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path)
        .with_context(|| format!("Replace {}", path.display()))?;
    Ok(())
}

pub(crate) fn identifier(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= 64
            && !value.starts_with('-')
            && !value.ends_with('-')
            && !value.contains("--")
            && value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'),
        "Invalid identifier: {value:?}"
    );
    Ok(())
}

pub(crate) fn relative(value: &str) -> Result<PathBuf> {
    ensure!(
        !value.is_empty() && !value.contains(['\\', ':']),
        "Expected portable relative path: {value:?}"
    );
    for part in value.split('/') {
        ensure!(
            !part.is_empty()
                && part != "."
                && part != ".."
                && !part.ends_with(['.', ' '])
                && !part
                    .chars()
                    .any(|c| c.is_control() || "<>\"|?*".contains(c)),
            "Unsafe filename: {value:?}"
        );
        let stem = part
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        ensure!(
            !matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
                && !(stem.len() == 4
                    && (stem.starts_with("COM") || stem.starts_with("LPT"))
                    && matches!(stem.as_bytes()[3], b'1'..=b'9')),
            "Windows reserved filename: {value:?}"
        );
    }
    let path = PathBuf::from(value);
    ensure!(
        path.components().all(|c| matches!(c, Component::Normal(_))),
        "Expected relative path"
    );
    Ok(path)
}

pub(crate) fn no_symlinks(path: &Path) -> Result<()> {
    ensure!(
        !path.components().any(|c| matches!(c, Component::ParentDir)),
        "Parent traversal is not allowed: {}",
        path.display()
    );
    let mut current = PathBuf::new();
    for part in path.components() {
        current.push(part);
        // A Windows drive/UNC prefix is not a complete filesystem path until its root follows.
        if matches!(part, Component::Prefix(_)) {
            continue;
        }
        match fs::symlink_metadata(&current) {
            Ok(meta) => ensure!(
                !meta.file_type().is_symlink(),
                "Symlinks are not allowed: {}",
                current.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
            Err(error) => bail!("Inspect {}: {error}", current.display()),
        }
    }
    Ok(())
}

pub(crate) fn overlaps(a: &Path, b: &Path) -> bool {
    let folded = |path: &Path| {
        let text = path.to_string_lossy();
        #[cfg(windows)]
        let text = if let Some(share) = text.strip_prefix("\\\\?\\UNC\\") {
            format!("\\\\{share}")
        } else {
            text.strip_prefix("\\\\?\\").unwrap_or(&text).to_owned()
        };
        PathBuf::from(text.to_lowercase())
    };
    let a = folded(a);
    let b = folded(b);
    a.starts_with(&b) || b.starts_with(&a)
}

pub(crate) fn state_lock(settings: &Settings, name: &str) -> Result<fs::File> {
    let path = settings.state_dir.join(&settings.machine).join(name);
    no_symlinks(&path)?;
    fs::create_dir_all(path.parent().context("State lock has no parent")?)?;
    Ok(fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)?)
}

pub(crate) struct OperationLock {
    file: fs::File,
    name: String,
}

impl Drop for OperationLock {
    fn drop(&mut self) {
        // Forked processes can briefly retain the descriptor even with close-on-exec.
        if let Err(error) = fs2::FileExt::unlock(&self.file) {
            eprintln!("Ratatoskr: release {}: {error}", self.name);
        }
    }
}

pub(crate) fn operation_lock(settings: &Settings, name: &str) -> Result<OperationLock> {
    let file = state_lock(settings, name)?;
    fs2::FileExt::try_lock_exclusive(&file)
        .with_context(|| format!("Another Ratatoskr operation holds {name}"))?;
    Ok(OperationLock {
        file,
        name: name.into(),
    })
}

#[cfg(test)]
mod lock_tests {
    #[test]
    fn operation_guard_unlocks_even_if_a_descriptor_was_inherited() {
        let temp = tempfile::tempdir().unwrap();
        let settings = super::Settings {
            state_dir: temp.path().canonicalize().unwrap(),
            machine: "test".into(),
            ..Default::default()
        };
        let guard = super::operation_lock(&settings, "apply.lock").unwrap();
        let inherited = guard.file.try_clone().unwrap();
        assert!(super::operation_lock(&settings, "apply.lock").is_err());
        drop(guard);
        assert!(super::operation_lock(&settings, "apply.lock").is_ok());
        drop(inherited);
    }
}
