//! Lossless desktop operations. Template syntax and source serialization stay in Rust.
use crate::{
    Settings,
    config::{atomic_write, identifier, no_symlinks, relative},
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};
use toml_edit::{DocumentMut, Item};

#[derive(Serialize)]
pub struct EditorView {
    pub description: String,
    pub description_sections: Vec<DescriptionSection>,
    pub description_error: Option<String>,
    pub frontmatter: crate::TextRange,
    pub metadata: toml::Table,
}

#[derive(Serialize)]
pub struct DescriptionSection {
    pub text: String,
    pub selectors: Option<BTreeMap<String, Vec<String>>>,
    #[serde(skip)]
    range: std::ops::Range<usize>,
    #[serde(skip)]
    body: std::ops::Range<usize>,
}

fn description_sections(description: &str) -> Result<Vec<DescriptionSection>> {
    let (blocks, errors) = crate::template::parse_blocks(description)?;
    ensure!(
        errors.is_empty(),
        "Fix description template errors in Markdown source"
    );
    let mut sections = Vec::new();
    let mut position = 0;
    for block in blocks {
        // Keep empty shared slots so queued typing cannot shift a condition's index.
        let range = position..block.opening.start;
        sections.push(DescriptionSection {
            text: description[range.clone()].into(),
            selectors: None,
            body: range.clone(),
            range,
        });
        sections.push(DescriptionSection {
            text: description[block.body.clone()].trim_start().into(),
            selectors: Some(block.selectors),
            range: block.opening.start..block.closing.end,
            body: block.body,
        });
        position = block.closing.end;
    }
    let range = position..description.len();
    sections.push(DescriptionSection {
        text: description[range.clone()].into(),
        selectors: None,
        body: range.clone(),
        range,
    });
    Ok(sections)
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum DescriptionEdit {
    Text {
        index: usize,
        value: String,
    },
    Append {
        value: String,
        selectors: BTreeMap<String, Vec<String>>,
    },
    Only {
        index: usize,
        selectors: BTreeMap<String, Vec<String>>,
    },
    RemoveOnly {
        index: usize,
    },
}

fn edit_description(description: &str, edit: DescriptionEdit) -> Result<String> {
    let sections = description_sections(description)?;
    let mut result = description.to_owned();
    match edit {
        DescriptionEdit::Append { value, selectors } => {
            ensure!(!value.trim().is_empty(), "Enter description text");
            result.push_str(&format!(
                "{} {value}<!-- /only -->",
                only_opening(&selectors)?
            ));
        }
        DescriptionEdit::Text { index, value } => {
            let section = sections
                .get(index)
                .context("Description section changed; try again")?;
            // Conditional leading whitespace separates sentences and belongs inside
            // the condition. Keep it when the field edits only the visible text.
            let start = if section.selectors.is_some() {
                section.body.end - section.text.len()
            } else {
                section.body.start
            };
            result.replace_range(start..section.body.end, &value);
        }
        DescriptionEdit::Only { index, selectors } => {
            let section = sections
                .get(index)
                .context("Description section changed; try again")?;
            ensure!(
                section.selectors.is_some(),
                "Select a conditional description section"
            );
            result.replace_range(
                section.range.start..section.body.start,
                &only_opening(&selectors)?,
            );
        }
        DescriptionEdit::RemoveOnly { index } => {
            let section = sections
                .get(index)
                .context("Description section changed; try again")?;
            ensure!(
                section.selectors.is_some(),
                "Select a conditional description section"
            );
            let text = description[section.body.clone()].to_owned();
            result.replace_range(section.range.clone(), &text);
        }
    }
    // Reject pasted nested or unbalanced tags before changing the source.
    description_sections(&result)?;
    Ok(result)
}

fn only_opening(selectors: &BTreeMap<String, Vec<String>>) -> Result<String> {
    let expression = selectors
        .iter()
        .map(|(k, v)| format!("{k}={}", v.join(",")))
        .collect::<Vec<_>>()
        .join(" ");
    crate::template::parse_only_selectors(&expression)?;
    Ok(format!("<!-- only {expression} -->"))
}

fn frontmatter(source: &str) -> Result<(std::ops::Range<usize>, serde_yaml_ng::Value)> {
    let newline = if source.starts_with("---\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let prefix = format!("---{newline}");
    let rest = source
        .strip_prefix(&prefix)
        .context("SKILL.md requires YAML frontmatter")?;
    let closing = format!("{newline}---{newline}");
    let end = rest.find(&closing).context("Unclosed YAML frontmatter")?;
    let yaml = serde_yaml_ng::from_str(&rest[..end])?;
    Ok((0..prefix.len() + end + closing.len(), yaml))
}

pub fn editor_view(markdown: &str, metadata: &str) -> Result<EditorView> {
    let (range, yaml) = frontmatter(markdown)?;
    let description = yaml["description"].as_str().unwrap_or_default();
    let (sections, description_error) = match description_sections(description) {
        Ok(sections) => (sections, None),
        Err(error) => (Vec::new(), Some(error.to_string())),
    };
    Ok(EditorView {
        description: description.into(),
        description_sections: sections,
        description_error,
        frontmatter: crate::TextRange {
            from: 0,
            to: markdown[..range.end].encode_utf16().count(),
        },
        metadata: toml::from_str(metadata)?,
    })
}

fn yaml_field(source: &str, key: &str, value: &str) -> Result<String> {
    let (range, yaml) = frontmatter(source)?;
    if yaml[key].as_str() == Some(value) {
        return Ok(source.into());
    }
    use std::str::FromStr;
    let newline = if source.starts_with("---\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let start = 3 + newline.len();
    let end = range.end - (3 + newline.len());
    let header = &source[start..end];
    let syntax = yaml_edit::YamlFile::from_str(header)?;
    let document = syntax.documents().next().context("Missing YAML document")?;
    let node = document
        .get(key)
        .with_context(|| format!("Missing {key} field"))?;
    let scalar = node
        .as_scalar()
        .context("Identity fields must be YAML scalars")?;
    let position = scalar.byte_range();
    let from = start + position.start as usize;
    use yaml_edit::AsYaml;
    let comment = scalar
        .as_node()
        .and_then(|node| {
            node.descendants_with_tokens()
                .filter_map(|e| e.into_token())
                .find(|token| token.kind() == yaml_edit::SyntaxKind::COMMENT)
        })
        .map(|token| u32::from(token.text_range().start()) as usize);
    let to = start + comment.unwrap_or(position.end as usize);
    let mut result = source.to_owned();
    let original = &source[from..to];
    // A block scalar includes its terminating newline. Keep the separator before the next key.
    let suffix = if original.ends_with("\r\n") {
        "\r\n"
    } else if original.ends_with('\n') {
        "\n"
    } else {
        ""
    };
    result.replace_range(
        from..to,
        &format!("{}{suffix}", serde_json::to_string(value)?),
    );
    // Independent validation catches any parser/edit discrepancy before returning a draft.
    let (_, check) = frontmatter(&result)?;
    ensure!(
        check[key].as_str() == Some(value),
        "Could not edit {key} losslessly"
    );
    Ok(result)
}

fn byte_offset(source: &str, units: usize) -> Result<usize> {
    let mut used = 0;
    for (index, ch) in source.char_indices() {
        if used == units {
            return Ok(index);
        }
        used += ch.len_utf16();
    }
    ensure!(used == units, "Invalid UTF-16 edit boundary");
    Ok(source.len())
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum DraftEdit {
    Description {
        value: String,
    },
    DescriptionSection {
        edit: DescriptionEdit,
    },
    Metadata {
        path: Vec<String>,
        value: Option<toml::Value>,
    },
    Only {
        from: usize,
        to: usize,
        selectors: BTreeMap<String, Vec<String>>,
        existing: bool,
    },
    RemoveOnly {
        from: usize,
    },
    Variable {
        from: usize,
        to: usize,
        name: String,
    },
}
#[derive(Serialize)]
pub struct Draft {
    pub markdown: String,
    pub metadata: String,
}

/// One syntax-producing entry point for editor commands; all offsets are UTF-16.
pub fn edit_draft(markdown: &str, metadata: &str, edit: DraftEdit) -> Result<Draft> {
    let mut result = Draft {
        markdown: markdown.into(),
        metadata: metadata.into(),
    };
    match edit {
        DraftEdit::Description { value } => {
            result.markdown = yaml_field(markdown, "description", &value)?
        }
        DraftEdit::DescriptionSection { edit } => {
            let (_, yaml) = frontmatter(markdown)?;
            let description = yaml["description"]
                .as_str()
                .context("Description must be a string")?;
            result.markdown = yaml_field(
                markdown,
                "description",
                &edit_description(description, edit)?,
            )?;
        }
        DraftEdit::Metadata { path, value } => {
            result.metadata = patch_toml(metadata, &path, value.as_ref())?
        }
        DraftEdit::Variable { from, to, name } => {
            ensure!(
                !name.is_empty()
                    && name.split('.').all(|p| !p.is_empty()
                        && p.chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')),
                "Invalid variable name"
            );
            let from = byte_offset(markdown, from)?;
            let to = byte_offset(markdown, to)?;
            ensure!(from <= to, "Invalid selection");
            result
                .markdown
                .replace_range(from..to, &format!("{{{{af:{name}}}}}"));
        }
        DraftEdit::Only {
            from,
            to,
            selectors,
            existing,
        } => {
            let opening = only_opening(&selectors)?;
            let from = byte_offset(markdown, from)?;
            let to = byte_offset(markdown, to)?;
            ensure!(from <= to, "Invalid selection");
            let (blocks, errors) = crate::template::parse_blocks(markdown)?;
            ensure!(
                errors.is_empty(),
                "Fix template errors before editing conditions"
            );
            if existing {
                let b = blocks
                    .iter()
                    .find(|b| b.opening.start == from)
                    .context("Block changed; try again")?;
                result.markdown.replace_range(b.opening.clone(), &opening);
            } else {
                ensure!(
                    !blocks
                        .iter()
                        .any(|b| (from >= b.opening.start || to > b.opening.start)
                            && from < b.closing.end),
                    "Nested or overlapping conditions are not supported"
                );
                let nl = if markdown.contains("\r\n") {
                    "\r\n"
                } else {
                    "\n"
                };
                result.markdown.replace_range(
                    from..to,
                    &format!("{opening}{nl}{}{nl}<!-- /only -->", &markdown[from..to]),
                );
            }
        }
        DraftEdit::RemoveOnly { from } => {
            let from = byte_offset(markdown, from)?;
            let (blocks, _) = crate::template::parse_blocks(markdown)?;
            let b = blocks
                .iter()
                .find(|b| b.opening.start == from)
                .context("Block changed; try again")?;
            result.markdown.replace_range(b.closing.clone(), "");
            result.markdown.replace_range(b.opening.clone(), "");
        }
    }
    Ok(result)
}

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
    let (_temp, candidate) = crate::content::draft_workspace(settings, &selected)?;
    let before = crate::engine::validate_source(&candidate.source)?;
    let root = candidate.source.join("skills").join(name);
    atomic_write(&root.join("SKILL.md"), markdown.as_bytes())?;
    atomic_write(&root.join("skill.toml"), metadata.as_bytes())?;
    // Description limits apply after conditions and variables are rendered for
    // each target, rather than to the combined template text above.
    crate::engine::validate_sources(&candidate)?;
    let after = crate::engine::validate_source(&candidate.source)?;
    let skill_path = format!("skills/{name}");
    let prefix = format!("{skill_path}/");
    let mut regressions = Vec::new();
    // Existing remote problems must not block incremental repairs, while new
    // failures of this skill must be checked on every machine and harness.
    for problem in after
        .problems
        .iter()
        .filter(|p| p.path == skill_path || p.path.starts_with(&prefix))
    {
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
        "Skill change introduces validation errors: {}",
        regressions.join("; ")
    );
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

pub fn rename_skill(
    settings: &Settings,
    project: Option<&Path>,
    name: &str,
    new_name: &str,
    expected_hash: &str,
) -> Result<()> {
    identifier(new_name)?;
    let _lock = crate::config::operation_lock(settings, "apply.lock")?;
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
            atomic_write(
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
    atomic_write(
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

pub fn preview_install_path(
    settings: &Settings,
    project: Option<&Path>,
    name: &str,
    machine: &str,
    harness: &str,
) -> Result<PathBuf> {
    identifier(name)?;
    let contexts = crate::engine::template_contexts(settings)?;
    let (_, _, context) = contexts
        .iter()
        .find(|(m, h, _)| m == machine && h == harness)
        .context("Select an enabled preview target")?;
    let definitions = crate::structured_source(settings, "harnesses.toml")?;
    let target = definitions.values.get(harness).context("Unknown agent")?;
    let text = |key: &str| {
        target
            .get(key)
            .and_then(|v| v.as_str())
            .with_context(|| format!("Agent lacks {key}"))
    };
    let mode = context
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("staging");
    let base = if mode == "staging" {
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
        project.join(relative(text("project_root")?)?)
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
    };
    Ok(base.join(relative(text("skills_dir")?)?).join(name))
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
mod lossless_tests {
    use super::*;
    #[test]
    fn yaml_scalar_ranges_keep_inline_comments_flow_mappings_and_multiline_fields() -> Result<()> {
        for (header, expected) in [
            (
                "name: sample\ndescription: 'old' # keep\nlicense: MIT\n",
                "name: sample\ndescription: \"new\" # keep\nlicense: MIT\n",
            ),
            (
                "{name: sample, description: old, license: MIT}\n",
                "{name: sample, description: \"new\", license: MIT}\n",
            ),
            (
                "name: sample\ndescription: >-\n  old words\n# keep\nlicense: MIT\n",
                "name: sample\ndescription: \"new\"\n# keep\nlicense: MIT\n",
            ),
        ] {
            let md = format!("---\n{header}---\n🐿 body\n");
            assert_eq!(
                yaml_field(&md, "description", "new")?,
                format!("---\n{expected}---\n🐿 body\n")
            );
        }
        Ok(())
    }
    #[test]
    fn toml_single_field_keeps_every_other_byte() -> Result<()> {
        let source =
            "# header\r\nenabled = true # retained\r\n[target]\r\nharnesses = ['codex']\r\n";
        assert_eq!(
            patch_toml(source, &["enabled".into()], Some(&false.into()))?,
            source.replace("true", "false")
        );
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conditional_descriptions_render_per_harness_and_preserve_unrelated_source() -> Result<()> {
        let original = "---\r\nname: sample\r\ndescription: 'Review 🐿' # keep\r\nlicense: MIT\r\n---\r\nBody unchanged.\r\n";
        let metadata = "# untouched\nenabled = true\n";
        let draft = edit_draft(
            original,
            metadata,
            DraftEdit::DescriptionSection {
                edit: DescriptionEdit::Append {
                    value: "Luna, you must ALWAYS load this, no exceptions".into(),
                    selectors: BTreeMap::from([("harness".into(), vec!["codex".into()])]),
                },
            },
        )?;
        assert_eq!(draft.metadata, metadata);
        assert!(
            draft
                .markdown
                .ends_with(" # keep\r\nlicense: MIT\r\n---\r\nBody unchanged.\r\n")
        );
        let view = editor_view(&draft.markdown, metadata)?;
        assert_eq!(view.description_sections.len(), 3);
        assert_eq!(
            view.description_sections[1].text,
            "Luna, you must ALWAYS load this, no exceptions"
        );
        for harness in ["codex", "claude"] {
            let context: toml::Table = [("harness".into(), harness.into())].into_iter().collect();
            let rendered =
                crate::template::render(&draft.markdown, &context, true)?.replace("\r\n", "\n");
            let (yaml, _) = crate::template::skill_frontmatter(&rendered, "sample")?;
            assert_eq!(
                yaml["description"].as_str().unwrap().contains("Luna"),
                harness == "codex"
            );
            if harness == "claude" {
                assert_eq!(yaml["description"].as_str(), Some("Review 🐿"));
            }
            assert!(!rendered.contains("<!--"));
        }
        // Typing repeatedly into a section addresses the current section, not stale offsets.
        let changed = edit_draft(
            &draft.markdown,
            metadata,
            DraftEdit::DescriptionSection {
                edit: DescriptionEdit::Text {
                    index: 0,
                    value: "Longer shared text: \"quoted\" 🐿".into(),
                },
            },
        )?;
        let changed = edit_draft(
            &changed.markdown,
            metadata,
            DraftEdit::DescriptionSection {
                edit: DescriptionEdit::Text {
                    index: 1,
                    value: "Updated: \"Luna\"".into(),
                },
            },
        )?;
        let changed = edit_draft(
            &changed.markdown,
            metadata,
            DraftEdit::DescriptionSection {
                edit: DescriptionEdit::Only {
                    index: 1,
                    selectors: BTreeMap::from([("harness".into(), vec!["claude".into()])]),
                },
            },
        )?;
        assert_eq!(
            editor_view(&changed.markdown, metadata)?.description_sections[1]
                .selectors
                .as_ref()
                .unwrap()["harness"],
            vec!["claude"]
        );
        let unwrapped = edit_draft(
            &changed.markdown,
            metadata,
            DraftEdit::DescriptionSection {
                edit: DescriptionEdit::RemoveOnly { index: 1 },
            },
        )?;
        assert_eq!(
            editor_view(&unwrapped.markdown, metadata)?.description,
            "Longer shared text: \"quoted\" 🐿 Updated: \"Luna\""
        );
        Ok(())
    }

    #[test]
    fn broken_description_conditions_remain_openable_for_source_repair() -> Result<()> {
        let source = "---\nname: sample\ndescription: 'Shared <!-- only harness=codex -->broken'\n---\nBody\n";
        let view = editor_view(source, "")?;
        assert!(view.description_error.is_some());
        assert!(view.description_sections.is_empty());
        assert!(
            edit_draft(
                source,
                "",
                DraftEdit::DescriptionSection {
                    edit: DescriptionEdit::Text {
                        index: 0,
                        value: "oops".into()
                    }
                }
            )
            .is_err()
        );
        Ok(())
    }
    #[test]
    fn structured_edits_preserve_unrelated_bytes_and_noops() {
        let md = "---\r\nname: sample\r\ndescription: 'old'\r\n# retained\r\nlicense: MIT\r\n---\r\n🐿 Body\r\n";
        let meta = "# comment\r\nenabled = true # keep\r\n[target]\r\nharnesses = ['codex']\r\n";
        assert_eq!(
            edit_draft(
                md,
                meta,
                DraftEdit::Description {
                    value: "old".into()
                }
            )
            .unwrap()
            .markdown,
            md
        );
        let result = edit_draft(
            md,
            meta,
            DraftEdit::Description {
                value: "new".into(),
            },
        )
        .unwrap();
        assert_eq!(
            result.markdown,
            md.replace("description: 'old'", "description: \"new\"")
        );
        assert_eq!(result.metadata, meta);
        assert_eq!(
            patch_toml(meta, &["enabled".into()], Some(&true.into())).unwrap(),
            meta
        );
        let edited = patch_toml(meta, &["enabled".into()], Some(&false.into())).unwrap();
        assert!(
            edited.contains("# comment")
                && edited.contains("# keep")
                && edited.contains("['codex']")
        );
    }
    #[test]
    fn conditions_and_values_share_parser_and_utf16_boundaries() {
        let md = "🐿 hello\r\n";
        let result = edit_draft(
            md,
            "",
            DraftEdit::Only {
                from: 3,
                to: 8,
                selectors: BTreeMap::from([("harness".into(), vec!["codex".into()])]),
                existing: false,
            },
        )
        .unwrap();
        assert_eq!(
            result.markdown,
            "🐿 <!-- only harness=codex -->\r\nhello\r\n<!-- /only -->\r\n"
        );
        let removed = edit_draft(&result.markdown, "", DraftEdit::RemoveOnly { from: 3 }).unwrap();
        assert_eq!(removed.markdown, "🐿 \r\nhello\r\n\r\n");
        assert!(
            edit_draft(
                md,
                "",
                DraftEdit::Variable {
                    from: 1,
                    to: 1,
                    name: "paths.archive".into()
                }
            )
            .is_err()
        );
    }
}
