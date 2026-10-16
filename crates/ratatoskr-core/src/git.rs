use crate::{
    Plan, Settings, apply,
    config::{atomic_write, no_symlinks, relative},
    load, save,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug, Serialize)]
pub struct ProjectStatus {
    pub root: PathBuf,
    pub source: Option<PathBuf>,
    pub changed_files: usize,
    pub outgoing_commits: Option<usize>,
    pub upstream: Option<String>,
    pub error: Option<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    source: String,
}

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .context("Run Git")?;
    ensure!(
        output.status.success(),
        "Git: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output.stdout)
}

pub fn project_status(root: &Path) -> ProjectStatus {
    let mut status = ProjectStatus {
        root: root.into(),
        source: None,
        changed_files: 0,
        outgoing_commits: None,
        upstream: None,
        error: None,
    };
    let result: Result<()> = (|| {
        no_symlinks(root)?;
        let marker_path = root.join(".ratatoskr.toml");
        no_symlinks(&marker_path)?;
        let marker: Marker = toml::from_str(&fs::read_to_string(marker_path)?)?;
        let relative_source = relative(&marker.source)?;
        let source = root.join(relative_source);
        no_symlinks(&source)?;
        ensure!(source.is_dir(), "Project skill source is missing");
        status.source = Some(source);
        let top = String::from_utf8(git(root, &["rev-parse", "--show-toplevel"])?)?;
        ensure!(
            Path::new(top.trim()).canonicalize()? == root.canonicalize()?,
            "Register the Git repository root"
        );
        let changes = git(
            root,
            &[
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--",
                &marker.source,
                ".ratatoskr.toml",
            ],
        )?;
        // Rename records include an additional NUL-delimited path, so parse the status prefix.
        let mut records = changes.split(|b| *b == 0).filter(|r| !r.is_empty());
        while let Some(record) = records.next() {
            status.changed_files += 1;
            if record.first() == Some(&b'R')
                || record.get(1) == Some(&b'R')
                || record.first() == Some(&b'C')
                || record.get(1) == Some(&b'C')
            {
                records.next();
            }
        }
        if let Ok(upstream) = git(
            root,
            &[
                "rev-parse",
                "--abbrev-ref",
                "--symbolic-full-name",
                "@{upstream}",
            ],
        ) {
            status.upstream = Some(String::from_utf8(upstream)?.trim().into());
            let commits = git(
                root,
                &[
                    "log",
                    "--format=%H",
                    "@{upstream}..HEAD",
                    "--",
                    &marker.source,
                    ".ratatoskr.toml",
                ],
            )?;
            status.outgoing_commits = Some(String::from_utf8(commits)?.lines().count());
        }
        Ok(())
    })();
    if let Err(error) = result {
        status.error = Some(format!("{error:#}"));
    }
    status
}

pub fn add_project(config_path: &Path, root: &Path, source: &str) -> Result<ProjectStatus> {
    relative(source)?;
    let mut settings = load(config_path)?;
    let root = root.canonicalize()?;
    let top = String::from_utf8(git(&root, &["rev-parse", "--show-toplevel"])?)?;
    ensure!(
        Path::new(top.trim()).canonicalize()? == root,
        "Register the Git repository root"
    );
    let marker_path = root.join(".ratatoskr.toml");
    ensure!(
        root.join(source).is_dir(),
        "Project source directory does not exist"
    );
    let marker = Marker {
        source: source.into(),
    };
    if marker_path.try_exists()? {
        no_symlinks(&marker_path)?;
        let existing: Marker = toml::from_str(&fs::read_to_string(&marker_path)?)?;
        ensure!(
            existing.source == source,
            "Existing project marker points to another source"
        );
    } else {
        git(&root, &["rev-parse", "--show-toplevel"])?;
        atomic_write(&marker_path, toml::to_string_pretty(&marker)?.as_bytes())?;
    }
    let status = project_status(&root);
    ensure!(
        status.error.is_none(),
        "Project cannot be monitored: {:?}",
        status.error
    );
    if !settings.projects.contains(&root) {
        settings.projects.push(root);
    }
    save(config_path, &settings)?;
    Ok(status)
}

pub fn remove_project(config_path: &Path, root: &Path) -> Result<()> {
    let mut settings = load(config_path)?;
    ensure!(root.is_absolute(), "Use an absolute project root");
    crate::engine::detach_project(&settings, root)?;
    settings.projects.retain(|p| p != root);
    save(config_path, &settings)
}

pub fn sync(settings: &Settings) -> Result<Plan> {
    settings.validate()?;
    use fs2::FileExt;
    let state = settings.state_dir.join(&settings.machine);
    fs::create_dir_all(&state)?;
    let lock_path = state.join("sync.lock");
    no_symlinks(&lock_path)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)?;
    lock.try_lock_exclusive()
        .context("Another Ratatoskr sync is running")?;
    let result = sync_inner(settings);
    let error = result.as_ref().err().map(|e| format!("{e:#}"));
    if let Err(notification) = crate::notifications::outcome(settings, error.as_deref()) {
        return Err(anyhow::anyhow!(
            "{}; notification error: {notification:#}",
            error.unwrap_or_else(|| "Sync completed".into())
        ));
    }
    result
}

fn sync_inner(settings: &Settings) -> Result<Plan> {
    let status = git(
        &settings.source,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?;
    ensure!(
        status.is_empty(),
        "Source checkout has uncommitted files; refusing to pull"
    );
    git(&settings.source, &["rev-parse", "--verify", "@{upstream}"])?;
    git(&settings.source, &["pull", "--ff-only", "--no-rebase"])?;
    let plan = apply(settings)?;
    if let Some(updates) = &settings.updates
        && updates.automatic
        && crate::updates::standalone_helper()?
        && let Some(release) = crate::updates::check(updates, env!("CARGO_PKG_VERSION"))?
    {
        crate::updates::install(updates, &release)?;
    }
    Ok(plan)
}
