//! Lossless desktop operations. Template syntax and source serialization stay in Rust.
mod draft;
use crate::{
    Settings,
    config::{atomic_write, identifier, no_symlinks, relative},
    content::GLOBAL_INSTRUCTIONS,
};
use anyhow::{Context, Result, ensure};
pub use draft::{
    DescriptionEdit, DescriptionSection, Draft, DraftEdit, EditorView, edit_draft, editor_view,
};
use draft::{frontmatter, yaml_field};
use serde::Serialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Item};

/// Patch only changed keys. toml_edit retains formatting/comments of all untouched items.
pub fn patch_toml(source: &str, path: &[String], value: Option<&toml::Value>) -> Result<String> {
    ensure!(!path.is_empty(), "Choose a field");
    let mut doc = source.parse::<DocumentMut>()?;
    let previous: toml::Table = toml::from_str(source)?;
    let previous = toml::Value::Table(previous);
    let mut old = Some(&previous);
    // Compare through a separately owned value, avoiding reserialization on no-op edits.
    for key in path {
        old = old.and_then(|v| v.get(key));
    }
    if old == value {
        return Ok(source.into());
    }
    let mut item = doc.as_item_mut();
    for key in &path[..path.len() - 1] {
        if item.get(key).is_none() {
            item[key] = Item::Table(toml_edit::Table::new());
        }
        item = &mut item[key];
    }
    let key = path.last().context("Missing field")?;
    match value {
        Some(value) => {
            let text = toml::to_string(&BTreeMap::from([("value", value)]))?;
            let mut parsed = text.parse::<DocumentMut>()?;
            let mut replacement = parsed.remove("value").context("Missing serialized value")?;
            if let (Some(old), Some(new)) = (
                item.get(key).and_then(Item::as_value),
                replacement.as_value_mut(),
            ) {
                *new.decor_mut() = old.decor().clone();
            }
            item[key] = replacement;
        }
        None => {
            item.as_table_like_mut()
                .context("Field parent is not a table")?
                .remove(key);
        }
    }
    Ok(preserve_newlines(source, &doc.to_string()))
}

/// toml_edit normalizes line breaks when materializing its CST. Copy equal spans from
/// the original source so no untouched CRLF/mixed separator changes on disk.
fn preserve_newlines(source: &str, edited: &str) -> String {
    let normalized = source.replace("\r\n", "\n");
    let newline = if source.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut cursor = 0;
    let mut output = String::new();
    for change in similar::TextDiff::from_chars(normalized.as_str(), edited).iter_all_changes() {
        match change.tag() {
            similar::ChangeTag::Equal | similar::ChangeTag::Delete => {
                for _ in change.value().chars() {
                    let length = if source[cursor..].starts_with("\r\n") {
                        2
                    } else {
                        source[cursor..].chars().next().map_or(0, char::len_utf8)
                    };
                    if change.tag() == similar::ChangeTag::Equal {
                        output.push_str(&source[cursor..cursor + length]);
                    }
                    cursor += length;
                }
            }
            similar::ChangeTag::Insert => output.push_str(&change.value().replace('\n', newline)),
        }
    }
    output
}

#[derive(Serialize)]
pub struct StructuredSource {
    pub file: String,
    pub hash: String,
    pub values: toml::Table,
}
pub fn structured_source(settings: &Settings, file: &str) -> Result<StructuredSource> {
    let source = crate::read_source_file(settings, file)?;
    Ok(StructuredSource {
        file: file.into(),
        hash: source.hash,
        values: toml::from_str(&source.text)?,
    })
}

pub fn save_structured_source(
    settings: &Settings,
    file: &str,
    hash: &str,
    values: toml::Table,
) -> Result<()> {
    let current = crate::read_source_file(settings, file)?;
    ensure!(current.hash == hash, "Source changed; reload before saving");
    let before: toml::Table = toml::from_str(&current.text)?;
    let text = edit_toml_values(&current.text, &before, &values)?;
    crate::write_source_file(settings, file, &text, hash)
}

fn edit_toml_values(source: &str, before: &toml::Table, after: &toml::Table) -> Result<String> {
    fn diff(
        text: &mut String,
        path: &mut Vec<String>,
        before: &toml::Table,
        after: &toml::Table,
    ) -> Result<()> {
        for key in before
            .keys()
            .chain(after.keys())
            .collect::<std::collections::BTreeSet<_>>()
        {
            path.push(key.clone());
            match (before.get(key), after.get(key)) {
                (Some(toml::Value::Table(a)), Some(toml::Value::Table(b))) => {
                    diff(text, path, a, b)?
                }
                (a, b) if a != b => *text = patch_toml(text, path, b)?,
                _ => (),
            }
            path.pop();
        }
        Ok(())
    }
    let mut text = source.to_owned();
    diff(&mut text, &mut vec![], before, after)?;
    Ok(text)
}

#[derive(Serialize)]
pub struct LocalConfiguration {
    pub settings: Settings,
    pub hash: String,
}
pub fn local_configuration(path: &Path) -> Result<LocalConfiguration> {
    no_symlinks(path)?;
    Ok(LocalConfiguration {
        settings: crate::load(path)?,
        hash: crate::content::hash(&fs::read(path)?),
    })
}
pub fn save_local_configuration(
    path: &Path,
    candidate: &Settings,
    expected_hash: &str,
) -> Result<()> {
    no_symlinks(path)?;
    let text = fs::read_to_string(path)?;
    ensure!(
        crate::content::hash(text.as_bytes()) == expected_hash,
        "Local configuration changed; reload before saving"
    );
    let current: Settings = toml::from_str(&text)?;
    ensure!(
        candidate.source == current.source
            && candidate.machine == current.machine
            && candidate.state_dir == current.state_dir
            && candidate.projects == current.projects,
        "Use workspace/project actions to change locations"
    );
    candidate.validate()?;
    let mut before = toml::Value::try_from(&current)?;
    // Compare the on-disk alert keys: deserialization upgrades v0.2.1 references.
    // Otherwise patching would leave token_env/user_env beside the new keys.
    if let Some(alert) = toml::from_str::<toml::Table>(&text)?.get("alert") {
        before["alert"] = alert.clone();
    }
    let after = toml::Value::try_from(candidate)?;
    let edited = edit_toml_values(
        &text,
        before.as_table().context("Invalid settings")?,
        after.as_table().context("Invalid settings")?,
    )?;
    atomic_write(path, edited.as_bytes())
}

#[derive(Serialize)]
pub struct ValueOption {
    pub name: String,
    pub label: String,
    pub value: String,
}
pub fn value_options(
    settings: &Settings,
    machine: &str,
    harness: &str,
) -> Result<Vec<ValueOption>> {
    let contexts = crate::engine::template_contexts(settings)?;
    let (_, _, context) = contexts
        .iter()
        .find(|(m, h, _)| m == machine && h == harness)
        .context("Select an enabled preview target")?;
    fn flatten(prefix: &str, table: &toml::Table, out: &mut Vec<ValueOption>) {
        for (key, value) in table {
            let name = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            if let Some(t) = value.as_table() {
                flatten(&name, t, out);
            } else if !value.is_array() {
                out.push(ValueOption {
                    name,
                    label: key.replace(['_', '-'], " "),
                    value: value
                        .as_str()
                        .map(str::to_owned)
                        .unwrap_or_else(|| value.to_string()),
                });
            }
        }
    }
    let mut out = vec![];
    flatten("", context, &mut out);
    Ok(out)
}

pub fn save_skill_draft(
    settings: &Settings,
    project: Option<&Path>,
    name: &str,
    markdown: &str,
    metadata: &str,
    markdown_hash: &str,
    metadata_hash: &str,
) -> Result<()> {
    let _lock = crate::config::operation_lock(settings, "apply.lock")?;
    save_skill_draft_locked(
        settings,
        project,
        name,
        markdown,
        metadata,
        markdown_hash,
        metadata_hash,
    )
}

pub(crate) fn save_skill_draft_locked(
    settings: &Settings,
    project: Option<&Path>,
    name: &str,
    markdown: &str,
    metadata: &str,
    markdown_hash: &str,
    metadata_hash: &str,
) -> Result<()> {
    let current = crate::read_skill(settings, project, name)?;
    ensure!(
        current.markdown_hash == markdown_hash && current.metadata_hash == metadata_hash,
        "Source changed; reload before saving"
    );
    let normalized = markdown.replace("\r\n", "\n");
    let (header, yaml) = frontmatter(&normalized)?;
    ensure!(
        yaml["name"].as_str() == Some(name),
        "Skill name must match directory {name}"
    );
    ensure!(
        yaml["description"].as_str().is_some(),
        "Skill description must be a string"
    );
    ensure!(
        !normalized[header.end..].trim().is_empty(),
        "Skill body is empty"
    );
    let (_, diagnostics) = crate::template::parse_blocks(markdown)?;
    ensure!(
        diagnostics.is_empty(),
        "Fix conditional-block errors before saving"
    );
    let selected = crate::content::source(settings, project)?;
    ensure_valid_skill_draft(
        settings,
        &selected,
        name,
        crate::engine::SkillDraft { markdown, metadata },
    )?;
    let root = selected.join("skills").join(name);
    if markdown != current.markdown {
        atomic_write(&root.join("SKILL.md"), markdown.as_bytes())?;
    }
    if metadata != current.metadata
        && let Err(error) = atomic_write(&root.join("skill.toml"), metadata.as_bytes())
    {
        atomic_write(&root.join("SKILL.md"), current.markdown.as_bytes())
            .context("Metadata save failed and Markdown rollback failed")?;
        return Err(error);
    }
    Ok(())
}

/// Description limits apply after conditions and variables are rendered for each target, rather
/// than to the template text. Only this skill can change, so only this skill is rendered, on
/// every machine × harness, and nothing is written.
pub(crate) fn ensure_valid_skill_draft(
    settings: &Settings,
    selected: &Path,
    name: &str,
    draft: crate::engine::SkillDraft<'_>,
) -> Result<()> {
    let validate = |draft| {
        crate::engine::validate_focus(
            &settings.source,
            selected,
            crate::engine::ValidationFocus::Skill { name, draft },
        )
    };
    let before = validate(None)?;
    let after = validate(Some(draft))?;
    let skill_path = format!("skills/{name}");
    let prefix = format!("{skill_path}/");
    ensure_no_new_problems(
        &before,
        &after,
        |path| path == skill_path || path.starts_with(&prefix),
        "Skill change",
    )
}

/// Existing problems elsewhere must not block incremental repairs, while new
/// failures in the edited source are checked on every machine and harness.
fn ensure_no_new_problems(
    before: &crate::ValidationReport,
    after: &crate::ValidationReport,
    edited: impl Fn(&str) -> bool,
    change: &str,
) -> Result<()> {
    let mut regressions = Vec::new();
    for problem in after.problems.iter().filter(|p| edited(&p.path)) {
        for context in &problem.contexts {
            let existing = before.problems.iter().any(|old| {
                old.path == problem.path
                    && old.message == problem.message
                    && old.contexts.contains(context)
            });
            if !existing {
                regressions.push(format!(
                    "{} ({}/{}): {}",
                    problem.path,
                    context.machine,
                    context.harness.as_deref().unwrap_or("profile"),
                    problem.message
                ));
            }
        }
    }
    ensure!(
        regressions.is_empty(),
        "{change} introduces validation errors: {}",
        regressions.join("; ")
    );
    Ok(())
}

/// Shared global instructions; an absent file reads as empty text.
pub fn read_global_instructions(settings: &Settings) -> Result<crate::SourceDocument> {
    crate::read_source_file(settings, GLOBAL_INSTRUCTIONS)
}

/// Validates the draft against every machine × harness before replacing
/// `global/AGENTS.md`; a missing file is created when `expected_hash` is the empty hash.
pub fn save_global_instructions(
    settings: &Settings,
    markdown: &str,
    expected_hash: &str,
) -> Result<()> {
    let _lock = crate::config::operation_lock(settings, "apply.lock")?;
    save_global_instructions_locked(settings, markdown, expected_hash)
}

pub(crate) fn save_global_instructions_locked(
    settings: &Settings,
    markdown: &str,
    expected_hash: &str,
) -> Result<()> {
    let current = read_global_instructions(settings)?;
    ensure!(
        current.hash == expected_hash,
        "Source changed; reload before saving"
    );
    ensure!(!markdown.trim().is_empty(), "Global instructions are empty");
    let (_, diagnostics) = crate::template::parse_blocks(markdown)?;
    ensure!(
        diagnostics.is_empty(),
        "Fix conditional-block errors before saving"
    );
    let validate = |draft| {
        crate::engine::validate_focus(
            &settings.source,
            &settings.source,
            crate::engine::ValidationFocus::GlobalInstructions { draft },
        )
    };
    let before = validate(None)?;
    let after = validate(Some(markdown))?;
    ensure_no_new_problems(
        &before,
        &after,
        |path| path == GLOBAL_INSTRUCTIONS,
        "Global instructions change",
    )?;
    atomic_write(
        &settings.source.join(GLOBAL_INSTRUCTIONS),
        markdown.as_bytes(),
    )
}

/// Renders a global instructions draft exactly as deployment does for one target.
pub fn preview_global_instructions(
    settings: &Settings,
    markdown: &str,
    machine: &str,
    harness: &str,
) -> Result<String> {
    let (context, _) = preview_target(settings, machine, harness)?;
    crate::engine::render_global_instructions(markdown, &context)
}

pub fn rename_skill(
    settings: &Settings,
    project: Option<&Path>,
    name: &str,
    new_name: &str,
    expected_hash: &str,
) -> Result<()> {
    let _lock = crate::config::operation_lock(settings, "apply.lock")?;
    rename_skill_locked(settings, project, name, new_name, expected_hash)
}

pub(crate) fn rename_skill_locked(
    settings: &Settings,
    project: Option<&Path>,
    name: &str,
    new_name: &str,
    expected_hash: &str,
) -> Result<()> {
    identifier(new_name)?;
    let document = crate::read_skill(settings, project, name)?;
    ensure!(
        document.markdown_hash == expected_hash,
        "Source changed; reload before renaming"
    );
    let source = crate::content::source(settings, project)?.join("skills");
    let to = source.join(new_name);
    no_symlinks(&to)?;
    ensure!(!to.try_exists()?, "A skill with that name already exists");
    let selected = crate::content::source(settings, project)?;
    let (_temp, candidate) = crate::content::draft_workspace(settings, &selected)?;
    let mut replacements = Vec::new();
    for (file, path) in crate::engine::file_paths(&source.join(name))? {
        if file.file_name().is_some_and(|n| n == "SKILL.md") {
            let original = fs::read_to_string(&path)?;
            let changed = yaml_field(&original, "name", new_name)?;
            crate::content::write_scratch(
                &candidate.source.join("skills").join(name).join(&file),
                changed.as_bytes(),
            )?;
            replacements.push((file, original, changed));
        }
    }
    fs::rename(
        candidate.source.join("skills").join(name),
        candidate.source.join("skills").join(new_name),
    )?;
    crate::engine::validate_sources(&candidate)?;
    // Rename the original directory only after validating every identity replacement.
    // Unrelated resources and empty directories travel with the original directory.
    for (index, (file, _, changed)) in replacements.iter().enumerate() {
        if let Err(error) = atomic_write(&source.join(name).join(file), changed.as_bytes()) {
            for (file, original, _) in &replacements[..index] {
                atomic_write(&source.join(name).join(file), original.as_bytes())?;
            }
            return Err(error);
        }
    }
    if let Err(error) = fs::rename(source.join(name), to) {
        for (file, original, _) in &replacements {
            atomic_write(&source.join(name).join(file), original.as_bytes())?;
        }
        return Err(error.into());
    }
    Ok(())
}

#[derive(Serialize)]
pub struct Resource {
    pub path: String,
    pub label: String,
}
pub fn skill_resources(
    settings: &Settings,
    project: Option<&Path>,
    name: &str,
) -> Result<Vec<Resource>> {
    identifier(name)?;
    let root = crate::content::source(settings, project)?
        .join("skills")
        .join(name);
    crate::engine::file_paths(&root)?
        .keys()
        .filter(|p| p.as_path() != Path::new("skill.toml"))
        .map(|p| {
            let path = p.to_string_lossy().replace('\\', "/");
            let label = if let Some(v) = path.strip_prefix("variants/") {
                let (condition, file) = v.split_once('/').context("Invalid variant resource")?;
                format!(
                    "{} version of {file}",
                    condition
                        .replace("os=", "")
                        .replace("harness=", "")
                        .replace("machine=", "")
                )
            } else {
                path.clone()
            };
            Ok(Resource { path, label })
        })
        .collect()
}

pub fn add_skill_resource(
    settings: &Settings,
    project: Option<&Path>,
    name: &str,
    file: &str,
    from: &Path,
) -> Result<()> {
    identifier(name)?;
    let file = relative(file)?;
    ensure!(
        file != Path::new("skill.toml") && file != Path::new("SKILL.md"),
        "Use the skill editor for identity files"
    );
    no_symlinks(from)?;
    let selected = crate::content::source(settings, project)?;
    let target = selected.join("skills").join(name).join(&file);
    no_symlinks(&target)?;
    ensure!(!target.try_exists()?, "Resource already exists");
    let (_temp, candidate) = crate::content::draft_workspace(settings, &selected)?;
    let bytes = fs::read(from)?;
    crate::content::write_scratch(
        &candidate.source.join("skills").join(name).join(file),
        &bytes,
    )?;
    crate::engine::validate_sources(&candidate)?;
    atomic_write(&target, &bytes)
}

pub fn skill_target_statuses(
    settings: &Settings,
) -> Result<BTreeMap<String, BTreeMap<String, bool>>> {
    let contexts = crate::engine::template_contexts(settings)?;
    let mut result = BTreeMap::new();
    for skill in crate::list_skills(settings)? {
        let doc = crate::read_skill(settings, skill.project.as_deref(), &skill.name)?;
        let table: toml::Table = toml::from_str(&doc.metadata)?;
        let mut targets = BTreeMap::new();
        for (_, h, context) in &contexts {
            let included = skill.enabled
                && match table.get("target") {
                    Some(target) => crate::template::selectors_match(
                        target.as_table().context("Target must be a table")?,
                        context,
                    )?,
                    None => true,
                };
            targets
                .entry(h.clone())
                .and_modify(|v| *v |= included)
                .or_insert(included);
        }
        result.insert(
            format!(
                "{}:{}",
                skill
                    .project
                    .as_deref()
                    .map(|p| p.to_string_lossy())
                    .unwrap_or_default(),
                skill.name
            ),
            targets,
        );
    }
    Ok(result)
}

/// The template context and `harnesses.toml` definition of an enabled machine × harness.
fn preview_target(
    settings: &Settings,
    machine: &str,
    harness: &str,
) -> Result<(toml::Table, toml::Table)> {
    let (_, _, context) = crate::engine::template_contexts(settings)?
        .into_iter()
        .find(|(m, h, _)| m == machine && h == harness)
        .context("Select an enabled preview target")?;
    let definitions = crate::structured_source(settings, "harnesses.toml")?;
    let target = definitions
        .values
        .get(harness)
        .and_then(|v| v.as_table())
        .context("Unknown agent")?
        .clone();
    Ok((context, target))
}

fn definition_text<'a>(target: &'a toml::Table, key: &str) -> Result<&'a str> {
    target
        .get(key)
        .and_then(|v| v.as_str())
        .with_context(|| format!("Agent lacks {key}"))
}

fn staging(context: &toml::Table) -> bool {
    context
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("staging")
        == "staging"
}

/// The agent's output root on `machine`, mirroring the deployment renderer.
fn preview_root(
    settings: &Settings,
    project: Option<&Path>,
    machine: &str,
    harness: &str,
    context: &toml::Table,
    target: &toml::Table,
) -> Result<PathBuf> {
    Ok(if staging(context) {
        let staging = settings.state_dir.join(machine).join("staging");
        if let Some(project) = project {
            use sha2::{Digest, Sha256};
            staging
                .join("projects")
                .join(format!(
                    "{:x}",
                    Sha256::digest(project.to_string_lossy().as_bytes())
                ))
                .join(harness)
        } else {
            staging.join(harness)
        }
    } else if let Some(project) = project {
        project.join(relative(definition_text(target, "project_root")?)?)
    } else {
        let os = context
            .get("os")
            .and_then(|v| v.as_str())
            .context("Machine lacks OS")?;
        let root = target
            .get("roots")
            .and_then(|v| v.get(os))
            .and_then(|v| v.as_str())
            .context("Agent lacks OS root")?;
        PathBuf::from(crate::template::render(root, context, false)?)
    })
}

pub fn preview_install_path(
    settings: &Settings,
    project: Option<&Path>,
    name: &str,
    machine: &str,
    harness: &str,
) -> Result<PathBuf> {
    identifier(name)?;
    let (context, target) = preview_target(settings, machine, harness)?;
    let root = preview_root(settings, project, machine, harness, &context, &target)?;
    Ok(root
        .join(relative(definition_text(&target, "skills_dir")?)?)
        .join(name))
}

/// Where `machine` installs `harness`'s global instructions file, including a
/// separate live `global_root` such as Codex's.
pub fn preview_global_instructions_path(
    settings: &Settings,
    machine: &str,
    harness: &str,
) -> Result<PathBuf> {
    let (context, target) = preview_target(settings, machine, harness)?;
    let root = match target.get("global_root").and_then(|v| v.as_str()) {
        Some(root) if !staging(&context) => {
            PathBuf::from(crate::template::render(root, &context, false)?)
        }
        _ => preview_root(settings, None, machine, harness, &context, &target)?,
    };
    Ok(root.join(relative(definition_text(&target, "global_file")?)?))
}

pub fn new_skill_markdown(name: &str, description: &str) -> Result<String> {
    identifier(name)?;
    ensure!(
        !description.trim().is_empty() && description.chars().count() <= 1024,
        "Description must contain 1–1024 characters"
    );
    Ok(format!(
        "---\nname: {name}\ndescription: {}\n---\n# {name}\n\n",
        serde_json::to_string(description)?
    ))
}

/// Align core-rendered versions so an omitted block doesn't mark identical later lines as changed.
pub fn different_preview_lines(text: &str, versions: &[String]) -> Vec<usize> {
    let mut changed = std::collections::BTreeSet::new();
    for version in versions {
        for change in similar::TextDiff::from_lines(version.as_str(), text).iter_all_changes() {
            if change.tag() == similar::ChangeTag::Insert
                && let Some(index) = change.new_index()
            {
                changed.insert(index);
            }
        }
    }
    changed.into_iter().collect()
}

#[cfg(test)]
mod lossless_tests;
#[cfg(test)]
mod tests;
