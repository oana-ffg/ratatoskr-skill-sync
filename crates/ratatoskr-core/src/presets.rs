use crate::{
    Settings, SourceDocument,
    config::{identifier, relative},
    read_source_file,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTarget {
    pub tag: String,
    pub global_folder: String,
    pub project_folder: String,
    pub instructions_file: String,
    /// An independent global instruction root, needed by Codex's current layout.
    pub instructions_folder: Option<String>,
}

#[derive(Serialize)]
pub struct AgentPreset {
    pub title: String,
    pub target: AgentTarget,
    pub sources: Vec<String>,
}

// Official path documentation, verified 2026-10-16. Defaults only; environment overrides
// such as CODEX_HOME and XDG_CONFIG_HOME belong in the editable source definitions.
pub fn agent_presets() -> Vec<AgentPreset> {
    vec![
AgentPreset {
 title: "Claude Code".into(),
 target: AgentTarget {
 tag: "claude".into(),
 global_folder: "{{af:home}}/.claude".into(),
 project_folder: ".claude".into(),
 instructions_file: "CLAUDE.md".into(),
 instructions_folder: None,
 },
 sources: vec!["https://code.claude.com/docs/en/skills".into(), "https://code.claude.com/docs/en/memory".into()],
},
AgentPreset {
 title: "Codex".into(),
 target: AgentTarget {
 tag: "codex".into(),
 global_folder: "{{af:home}}/.agents".into(),
 project_folder: ".agents".into(),
 instructions_file: "AGENTS.md".into(),
 instructions_folder: Some("{{af:home}}/.codex".into()),
 },
 sources: vec!["https://developers.openai.com/codex/skills/".into(), "https://developers.openai.com/codex/guides/agents-md/".into()],
},
AgentPreset {
 title: "Gemini CLI".into(),
 target: AgentTarget {
 tag: "gemini".into(),
 global_folder: "{{af:home}}/.gemini".into(),
 project_folder: ".gemini".into(),
 instructions_file: "GEMINI.md".into(),
 instructions_folder: None,
 },
 sources: vec!["https://geminicli.com/docs/cli/skills/".into(), "https://geminicli.com/docs/cli/gemini-md/".into()],
},
AgentPreset {
 title: "GitHub Copilot CLI".into(),
 target: AgentTarget {
 tag: "copilot".into(),
 global_folder: "{{af:home}}/.copilot".into(),
 project_folder: ".github".into(),
 instructions_file: "copilot-instructions.md".into(),
 instructions_folder: None,
 },
 sources: vec!["https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-skills".into(), "https://docs.github.com/en/copilot/how-tos/copilot-cli/customize-copilot/add-custom-instructions".into()],
},
AgentPreset {
 title: "OpenCode".into(),
 target: AgentTarget {
 tag: "opencode".into(),
 global_folder: "{{af:home}}/.config/opencode".into(),
 project_folder: ".opencode".into(),
 instructions_file: "AGENTS.md".into(),
 instructions_folder: None,
 },
 sources: vec!["https://opencode.ai/docs/skills/".into(), "https://opencode.ai/docs/rules/".into()],
}
    ]
}

pub(crate) fn target_definition(target: &AgentTarget) -> Result<toml::Table> {
    identifier(&target.tag)?;
    relative(&target.project_folder)?;
    relative(&target.instructions_file)?;
    ensure!(
        std::path::Path::new(&target.instructions_file)
            .components()
            .count()
            == 1,
        "Instructions filename must be one filename"
    );
    ensure!(
        !target.global_folder.trim().is_empty(),
        "Global folder is required"
    );
    let mut table = toml::Table::new();
    table.insert("skills_dir".into(), "skills".into());
    table.insert(
        "global_file".into(),
        target.instructions_file.clone().into(),
    );
    table.insert("project_root".into(), target.project_folder.clone().into());
    let roots: BTreeMap<_, _> = ["macos", "windows", "linux"]
        .into_iter()
        .map(|os| (os, &target.global_folder))
        .collect();
    table.insert("roots".into(), toml::Value::try_from(roots)?);
    if let Some(root) = &target.instructions_folder {
        ensure!(
            !root.trim().is_empty(),
            "Instructions folder cannot be empty"
        );
        table.insert("global_root".into(), root.clone().into());
    }
    Ok(table)
}

/// Returns the same source text used by the power-user editor, without writing it.
pub fn preview_agent_target(settings: &Settings, target: &AgentTarget) -> Result<SourceDocument> {
    let mut document = read_source_file(settings, "harnesses.toml")?;
    let table: toml::Table = toml::from_str(&document.text)?;
    ensure!(
        !table.contains_key(&target.tag),
        "Agent tag already exists; edit its source definition instead"
    );
    let definition = target_definition(target)?;
    // Validate templates even if this tag isn't yet enabled by a machine profile.
    let mut context = toml::Table::new();
    context.insert(
        "home".into(),
        directories::BaseDirs::new()
            .context("Home directory unavailable")?
            .home_dir()
            .to_string_lossy()
            .into_owned()
            .into(),
    );
    for folder in std::iter::once(&target.global_folder).chain(target.instructions_folder.iter()) {
        let rendered = crate::template::render(folder, &context, false)?;
        ensure!(
            rendered.starts_with('/')
                || rendered.as_bytes().get(1) == Some(&b':')
                || rendered.starts_with("\\\\"),
            "Global folders must be absolute or start with {{{{af:home}}}}"
        );
        ensure!(
            !rendered.contains("/../") && !rendered.ends_with("/.."),
            "Global folder must not traverse parents"
        );
    }
    let section = BTreeMap::from([(target.tag.clone(), definition)]);
    document.text.push('\n');
    document
        .text
        .push_str(&toml::to_string_pretty(&section).context("Serialize agent definition")?);
    Ok(document)
}

pub fn add_agent_target(
    settings: &Settings,
    target: &AgentTarget,
    expected_hash: &str,
) -> Result<()> {
    let candidate = preview_agent_target(settings, target)?;
    crate::write_source_file(settings, "harnesses.toml", &candidate.text, expected_hash)
}
