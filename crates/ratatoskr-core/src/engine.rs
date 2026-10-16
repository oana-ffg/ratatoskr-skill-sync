mod conflicts;
use crate::{
    Settings,
    config::{atomic_write, identifier, no_symlinks, overlaps, relative},
    git::{ProjectStatus, project_status},
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
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
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
pub struct Plan {
    pub source: PathBuf,
    pub machine: String,
    pub mode: String,
    pub skills: usize,
    pub targets: usize,
    pub changes: Vec<Change>,
    pub conflicts: Vec<String>,
    pub conflict_details: Vec<Conflict>,
    pub projects: Vec<ProjectStatus>,
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
    source: PathBuf,
    reversible: bool,
    bytes: Vec<u8>,
    executable: bool,
}

struct Desired {
    files: BTreeMap<PathBuf, Rendered>,
    roots: BTreeSet<PathBuf>,
    allowed: Vec<(PathBuf, PathBuf)>,
    mode: String,
    skills: usize,
    targets: usize,
    project_roots: BTreeMap<PathBuf, BTreeSet<PathBuf>>,
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

fn skill_markdown(
    source: &str,
    name: &str,
    context: &toml::Table,
    metadata: &toml::Table,
) -> Result<String> {
    let rendered = render(source, context, true)?;
    let rendered = rendered.replace("\r\n", "\n");
    let (mut yaml, body) = crate::template::skill_frontmatter(&rendered, name)?;
    ensure!(!body.trim().is_empty(), "Skill body is empty");
    if let Some(additions) = metadata.get("frontmatter") {
        let frontmatters = additions
            .as_table()
            .context("frontmatter must be a table")?;
        if let Some(selected) = frontmatters.get(string(context, "harness")?) {
            for (key, value) in selected
                .as_table()
                .context("Harness frontmatter must be a table")?
            {
                ensure!(
                    !matches!(key.as_str(), "name" | "description"),
                    "Harness frontmatter cannot replace skill identity"
                );
                ensure!(
                    yaml.get(key.as_str()).is_none(),
                    "Harness frontmatter cannot overwrite shared field {key}"
                );
                yaml.as_mapping_mut()
                    .context("Frontmatter must be a mapping")?
                    .insert(
                        serde_yaml_ng::Value::String(key.clone()),
                        serde_yaml_ng::to_value(value)?,
                    );
            }
        }
    }
    Ok(format!(
        "---\n{}---\n{body}",
        serde_yaml_ng::to_string(&yaml)?
    ))
}

fn render_scope(
    settings: &Settings,
    content: &Path,
    project: Option<&Path>,
    preview: bool,
) -> Result<Desired> {
    settings.validate()?;
    no_symlinks(&settings.source)?;
    no_symlinks(&settings.state_dir)?;
    identifier(&settings.machine)?;
    let machine = read_toml(
        &settings
            .source
            .join("machines")
            .join(format!("{}.toml", settings.machine)),
    )?;
    let harnesses: BTreeMap<String, Harness> =
        read_toml(&settings.source.join("harnesses.toml"))?.try_into()?;
    let os = string(&machine, "os")?;
    ensure!(
        matches!(os, "macos" | "windows" | "linux"),
        "Unknown machine OS"
    );
    let mode = machine
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("staging");
    ensure!(
        matches!(mode, "staging" | "live"),
        "Mode must be staging or live"
    );
    let enabled = machine
        .get("harnesses")
        .and_then(|v| v.as_array())
        .context("Machine must enable harnesses")?;
    ensure!(
        !enabled.is_empty(),
        "Machine must enable at least one harness"
    );
    let mut result = Desired {
        files: BTreeMap::new(),
        roots: BTreeSet::new(),
        allowed: vec![],
        mode: mode.into(),
        skills: 0,
        targets: enabled.len(),
        project_roots: BTreeMap::new(),
    };
    let mut base_roots = Vec::new();
    for harness_name in enabled {
        let name = harness_name
            .as_str()
            .context("Harness name must be a string")?;
        identifier(name)?;
        let harness = harnesses
            .get(name)
            .with_context(|| format!("Unknown harness: {name}"))?;
        let mut context = machine.clone();
        context.insert("machine".into(), settings.machine.clone().into());
        context.insert("harness".into(), name.into());
        let base = if mode == "staging" || preview {
            let staging = settings.state_dir.join(&settings.machine).join("staging");
            if let Some(project) = project {
                staging
                    .join("projects")
                    .join(format!(
                        "{:x}",
                        Sha256::digest(project.to_string_lossy().as_bytes())
                    ))
                    .join(name)
            } else {
                staging.join(name)
            }
        } else if let Some(project) = project {
            ensure!(os == host_os(), "Project profile does not match host OS");
            project.join(relative(
                harness
                    .project_root
                    .as_deref()
                    .unwrap_or(&format!(".{name}")),
            )?)
        } else {
            ensure!(
                os == host_os(),
                "Cannot apply {os} profile on {}",
                host_os()
            );
            let home = Path::new(string(&machine, "home")?);
            let actual_home = directories::BaseDirs::new()
                .context("Cannot find home")?
                .home_dir()
                .canonicalize()?;
            ensure!(
                home.canonicalize()? == actual_home,
                "Live machine profile belongs to another home"
            );
            let root = harness
                .roots
                .get(os)
                .context("Harness lacks a root for machine OS")?;
            PathBuf::from(render(root, &context, false)?)
        };
        ensure!(base.is_absolute(), "Output root must be absolute");
        no_symlinks(&base)?;
        ensure!(!overlaps(&base, content), "Output overlaps source checkout");
        if mode == "live" && !preview {
            ensure!(
                !overlaps(&base, &settings.state_dir),
                "Output overlaps state"
            );
        }
        ensure!(
            !base_roots
                .iter()
                .any(|other: &PathBuf| overlaps(other, &base)),
            "Harness output roots overlap"
        );
        base_roots.push(base.clone());
        let skills_base = base.join(relative(&harness.skills_dir)?);
        let global_base = if mode == "live" && !preview && project.is_none() {
            if let Some(root) = &harness.global_root {
                PathBuf::from(render(root, &context, false)?)
            } else {
                base.clone()
            }
        } else {
            base.clone()
        };
        ensure!(
            global_base.is_absolute(),
            "Global instruction root must be absolute"
        );
        no_symlinks(&global_base)?;
        ensure!(
            !overlaps(&global_base, content),
            "Instruction root overlaps source"
        );
        if mode == "live" && !preview {
            ensure!(
                !overlaps(&global_base, &settings.state_dir),
                "Instruction root overlaps state"
            );
        }
        let global_file = global_base.join(relative(&harness.global_file)?);
        for (other_skills, other_global) in &result.allowed {
            ensure!(
                !overlaps(&skills_base, other_skills)
                    && !overlaps(&skills_base, other_global)
                    && !overlaps(&global_file, other_skills)
                    && global_file != *other_global,
                "Harness destinations overlap"
            );
        }
        ensure!(
            !global_file.starts_with(&skills_base),
            "Global file overlaps skills directory"
        );
        result
            .allowed
            .push((skills_base.clone(), global_file.clone()));
        let skills_dir = content.join("skills");
        no_symlinks(&skills_dir)?;
        if skills_dir.try_exists()? {
            let mut names = BTreeSet::new();
            for skill in fs::read_dir(&skills_dir)? {
                let skill = skill?;
                ensure!(
                    skill.file_type()?.is_dir(),
                    "Skills root must contain directories only"
                );
                let skill_name = skill
                    .file_name()
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("Non-UTF8 skill name"))?;
                identifier(&skill_name)?;
                ensure!(
                    !matches!(skill_name.as_str(), "synced" | "anthropic-skills"),
                    "Reserved skill name"
                );
                ensure!(
                    names.insert(skill_name.to_lowercase()),
                    "Duplicate skill name"
                );
                let metadata = read_toml(&skill.path().join("skill.toml"))?;
                ensure!(
                    metadata
                        .keys()
                        .all(|k| matches!(k.as_str(), "target" | "frontmatter" | "enabled")),
                    "Unknown skill.toml fields in {skill_name}"
                );
                if let Some(enabled) = metadata.get("enabled") {
                    ensure!(enabled.is_bool(), "enabled must be a boolean");
                    if !enabled.as_bool().unwrap_or(false) {
                        continue;
                    }
                }
                if let Some(target) = metadata.get("target")
                    && !selectors_match(
                        target.as_table().context("Target must be a table")?,
                        &context,
                    )?
                {
                    continue;
                }
                let mut sources = file_paths(&skill.path())?;
                sources.retain(|p, _| p != Path::new("skill.toml") && !p.starts_with("variants"));
                let variants = skill.path().join("variants");
                let mut replacements = BTreeSet::new();
                if variants.try_exists()? {
                    no_symlinks(&variants)?;
                    for variant in fs::read_dir(&variants)? {
                        let variant = variant?;
                        ensure!(
                            variant.file_type()?.is_dir(),
                            "Variants must be directories"
                        );
                        let variant_name = variant
                            .file_name()
                            .into_string()
                            .map_err(|_| anyhow::anyhow!("Non-UTF8 variant"))?;
                        let (key, value) = variant_name
                            .split_once('=')
                            .context("Variant requires key=value")?;
                        ensure!(
                            matches!(key, "harness" | "machine" | "os"),
                            "Unknown variant key"
                        );
                        identifier(value)?;
                        let variant_files = file_paths(&variant.path())?;
                        if context.get(key).and_then(|v| v.as_str()) == Some(value) {
                            for (path, source) in variant_files {
                                ensure!(
                                    replacements.insert(path.clone()),
                                    "Two matching variants replace {}",
                                    path.display()
                                );
                                ensure!(
                                    path != Path::new("skill.toml"),
                                    "Variants cannot replace skill metadata"
                                );
                                sources.insert(path, source);
                            }
                        }
                    }
                }
                ensure!(
                    sources.contains_key(Path::new("SKILL.md")),
                    "Skill {skill_name} lacks SKILL.md"
                );
                let root = skills_base.join(&skill_name);
                result.roots.insert(root.clone());
                result.skills += 1;
                let mut cases = BTreeSet::new();
                for (path, source) in sources {
                    ensure!(
                        cases.insert(path.to_string_lossy().to_lowercase()),
                        "Variant filename collision"
                    );
                    let original = fs::read(&source)?;
                    let reversible = std::str::from_utf8(&original).is_ok_and(|text| {
                        !text.contains("{{af:")
                            && !text.contains("<!--")
                            && !(path == Path::new("SKILL.md")
                                && metadata.contains_key("frontmatter"))
                    });
                    let bytes = original.clone();
                    let required_text = matches!(
                        path.extension().and_then(|e| e.to_str()),
                        Some("md" | "toml" | "py" | "txt")
                    );
                    let bytes = match std::str::from_utf8(&bytes) {
                        Ok(text) if path == Path::new("SKILL.md") => {
                            skill_markdown(text, &skill_name, &context, &metadata)?.into_bytes()
                        }
                        Ok(text) => {
                            render(text, &context, path.extension().is_some_and(|e| e == "md"))?
                                .into_bytes()
                        }
                        Err(error) if required_text => {
                            return Err(error).context("Required text file is not UTF-8");
                        }
                        Err(_) => bytes,
                    };
                    ensure!(
                        result
                            .files
                            .insert(
                                root.join(path),
                                Rendered {
                                    source: source.clone(),
                                    reversible,
                                    bytes,
                                    executable: executable(&source)?
                                }
                            )
                            .is_none(),
                        "Duplicate output file"
                    );
                }
            }
        }
        let global = content.join("global/AGENTS.md");
        no_symlinks(&global)?;
        if project.is_none() && global.try_exists()? {
            result.files.insert(
                global_file,
                Rendered {
                    source: global.clone(),
                    reversible: {
                        let text = fs::read_to_string(&global)?;
                        !text.contains("{{af:") && !text.contains("<!--")
                    },
                    bytes: render(&fs::read_to_string(global)?, &context, true)?.into_bytes(),
                    executable: false,
                },
            );
        }
    }
    Ok(result)
}

fn desired(settings: &Settings) -> Result<Desired> {
    let mut result = render_scope(settings, &settings.source, None, false)?;
    for root in &settings.projects {
        let status = project_status(root);
        ensure!(
            status.error.is_none(),
            "Project {} cannot be rendered: {:?}",
            root.display(),
            status.error
        );
        let content = status.source.context("Project has no skill source")?;
        let project = render_scope(settings, &content, Some(root), false)?;
        result
            .project_roots
            .insert(root.clone(), project.roots.clone());
        for (path, file) in project.files {
            ensure!(
                result.files.insert(path.clone(), file).is_none(),
                "Project/global output collision: {}",
                path.display()
            );
        }
        result.roots.extend(project.roots);
        result.allowed.extend(project.allowed);
        result.skills += project.skills;
        result.targets += project.targets;
    }
    Ok(result)
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

fn manifest(settings: &Settings, desired: &Desired) -> Result<Manifest> {
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
    for path in manifest.files.keys() {
        ensure!(
            desired
                .allowed
                .iter()
                .any(|(skills, global)| path.starts_with(skills) || path == global),
            "Ownership path outside current targets: {}",
            path.display()
        );
    }
    for root in &manifest.roots {
        ensure!(
            desired
                .allowed
                .iter()
                .any(|(skills, _)| root.parent() == Some(skills)),
            "Ownership root outside current skills targets"
        );
    }
    Ok(manifest)
}

fn preflight(settings: &Settings, desired: &Desired, manifest: &Manifest) -> Result<Plan> {
    let mut conflicts = Vec::new();
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
    Ok(Plan {
        source: settings.source.clone(),
        machine: settings.machine.clone(),
        mode: desired.mode.clone(),
        skills: desired.skills,
        targets: desired.targets,
        changes,
        conflicts,
        conflict_details,
        projects: settings
            .projects
            .iter()
            .map(|p| project_status(p))
            .collect(),
    })
}

pub fn plan(settings: &Settings) -> Result<Plan> {
    let desired = desired(settings)?;
    let manifest = manifest(settings, &desired)?;
    preflight(settings, &desired, &manifest)
}

pub fn apply(settings: &Settings) -> Result<Plan> {
    apply_inner(settings, None)
}

pub fn apply_reviewed(settings: &Settings, expected: &str) -> Result<Plan> {
    apply_inner(settings, Some(serde_json::from_str(expected)?))
}

fn apply_inner(settings: &Settings, expected: Option<serde_json::Value>) -> Result<Plan> {
    settings.validate()?;
    no_symlinks(&settings.state_dir)?;
    let state = settings.state_dir.join(&settings.machine);
    fs::create_dir_all(&state)?;
    let lock = crate::config::operation_lock(settings, "apply.lock")?;
    let desired = desired(settings)?;
    let old = manifest(settings, &desired)?;
    let preflight = preflight(settings, &desired, &old)?;
    if let Some(expected) = expected {
        ensure!(
            serde_json::to_value(&preflight)? == expected,
            "Workspace changed since preview. Refresh and review the new plan."
        );
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
            atomic_write(&change.path, &file.bytes)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(
                    &change.path,
                    fs::Permissions::from_mode(if file.executable { 0o755 } else { 0o644 }),
                )?;
            }
        } else {
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
        project_roots: desired.project_roots,
        files: desired
            .files
            .iter()
            .map(|(p, f)| (p.clone(), fingerprint(&f.bytes, f.executable)))
            .collect(),
    };
    atomic_write(&manifest_path(settings), &serde_json::to_vec_pretty(&new)?)?;
    drop(lock);
    plan(settings)
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
}

#[derive(Deserialize, Serialize)]
struct SourceBackup {
    stored: String,
    fingerprint: Fingerprint,
    after_fingerprint: Fingerprint,
}

fn backup(settings: &Settings, manifest: &Manifest) -> Result<PathBuf> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let directory = settings
        .state_dir
        .join(&settings.machine)
        .join("backups")
        .join(nonce.to_string());
    no_symlinks(&directory)?;
    let mut files = BTreeMap::new();
    for (index, (path, expected)) in manifest.files.iter().enumerate() {
        no_symlinks(path)?;
        let bytes = fs::read(path)?;
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
    let mut owned = manifest(settings, &desired)?;
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
    owned.project_roots = desired.project_roots;
    let directory = backup(settings, &owned)?;
    atomic_write(
        &manifest_path(settings),
        &serde_json::to_vec_pretty(&owned)?,
    )?;
    Ok(directory)
}

pub fn restore_backup(settings: &Settings, directory: &Path) -> Result<Plan> {
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
    let lock = crate::config::operation_lock(settings, "apply.lock")?;
    let current = manifest(settings, &desired)?;
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
        source_restore.insert(destination, (bytes, source.fingerprint.executable));
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
        ensure!(
            !destination.try_exists()? || current.files.contains_key(destination),
            "Backup would overwrite an unmanaged file"
        );
        restore.insert(destination.clone(), (bytes, expected.executable));
    }
    for (path, (bytes, executable)) in source_restore {
        write_rendered(path, &bytes, executable)?;
    }
    for (path, (bytes, executable)) in restore {
        atomic_write(&path, &bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(
                path,
                fs::Permissions::from_mode(if executable { 0o755 } else { 0o644 }),
            )?;
        }
        #[cfg(not(unix))]
        {
            let _ = executable;
        }
    }
    for path in current
        .files
        .keys()
        .filter(|p| !snapshot.manifest.files.contains_key(*p))
    {
        no_symlinks(path)?;
        fs::remove_file(path)?;
    }
    atomic_write(
        &manifest_path(settings),
        &serde_json::to_vec_pretty(snapshot.ownership.as_ref().unwrap_or(&snapshot.manifest))?,
    )?;
    drop(lock);
    plan(settings)
}

pub(crate) fn preview_files(
    settings: &Settings,
    name: &str,
) -> Result<Vec<crate::content::PreviewFile>> {
    let desired = render_scope(settings, &settings.source, None, true)?;
    let mut files = Vec::new();
    for (path, file) in desired.files {
        if let Some(root) = desired
            .roots
            .iter()
            .find(|root| root.file_name().is_some_and(|n| n == name) && path.starts_with(root))
        {
            files.push(crate::content::PreviewFile {
                path: path.strip_prefix(root)?.to_path_buf(),
                text: String::from_utf8(file.bytes).ok(),
            });
        }
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
