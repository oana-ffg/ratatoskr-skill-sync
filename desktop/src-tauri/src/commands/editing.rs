//! Source, skill and draft editing commands.
use super::*;

#[tauri::command]
pub(crate) async fn read_global_instructions() -> Reply<core::SourceDocument> {
    off_main_thread(move || {
        core::read_global_instructions(&settings()?).map_err(|e| format!("{e:#}"))
    })
    .await
}
#[tauri::command]
pub(crate) async fn save_global_instructions(
    on_progress: Channel<String>,
    markdown: String,
    expected_hash: String,
) -> Reply<Snapshot> {
    off_main_thread(move || {
        let progress = MutationProgress::new(Some(on_progress));
        mutate_with_progress(&progress, |s| {
            progress
                .edit(
                    s,
                    None,
                    Edit::GlobalInstructions {
                        markdown: &markdown,
                        expected_hash: &expected_hash,
                    },
                )
                .map(|_| ())
        })
    })
    .await
}
#[tauri::command]
pub(crate) async fn analyze_global_instructions(
    markdown: String,
    machine: String,
    harness: String,
) -> Reply<core::TemplateAnalysis> {
    off_main_thread(move || {
        core::analyze_global_instructions(&settings()?, &markdown, &machine, &harness)
            .map_err(|e| format!("{e:#}"))
    })
    .await
}
#[tauri::command]
pub(crate) async fn preview_global_instructions(
    markdown: String,
    machine: String,
    harness: String,
) -> Reply<String> {
    off_main_thread(move || {
        core::preview_global_instructions(&settings()?, &markdown, &machine, &harness)
            .map_err(|e| format!("{e:#}"))
    })
    .await
}
#[tauri::command]
pub(crate) async fn preview_global_instructions_path(
    machine: String,
    harness: String,
) -> Reply<PathBuf> {
    off_main_thread(move || {
        core::preview_global_instructions_path(&settings()?, &machine, &harness)
            .map_err(|e| format!("{e:#}"))
    })
    .await
}

#[tauri::command]
pub(crate) async fn read_source_skill(
    name: String,
    project: Option<PathBuf>,
) -> Reply<SkillDocument> {
    off_main_thread(move || {
        core::read_skill(&settings()?, project.as_deref(), &name).map_err(|e| format!("{e:#}"))
    })
    .await
}
#[tauri::command]
pub(crate) async fn set_skill_enabled(
    on_progress: Channel<String>,
    name: String,
    project: Option<PathBuf>,
    enabled: bool,
) -> Reply<Snapshot> {
    off_main_thread(move || {
        let progress = MutationProgress::new(Some(on_progress));
        mutate_with_progress(&progress, |s| {
            progress
                .edit(
                    s,
                    project.as_deref(),
                    Edit::SetEnabled {
                        name: &name,
                        enabled,
                    },
                )
                .map(|_| ())
        })
    })
    .await
}
#[tauri::command]
pub(crate) async fn archive_source_skill(
    on_progress: Channel<String>,
    name: String,
    project: Option<PathBuf>,
) -> Reply<SavedPath> {
    off_main_thread(move || {
        let progress = MutationProgress::new(Some(on_progress));
        let mut path = PathBuf::new();
        let workspace = mutate_with_progress(&progress, |s| {
            path = progress
                .edit(s, project.as_deref(), Edit::Archive { name: &name })?
                .ok_or_else(|| anyhow::anyhow!("Archive returned no path"))?;
            Ok(())
        })?;
        Ok(SavedPath { path, workspace })
    })
    .await
}

#[tauri::command]
pub(crate) async fn read_conflict_source(path: PathBuf) -> Reply<core::SourceDocument> {
    off_main_thread(move || {
        core::read_conflict_source(&settings()?, &path).map_err(|e| format!("{e:#}"))
    })
    .await
}
#[tauri::command]
pub(crate) async fn preview_source_edit(
    path: PathBuf,
    text: String,
) -> Reply<core::ResolutionPreview> {
    off_main_thread(move || {
        core::preview_source_edit(&settings()?, &path, &text).map_err(|e| format!("{e:#}"))
    })
    .await
}

#[tauri::command]
pub(crate) async fn analyze_skill(
    name: String,
    markdown: String,
    machine: String,
    harness: String,
) -> Reply<core::TemplateAnalysis> {
    off_main_thread(move || {
        core::analyze_skill(&settings()?, &name, &markdown, &machine, &harness)
            .map_err(|e| format!("{e:#}"))
    })
    .await
}

#[tauri::command]
pub(crate) async fn add_agent_target(
    on_progress: Channel<String>,
    target: core::AgentTarget,
    expected_hash: String,
) -> Reply<Snapshot> {
    off_main_thread(move || {
        let progress = MutationProgress::new(Some(on_progress));
        mutate_with_progress(&progress, |s| {
            progress
                .edit(
                    s,
                    None,
                    Edit::AddAgentTarget {
                        target: &target,
                        expected_hash: &expected_hash,
                    },
                )
                .map(|_| ())
        })
    })
    .await
}

#[tauri::command]
pub(crate) fn editor_view(markdown: String, metadata: String) -> Reply<core::EditorView> {
    core::editor_view(&markdown, &metadata).map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub(crate) fn edit_skill_draft(
    markdown: String,
    metadata: String,
    edit: core::DraftEdit,
) -> Reply<core::Draft> {
    core::edit_draft(&markdown, &metadata, edit).map_err(|e| format!("{e:#}"))
}
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn save_skill_draft(
    on_progress: Channel<String>,
    name: String,
    project: Option<PathBuf>,
    markdown: String,
    metadata: String,
    markdown_hash: String,
    metadata_hash: String,
) -> Reply<Snapshot> {
    off_main_thread(move || {
        let progress = MutationProgress::new(Some(on_progress));
        mutate_with_progress(&progress, |s| {
            progress
                .edit(
                    s,
                    project.as_deref(),
                    Edit::SkillDraft {
                        name: &name,
                        markdown: &markdown,
                        metadata: &metadata,
                        markdown_hash: &markdown_hash,
                        metadata_hash: &metadata_hash,
                    },
                )
                .map(|_| ())
        })
    })
    .await
}
#[tauri::command]
pub(crate) async fn read_structured_source(file: String) -> Reply<core::StructuredSource> {
    off_main_thread(move || {
        core::structured_source(&settings()?, &file).map_err(|e| format!("{e:#}"))
    })
    .await
}
#[tauri::command]
pub(crate) async fn save_structured_source(
    on_progress: Channel<String>,
    file: String,
    hash: String,
    values: toml::Table,
) -> Reply<Snapshot> {
    off_main_thread(move || {
        let progress = MutationProgress::new(Some(on_progress));
        mutate_with_progress(&progress, |s| {
            progress
                .edit(
                    s,
                    None,
                    Edit::StructuredSource {
                        file: &file,
                        hash: &hash,
                        values,
                    },
                )
                .map(|_| ())
        })
    })
    .await
}
#[tauri::command]
pub(crate) async fn list_values(machine: String, harness: String) -> Reply<Vec<core::ValueOption>> {
    off_main_thread(move || {
        core::value_options(&settings()?, &machine, &harness).map_err(|e| format!("{e:#}"))
    })
    .await
}
#[tauri::command]
pub(crate) async fn list_skill_resources(
    name: String,
    project: Option<PathBuf>,
) -> Reply<Vec<core::Resource>> {
    off_main_thread(move || {
        core::skill_resources(&settings()?, project.as_deref(), &name).map_err(|e| format!("{e:#}"))
    })
    .await
}
#[tauri::command]
pub(crate) async fn add_skill_resource(
    on_progress: Channel<String>,
    name: String,
    project: Option<PathBuf>,
    file: String,
    from: PathBuf,
) -> Reply<Snapshot> {
    off_main_thread(move || {
        let progress = MutationProgress::new(Some(on_progress));
        mutate_with_progress(&progress, |s| {
            progress
                .edit(
                    s,
                    project.as_deref(),
                    Edit::AddResource {
                        name: &name,
                        file: &file,
                        from: &from,
                    },
                )
                .map(|_| ())
        })
    })
    .await
}
#[tauri::command]
pub(crate) async fn rename_source_skill(
    on_progress: Channel<String>,
    name: String,
    project: Option<PathBuf>,
    new_name: String,
    expected_hash: String,
) -> Reply<Snapshot> {
    off_main_thread(move || {
        let progress = MutationProgress::new(Some(on_progress));
        mutate_with_progress(&progress, |s| {
            progress
                .edit(
                    s,
                    project.as_deref(),
                    Edit::Rename {
                        name: &name,
                        new_name: &new_name,
                        expected_hash: &expected_hash,
                    },
                )
                .map(|_| ())
        })
    })
    .await
}
#[tauri::command]
pub(crate) async fn choose_folder(app: tauri::AppHandle) -> Reply<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .blocking_pick_folder()
            .map(|p| p.to_string())
    })
    .await
    .map_err(|e| e.to_string())
}
#[tauri::command]
pub(crate) async fn choose_resource(app: tauri::AppHandle) -> Reply<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog()
            .file()
            .blocking_pick_file()
            .map(|p| p.to_string())
    })
    .await
    .map_err(|e| e.to_string())
}
#[tauri::command]
pub(crate) async fn read_local_settings() -> Reply<core::LocalConfiguration> {
    off_main_thread(move || core::local_configuration(&config()?).map_err(|e| format!("{e:#}")))
        .await
}
#[tauri::command]
pub(crate) async fn save_local_settings(
    on_progress: Channel<String>,
    candidate: Settings,
    expected_hash: String,
) -> Reply<Snapshot> {
    off_main_thread(move || {
        let progress = MutationProgress::new(Some(on_progress));
        mutate_with_progress(&progress, |_| {
            progress.stage(core::SourceEditStage::Writing);
            core::save_local_configuration(
                &core::default_config_path()?,
                &candidate,
                &expected_hash,
            )
        })
    })
    .await
}

#[tauri::command]
pub(crate) async fn preview_legacy_resolution(name: String) -> Reply<core::LegacyResolution> {
    off_main_thread(move || {
        core::preview_legacy_resolution(&settings()?, &name).map_err(|e| format!("{e:#}"))
    })
    .await
}
#[tauri::command]
pub(crate) async fn resolve_legacy(expected: String) -> Reply<PathBuf> {
    off_main_thread(move || {
        let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
        core::resolve_legacy(&settings()?, &expected).map_err(|e| format!("{e:#}"))
    })
    .await
}
#[tauri::command]
pub(crate) async fn skill_target_statuses()
-> Reply<std::collections::BTreeMap<String, std::collections::BTreeMap<String, bool>>> {
    off_main_thread(move || core::skill_target_statuses(&settings()?).map_err(|e| format!("{e:#}")))
        .await
}

#[tauri::command]
pub(crate) async fn preview_install_path(
    name: String,
    project: Option<PathBuf>,
    machine: String,
    harness: String,
) -> Reply<PathBuf> {
    off_main_thread(move || {
        core::preview_install_path(&settings()?, project.as_deref(), &name, &machine, &harness)
            .map_err(|e| format!("{e:#}"))
    })
    .await
}
#[tauri::command]
pub(crate) async fn create_desktop_skill(
    on_progress: Channel<String>,
    name: String,
    project: Option<PathBuf>,
    description: String,
) -> Reply<Snapshot> {
    off_main_thread(move || {
        let progress = MutationProgress::new(Some(on_progress));
        mutate_with_progress(&progress, |s| {
            progress
                .edit(
                    s,
                    project.as_deref(),
                    Edit::Create {
                        name: &name,
                        description: &description,
                    },
                )
                .map(|_| ())
        })
    })
    .await
}

#[tauri::command]
pub(crate) fn different_preview_lines(text: String, versions: Vec<String>) -> Vec<usize> {
    core::different_preview_lines(&text, &versions)
}
