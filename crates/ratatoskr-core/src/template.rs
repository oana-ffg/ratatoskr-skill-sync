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
        let tags = Regex::new(r"<!--\s*(only\b.*?)-->|<!--\s*(/only)\s*-->")?;
        let mut active = None;
        let mut position = 0;
        for caps in tags.captures_iter(source) {
            let tag = caps.get(0).context("Missing template match")?;
            if active != Some(false) {
                output.push_str(&source[position..tag.start()]);
            }
            if caps.get(2).is_some() {
                ensure!(active.is_some(), "Unexpected /only tag");
                active = None;
            } else {
                ensure!(active.is_none(), "Nested only blocks are not supported");
                let expression =
                    caps.get(1).context("Missing only expression")?.as_str()[4..].trim();
                let mut conditions = BTreeMap::new();
                for term in expression.split_whitespace() {
                    let (key, values) = term
                        .split_once('=')
                        .context("Expected key=value in only block")?;
                    ensure!(
                        matches!(key, "harness" | "machine" | "os"),
                        "Unknown only key: {key}"
                    );
                    ensure!(
                        conditions.insert(key, values).is_none(),
                        "Duplicate only key: {key}"
                    );
                    for value in values.split(',') {
                        super::config::identifier(value)?;
                    }
                }
                ensure!(!conditions.is_empty(), "Only blocks need a condition");
                active = Some(conditions.iter().all(|(key, values)| {
                    context
                        .get(*key)
                        .and_then(|v| v.as_str())
                        .is_some_and(|v| values.split(',').any(|candidate| candidate == v))
                }));
            }
            position = tag.end();
        }
        ensure!(active.is_none(), "Unclosed only block");
        output.push_str(&source[position..]);
    } else {
        output.push_str(source);
    }
    let variables = Regex::new(r"\{\{af:([^{}]+)\}\}")?;
    let mut rendered = String::new();
    let mut position = 0;
    for caps in variables.captures_iter(&output) {
        let tag = caps.get(0).context("Missing variable match")?;
        rendered.push_str(&output[position..tag.start()]);
        let name = &caps[1];
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
            toml::Value::String(s) => rendered.push_str(s),
            toml::Value::Integer(i) => rendered.push_str(&i.to_string()),
            toml::Value::Boolean(b) => rendered.push_str(&b.to_string()),
            _ => bail!("Variable must be a scalar: {name}"),
        }
        position = tag.end();
    }
    rendered.push_str(&output[position..]);
    ensure!(
        !rendered.contains("{{af:"),
        "Malformed or recursive variable"
    );
    Ok(rendered)
}
