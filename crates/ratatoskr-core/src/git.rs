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

impl ProjectStatus {
    pub fn needs_attention(&self) -> bool {
        self.changed_files != 0 || self.outgoing_commits != Some(0) || self.error.is_some()
    }
}

/// Local content state. Upstream counts use the last fetched reference; desktop
/// saves, Push now and sync contact the remote before acting on it.
#[derive(Clone, Debug, Serialize)]
pub struct ContentStatus {
    pub changed_files: usize,
    pub outgoing_commits: Option<usize>,
    pub upstream: Option<String>,
    pub error: Option<String>,
}
impl ContentStatus {
    pub fn needs_attention(&self) -> bool {
        self.changed_files != 0 || self.outgoing_commits != Some(0) || self.error.is_some()
    }
}

fn changed_file_count(bytes: &[u8]) -> usize {
    let mut records = bytes.split(|b| *b == 0).filter(|r| !r.is_empty());
    let mut count = 0;
    while let Some(record) = records.next() {
        count += 1;
        if record.iter().take(2).any(|b| matches!(b, b'R' | b'C')) {
            records.next();
        }
    }
    count
}

pub fn content_status(settings: &Settings) -> ContentStatus {
    let mut status = ContentStatus {
        changed_files: 0,
        outgoing_commits: None,
        upstream: None,
        error: None,
    };
    let result: Result<()> = (|| {
        no_symlinks(&settings.source)?;
        let top = String::from_utf8(git(&settings.source, &["rev-parse", "--show-toplevel"])?)?;
        ensure!(
            Path::new(top.trim()).canonicalize()? == settings.source.canonicalize()?,
            "Content source must be the Git checkout root"
        );
        status.changed_files = changed_file_count(&git(
            &settings.source,
            &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
        )?);
        status.upstream = Some(
            String::from_utf8(git(
                &settings.source,
                &[
                    "rev-parse",
                    "--abbrev-ref",
                    "--symbolic-full-name",
                    "@{upstream}",
                ],
            )?)?
            .trim()
            .into(),
        );
        status.outgoing_commits = Some(commit_count(settings, "@{upstream}..HEAD")?);
        validate_deploy_branch(settings)?;
        Ok(())
    })();
    if let Err(error) = result {
        status.error = Some(format!("{error:#}"));
    }
    status
}

fn commit_count(settings: &Settings, range: &str) -> Result<usize> {
    Ok(
        String::from_utf8(git(&settings.source, &["rev-list", "--count", range])?)?
            .trim()
            .parse()?,
    )
}

// Caller holds sync.lock then apply.lock throughout freshness, write and publication.
fn prepare_source_edit(settings: &Settings) -> Result<String> {
    let remote = validate_deploy_branch(settings)?;
    git(&settings.source, &["fetch", "--", &remote])
        .context("Fetch content before saving; source was not written")?;
    let clean = git(
        &settings.source,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?
    .is_empty();
    if clean && commit_count(settings, "HEAD..@{upstream}")? != 0 {
        git(
            &settings.source,
            &[
                "pull",
                "--ff-only",
                "--no-rebase",
                "--",
                &remote,
                &settings.deploy_branch,
            ],
        )
        .context("Fast-forward content before saving; source was not written")?;
    }
    Ok(remote)
}

fn source_fingerprints(
    paths: &[PathBuf],
) -> Result<std::collections::BTreeMap<PathBuf, (String, bool)>> {
    let mut files = std::collections::BTreeMap::new();
    for path in paths {
        no_symlinks(path)?;
        let entries = if path.is_dir() {
            crate::engine::file_paths(path)?.into_values().collect()
        } else if path.try_exists()? {
            vec![path.clone()]
        } else {
            vec![]
        };
        for file in entries {
            #[cfg(unix)]
            let executable = {
                use std::os::unix::fs::PermissionsExt;
                fs::metadata(&file)?.permissions().mode() & 0o111 != 0
            };
            #[cfg(not(unix))]
            let executable = false;
            files.insert(
                file.clone(),
                (crate::content::hash(&fs::read(file)?), executable),
            );
        }
    }
    Ok(files)
}

fn git_paths(root: &Path, args: &[&str], paths: &[PathBuf]) -> Result<Vec<u8>> {
    git_command(root, args, Some(paths))
}

pub(crate) fn publish_source_edit<T>(
    settings: &Settings,
    scopes: &[PathBuf],
    message: &str,
    write: impl FnOnce() -> Result<T>,
) -> Result<T> {
    let remote = prepare_source_edit(settings)?;
    let before = source_fingerprints(scopes)?;
    let result = write()?;
    let after = source_fingerprints(scopes)
        .context("Source saved but inspecting changed paths failed; NOT committed or pushed")?;
    let paths: std::collections::BTreeSet<_> = before
        .keys()
        .chain(after.keys())
        .filter(|p| before.get(*p) != after.get(*p))
        .cloned()
        .collect();
    let paths: Vec<_> = paths
        .iter()
        .map(|p| p.strip_prefix(&settings.source).map(Path::to_path_buf))
        .collect::<std::result::Result<_, _>>()?;
    if !paths.is_empty() {
        let commit: Result<()> = (|| {
            git_paths(&settings.source, &["add", "-A"], &paths)?;
            // --only excludes even unrelated entries already in the user's index.
            // A save that returns a file to HEAD needs no empty commit.
            if !git_paths(
                &settings.source,
                &["diff", "--cached", "--name-only"],
                &paths,
            )?
            .is_empty()
            {
                git_paths(
                    &settings.source,
                    &["commit", "--only", "-m", message],
                    &paths,
                )?;
            }
            Ok(())
        })();
        commit.context("Source saved in place but Git commit failed; NOT pushed")?;
    }
    push_to_upstream(settings, &remote).context("saved and committed locally but NOT pushed")?;
    Ok(result)
}

fn push_to_upstream(settings: &Settings, remote: &str) -> Result<()> {
    // Explicit refspec ignores push.default and remote.*.push customizations.
    git(
        &settings.source,
        &[
            "push",
            "--",
            remote,
            &format!("HEAD:refs/heads/{}", settings.deploy_branch),
        ],
    )?;
    Ok(())
}

pub fn push_content(settings: &Settings) -> Result<()> {
    settings.validate()?;
    let _sync = crate::config::operation_lock(settings, "sync.lock")?;
    let _apply = crate::config::operation_lock(settings, "apply.lock")?;
    let remote = validate_deploy_branch(settings)?;
    push_to_upstream(settings, &remote).context("saved and committed locally but NOT pushed")
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Marker {
    source: String,
}

#[derive(Debug)]
pub(crate) struct GitFailure {
    pub operation: String,
    message: String,
}
impl std::fmt::Display for GitFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Git: {}", self.message)
    }
}
impl std::error::Error for GitFailure {}

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>> {
    git_command(root, args, None)
}

fn git_command(root: &Path, args: &[&str], paths: Option<&[PathBuf]>) -> Result<Vec<u8>> {
    let mut command = crate::process::command("git");
    command
        .arg("--literal-pathspecs")
        .arg("-C")
        .arg(root)
        .args(args);
    if let Some(paths) = paths {
        command.arg("--").args(paths);
    }
    let output = command.output().context("Run Git")?;
    if !output.status.success() {
        return Err(GitFailure {
            operation: args.first().copied().unwrap_or("inspect").into(),
            message: String::from_utf8_lossy(&output.stderr).trim().into(),
        }
        .into());
    }
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
        status.changed_files = changed_file_count(&changes);
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
    let root = settings.registered_project(root)?;
    crate::engine::detach_project(&settings, &root)?;
    settings.projects.retain(|p| *p != root);
    save(config_path, &settings)
}

pub fn sync(settings: &Settings) -> Result<Plan> {
    sync_with_update(settings, || crate::updates::helper_update(settings))
}

fn sync_with_update(settings: &Settings, update: impl FnOnce() -> Result<()>) -> Result<Plan> {
    settings.validate()?;
    let state = settings.state_dir.join(&settings.machine);
    fs::create_dir_all(&state)?;
    let _lock = crate::config::operation_lock(settings, "sync.lock")?;
    sync_locked(settings, update, false)
}

/// The desktop owns helper updates and native notifications itself.
pub(crate) fn sync_in_process(settings: &Settings) -> Result<Option<Plan>> {
    settings.validate()?;
    let Some(_lock) = crate::config::try_operation_lock(settings, "sync.lock")? else {
        return Ok(None);
    };
    sync_locked(settings, || Ok(()), true).map(Some)
}

fn sync_locked(
    settings: &Settings,
    update: impl FnOnce() -> Result<()>,
    desktop: bool,
) -> Result<Plan> {
    let mut run = crate::jobs::start(settings)?;
    #[cfg(target_os = "macos")]
    let desktop_health = if desktop {
        Ok(())
    } else {
        crate::updates::desktop_watchdog(settings)
    };
    // Updating and syncing have separate outcomes and notification ledgers.
    let deployment = validate_deploy_branch(settings).map_err(|error| {
        if desktop {
            crate::tray::Problem::from_error("Deploy branch", &error).into()
        } else {
            error
        }
    });
    let update_result = if deployment.is_ok() { update() } else { Ok(()) };
    let result = deployment.and_then(|_| {
        let plan = sync_inner(settings).map_err(|error| {
            if desktop {
                crate::tray::Problem::from_error("Sync outputs", &error).into()
            } else {
                error
            }
        })?;
        if desktop && !plan.conflicts.is_empty() {
            return Err(crate::tray::Problem::message(
                "Sync conflicts",
                format!(
                    "Sync completed with conflicts: {}",
                    plan.conflicts.join("; ")
                ),
            )
            .into());
        }
        Ok(plan)
    });
    let sync_error = result.as_ref().err().map(|e| format!("{e:#}"));
    let notification = if desktop && matches!(settings.alert, Some(crate::AlertSettings::System)) {
        let mut alerts = settings.clone();
        alerts.alert = None;
        alerts.alert_command.clear();
        crate::notifications::outcome(&alerts, sync_error.as_deref())
    } else {
        crate::notifications::outcome(settings, sync_error.as_deref())
    };
    let mut errors = Vec::new();
    #[cfg(target_os = "macos")]
    if let Err(error) = desktop_health {
        errors.push(format!("Desktop update monitoring failed: {error:#}"));
    }
    if let Err(error) = update_result {
        errors.push(format!("Helper update failed: {error:#}"));
    }
    if let Some(error) = sync_error {
        errors.push(format!("Sync failed: {error}"));
    }
    if let Err(error) = &notification {
        errors.push(format!("Sync alert delivery failed: {error:#}"));
    }
    let summary = (!errors.is_empty()).then(|| errors.join("; "));
    let problem = if desktop {
        summary.as_ref().map(|summary| {
            let mut problem = if let Some(error) = result
                .as_ref()
                .err()
                .or_else(|| notification.as_ref().err())
            {
                crate::tray::Problem::from_error("Sync", error)
            } else {
                crate::tray::Problem::message("Sync", summary)
            };
            problem.message = summary.clone();
            problem
        })
    } else {
        None
    };
    run.problem_identity = problem.as_ref().map(|problem| problem.identity.clone());
    crate::jobs::finish(settings, run, summary.as_deref())?;
    if let Some(problem) = problem {
        return Err(problem.into());
    }
    ensure!(errors.is_empty(), "{}", errors.join("; "));
    result
}

fn sync_inner(settings: &Settings) -> Result<Plan> {
    let status = git(
        &settings.source,
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )?;
    if !status.is_empty() {
        return Err(crate::tray::Problem::message(
            "Source checkout dirty",
            "Source checkout has uncommitted files; refusing to pull",
        )
        .into());
    }
    git(&settings.source, &["rev-parse", "--verify", "@{upstream}"])?;
    git(&settings.source, &["pull", "--ff-only", "--no-rebase"])?;
    let outgoing = commit_count(settings, "@{upstream}..HEAD")?;
    if outgoing != 0 {
        return Err(crate::tray::Problem::message(
            "Content work pending",
            format!(
                "Content checkout has {outgoing} unpushed commits; use Push now in the desktop"
            ),
        )
        .into());
    }
    let plan = apply(settings)?;
    let pending: Vec<_> = plan
        .projects
        .iter()
        .filter(|project| project.needs_attention())
        .map(|project| {
            format!(
                "{} ({} uncommitted skill files, {} unpushed skill commits)",
                project.root.display(),
                project.changed_files,
                project
                    .outgoing_commits
                    .map_or("unknown: missing upstream".into(), |n| n.to_string())
            )
        })
        .collect();
    if !pending.is_empty() {
        return Err(crate::tray::Problem::message(
            "Project work pending",
            format!(
                "Outputs synced; project skill work needs attention: {}",
                pending.join("; ")
            ),
        )
        .into());
    }
    Ok(plan)
}

fn validate_deploy_branch(settings: &Settings) -> Result<String> {
    let top = String::from_utf8(git(&settings.source, &["rev-parse", "--show-toplevel"])?)?;
    ensure!(
        Path::new(top.trim()).canonicalize()? == settings.source.canonicalize()?,
        "Content source must be the Git checkout root"
    );
    git(
        &settings.source,
        &["check-ref-format", "--branch", &settings.deploy_branch],
    )?;
    let branch = String::from_utf8(
        git(
            &settings.source,
            &["symbolic-ref", "--quiet", "--short", "HEAD"],
        )
        .context("Content checkout is detached; refusing to pull or apply")?,
    )?;
    ensure!(
        branch.trim() == settings.deploy_branch,
        "Current content branch is {}; deploy_branch is {}; refusing to pull or apply",
        branch.trim(),
        settings.deploy_branch
    );
    let remote = String::from_utf8(
        git(
            &settings.source,
            &[
                "config",
                "--get",
                &format!("branch.{}.remote", settings.deploy_branch),
            ],
        )
        .context("Deploy branch has no configured remote; refusing to pull or apply")?,
    )?;
    ensure!(
        !remote.trim().is_empty() && remote.trim() != ".",
        "Deploy branch must track a remote branch"
    );
    let upstream = String::from_utf8(
        git(
            &settings.source,
            &["rev-parse", "--symbolic-full-name", "@{upstream}"],
        )
        .context("Deploy branch has no valid upstream; refusing to pull or apply")?,
    )?;
    let expected = format!("refs/remotes/{}/{}", remote.trim(), settings.deploy_branch);
    ensure!(
        upstream.trim() == expected,
        "Content upstream is {}; expected {}; refusing to pull or apply",
        upstream.trim(),
        expected
    );
    Ok(remote.trim().into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_update_still_syncs_and_alerts_independently() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let source = root.join("content");
        fs::create_dir_all(source.join("machines")).unwrap();
        fs::create_dir_all(source.join("skills/test")).unwrap();
        fs::write(source.join("skills/test/skill.toml"), "").unwrap();
        fs::write(source.join("harnesses.toml"), "[codex]\nskills_dir='skills'\nglobal_file='AGENTS.md'\n[codex.roots]\nmacos='{{af:home}}/.codex'\nwindows='{{af:home}}/.codex'\nlinux='{{af:home}}/.codex'\n").unwrap();
        fs::write(
            source.join("machines/test.toml"),
            "os='macos'\nmode='staging'\nharnesses=['codex']\n",
        )
        .unwrap();
        fs::write(
            source.join("skills/test/SKILL.md"),
            "---\nname: test\ndescription: Test sync\n---\nSynced content\n",
        )
        .unwrap();
        let settings = Settings {
            source: source.clone(),
            machine: "test".into(),
            state_dir: root.join("state"),
            alert_command: vec![
                "git".into(),
                "-C".into(),
                source.display().to_string(),
                "config".into(),
                "--add".into(),
                "ratatoskr.alert".into(),
            ],
            ..Default::default()
        };
        let remote = root.join("remote.git");
        fs::create_dir(&remote).unwrap();
        git(&remote, &["init", "--bare", "-b", "main"]).unwrap();
        git(&source, &["init", "-b", "main"]).unwrap();
        git(&source, &["config", "user.name", "Ratatoskr tests"]).unwrap();
        git(&source, &["config", "user.email", "tests@localhost"]).unwrap();
        git(&source, &["config", "commit.gpgsign", "false"]).unwrap();
        git(&source, &["add", "."]).unwrap();
        git(&source, &["commit", "-m", "Content"]).unwrap();
        git(&source, &["remote", "add", "origin", "../remote.git"]).unwrap();
        git(&source, &["push", "-u", "origin", "main"]).unwrap();
        let alerts = || {
            String::from_utf8(git(&source, &["config", "--get-all", "ratatoskr.alert"]).unwrap())
                .unwrap()
        };
        let run = |date: &str| {
            sync_with_update(&settings, || {
                crate::updates::helper_update_with(
                    &settings,
                    chrono::DateTime::parse_from_rfc3339(date).unwrap(),
                    || anyhow::bail!("update endpoint offline"),
                )
            })
        };
        let error = run("2026-10-16T00:00:00+02:00").unwrap_err().to_string();
        assert!(error.contains("update endpoint offline"));
        assert!(!error.contains("Sync failed"), "{error}");
        let output = settings
            .state_dir
            .join("test/staging/codex/skills/test/SKILL.md");
        assert!(
            fs::read_to_string(&output)
                .unwrap()
                .contains("Synced content")
        );
        let update_alert = fs::read(
            settings
                .state_dir
                .join("test/helper-update-notification.json"),
        )
        .unwrap();
        assert!(alerts().contains("Helper update: update endpoint offline"));
        assert!(
            crate::notifications::current_error(&settings, "notification.json")
                .unwrap()
                .is_none()
        );

        // Skipped checks neither repeat the alert nor falsely report recovery.
        run("2026-10-16T01:00:00+02:00").unwrap();
        assert_eq!(alerts().lines().count(), 1);
        assert_eq!(
            fs::read(
                settings
                    .state_dir
                    .join("test/helper-update-notification.json")
            )
            .unwrap(),
            update_alert
        );
        // The same failure on a later attempt is deduplicated too.
        assert!(run("2026-10-17T00:00:00+02:00").is_err());
        assert_eq!(alerts().lines().count(), 1);

        fs::write(source.join("dirty.txt"), "Uncommitted").unwrap();
        let error = run("2026-10-18T00:00:00+02:00").unwrap_err().to_string();
        assert!(error.contains("update endpoint offline"));
        assert!(error.contains("uncommitted files"));
        assert_eq!(alerts().lines().count(), 2);
        assert!(alerts().contains("Source checkout has uncommitted files"));
        assert_eq!(
            fs::read(
                settings
                    .state_dir
                    .join("test/helper-update-notification.json")
            )
            .unwrap(),
            update_alert
        );

        // Even failed update-alert delivery must not prevent applying synced content.
        fs::remove_file(source.join("dirty.txt")).unwrap();
        let mut undeliverable = settings.clone();
        undeliverable.alert_command = vec!["git".into(), "ratatoskr-invalid-command".into()];
        undeliverable.state_dir = root.join("undeliverable-state");
        let error = sync_with_update(&undeliverable, || {
            crate::updates::helper_update_with(
                &undeliverable,
                chrono::DateTime::parse_from_rfc3339("2026-10-19T00:00:00+02:00").unwrap(),
                || anyhow::bail!("offline"),
            )
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("alert delivery failed"));
        assert!(
            undeliverable
                .state_dir
                .join("test/staging/codex/skills/test/SKILL.md")
                .exists()
        );
    }
}
