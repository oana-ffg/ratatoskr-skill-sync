use crate::{
    Settings,
    config::{atomic_write, identifier, no_symlinks, relative},
    engine::{file_paths, validate_sources},
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize)]
pub struct SkillEntry {
    pub name: String,
    pub enabled: bool,
    pub project: Option<PathBuf>,
}

#[derive(Serialize)]
pub struct SkillDocument {
    pub name: String,
    pub markdown: String,
    pub metadata: String,
    pub markdown_hash: String,
    pub metadata_hash: String,
}

#[derive(Serialize)]
pub struct SourceDocument {
    pub file: String,
    pub text: String,
    pub hash: String,
}

fn config_file(value: &str) -> Result<PathBuf> {
    let path = relative(value)?;
    ensure!(
        value == "harnesses.toml"
            || value == "global/AGENTS.md"
            || (path.parent() == Some(Path::new("machines"))
                && path.extension().is_some_and(|x| x == "toml")),
        "Choose harnesses.toml, a machine profile, or global/AGENTS.md"
    );
    Ok(path)
}

pub fn source_files(settings: &Settings) -> Result<Vec<String>> {
    settings.validate()?;
    let mut result = vec!["harnesses.toml".into(), "global/AGENTS.md".into()];
    for entry in fs::read_dir(settings.source.join("machines"))? {
        let entry = entry?;
        if entry.path().extension().is_some_and(|x| x == "toml") {
            result.push(format!("machines/{}", entry.file_name().to_string_lossy()));
        }
    }
    result.sort();
    Ok(result)
}

pub fn read_source_file(settings: &Settings, file: &str) -> Result<SourceDocument> {
    settings.validate()?;
    let path = settings.source.join(config_file(file)?);
    no_symlinks(&path)?;
    let text = if path.try_exists()? {
        fs::read_to_string(path)?
    } else {
        String::new()
    };
    Ok(SourceDocument {
        file: file.into(),
        hash: hash(text.as_bytes()),
        text,
    })
}

pub fn write_source_file(
    settings: &Settings,
    file: &str,
    text: &str,
    expected_hash: &str,
) -> Result<()> {
    let path = config_file(file)?;
    if path.extension().is_some_and(|x| x == "toml") {
        let _: toml::Table = toml::from_str(text)?;
    }
    let current = read_source_file(settings, file)?;
    ensure!(
        current.hash == expected_hash,
        "Source changed since opening the editor"
    );
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    let mut candidate = settings.clone();
    candidate.source = root.join("content");
    candidate.state_dir = root.join("state");
    candidate.projects.clear();
    for item in ["harnesses.toml", "machines", "skills", "global"] {
        let from = settings.source.join(item);
        no_symlinks(&from)?;
        if from.is_dir() {
            for (relative, source) in file_paths(&from)? {
                atomic_write(
                    &candidate.source.join(item).join(relative),
                    &fs::read(source)?,
                )?;
            }
        } else if from.is_file() {
            atomic_write(&candidate.source.join(item), &fs::read(from)?)?;
        }
    }
    atomic_write(&candidate.source.join(&path), text.as_bytes())?;
    validate_sources(&candidate)?;
    atomic_write(&settings.source.join(path), text.as_bytes())
}

fn source(settings: &Settings, project: Option<&Path>) -> Result<PathBuf> {
    settings.validate()?;
    if let Some(root) = project {
        ensure!(
            settings.projects.contains(&root.to_path_buf()),
            "Project is not registered"
        );
        let status = crate::project_status(root);
        ensure!(
            status.error.is_none(),
            "Project cannot be read: {:?}",
            status.error
        );
        status.source.context("Project has no source")
    } else {
        Ok(settings.source.clone())
    }
}

pub fn list_skills(settings: &Settings) -> Result<Vec<SkillEntry>> {
    let mut result = Vec::new();
    for project in std::iter::once(None).chain(settings.projects.iter().map(|p| Some(p.as_path())))
    {
        let path = source(settings, project)?.join("skills");
        no_symlinks(&path)?;
        if !path.try_exists()? {
            continue;
        }
        for skill in fs::read_dir(path)? {
            let skill = skill?;
            ensure!(
                skill.file_type()?.is_dir(),
                "Skills root must contain directories"
            );
            let name = skill
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("Non-UTF8 skill name"))?;
            identifier(&name)?;
            let metadata = skill.path().join("skill.toml");
            no_symlinks(&metadata)?;
            let table: toml::Table = toml::from_str(&fs::read_to_string(metadata)?)?;
            result.push(SkillEntry {
                name,
                enabled: table
                    .get("enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true),
                project: project.map(Path::to_path_buf),
            });
        }
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(result)
}

fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn read_skill(
    settings: &Settings,
    project: Option<&Path>,
    name: &str,
) -> Result<SkillDocument> {
    identifier(name)?;
    let root = source(settings, project)?.join("skills").join(name);
    no_symlinks(&root)?;
    let markdown_path = root.join("SKILL.md");
    let metadata_path = root.join("skill.toml");
    no_symlinks(&markdown_path)?;
    no_symlinks(&metadata_path)?;
    let markdown = fs::read_to_string(markdown_path)?;
    let metadata = fs::read_to_string(metadata_path)?;
    Ok(SkillDocument {
        name: name.into(),
        markdown_hash: hash(markdown.as_bytes()),
        metadata_hash: hash(metadata.as_bytes()),
        markdown,
        metadata,
    })
}

fn validate_draft(
    settings: &Settings,
    selected_source: &Path,
    name: &str,
    file: &str,
    text: &str,
    create: bool,
) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    let mut candidate = settings.clone();
    candidate.source = root.join("content");
    candidate.projects.clear();
    candidate.state_dir = root.join("state");
    for item in ["harnesses.toml", "machines", "skills", "global"] {
        let from = if item == "skills" || item == "global" {
            selected_source.join(item)
        } else {
            settings.source.join(item)
        };
        if from.is_dir() {
            // Project editing uses the same machine/harness definitions, with its own skills.
            for (relative, source) in file_paths(&from)? {
                atomic_write(
                    &candidate.source.join(item).join(relative),
                    &fs::read(source)?,
                )?;
            }
        } else if from.is_file() {
            no_symlinks(&from)?;
            atomic_write(&candidate.source.join(item), &fs::read(from)?)?;
        }
    }
    let skill = candidate.source.join("skills").join(name);
    if create {
        atomic_write(&skill.join("skill.toml"), b"")?;
    }
    atomic_write(&skill.join(relative(file)?), text.as_bytes())?;
    validate_sources(&candidate)
}

pub fn write_skill_file(
    settings: &Settings,
    project: Option<&Path>,
    name: &str,
    file: &str,
    text: &str,
    expected_hash: &str,
) -> Result<()> {
    identifier(name)?;
    ensure!(
        matches!(file, "SKILL.md" | "skill.toml"),
        "Editor supports SKILL.md and skill.toml"
    );
    let selected = source(settings, project)?;
    let path = selected.join("skills").join(name).join(file);
    no_symlinks(&path)?;
    ensure!(
        hash(&fs::read(&path)?) == expected_hash,
        "Source changed since opening the editor. Reload before saving."
    );
    validate_draft(settings, &selected, name, file, text, false)?;
    atomic_write(&path, text.as_bytes())
}

pub fn create_skill(
    settings: &Settings,
    project: Option<&Path>,
    name: &str,
    markdown: &str,
) -> Result<()> {
    identifier(name)?;
    let selected = source(settings, project)?;
    let target = selected.join("skills").join(name);
    no_symlinks(&target)?;
    ensure!(!target.try_exists()?, "Skill already exists");
    validate_draft(settings, &selected, name, "SKILL.md", markdown, true)?;
    fs::create_dir_all(target.parent().context("Missing parent")?)?;
    let draft = tempfile::tempdir_in(target.parent().context("Missing parent")?)?;
    atomic_write(&draft.path().join("SKILL.md"), markdown.as_bytes())?;
    atomic_write(&draft.path().join("skill.toml"), b"")?;
    ensure!(!target.try_exists()?, "Skill appeared during creation");
    fs::rename(draft.path(), target)?;
    Ok(())
}

pub fn disable_skill(
    settings: &Settings,
    project: Option<&Path>,
    name: &str,
    disabled: bool,
) -> Result<()> {
    let document = read_skill(settings, project, name)?;
    let mut metadata: toml::Table = toml::from_str(&document.metadata)?;
    metadata.insert("enabled".into(), (!disabled).into());
    write_skill_file(
        settings,
        project,
        name,
        "skill.toml",
        &toml::to_string_pretty(&metadata)?,
        &document.metadata_hash,
    )
}

pub fn archive_skill(settings: &Settings, project: Option<&Path>, name: &str) -> Result<PathBuf> {
    identifier(name)?;
    let selected = source(settings, project)?;
    let skill = selected.join("skills").join(name);
    no_symlinks(&skill)?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let destination = selected
        .join(".ratatoskr-archive")
        .join(format!("{name}-{nonce}"));
    file_paths(&skill)?;
    no_symlinks(&destination)?;
    fs::create_dir_all(destination.parent().context("Missing archive parent")?)?;
    ensure!(!destination.try_exists()?, "Archive path already exists");
    // Same-volume rename preserves every file, including runtime artifacts, without recursive deletion.
    fs::rename(skill, &destination)?;
    Ok(destination)
}
