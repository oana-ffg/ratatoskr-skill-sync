mod rendering;
pub(crate) use rendering::render_global_instructions;
use rendering::{
    Output, RenderScope, SkillSources, desired, ensure_skill_name_allowed, render_scope_inner,
    render_skill_file, skill_file_bytes, skill_sources,
};
mod conflicts;
pub(crate) use conflicts::resolve_conflict_locked;
mod stale;
mod validation;
use crate::{
    Settings,
    config::{atomic_write, identifier, no_symlinks, overlaps, relative},
    git::{ProjectStatus, project_status},
    protection::{self, Protection},
    template::{render, selectors_match},
};
use anyhow::{Context, Result, ensure};
pub use conflicts::{
    Conflict, ConflictAction, ResolutionPreview, preview_resolution, preview_source_edit,
    read_conflict_source, resolve_conflict,
};
use conflicts::{conflict, write_rendered};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
pub use stale::{
    StaleAction, StaleFile, StaleLocation, StalePreview, preview_stale_target,
    resolve_stale_target, stale_targets,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use validation::RenderDiagnostics;
pub(crate) use validation::validate_focus;
pub use validation::{
    HarnessValidation, ProfileValidation, ValidationContext, ValidationProblem, ValidationReport,
    ValidationWarning, validate_source,
};
use walkdir::WalkDir;

#[derive(Clone, Debug, Serialize)]
pub struct Change {
    pub path: PathBuf,
    pub action: String,
    pub diff: Option<String>,
    pub before_sha256: Option<String>,
    pub after_sha256: Option<String>,
    pub executable: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
pub struct InstallTarget {
    pub harness: String,
    pub skills_root: PathBuf,
    pub instructions_file: PathBuf,
    pub project: Option<PathBuf>,
}

/// Writes performed by the `apply` call that returned this plan.
/// `plan` and `status` leave [`Plan::applied`] unset.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct AppliedChanges {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

impl AppliedChanges {
    fn from_changes(changes: &[Change]) -> Self {
        let count = |action: &str| {
            changes
                .iter()
                .filter(|change| change.action == action)
                .count()
        };
        Self {
            added: count("add"),
            updated: count("update"),
            removed: count("remove"),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Plan {
    pub source: PathBuf,
    pub machine: String,
    pub mode: String,
    pub skills: usize,
    pub targets: usize,
    pub install_targets: Vec<InstallTarget>,
    pub staging_root: Option<PathBuf>,
    pub changes: Vec<Change>,
    pub conflicts: Vec<String>,
    pub conflict_details: Vec<Conflict>,
    pub stale_targets: Vec<StaleLocation>,
    pub projects: Vec<ProjectStatus>,
    pub content_status: crate::ContentStatus,
    pub revision: String,
    /// Counts from the writes this apply performed under its lock.
    /// Absent for `plan` and `status`, which do not write.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub applied: Option<AppliedChanges>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Fingerprint {
    sha256: String,
    executable: bool,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    source: PathBuf,
    machine: String,
    files: BTreeMap<PathBuf, Fingerprint>,
    roots: BTreeSet<PathBuf>,
    #[serde(default)]
    project_roots: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    /// Harness identity captured when targets are applied or adopted.
    #[serde(default)]
    target_harnesses: BTreeMap<PathBuf, String>,
}

impl Manifest {
    fn retain_owned_targets(&mut self) {
        self.target_harnesses.retain(|target, _| {
            self.files.keys().any(|path| path.starts_with(target))
                || self.roots.iter().any(|root| root.parent() == Some(target))
        });
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Harness {
    skills_dir: String,
    #[serde(default)]
    global_root: Option<String>,
    global_file: String,
    roots: BTreeMap<String, String>,
    #[serde(default)]
    project_root: Option<String>,
}

struct Rendered {
    harness: String,
    skill: Option<String>,
    source: PathBuf,
    reversible: bool,
    bytes: Vec<u8>,
    executable: bool,
}

struct Desired {
    files: BTreeMap<PathBuf, Rendered>,
    roots: BTreeSet<PathBuf>,
    allowed: Vec<(PathBuf, PathBuf)>,
    install_targets: Vec<InstallTarget>,
    mode: String,
    skills: usize,
    targets: usize,
    project_roots: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
    /// Rendering needs each project's status; `preflight` reports the same one.
    project_statuses: Vec<ProjectStatus>,
}

impl Desired {
    fn target_harnesses(&self) -> BTreeMap<PathBuf, String> {
        self.install_targets
            .iter()
            .flat_map(|target| {
                [target.skills_root.clone(), target.instructions_file.clone()]
                    .into_iter()
                    .map(|path| (path, target.harness.clone()))
            })
            .collect()
    }
}

fn read_toml(path: &Path) -> Result<toml::Table> {
    no_symlinks(path)?;
    toml::from_str(&fs::read_to_string(path).with_context(|| format!("Read {}", path.display()))?)
        .map_err(Into::into)
}

fn string<'a>(table: &'a toml::Table, key: &str) -> Result<&'a str> {
    table
        .get(key)
        .and_then(|v| v.as_str())
        .with_context(|| format!("Missing string: {key}"))
}

fn runtime(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == "__pycache__")
        || path.file_name().is_some_and(|n| n == ".DS_Store")
        || path.extension().is_some_and(|n| n == "pyc")
}

/// Finder and Python drop these beside real skill directories. A symlink keeps its
/// own error even when its name looks like a runtime artifact.
pub(crate) fn skills_root_runtime_file(entry: &fs::DirEntry) -> Result<bool> {
    let kind = entry.file_type()?;
    Ok(runtime(&entry.path()) && !kind.is_dir() && !kind.is_symlink())
}

pub(crate) fn variant_runtime_entry(entry: &fs::DirEntry) -> Result<bool> {
    Ok(skills_root_runtime_file(entry)?
        || (entry.file_name() == "__pycache__" && entry.file_type()?.is_dir()))
}

pub(crate) fn require_skill_directory(entry: &fs::DirEntry) -> Result<()> {
    let kind = entry.file_type()?;
    let name = entry.file_name().to_string_lossy().into_owned();
    ensure!(!kind.is_symlink(), "Skills root entry is a symlink: {name}");
    ensure!(
        kind.is_dir(),
        "Skills root must contain directories only: {name}"
    );
    Ok(())
}

pub(crate) fn file_paths(root: &Path) -> Result<BTreeMap<PathBuf, PathBuf>> {
    let mut files = BTreeMap::new();
    let mut cases = BTreeSet::new();
    if !root.try_exists()? {
        return Ok(files);
    }
    no_symlinks(root)?;
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry?;
        ensure!(
            !entry.file_type().is_symlink(),
            "Source symlink: {}",
            entry.path().display()
        );
        let relative_path = entry.path().strip_prefix(root)?;
        if relative_path.as_os_str().is_empty() {
            continue;
        }
        if runtime(relative_path) {
            continue;
        }
        let portable = relative_path
            .to_str()
            .context("Non-UTF8 source filename")?
            .replace(std::path::MAIN_SEPARATOR, "/");
        relative(&portable)?;
        ensure!(
            cases.insert(portable.to_lowercase()),
            "Case-insensitive source collision: {portable}"
        );
        if entry.file_type().is_file() {
            files.insert(relative_path.to_path_buf(), entry.path().to_path_buf());
        } else {
            ensure!(entry.file_type().is_dir(), "Unsupported source file type");
        }
    }
    Ok(files)
}

fn executable(path: &Path) -> Result<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Ok(fs::metadata(path)?.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(false)
    }
}

fn fingerprint(bytes: &[u8], executable: bool) -> Fingerprint {
    Fingerprint {
        sha256: format!("{:x}", Sha256::digest(bytes)),
        executable,
    }
}

/// Unsaved text standing in for a skill's `SKILL.md` and `skill.toml`. Rendering it never
/// touches the disk: only the skill's other files are read.
#[derive(Clone, Copy)]
pub(crate) struct SkillDraft<'a> {
    pub markdown: &'a str,
    pub metadata: &'a str,
}

/// Limits validation to what one edit can affect, so it does not render the whole library.
#[derive(Clone, Copy)]
pub(crate) enum ValidationFocus<'a> {
    Skill {
        name: &'a str,
        draft: Option<SkillDraft<'a>>,
    },
    GlobalInstructions {
        draft: Option<&'a str>,
    },
}

pub(crate) fn validate_sources(settings: &Settings) -> Result<()> {
    desired(settings).map(|_| ())
}

fn host_os() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macos",
        "windows" => "windows",
        _ => "linux",
    }
}

fn manifest_path(settings: &Settings) -> PathBuf {
    settings
        .state_dir
        .join(&settings.machine)
        .join("manifest.json")
}

fn manifest(settings: &Settings) -> Result<Manifest> {
    let path = manifest_path(settings);
    no_symlinks(&path)?;
    if !path.try_exists()? {
        return Ok(Manifest {
            source: settings.source.clone(),
            machine: settings.machine.clone(),
            ..Manifest::default()
        });
    }
    let manifest: Manifest = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        manifest.source == settings.source && manifest.machine == settings.machine,
        "Ownership manifest belongs to another source or machine"
    );
    for path in manifest.files.keys().chain(manifest.roots.iter()) {
        ensure!(
            path.is_absolute()
                && !path.components().any(|c| matches!(
                    c,
                    std::path::Component::ParentDir | std::path::Component::CurDir
                )),
            "Invalid ownership path: {}",
            path.display()
        );
        no_symlinks(path)?;
    }
    Ok(manifest)
}

fn stale_ownership_message(machine: &str, location: &Path) -> String {
    format!(
        "Pending stale-target decision for machine `{machine}` at {}. This location is outside current targets after removing a harness, switching staging/live mode, or changing roots. Run `ratatoskr stale-target list`, then `ratatoskr stale-target preview <path> --action forget|remove` and `ratatoskr stale-target resolve --preview <file>`. Nothing is forgotten or removed automatically.",
        location.display()
    )
}

fn preflight(settings: &Settings, desired: &Desired, manifest: &Manifest) -> Result<Plan> {
    protection::capture(&manifest_path(settings))?;
    let stale_targets = stale::locations(desired, manifest)?;
    // Stale ownership requires its own explicit decision; it is never a normal remove.
    let active = stale::active_manifest(manifest, desired);
    let manifest = &active;
    let mut revision = Sha256::new();
    revision.update(serde_json::to_vec(&stale_targets)?);
    let mut conflicts: Vec<String> = stale_targets
        .iter()
        .map(|location| stale_ownership_message(&settings.machine, &location.path))
        .collect();
    let mut conflict_details = Vec::new();
    let mut changes = Vec::new();
    for root in &desired.roots {
        no_symlinks(root)?;
        if root.try_exists()? && !manifest.roots.contains(root) {
            conflict_details.push(conflict(root, "unmanaged-directory", desired)?);
            conflicts.push(format!(
                "Pre-existing unmanaged skill directory: {}",
                root.display()
            ));
        }
    }
    for root in &manifest.roots {
        no_symlinks(root)?;
        if root.try_exists()? {
            for entry in WalkDir::new(root).follow_links(false) {
                let entry = entry?;
                ensure!(
                    !entry.file_type().is_symlink(),
                    "Output symlink: {}",
                    entry.path().display()
                );
                if entry.file_type().is_file()
                    && !runtime(entry.path())
                    && !manifest.files.contains_key(entry.path())
                {
                    conflict_details.push(conflict(entry.path(), "unmanaged-collision", desired)?);
                    conflicts.push(format!(
                        "Unmanaged file in owned skill: {}",
                        entry.path().display()
                    ));
                }
            }
        }
    }
    let paths: BTreeSet<_> = desired.files.keys().chain(manifest.files.keys()).collect();
    for path in paths {
        no_symlinks(path)?;
        revision.update(path.to_string_lossy().as_bytes());
        revision.update(serde_json::to_vec(&protection::capture(path)?)?);
        let existing = if path.try_exists()? {
            Some(fs::read(path).with_context(|| format!("Read output {}", path.display()))?)
        } else {
            None
        };
        let current = existing
            .as_ref()
            .map(|b| executable(path).map(|x| fingerprint(b, x)))
            .transpose()?;
        if let Some(owned) = manifest.files.get(path) {
            if current.as_ref() != Some(owned) {
                let kind = match &current {
                    None => "removed",
                    Some(now) if now.sha256 == owned.sha256 => "permission-change",
                    Some(_) => "hand-edited",
                };
                conflict_details.push(conflict(path, kind, desired)?);
                conflicts.push(format!(
                    "Owned file was edited, removed, or changed permissions: {}",
                    path.display()
                ));
            }
        } else if current.is_some() {
            if !conflict_details.iter().any(|c| c.path == *path) {
                conflict_details.push(conflict(path, "unmanaged-collision", desired)?);
            }
            conflicts.push(format!("Pre-existing unmanaged file: {}", path.display()));
        }
        let next = desired.files.get(path);
        let next_hash = next.map(|f| fingerprint(&f.bytes, f.executable));
        if current != next_hash {
            let action = if next.is_none() {
                "remove"
            } else if current.is_none() {
                "add"
            } else {
                "update"
            };
            let before = existing.as_deref().unwrap_or_default();
            let after = next.map(|f| f.bytes.as_slice()).unwrap_or_default();
            let diff = if before.len() + after.len() < 131_072 {
                match (std::str::from_utf8(before), std::str::from_utf8(after)) {
                    (Ok(before), Ok(after)) => Some(
                        similar::TextDiff::from_lines(before, after)
                            .unified_diff()
                            .header("installed", "rendered")
                            .to_string(),
                    ),
                    _ => None,
                }
            } else {
                None
            };
            changes.push(Change {
                path: path.clone(),
                action: action.into(),
                diff,
                before_sha256: current.as_ref().map(|f| f.sha256.clone()),
                after_sha256: next_hash.as_ref().map(|f| f.sha256.clone()),
                executable: next_hash.as_ref().map(|f| f.executable),
            });
        }
    }
    if desired.mode == "live" {
        for duplicate in crate::legacy_duplicates(settings)? {
            conflicts.push(format!("Codex still scans a legacy copy of {} at {}. Choose whether to back up and retire it or keep both before Apply.", duplicate.name, duplicate.path.display()));
        }
    }
    Ok(Plan {
        content_status: crate::content_status(settings),
        source: settings.source.clone(),
        machine: settings.machine.clone(),
        mode: desired.mode.clone(),
        skills: desired.skills,
        targets: desired.targets,
        install_targets: desired.install_targets.clone(),
        staging_root: (desired.mode == "staging")
            .then(|| settings.state_dir.join(&settings.machine).join("staging")),
        changes,
        conflicts,
        conflict_details,
        stale_targets,
        projects: desired.project_statuses.clone(),
        revision: format!("{:x}", revision.finalize()),
        applied: None,
    })
}

pub fn plan(settings: &Settings) -> Result<Plan> {
    let desired = desired(settings)?;
    let manifest = manifest(settings)?;
    preflight(settings, &desired, &manifest)
}

pub fn apply(settings: &Settings) -> Result<Plan> {
    apply_inner(settings, None)
}

pub fn apply_reviewed(settings: &Settings, expected: &str) -> Result<Plan> {
    apply_inner(settings, Some(serde_json::from_str(expected)?))
}

/// A reviewed plan was overtaken by another operation. No writes were made.
#[derive(Debug)]
pub struct PlanChanged {
    pub plan: Box<Plan>,
}

impl std::fmt::Display for PlanChanged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Workspace changed since preview. Review the new plan.")
    }
}
impl std::error::Error for PlanChanged {}

fn apply_inner(settings: &Settings, expected: Option<serde_json::Value>) -> Result<Plan> {
    settings.validate()?;
    let _lock = crate::config::operation_lock(settings, "apply.lock")?;
    apply_locked(settings, expected)
}

/// Caller validated `settings` and holds apply.lock (and, for Save, sync.lock before it).
pub(crate) fn apply_locked(
    settings: &Settings,
    expected: Option<serde_json::Value>,
) -> Result<Plan> {
    no_symlinks(&settings.state_dir)?;
    let state = settings.state_dir.join(&settings.machine);
    fs::create_dir_all(&state)?;
    let desired = desired(settings)?;
    let old = manifest(settings)?;
    let preflight = preflight(settings, &desired, &old)?;
    // Count the plan computed under the lock. A later unlocked `plan` can disagree if the
    // checkout changes between the two calls.
    let applied = AppliedChanges::from_changes(&preflight.changes);
    let nothing_to_write = preflight.changes.is_empty();
    if let Some(expected) = expected
        && serde_json::to_value(&preflight)? != expected
    {
        return Err(PlanChanged {
            plan: Box::new(preflight),
        }
        .into());
    }
    ensure!(
        preflight.conflicts.is_empty(),
        "Refusing all writes because of conflicts:\n{}",
        preflight.conflicts.join("\n")
    );
    // Recheck immediately before each write. Per-file atomicity does not imply a tree transaction.
    for change in &preflight.changes {
        no_symlinks(&change.path)?;
        if let Some(owned) = old.files.get(&change.path) {
            ensure!(
                fingerprint(&fs::read(&change.path)?, executable(&change.path)?) == *owned,
                "Output changed during apply"
            );
        } else {
            ensure!(!change.path.try_exists()?, "Output appeared during apply");
        }
        if let Some(file) = desired.files.get(&change.path) {
            write_rendered(&change.path, &file.bytes, file.executable)?;
        } else {
            protection::capture(&change.path)?;
            fs::remove_file(&change.path)?;
        }
    }
    let mut roots = desired.roots.clone();
    for root in old.roots.difference(&desired.roots) {
        for entry in WalkDir::new(root).contents_first(true).follow_links(false) {
            let entry = entry?;
            if entry.file_type().is_dir() {
                match fs::remove_dir(entry.path()) {
                    Ok(()) => (),
                    Err(error) if error.kind() == std::io::ErrorKind::DirectoryNotEmpty => (),
                    Err(error) => return Err(error).context("Remove empty owned directory"),
                }
            }
        }
        if root.try_exists()? {
            roots.insert(root.clone());
        }
    }
    let new = Manifest {
        source: settings.source.clone(),
        machine: settings.machine.clone(),
        roots,
        target_harnesses: desired.target_harnesses(),
        project_roots: desired.project_roots,
        files: desired
            .files
            .iter()
            .map(|(p, f)| (p.clone(), fingerprint(&f.bytes, f.executable)))
            .collect(),
    };
    let path = manifest_path(settings);
    let serialized = serde_json::to_vec_pretty(&new)?;
    // Background sync runs every minute and almost always changes nothing. Then the
    // preflight above is already the report, and there is nothing to write or re-render.
    if nothing_to_write && path.try_exists()? && fs::read(&path)? == serialized {
        let mut report = preflight;
        report.applied = Some(applied);
        return Ok(report);
    }
    atomic_write(&path, &serialized)?;
    let mut report = plan(settings)?;
    report.applied = Some(applied);
    Ok(report)
}

pub(crate) fn detach_project(settings: &Settings, project: &Path) -> Result<()> {
    let path = manifest_path(settings);
    no_symlinks(&path)?;
    if !path.try_exists()? {
        return Ok(());
    }
    let _lock = crate::config::operation_lock(settings, "apply.lock")?;
    let mut owned: Manifest = serde_json::from_slice(&fs::read(&path)?)?;
    ensure!(
        owned.source == settings.source && owned.machine == settings.machine,
        "Ownership belongs to another workspace"
    );
    if let Some(roots) = owned.project_roots.remove(project) {
        owned
            .files
            .retain(|path, _| !roots.iter().any(|root| path.starts_with(root)));
        owned.roots.retain(|root| !roots.contains(root));
        owned.retain_owned_targets();
        atomic_write(&path, &serde_json::to_vec_pretty(&owned)?)?;
    }
    Ok(())
}

#[derive(Deserialize, Serialize)]
struct Backup {
    manifest: Manifest,
    files: BTreeMap<PathBuf, String>,
    #[serde(default)]
    ownership: Option<Manifest>,
    #[serde(default)]
    absent: BTreeSet<PathBuf>,
    #[serde(default)]
    sources: BTreeMap<PathBuf, SourceBackup>,
    #[serde(default)]
    protections: BTreeMap<PathBuf, Protection>,
}

#[derive(Deserialize, Serialize)]
struct SourceBackup {
    stored: String,
    fingerprint: Fingerprint,
    after_fingerprint: Fingerprint,
    #[serde(default)]
    protection: Option<Protection>,
}

fn backup(settings: &Settings, manifest: &Manifest) -> Result<PathBuf> {
    let mut protections = BTreeMap::new();
    for path in manifest.files.keys() {
        if let Some(protection) = protection::capture(path)? {
            protections.insert(path.clone(), protection);
        }
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let directory = settings
        .state_dir
        .join(&settings.machine)
        .join("backups")
        .join(nonce.to_string());
    no_symlinks(&directory)?;
    protection::capture(&directory.join("snapshot.json"))?;
    let mut files = BTreeMap::new();
    for (index, (path, expected)) in manifest.files.iter().enumerate() {
        no_symlinks(path)?;
        let bytes = fs::read(path)?;
        ensure!(
            protection::capture(path)?.as_ref() == protections.get(path),
            "File protections changed while backing up: {}",
            path.display()
        );
        ensure!(
            fingerprint(&bytes, executable(path)?) == *expected,
            "File changed while backing up: {}",
            path.display()
        );
        let stored = format!("files/{index}");
        atomic_write(&directory.join(&stored), &bytes)?;
        files.insert(path.clone(), stored);
    }
    atomic_write(
        &directory.join("snapshot.json"),
        &serde_json::to_vec_pretty(&Backup {
            manifest: manifest.clone(),
            files,
            ownership: None,
            absent: BTreeSet::new(),
            sources: BTreeMap::new(),
            protections,
        })?,
    )?;
    Ok(directory)
}

pub fn adopt(settings: &Settings) -> Result<PathBuf> {
    settings.validate()?;
    let desired = desired(settings)?;
    let state = settings.state_dir.join(&settings.machine);
    fs::create_dir_all(&state)?;
    let _lock = crate::config::operation_lock(settings, "apply.lock")?;
    let mut owned = manifest(settings)?;
    ensure!(
        stale::locations(&desired, &owned)?.is_empty(),
        "Resolve pending stale-target decisions with `ratatoskr stale-target list` before adopt"
    );
    for (path, expected) in &owned.files {
        no_symlinks(path)?;
        ensure!(
            fingerprint(&fs::read(path)?, executable(path)?) == *expected,
            "Cannot adopt over existing ownership drift: {}",
            path.display()
        );
    }
    for root in &desired.roots {
        if root.try_exists()? && !owned.roots.contains(root) {
            for (_, path) in file_paths(root)? {
                owned.files.insert(
                    path.clone(),
                    fingerprint(&fs::read(&path)?, executable(&path)?),
                );
            }
            owned.roots.insert(root.clone());
        }
    }
    for (_, path) in &desired.allowed {
        if desired.files.contains_key(path) && path.try_exists()? && !owned.files.contains_key(path)
        {
            no_symlinks(path)?;
            owned.files.insert(
                path.clone(),
                fingerprint(&fs::read(path)?, executable(path)?),
            );
        }
    }
    owned.target_harnesses = desired.target_harnesses();
    owned.project_roots = desired.project_roots;
    let directory = backup(settings, &owned)?;
    atomic_write(
        &manifest_path(settings),
        &serde_json::to_vec_pretty(&owned)?,
    )?;
    Ok(directory)
}

pub(crate) fn backup_source_paths(settings: &Settings, directory: &Path) -> Result<Vec<PathBuf>> {
    ensure!(
        directory.starts_with(settings.state_dir.join(&settings.machine).join("backups")),
        "Choose a backup from this machine's backup directory"
    );
    let path = directory.join("snapshot.json");
    no_symlinks(&path)?;
    let snapshot: Backup = serde_json::from_slice(&fs::read(path)?)?;
    Ok(snapshot.sources.into_keys().collect())
}

pub fn restore_backup(settings: &Settings, directory: &Path) -> Result<Plan> {
    let _lock = crate::config::operation_lock(settings, "apply.lock")?;
    restore_backup_locked(settings, directory)
}

pub(crate) fn restore_backup_locked(settings: &Settings, directory: &Path) -> Result<Plan> {
    settings.validate()?;
    no_symlinks(directory)?;
    ensure!(
        directory.starts_with(settings.state_dir.join(&settings.machine).join("backups")),
        "Choose a backup from this machine's backup directory"
    );
    let path = directory.join("snapshot.json");
    no_symlinks(&path)?;
    let snapshot: Backup = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        snapshot.manifest.source == settings.source
            && snapshot.manifest.machine == settings.machine,
        "Backup belongs to another source or machine"
    );
    let desired = desired(settings)?;
    for path in snapshot
        .manifest
        .files
        .keys()
        .chain(snapshot.absent.iter())
        .chain(snapshot.ownership.iter().flat_map(|m| m.files.keys()))
    {
        ensure!(
            desired
                .allowed
                .iter()
                .any(|(skills, global)| path.starts_with(skills) || path == global),
            "Backup destination is outside current targets"
        );
    }
    let current = manifest(settings)?;
    let check = preflight(settings, &desired, &current)?;
    ensure!(
        check.conflicts.is_empty(),
        "Resolve current ownership conflicts before restoring a backup"
    );
    let mut source_restore = BTreeMap::new();
    for (destination, source) in &snapshot.sources {
        ensure!(
            desired.files.values().any(|f| f.source == *destination),
            "Backup source is outside current source mappings"
        );
        let stored = directory.join(relative(&source.stored)?);
        no_symlinks(&stored)?;
        no_symlinks(destination)?;
        ensure!(
            fingerprint(&fs::read(destination)?, executable(destination)?)
                == source.after_fingerprint,
            "Source changed since the backed-up action; refusing to overwrite {}",
            destination.display()
        );
        let bytes = fs::read(stored)?;
        ensure!(
            fingerprint(&bytes, source.fingerprint.executable) == source.fingerprint,
            "Backup source hash does not match"
        );
        crate::content::validate_source_replacement(settings, destination, &bytes)?;
        let protection = Protection::for_restore(
            destination,
            source.protection.as_ref(),
            source.fingerprint.executable,
        )?;
        source_restore.insert(
            destination,
            (bytes, source.fingerprint.executable, protection),
        );
    }
    let mut restore = BTreeMap::new();
    for (destination, expected) in &snapshot.manifest.files {
        let stored = snapshot
            .files
            .get(destination)
            .context("Backup is missing a file")?;
        let stored = directory.join(relative(stored)?);
        no_symlinks(&stored)?;
        let bytes = fs::read(stored)?;
        ensure!(
            fingerprint(&bytes, expected.executable) == *expected,
            "Backup file hash does not match"
        );
        no_symlinks(destination)?;
        let protection = Protection::for_restore(
            destination,
            snapshot.protections.get(destination),
            expected.executable,
        )?;
        ensure!(
            !destination.try_exists()? || current.files.contains_key(destination),
            "Backup would overwrite an unmanaged file"
        );
        restore.insert(
            destination.clone(),
            (bytes, expected.executable, protection),
        );
    }
    for path in current.files.keys() {
        protection::capture(path)?;
    }
    for (path, (bytes, executable, protection)) in source_restore {
        crate::config::protected_write(path, &bytes, Some(executable), protection.as_ref())?;
    }
    for (path, (bytes, executable, protection)) in restore {
        crate::config::protected_write(&path, &bytes, Some(executable), protection.as_ref())?;
    }
    for path in current
        .files
        .keys()
        .filter(|p| !snapshot.manifest.files.contains_key(*p))
    {
        no_symlinks(path)?;
        protection::capture(path)?;
        fs::remove_file(path)?;
    }
    atomic_write(
        &manifest_path(settings),
        &serde_json::to_vec_pretty(snapshot.ownership.as_ref().unwrap_or(&snapshot.manifest))?,
    )?;
    plan(settings)
}

/// Renders one draft skill for one target. The editor calls this on every pause in typing, so
/// it writes nothing and reads only this skill's files. Other skills cannot affect the result.
pub(crate) fn preview_files(
    skill: &Path,
    name: &str,
    draft: SkillDraft<'_>,
    harness: &str,
    context: &toml::Table,
) -> Result<Vec<crate::content::PreviewFile>> {
    ensure_skill_name_allowed(name)?;
    no_symlinks(skill)?;
    ensure!(skill.is_dir(), "Skill no longer exists");
    let Some(SkillSources {
        metadata,
        files: sources,
    }) = skill_sources(skill, name, context, Some(draft))?
    else {
        return Ok(vec![]);
    };
    let mut cases = BTreeSet::new();
    let mut files = Vec::with_capacity(sources.len());
    for (path, source) in sources {
        ensure!(
            cases.insert(path.to_string_lossy().to_lowercase()),
            "Variant filename collision"
        );
        let rendered = render_skill_file(
            &path,
            &source,
            skill_file_bytes(skill, &source, Some(draft))?,
            name,
            harness,
            context,
            &metadata,
        )?;
        files.push(crate::content::PreviewFile {
            body: if path.file_name().is_some_and(|n| n == "SKILL.md") {
                std::str::from_utf8(&rendered.bytes)
                    .ok()
                    .map(|text| {
                        crate::template::skill_frontmatter(text, name)
                            .map(|(_, body)| body.to_owned())
                    })
                    .transpose()?
            } else {
                None
            },
            text: String::from_utf8(rendered.bytes).ok(),
            path,
        });
    }
    Ok(files)
}

pub(crate) fn template_contexts(settings: &Settings) -> Result<Vec<(String, String, toml::Table)>> {
    settings.validate()?;
    let harnesses = read_toml(&settings.source.join("harnesses.toml"))?;
    let mut contexts = Vec::new();
    for file in crate::source_files(settings)? {
        if !file.starts_with("machines/") {
            continue;
        }
        let machine = Path::new(&file)
            .file_stem()
            .context("Missing machine name")?
            .to_str()
            .context("Non-UTF8 machine name")?;
        identifier(machine)?;
        let profile = read_toml(&settings.source.join(&file))?;
        for value in profile
            .get("harnesses")
            .and_then(|v| v.as_array())
            .context("Machine must enable harnesses")?
        {
            let harness = value.as_str().context("Harness name must be a string")?;
            identifier(harness)?;
            ensure!(
                harnesses.contains_key(harness),
                "Unknown harness: {harness}"
            );
            let mut context = profile.clone();
            context.insert("machine".into(), machine.into());
            context.insert("harness".into(), harness.into());
            contexts.push((machine.into(), harness.into(), context));
        }
    }
    Ok(contexts)
}

pub(crate) fn legacy_ownership(settings: &Settings, root: &Path) -> Result<Option<String>> {
    let path = manifest_path(settings);
    no_symlinks(&path)?;
    if !path.try_exists()? {
        return Ok(None);
    }
    let text = fs::read_to_string(path)?;
    let manifest: Manifest = serde_json::from_str(&text)?;
    ensure!(
        manifest.source == settings.source && manifest.machine == settings.machine,
        "Ownership belongs to another workspace"
    );
    Ok(
        (manifest.roots.contains(root) || manifest.files.keys().any(|p| p.starts_with(root)))
            .then_some(text),
    )
}
pub(crate) fn detach_legacy_ownership(
    settings: &Settings,
    root: &Path,
    expected: &str,
) -> Result<()> {
    let current =
        legacy_ownership(settings, root)?.context("Legacy ownership changed; review again")?;
    ensure!(
        current == expected,
        "Legacy ownership changed; review again"
    );
    let mut manifest: Manifest = serde_json::from_str(expected)?;
    manifest.files.retain(|path, _| !path.starts_with(root));
    manifest.roots.remove(root);
    manifest.retain_owned_targets();
    atomic_write(
        &manifest_path(settings),
        &serde_json::to_vec_pretty(&manifest)?,
    )
}
