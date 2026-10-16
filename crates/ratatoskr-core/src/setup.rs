use crate::{
    Settings,
    config::{atomic_write, identifier, no_symlinks},
    initialize,
};
use anyhow::{Context, Result, ensure};
use std::{collections::BTreeMap, fs, path::Path, process::Command};

pub fn create_content(config: &Path, source: &Path, machine: &str) -> Result<Settings> {
    identifier(machine)?;
    no_symlinks(source)?;
    ensure!(source.is_absolute(), "Choose an absolute content directory");
    ensure!(
        !source.try_exists()? || fs::read_dir(source)?.next().is_none(),
        "New content directory must be empty"
    );
    ensure!(!config.try_exists()?, "Local configuration already exists");
    let home = directories::BaseDirs::new()
        .context("Cannot find home directory")?
        .home_dir()
        .to_string_lossy()
        .into_owned();
    let os = std::env::consts::OS;
    let mut profile = toml::Table::new();
    profile.insert("home".into(), home.into());
    profile.insert("os".into(), os.into());
    profile.insert("mode".into(), "staging".into());
    profile.insert(
        "harnesses".into(),
        toml::Value::try_from(vec!["claude", "codex"])?,
    );
    let mut targets = BTreeMap::new();
    for preset in crate::agent_presets()
        .into_iter()
        .filter(|p| matches!(p.target.tag.as_str(), "claude" | "codex"))
    {
        targets.insert(
            preset.target.tag.clone(),
            crate::presets::target_definition(&preset.target)?,
        );
    }
    atomic_write(
        &source.join("harnesses.toml"),
        toml::to_string_pretty(&targets)?.as_bytes(),
    )?;
    atomic_write(
        &source.join("machines").join(format!("{machine}.toml")),
        toml::to_string_pretty(&profile)?.as_bytes(),
    )?;
    fs::create_dir_all(source.join("skills"))?;
    let git = Command::new("git")
        .arg("init")
        .arg("-b")
        .arg("main")
        .arg(source)
        .output()?;
    ensure!(
        git.status.success(),
        "Git initialization failed: {}",
        String::from_utf8_lossy(&git.stderr)
    );
    initialize(config, source, machine)
}

pub fn clone_content(
    config: &Path,
    remote: &str,
    destination: &Path,
    machine: &str,
) -> Result<Settings> {
    ensure!(
        !remote.is_empty() && !remote.starts_with('-') && !remote.chars().any(char::is_control),
        "Invalid Git remote"
    );
    no_symlinks(destination)?;
    ensure!(
        destination.is_absolute() && !destination.try_exists()?,
        "Clone destination must be a new absolute path"
    );
    ensure!(!config.try_exists()?, "Local configuration already exists");
    let output = Command::new("git")
        .args(["clone", "--single-branch", "--", remote])
        .arg(destination)
        .output()?;
    ensure!(
        output.status.success(),
        "Clone failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    initialize(config, destination, machine)
}
