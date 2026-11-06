//! Pure Markdown, frontmatter and metadata draft edits.
use super::*;
use serde::Deserialize;

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

pub(super) fn frontmatter(source: &str) -> Result<(std::ops::Range<usize>, serde_yaml_ng::Value)> {
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

pub(super) fn yaml_field(source: &str, key: &str, value: &str) -> Result<String> {
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
                // Selecting whole lines picks up the last line's break. Left inside, it would
                // become a blank line at the end of the section in every rendered file. The break
                // stays exactly as selected, because a file can mix line endings.
                let selected = &markdown[from..to];
                let body = selected
                    .strip_suffix("\r\n")
                    .or_else(|| selected.strip_suffix('\n'))
                    .unwrap_or(selected);
                let after = &selected[body.len()..];
                result.markdown.replace_range(
                    from..to,
                    &format!("{opening}{nl}{body}{nl}<!-- /only -->{after}"),
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
