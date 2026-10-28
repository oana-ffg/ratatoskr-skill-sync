mod conflicts;
pub(crate) use conflicts::resolve_conflict_locked;
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
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};
use validation::RenderDiagnostics;
pub use validation::{
    HarnessValidation, ProfileValidation, ValidationContext, ValidationProblem, ValidationReport,
    validate_source,
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
    pub projects: Vec<ProjectStatus>,
    pub content_status: crate::ContentStatus,
    pub revision: String,
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

struct SkillSources {
    metadata: toml::Table,
    files: BTreeMap<PathBuf, PathBuf>,
}

fn skill_sources(
    skill: &Path,
    skill_name: &str,
    context: &toml::Table,
) -> Result<Option<SkillSources>> {
    let metadata = read_toml(&skill.join("skill.toml"))?;
    ensure!(
        metadata
            .keys()
            .all(|k| matches!(k.as_str(), "target" | "frontmatter" | "enabled")),
        "Unknown skill.toml fields in {skill_name}"
    );
    if let Some(enabled) = metadata.get("enabled") {
        ensure!(enabled.is_bool(), "enabled must be a boolean");
        if !enabled.as_bool().unwrap_or(false) {
            return Ok(None);
        }
    }
    if let Some(target) = metadata.get("target")
        && !selectors_match(
            target.as_table().context("Target must be a table")?,
            context,
        )?
    {
        return Ok(None);
    }
    let mut sources = file_paths(skill)?;
    sources.retain(|p, _| p != Path::new("skill.toml") && !p.starts_with("variants"));
    let variants = skill.join("variants");
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
    Ok(Some(SkillSources {
        metadata,
        files: sources,
    }))
}

fn render_skill_file(
    path: &Path,
    source: &Path,
    skill_name: &str,
    harness: &str,
    context: &toml::Table,
    metadata: &toml::Table,
) -> Result<Rendered> {
    let original = fs::read(source)?;
    let reversible = std::str::from_utf8(&original).is_ok_and(|text| {
        !(text.contains("{{af:")
            || text.contains("<!--")
            || path == Path::new("SKILL.md") && metadata.contains_key("frontmatter"))
    });
    let bytes = original;
    let required_text = matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("md" | "toml" | "py" | "txt")
    );
    let bytes = match std::str::from_utf8(&bytes) {
        Ok(text) if path == Path::new("SKILL.md") => {
            skill_markdown(text, skill_name, context, metadata)?.into_bytes()
        }
        Ok(text) => {
            render(text, context, path.extension().is_some_and(|e| e == "md"))?.into_bytes()
        }
        Err(error) if required_text => {
            return Err(error).context("Required text file is not UTF-8");
        }
        Err(_) => bytes,
    };
    Ok(Rendered {
        harness: harness.into(),
        skill: Some(skill_name.into()),
        source: source.into(),
        reversible,
        bytes,
        executable: executable(source)?,
    })
}

fn render_scope(
    settings: &Settings,
    content: &Path,
    project: Option<&Path>,
    preview: bool,
) -> Result<Desired> {
    settings.validate()?;
    render_scope_inner(
        &RenderScope {
            source: &settings.source,
            machine: &settings.machine,
            content,
            project,
            output: Output::Workspace { settings, preview },
        },
        &mut RenderDiagnostics::default(),
    )
}

#[derive(Clone, Copy)]
enum Output<'a> {
    Workspace {
        settings: &'a Settings,
        preview: bool,
    },
    Validation,
}

struct RenderScope<'a> {
    source: &'a Path,
    machine: &'a str,
    content: &'a Path,
    project: Option<&'a Path>,
    output: Output<'a>,
}

fn render_scope_inner(
    scope: &RenderScope<'_>,
    diagnostics: &mut RenderDiagnostics,
) -> Result<Desired> {
    let RenderScope {
        source,
        machine: machine_name,
        ..
    } = *scope;
    no_symlinks(source)?;
    identifier(machine_name)?;
    let machine = read_toml(&source.join("machines").join(format!("{machine_name}.toml")))?;
    let harnesses: BTreeMap<String, Harness> =
        read_toml(&source.join("harnesses.toml"))?.try_into()?;
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
        install_targets: vec![],
        mode: mode.into(),
        skills: 0,
        targets: enabled.len(),
        project_roots: BTreeMap::new(),
    };
    let mut base_roots = Vec::new();
    let profile_path = source.join("machines").join(format!("{machine_name}.toml"));
    for harness_name in enabled {
        let name_result = harness_name
            .as_str()
            .with_context(|| format!("Harness name must be a string: {harness_name}"));
        let Some(name) =
            diagnostics.capture(name_result, machine_name, None, None, &profile_path)?
        else {
            continue;
        };
        let harness_result = render_harness(
            scope,
            &machine,
            name,
            &harnesses,
            &mut result,
            &mut base_roots,
            diagnostics,
        );
        if let Some(summary) = diagnostics.capture(
            harness_result,
            machine_name,
            Some(name),
            None,
            &source.join("harnesses.toml"),
        )? {
            diagnostics.harnesses.push(summary);
        }
    }
    Ok(result)
}

struct HarnessRender<'a, 's> {
    scope: &'a RenderScope<'s>,
    name: &'a str,
    definition: &'a Harness,
    context: &'a toml::Table,
    os: &'a str,
    mode: &'a str,
}

fn harness_base(
    rendering: &HarnessRender<'_, '_>,
    diagnostics: &mut RenderDiagnostics,
) -> Result<PathBuf> {
    let HarnessRender {
        scope,
        name,
        definition: harness,
        context,
        os,
        mode,
    } = *rendering;
    let RenderScope {
        source,
        machine: machine_name,
        project,
        output,
        ..
    } = *scope;
    let (settings, preview) = match output {
        Output::Workspace { settings, preview } => (Some(settings), preview),
        Output::Validation => (None, true),
    };
    // Validation uses logical destinations only. They are never inspected or created.
    Ok(if let Some(settings) = settings {
        if mode == "staging" || preview {
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
            let home = Path::new(string(context, "home")?);
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
            PathBuf::from(render(root, context, false)?)
        }
    } else {
        if mode == "live" {
            let root = harness
                .roots
                .get(os)
                .context("Harness lacks a root for machine OS")
                .and_then(|root| render(root, context, false))
                .and_then(|root| validation::validate_root(&root, os));
            diagnostics.capture(
                root,
                machine_name,
                Some(name),
                None,
                &source.join("harnesses.toml"),
            )?;
            if let Some(root) = &harness.global_root {
                let root = render(root, context, false)
                    .and_then(|root| validation::validate_root(&root, os));
                diagnostics.capture(
                    root,
                    machine_name,
                    Some(name),
                    None,
                    &source.join("harnesses.toml"),
                )?;
            }
        }
        if let Some(root) = &harness.project_root {
            diagnostics.capture(
                relative(root),
                machine_name,
                Some(name),
                None,
                &source.join("harnesses.toml"),
            )?;
        }
        source.join(".validation").join(name)
    })
}

fn render_harness(
    scope: &RenderScope<'_>,
    machine: &toml::Table,
    name: &str,
    harnesses: &BTreeMap<String, Harness>,
    result: &mut Desired,
    base_roots: &mut Vec<PathBuf>,
    diagnostics: &mut RenderDiagnostics,
) -> Result<HarnessValidation> {
    let RenderScope {
        machine: machine_name,
        content,
        project,
        output,
        ..
    } = *scope;
    let mut summary = HarnessValidation::new(name);
    identifier(name)?;
    let harness = harnesses
        .get(name)
        .with_context(|| format!("Unknown harness: {name}"))?;
    let mut context = machine.clone();
    context.insert("machine".into(), machine_name.into());
    context.insert("harness".into(), name.into());
    let os = string(machine, "os")?;
    let mode = result.mode.clone();
    let mode = mode.as_str();
    let rendering = &HarnessRender {
        scope,
        name,
        definition: harness,
        context: &context,
        os,
        mode,
    };
    let (settings, preview) = match output {
        Output::Workspace { settings, preview } => (Some(settings), preview),
        Output::Validation => (None, true),
    };
    let base = harness_base(rendering, diagnostics)?;
    ensure!(base.is_absolute(), "Output root must be absolute");
    if settings.is_some() {
        no_symlinks(&base)?;
    }
    if let Some(settings) = settings {
        ensure!(!overlaps(&base, content), "Output overlaps source checkout");
        if mode == "live" && !preview {
            ensure!(
                !overlaps(&base, &settings.state_dir),
                "Output overlaps state"
            );
        }
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
    if settings.is_some() {
        no_symlinks(&global_base)?;
    }
    if let Some(settings) = settings {
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
    result.install_targets.push(InstallTarget {
        harness: name.into(),
        skills_root: skills_base.clone(),
        instructions_file: global_file.clone(),
        project: project.map(Path::to_path_buf),
    });
    let skills_dir = content.join("skills");
    no_symlinks(&skills_dir)?;
    if skills_dir.try_exists()? {
        let mut names = BTreeSet::new();
        for skill in fs::read_dir(&skills_dir)? {
            let skill = match diagnostics.capture(
                skill.map_err(Into::into),
                machine_name,
                Some(name),
                None,
                &skills_dir,
            )? {
                Some(skill) => skill,
                None => continue,
            };
            let skill_path = skill.path();
            // `.DS_Store` and `.pyc` files are ignored inside a skill; ignore them at the root too.
            if skills_root_runtime_file(&skill)? {
                continue;
            }
            let skill_label = skill.file_name().to_string_lossy().into_owned();
            let skill_result = render_skill(
                rendering,
                &skill,
                &skills_base,
                &mut names,
                result,
                &mut summary,
                diagnostics,
            );
            if let Some(false) = diagnostics.capture(
                skill_result,
                machine_name,
                Some(name),
                Some(&skill_label),
                &skill_path,
            )? {
                summary.skills_skipped += 1;
            }
        }
    }

    let global = content.join(crate::content::GLOBAL_INSTRUCTIONS);
    let global_result = render_global(rendering, &global, global_file, result);
    if let Some(rendered) =
        diagnostics.capture(global_result, machine_name, Some(name), None, &global)?
    {
        summary.global_instructions_rendered = rendered;
    }
    Ok(summary)
}

fn render_skill(
    rendering: &HarnessRender<'_, '_>,
    skill: &fs::DirEntry,
    skills_base: &Path,
    names: &mut BTreeSet<String>,
    result: &mut Desired,
    summary: &mut HarnessValidation,
    diagnostics: &mut RenderDiagnostics,
) -> Result<bool> {
    let HarnessRender {
        scope,
        name,
        context,
        ..
    } = *rendering;
    let machine_name = scope.machine;
    let skill_path = skill.path();
    require_skill_directory(skill)?;
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
    let Some(SkillSources {
        metadata,
        files: sources,
    }) = skill_sources(&skill_path, &skill_name, context)?
    else {
        return Ok(false);
    };
    let root = skills_base.join(&skill_name);
    result.roots.insert(root.clone());
    let mut clean = true;
    let mut cases = BTreeSet::new();
    for (path, source) in sources {
        let file_result = render_skill_output(
            &SkillRender {
                harness: rendering,
                name: &skill_name,
                root: &root,
                metadata: &metadata,
            },
            &path,
            &source,
            &mut cases,
            result,
        );
        if diagnostics
            .capture(
                file_result,
                machine_name,
                Some(name),
                Some(&skill_name),
                &source,
            )?
            .is_some()
        {
            summary.files_rendered += 1;
        } else {
            clean = false;
        }
    }
    if clean {
        summary.skills_rendered += 1;
        result.skills += 1;
    }
    Ok(true)
}

struct SkillRender<'a, 's> {
    harness: &'a HarnessRender<'a, 's>,
    name: &'a str,
    root: &'a Path,
    metadata: &'a toml::Table,
}

fn render_skill_output(
    skill: &SkillRender<'_, '_>,
    path: &Path,
    source: &Path,
    cases: &mut BTreeSet<String>,
    result: &mut Desired,
) -> Result<()> {
    let SkillRender {
        harness,
        name: skill_name,
        root,
        metadata,
    } = *skill;
    let HarnessRender { name, context, .. } = *harness;
    ensure!(
        cases.insert(path.to_string_lossy().to_lowercase()),
        "Variant filename collision"
    );
    ensure!(
        result
            .files
            .insert(
                root.join(path),
                render_skill_file(path, source, skill_name, name, context, metadata)?
            )
            .is_none(),
        "Duplicate output file"
    );
    Ok(())
}

fn render_global(
    rendering: &HarnessRender<'_, '_>,
    global: &Path,
    global_file: PathBuf,
    result: &mut Desired,
) -> Result<bool> {
    let HarnessRender {
        scope,
        name,
        context,
        ..
    } = *rendering;
    let project = scope.project;
    no_symlinks(global)?;
    if project.is_none() && global.try_exists()? {
        result.files.insert(
            global_file,
            Rendered {
                harness: name.into(),
                skill: None,
                source: global.into(),
                reversible: {
                    let text = fs::read_to_string(global)?;
                    !text.contains("{{af:") && !text.contains("<!--")
                },
                bytes: render_global_instructions(&fs::read_to_string(global)?, context)?
                    .into_bytes(),
                executable: false,
            },
        );
        return Ok(true);
    }
    Ok(false)
}

/// The single renderer for global instructions, shared by deployment and editor previews.
pub(crate) fn render_global_instructions(text: &str, context: &toml::Table) -> Result<String> {
    render(text, context, true)
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
        result.install_targets.extend(project.install_targets);
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
    protection::capture(&manifest_path(settings))?;
    let mut revision = Sha256::new();
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
        projects: settings
            .projects
            .iter()
            .map(|p| project_status(p))
            .collect(),
        revision: format!("{:x}", revision.finalize()),
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
                body: if path.file_name().is_some_and(|n| n == "SKILL.md") {
                    std::str::from_utf8(&file.bytes)
                        .ok()
                        .map(|text| {
                            crate::template::skill_frontmatter(text, name)
                                .map(|(_, body)| body.to_owned())
                        })
                        .transpose()?
                } else {
                    None
                },
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
    atomic_write(
        &manifest_path(settings),
        &serde_json::to_vec_pretty(&manifest)?,
    )
}
