//! Desktop-owned edits publish directly; the ordinary core/CLI writers stay write-only.
use crate::{
    Settings,
    config::{identifier, relative},
    git,
};
use anyhow::{Context, Result, ensure};
use std::path::{Path, PathBuf};

pub enum DesktopSourceEdit<'a> {
    GlobalInstructions {
        markdown: &'a str,
        expected_hash: &'a str,
    },
    SkillDraft {
        name: &'a str,
        markdown: &'a str,
        metadata: &'a str,
        markdown_hash: &'a str,
        metadata_hash: &'a str,
    },
    SetEnabled {
        name: &'a str,
        enabled: bool,
    },
    Archive {
        name: &'a str,
    },
    Rename {
        name: &'a str,
        new_name: &'a str,
        expected_hash: &'a str,
    },
    Create {
        name: &'a str,
        description: &'a str,
    },
    AddResource {
        name: &'a str,
        file: &'a str,
        from: &'a Path,
    },
    StructuredSource {
        file: &'a str,
        hash: &'a str,
        values: toml::Table,
    },
    AddAgentTarget {
        target: &'a crate::AgentTarget,
        expected_hash: &'a str,
    },
    ResolveConflict {
        expected: &'a str,
    },
    RestoreBackup {
        directory: &'a Path,
    },
}

impl DesktopSourceEdit<'_> {
    fn message(&self) -> &'static str {
        match self {
            Self::GlobalInstructions { .. } => "update global instructions",
            Self::SkillDraft { .. } => "update skill draft",
            Self::SetEnabled { .. } => "set skill enabled",
            Self::Archive { .. } => "archive skill",
            Self::Rename { .. } => "rename skill",
            Self::Create { .. } => "create skill",
            Self::AddResource { .. } => "add skill resource",
            Self::StructuredSource { .. } => "update source configuration",
            Self::AddAgentTarget { .. } => "add agent target",
            Self::ResolveConflict { .. } => "resolve source conflict",
            Self::RestoreBackup { .. } => "restore source backup",
        }
    }

    // Scopes limit before/after fingerprints, not Git staging: the transaction
    // stages individual changed files only, including both sides of a move.
    fn paths(&self, settings: &Settings) -> Result<Vec<PathBuf>> {
        let skill = |name: &str| -> Result<PathBuf> {
            identifier(name)?;
            Ok(settings.source.join("skills").join(name))
        };
        Ok(match self {
            Self::GlobalInstructions { .. } => {
                vec![settings.source.join(crate::content::GLOBAL_INSTRUCTIONS)]
            }
            Self::SkillDraft { name, .. } => vec![
                skill(name)?.join("SKILL.md"),
                skill(name)?.join("skill.toml"),
            ],
            Self::SetEnabled { name, .. } => vec![skill(name)?.join("skill.toml")],
            Self::Archive { name } => {
                vec![skill(name)?, settings.source.join(".ratatoskr-archive")]
            }
            Self::Rename { name, new_name, .. } => vec![skill(name)?, skill(new_name)?],
            Self::Create { name, .. } => vec![skill(name)?],
            Self::AddResource { name, file, .. } => vec![skill(name)?.join(relative(file)?)],
            Self::StructuredSource { file, .. } => vec![settings.source.join(relative(file)?)],
            Self::AddAgentTarget { .. } => vec![settings.source.join("harnesses.toml")],
            Self::ResolveConflict { expected } => {
                let preview: serde_json::Value = serde_json::from_str(expected)?;
                let action: crate::ConflictAction =
                    serde_json::from_value(preview["action"].clone())?;
                if matches!(
                    action,
                    crate::ConflictAction::CopyBack | crate::ConflictAction::EditSource
                ) {
                    let paths: Vec<PathBuf> = preview["effects"]
                        .as_array()
                        .context("Preview has no effects")?
                        .iter()
                        .map(|effect| serde_json::from_value(effect["path"].clone()))
                        .collect::<std::result::Result<_, _>>()?;
                    paths
                        .into_iter()
                        .filter(|p| p.starts_with(&settings.source))
                        .collect()
                } else {
                    vec![]
                }
            }
            Self::RestoreBackup { directory } => {
                crate::engine::backup_source_paths(settings, directory)?
                    .into_iter()
                    .filter(|p| p.starts_with(&settings.source))
                    .collect()
            }
        })
    }

    fn write(self, settings: &Settings, project: Option<&Path>) -> Result<Option<PathBuf>> {
        match self {
            Self::GlobalInstructions {
                markdown,
                expected_hash,
            } => crate::editor::save_global_instructions_locked(settings, markdown, expected_hash)?,
            Self::SkillDraft {
                name,
                markdown,
                metadata,
                markdown_hash,
                metadata_hash,
            } => crate::editor::save_skill_draft_locked(
                settings,
                project,
                name,
                markdown,
                metadata,
                markdown_hash,
                metadata_hash,
            )?,
            Self::SetEnabled { name, enabled } => {
                crate::disable_skill(settings, project, name, !enabled)?
            }
            Self::Archive { name } => {
                return crate::content::archive_skill_locked(settings, project, name).map(Some);
            }
            Self::Rename {
                name,
                new_name,
                expected_hash,
            } => crate::editor::rename_skill_locked(
                settings,
                project,
                name,
                new_name,
                expected_hash,
            )?,
            Self::Create { name, description } => crate::create_skill(
                settings,
                project,
                name,
                &crate::new_skill_markdown(name, description)?,
            )?,
            Self::AddResource { name, file, from } => {
                crate::add_skill_resource(settings, project, name, file, from)?
            }
            Self::StructuredSource { file, hash, values } => {
                crate::save_structured_source(settings, file, hash, values)?
            }
            Self::AddAgentTarget {
                target,
                expected_hash,
            } => crate::add_agent_target(settings, target, expected_hash)?,
            Self::ResolveConflict { expected } => {
                return crate::engine::resolve_conflict_locked(settings, expected).map(Some);
            }
            Self::RestoreBackup { directory } => {
                crate::engine::restore_backup_locked(settings, directory)?;
            }
        }
        Ok(None)
    }
}

/// Serializes a complete desktop save with sync and apply. Project edits remain
/// write-only; central edits use the user's Git identity and configured upstream.
/// The returned path is the archive/backup receipt for operations producing one.
pub fn desktop_source_edit(
    settings: &Settings,
    project: Option<&Path>,
    edit: DesktopSourceEdit<'_>,
) -> Result<Option<PathBuf>> {
    settings.validate()?;
    // Match sync_locked -> apply. Never acquire sync.lock from inside apply.lock.
    let _sync = crate::config::operation_lock(settings, "sync.lock")?;
    let _apply = crate::config::operation_lock(settings, "apply.lock")?;
    if project.is_some() {
        ensure!(
            matches!(
                edit,
                DesktopSourceEdit::SkillDraft { .. }
                    | DesktopSourceEdit::SetEnabled { .. }
                    | DesktopSourceEdit::Archive { .. }
                    | DesktopSourceEdit::Rename { .. }
                    | DesktopSourceEdit::Create { .. }
                    | DesktopSourceEdit::AddResource { .. }
            ),
            "This action does not accept a project scope"
        );
        return edit.write(settings, project);
    }
    let paths = edit.paths(settings)?;
    if paths.is_empty() {
        return edit.write(settings, None);
    }
    let message = format!("Ratatoskr editor: {}", edit.message());
    git::publish_source_edit(settings, &paths, &message, || edit.write(settings, None))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_acquires_sync_before_apply_and_releases_partial_acquisitions() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let settings = Settings {
            source: root.join("content"),
            state_dir: root.join("state"),
            machine: "test".into(),
            ..Default::default()
        };
        std::fs::create_dir(&settings.source).unwrap();
        let sync = crate::config::operation_lock(&settings, "sync.lock").unwrap();
        let apply = crate::config::operation_lock(&settings, "apply.lock").unwrap();
        let run = || {
            desktop_source_edit(
                &settings,
                None,
                DesktopSourceEdit::Create {
                    name: "new",
                    description: "New skill",
                },
            )
        };
        // When both are held the first refusal must name sync.lock.
        assert!(format!("{:#}", run().unwrap_err()).contains("sync.lock"));
        drop(sync);
        assert!(format!("{:#}", run().unwrap_err()).contains("apply.lock"));
        // Refusing apply must release the already-acquired sync lock.
        assert!(
            crate::config::try_operation_lock(&settings, "sync.lock")
                .unwrap()
                .is_some()
        );
        drop(apply);
        assert!(
            std::fs::read_dir(&settings.source)
                .unwrap()
                .next()
                .is_none()
        );
    }
}
