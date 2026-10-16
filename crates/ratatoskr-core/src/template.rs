use anyhow::{Context, Result, bail, ensure};
use regex::Regex;
use std::collections::BTreeMap;

pub(crate) fn selectors_match(selectors: &toml::Table, context: &toml::Table) -> Result<bool> {
    let mut matched = true;
    for (key, value) in selectors {
        let context_key = match key.as_str() {
            "harnesses" => "harness",
            "machines" => "machine",
            "os" => "os",
            "capabilities" => "capabilities",
            _ => bail!("Unknown target selector: {key}"),
        };
        let values = value
            .as_array()
            .context("Target selectors must be arrays")?;
        let values: Vec<&str> = values
            .iter()
            .map(|v| v.as_str().context("Selectors must contain strings"))
            .collect::<Result<_>>()?;
        if key == "capabilities" {
            let available = context.get(context_key).and_then(|v| v.as_array());
            matched &= values
                .iter()
                .all(|v| available.is_some_and(|a| a.iter().any(|x| x.as_str() == Some(v))));
        } else {
            let actual = context
                .get(context_key)
                .and_then(|v| v.as_str())
                .context("Missing selector context")?;
            matched &= values.contains(&actual);
        }
    }
    Ok(matched)
}

pub(crate) fn render(source: &str, context: &toml::Table, markdown: bool) -> Result<String> {
    let mut output = String::new();
    if markdown {
        let (blocks, diagnostics) = parse_blocks(source)?;
        ensure!(
            diagnostics.is_empty(),
            "{}",
            diagnostics
                .iter()
                .map(|d| d.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        );
        let mut position = 0;
        for block in blocks {
            output.push_str(&source[position..block.opening.start]);
            if block_matches(&block.selectors, context) {
                output.push_str(&source[block.body]);
            }
            position = block.closing.end;
        }
        output.push_str(&source[position..]);
    } else {
        output.push_str(source);
    }
    let variables = variable_regex()?;
    let mut rendered = String::new();
    let mut position = 0;
    for caps in variables.captures_iter(&output) {
        let tag = caps.get(0).context("Missing variable match")?;
        rendered.push_str(&output[position..tag.start()]);
        let name = &caps[1];
        rendered.push_str(&variable_value(name, context)?);
        position = tag.end();
    }
    rendered.push_str(&output[position..]);
    ensure!(
        !rendered.contains("{{af:"),
        "Malformed or recursive variable"
    );
    Ok(rendered)
}

use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct TextRange {
    pub from: usize,
    pub to: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillDiagnostic {
    pub range: TextRange,
    pub severity: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillBlock {
    pub range: TextRange,
    pub opening: TextRange,
    pub body: TextRange,
    pub closing: TextRange,
    pub selectors: BTreeMap<String, Vec<String>>,
    pub active: bool,
    pub reachable: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct SkillVariable {
    pub range: TextRange,
    pub name: String,
    pub value: Option<String>,
    pub active: bool,
}

#[derive(Serialize)]
pub struct SkillAnalysis {
    /// Half-open UTF-16 code-unit offsets, directly usable by JavaScript editors.
    pub blocks: Vec<SkillBlock>,
    pub variables: Vec<SkillVariable>,
    pub diagnostics: Vec<SkillDiagnostic>,
}

type OnlySelectors = BTreeMap<String, Vec<String>>;

pub(crate) struct ParsedBlock {
    pub(crate) opening: std::ops::Range<usize>,
    pub(crate) body: std::ops::Range<usize>,
    pub(crate) closing: std::ops::Range<usize>,
    pub(crate) selectors: BTreeMap<String, Vec<String>>,
}

pub(crate) fn parse_only_selectors(expression: &str) -> Result<BTreeMap<String, Vec<String>>> {
    let mut conditions = BTreeMap::new();
    for term in expression.split_whitespace() {
        let (key, values) = term
            .split_once('=')
            .context("Expected key=value in only block")?;
        ensure!(
            matches!(key, "harness" | "machine" | "os"),
            "Unknown only key: {key}"
        );
        let values: Vec<String> = values.split(',').map(str::to_owned).collect();
        for value in &values {
            super::config::identifier(value)?;
        }
        ensure!(
            conditions.insert(key.into(), values).is_none(),
            "Duplicate only key: {key}"
        );
    }
    ensure!(!conditions.is_empty(), "Only blocks need a condition");
    Ok(conditions)
}

fn block_matches(selectors: &BTreeMap<String, Vec<String>>, context: &toml::Table) -> bool {
    selectors.iter().all(|(key, values)| {
        context
            .get(key)
            .and_then(|v| v.as_str())
            .is_some_and(|v| values.iter().any(|candidate| candidate == v))
    })
}

fn utf16_range(source: &str, range: std::ops::Range<usize>) -> TextRange {
    TextRange {
        from: source[..range.start].encode_utf16().count(),
        to: source[..range.end].encode_utf16().count(),
    }
}

fn diagnostic(
    source: &str,
    range: std::ops::Range<usize>,
    severity: &str,
    message: String,
) -> SkillDiagnostic {
    SkillDiagnostic {
        range: utf16_range(source, range),
        severity: severity.into(),
        message,
    }
}

/// One parser serves deployment and editor analysis; ranges internally use UTF-8 bytes.
pub(crate) fn parse_blocks(source: &str) -> Result<(Vec<ParsedBlock>, Vec<SkillDiagnostic>)> {
    let tags = Regex::new(r"<!--\s*(only\b.*?)-->|<!--\s*(/only)\s*-->")?;
    let mut blocks = Vec::new();
    let mut diagnostics = Vec::new();
    let mut active: Option<(std::ops::Range<usize>, OnlySelectors)> = None;
    for caps in tags.captures_iter(source) {
        let tag = caps.get(0).context("Missing template match")?;
        if caps.get(2).is_some() {
            if let Some((opening, selectors)) = active.take() {
                blocks.push(ParsedBlock {
                    body: opening.end..tag.start(),
                    opening,
                    closing: tag.range(),
                    selectors,
                });
            } else {
                diagnostics.push(diagnostic(
                    source,
                    tag.range(),
                    "error",
                    "Unexpected /only tag".into(),
                ));
            }
        } else if active.is_some() {
            diagnostics.push(diagnostic(
                source,
                tag.range(),
                "error",
                "Nested only blocks are not supported".into(),
            ));
        } else {
            let expression = caps.get(1).context("Missing only expression")?.as_str()[4..].trim();
            let selectors = match parse_only_selectors(expression) {
                Ok(selectors) => selectors,
                Err(error) => {
                    diagnostics.push(diagnostic(source, tag.range(), "error", error.to_string()));
                    BTreeMap::new()
                }
            };
            active = Some((tag.range(), selectors));
        }
    }
    if let Some((opening, _)) = active {
        diagnostics.push(diagnostic(
            source,
            opening,
            "error",
            "Unclosed only block".into(),
        ));
    }
    Ok((blocks, diagnostics))
}

fn variable_value(name: &str, context: &toml::Table) -> Result<String> {
    let mut pieces = name.split('.');
    let mut value = context
        .get(pieces.next().context("Empty variable")?)
        .with_context(|| format!("Undefined variable: {name}"))?;
    for piece in pieces {
        value = value
            .get(piece)
            .with_context(|| format!("Undefined variable: {name}"))?;
    }
    match value {
        toml::Value::String(s) => Ok(s.clone()),
        toml::Value::Integer(i) => Ok(i.to_string()),
        toml::Value::Boolean(b) => Ok(b.to_string()),
        _ => bail!("Variable must be a scalar: {name}"),
    }
}

fn variable_regex() -> Result<Regex> {
    Ok(Regex::new(r"\{\{af:([^{}]+)\}\}")?)
}

pub fn analyze_skill(
    settings: &crate::Settings,
    name: &str,
    markdown: &str,
    machine: &str,
    harness: &str,
) -> Result<SkillAnalysis> {
    super::config::identifier(name)?;
    let contexts = crate::engine::template_contexts(settings)?;
    let context = contexts
        .iter()
        .find(|(m, h, _)| m == machine && h == harness)
        .context("Select an enabled machine × harness")?;
    let context = &context.2;
    let (parsed, mut diagnostics) = parse_blocks(markdown)?;
    let mut blocks = Vec::new();
    for block in &parsed {
        let reachable = contexts
            .iter()
            .any(|(_, _, c)| block_matches(&block.selectors, c));
        if !reachable {
            diagnostics.push(diagnostic(
                markdown,
                block.opening.clone(),
                "warning",
                "This section matches no enabled machine × harness".into(),
            ));
        }
        blocks.push(SkillBlock {
            range: utf16_range(markdown, block.opening.start..block.closing.end),
            opening: utf16_range(markdown, block.opening.clone()),
            body: utf16_range(markdown, block.body.clone()),
            closing: utf16_range(markdown, block.closing.clone()),
            selectors: block.selectors.clone(),
            active: block_matches(&block.selectors, context),
            reachable,
        });
    }
    let regex = variable_regex()?;
    let mut variables = Vec::new();
    for caps in regex.captures_iter(markdown) {
        let tag = caps.get(0).context("Missing variable match")?;
        let name = &caps[1];
        let active = parsed
            .iter()
            .filter(|b| b.body.contains(&tag.start()))
            .all(|b| block_matches(&b.selectors, context));
        let value = match variable_value(name, context) {
            Ok(value) => Some(value),
            Err(error) => {
                if active {
                    diagnostics.push(diagnostic(
                        markdown,
                        tag.range(),
                        "error",
                        error.to_string(),
                    ));
                }
                None
            }
        };
        variables.push(SkillVariable {
            range: utf16_range(markdown, tag.range()),
            name: name.into(),
            value,
            active,
        });
    }
    if !diagnostics.iter().any(|d| d.severity == "error") {
        // Reuse the renderer and frontmatter validation instead of inventing an editor grammar.
        match render(markdown, context, true) {
            Ok(rendered) => {
                let rendered = rendered.replace("\r\n", "\n");
                let header_end = markdown
                    .find("\n---\n")
                    .map_or(markdown.len(), |end| end + 5);
                match skill_frontmatter(&rendered, name) {
                    Ok((_, body)) if body.trim().is_empty() => diagnostics.push(diagnostic(
                        markdown,
                        0..markdown.len(),
                        "error",
                        "Skill body is empty".into(),
                    )),
                    Ok(_) => (),
                    Err(error) => diagnostics.push(diagnostic(
                        markdown,
                        0..header_end,
                        "error",
                        error.to_string(),
                    )),
                }
            }
            Err(error) => diagnostics.push(diagnostic(
                markdown,
                0..markdown.len(),
                "error",
                error.to_string(),
            )),
        }
    }
    Ok(SkillAnalysis {
        blocks,
        variables,
        diagnostics,
    })
}

pub(crate) fn skill_frontmatter<'a>(
    rendered: &'a str,
    name: &str,
) -> Result<(serde_yaml_ng::Value, &'a str)> {
    let rest = rendered
        .strip_prefix("---\n")
        .context("SKILL.md requires YAML frontmatter")?;
    let (frontmatter, body) = rest
        .split_once("\n---\n")
        .context("Unclosed YAML frontmatter")?;
    let yaml: serde_yaml_ng::Value = serde_yaml_ng::from_str(frontmatter)?;
    let mut errors = Vec::new();
    if yaml["name"].as_str() != Some(name) {
        errors.push(format!("Skill name must match directory {name}"));
    }
    match yaml["description"].as_str() {
        None => errors.push("Skill description must be a string".into()),
        Some(description)
            if description.trim().is_empty() || description.chars().count() > 1024 =>
        {
            errors.push("Invalid skill description (expected 1–1024 characters)".into())
        }
        Some(_) => (),
    }
    ensure!(errors.is_empty(), "{}", errors.join("; "));
    Ok((yaml, body))
}
