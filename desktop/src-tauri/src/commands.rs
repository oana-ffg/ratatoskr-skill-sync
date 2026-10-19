use crate::desktop_updates::{UpdateContext, automatic_update_with, manual_update_with};
use ratatoskr_core::{self as core, Plan, Settings, SkillDocument, SkillEntry};
use serde::Serialize;
use std::{fs, path::PathBuf, sync::Mutex};
use tauri::Manager;
use tauri_plugin_updater::UpdaterExt;

static MUTATION: Mutex<()> = Mutex::new(());
type Reply<T> = Result<T, String>;

#[derive(Serialize)]
pub(crate) struct Snapshot {
    config_path: PathBuf,
    config_text: String,
    plan: Plan,
    skills: Vec<SkillEntry>,
    helper: Option<PathBuf>,
    helper_error: Option<String>,
    jobs: core::JobStatus,
    presets: Vec<core::AgentPreset>,
    preview_targets: Vec<core::PreviewTarget>,
    source_files: Vec<String>,
    legacy: Vec<core::LegacySkill>,
    updates: UpdateStatus,
}

#[derive(Serialize)]
pub(crate) struct UpdateStatus {
    endpoint: String,
    override_active: bool,
    automatic: bool,
    time: String,
}

fn detected_helper() -> anyhow::Result<PathBuf> {
    core::detect_helper(
        &std::env::current_exe()?,
        std::env::var_os("PATH").as_deref(),
    )
}

fn config() -> Reply<PathBuf> {
    core::default_config_path().map_err(|e| e.to_string())
}
fn settings() -> Reply<Settings> {
    core::load(&config()?).map_err(|e| format!("{e:#}"))
}
fn snapshot() -> Reply<Snapshot> {
    let path = config()?;
    let settings = settings()?;
    let mut plan = core::plan(&settings).unwrap_or_else(|error| Plan {
        source: settings.source.clone(),
        machine: settings.machine.clone(),
        mode: "blocked".into(),
        skills: 0,
        targets: 0,
        install_targets: vec![],
        staging_root: None,
        changes: vec![],
        conflicts: vec![format!("Preview failed: {error:#}")],
        conflict_details: vec![],
        projects: settings
            .projects
            .iter()
            .map(|root| core::project_status(root))
            .collect(),
    });
    let skills = core::list_skills(&settings).unwrap_or_else(|error| {
        plan.conflicts
            .push(format!("Cannot list source skills: {error:#}"));
        vec![]
    });
    let source_files = core::source_files(&settings).unwrap_or_else(|error| {
        plan.conflicts
            .push(format!("Cannot list source configuration: {error:#}"));
        vec![]
    });
    let helper = detected_helper();
    let preview_targets = core::preview_targets(&settings).unwrap_or_else(|error| {
        plan.conflicts
            .push(format!("Cannot list preview targets: {error:#}"));
        vec![]
    });
    Ok(Snapshot {
        updates: UpdateStatus {
            endpoint: settings.updates.endpoint().into(),
            override_active: settings.updates.override_active(),
            automatic: settings.updates.automatic,
            time: settings.updates.time.clone(),
        },
        config_text: fs::read_to_string(&path).map_err(|e| e.to_string())?,
        config_path: path,
        plan,
        skills,
        helper: helper.as_ref().ok().cloned(),
        helper_error: helper.err().map(|e| format!("{e:#}")),
        jobs: core::job_status(&settings).map_err(|e| format!("{e:#}"))?,
        presets: core::agent_presets(),
        preview_targets,
        source_files,
        legacy: core::legacy_duplicates(&settings).map_err(|e| format!("{e:#}"))?,
    })
}
fn mutate(action: impl FnOnce(&Settings) -> anyhow::Result<()>) -> Reply<Snapshot> {
    let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
    action(&settings()?).map_err(|e| format!("{e:#}"))?;
    snapshot()
}

#[tauri::command]
pub(crate) fn read_workspace() -> Reply<Option<Snapshot>> {
    if !config()?.try_exists().map_err(|e| e.to_string())? {
        return Ok(None);
    }
    snapshot().map(Some)
}

#[tauri::command]
pub(crate) fn connect_workspace(
    source: String,
    machine: String,
    create: bool,
    remote: Option<String>,
    clone_name: Option<String>,
) -> Reply<Snapshot> {
    let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
    let path = config()?;
    if create {
        core::create_content(&path, &PathBuf::from(source), &machine)
    } else if let Some(remote) = remote.filter(|r| !r.trim().is_empty()) {
        let parent = PathBuf::from(source);
        let name = clone_name
            .filter(|n| !n.is_empty())
            .ok_or("Choose a new checkout folder name")?;
        let leaf = std::path::Path::new(&name);
        if leaf.components().count() != 1
            || !leaf
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)))
        {
            return Err("Checkout name must be one folder name".into());
        }
        core::clone_content(&path, &remote, &parent.join(leaf), &machine)
    } else {
        core::initialize(&path, &PathBuf::from(source), &machine)
    }
    .map_err(|e| format!("{e:#}"))?;
    snapshot()
}

#[tauri::command]
pub(crate) fn apply_workspace(expected_plan: String) -> Reply<Snapshot> {
    mutate(|s| core::apply_reviewed(s, &expected_plan).map(|_| ()))
}

#[tauri::command]
pub(crate) fn save_configuration(text: String) -> Reply<Snapshot> {
    let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
    let candidate: Settings = toml::from_str(&text).map_err(|e| e.to_string())?;
    core::plan(&candidate).map_err(|e| format!("{e:#}"))?;
    core::save(&config()?, &candidate).map_err(|e| format!("{e:#}"))?;
    snapshot()
}

#[tauri::command]
pub(crate) fn read_source_configuration(file: String) -> Reply<core::SourceDocument> {
    core::read_source_file(&settings()?, &file).map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub(crate) fn save_source_configuration(
    file: String,
    text: String,
    expected_hash: String,
) -> Reply<Snapshot> {
    mutate(|s| core::write_source_file(s, &file, &text, &expected_hash))
}

#[tauri::command]
pub(crate) fn add_monitored_project(root: String, source: String) -> Reply<Snapshot> {
    mutate(|_| {
        core::add_project(&core::default_config_path()?, &PathBuf::from(root), &source).map(|_| ())
    })
}
#[tauri::command]
pub(crate) fn remove_monitored_project(root: String) -> Reply<Snapshot> {
    mutate(|_| core::remove_project(&core::default_config_path()?, &PathBuf::from(root)))
}
#[tauri::command]
pub(crate) fn read_source_skill(name: String, project: Option<PathBuf>) -> Reply<SkillDocument> {
    core::read_skill(&settings()?, project.as_deref(), &name).map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub(crate) fn save_source_skill(
    name: String,
    project: Option<PathBuf>,
    file: String,
    text: String,
    expected_hash: String,
) -> Reply<Snapshot> {
    mutate(|s| core::write_skill_file(s, project.as_deref(), &name, &file, &text, &expected_hash))
}
#[tauri::command]
pub(crate) fn new_source_skill(
    name: String,
    project: Option<PathBuf>,
    markdown: String,
) -> Reply<Snapshot> {
    mutate(|s| core::create_skill(s, project.as_deref(), &name, &markdown))
}
#[tauri::command]
pub(crate) fn set_skill_enabled(
    name: String,
    project: Option<PathBuf>,
    enabled: bool,
) -> Reply<Snapshot> {
    mutate(|s| core::disable_skill(s, project.as_deref(), &name, !enabled))
}
#[tauri::command]
pub(crate) fn archive_source_skill(name: String, project: Option<PathBuf>) -> Reply<String> {
    let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
    core::archive_skill(&settings()?, project.as_deref(), &name)
        .map(|p| p.display().to_string())
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub(crate) fn restore_workspace(backup: PathBuf) -> Reply<Snapshot> {
    mutate(|s| core::restore_backup(s, &backup).map(|_| ()))
}
#[tauri::command]
pub(crate) async fn sync_workspace(app: tauri::AppHandle) -> Reply<Snapshot> {
    #[cfg(windows)]
    {
        crate::tray::request_sync(&app);
        snapshot()
    }
    #[cfg(not(windows))]
    let _ = app;
    #[cfg(not(windows))]
    tauri::async_runtime::spawn_blocking(|| mutate(|s| core::sync(s).map(|_| ())))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub(crate) fn preview_schedule(seconds: u32) -> Reply<core::Schedule> {
    let helper = detected_helper().map_err(|e| format!("{e:#}"))?;
    core::schedule_plan(&settings()?, &config()?, &helper, seconds).map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub(crate) fn enable_schedule(seconds: u32) -> Reply<String> {
    let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
    let settings = settings()?;
    let helper = detected_helper().map_err(|e| format!("{e:#}"))?;
    let plan = core::schedule_plan(&settings, &config()?, &helper, seconds)
        .map_err(|e| format!("{e:#}"))?;
    core::install_schedule(&settings, &plan).map_err(|e| format!("{e:#}"))?;
    Ok(format!("Installed {}", plan.label))
}
#[tauri::command]
pub(crate) fn disable_schedule() -> Reply<String> {
    let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
    core::remove_schedule(&settings()?).map_err(|e| format!("{e:#}"))?;
    Ok("Background schedule removed.".into())
}

fn install_desktop_update(
    app: &tauri::AppHandle,
    update: tauri_plugin_updater::Update,
) -> anyhow::Result<()> {
    // The updater verifies the signed download before invoking the installer.
    let bytes = tauri::async_runtime::block_on(update.download(|_, _| (), || ()))?;
    install_downloaded_update(app, &update, bytes)
}

fn install_downloaded_update(
    app: &tauri::AppHandle,
    update: &tauri_plugin_updater::Update,
    bytes: Vec<u8>,
) -> anyhow::Result<()> {
    #[cfg(not(windows))]
    let _ = app;
    #[cfg(windows)]
    crate::tray::prepare_restart(app)?;
    let result = update.install(bytes);
    #[cfg(windows)]
    if result.is_err() {
        crate::tray::cancel_restart(app)?;
    }
    result.map_err(Into::into)
}

#[tauri::command]
pub(crate) fn set_desktop_busy(app: tauri::AppHandle, busy: bool) {
    crate::update_scheduler::set_busy(&app, busy);
}
#[tauri::command]
pub(crate) fn background_status(app: tauri::AppHandle) -> Reply<Option<serde_json::Value>> {
    #[cfg(windows)]
    {
        crate::tray::status(&app)
            .and_then(|s| serde_json::to_value(s).map(Some).map_err(Into::into))
            .map_err(|e| format!("{e:#}"))
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Ok(None)
    }
}
#[tauri::command]
pub(crate) fn set_background_preferences(
    app: tauri::AppHandle,
    paused: Option<bool>,
    interval_seconds: Option<u32>,
) -> Reply<()> {
    #[cfg(windows)]
    {
        crate::tray::set_preferences(&app, paused, interval_seconds)
            .map_err(|e| format!("{e:#}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (app, paused, interval_seconds);
        Err("Tray sync is Windows-only".into())
    }
}
#[tauri::command]
pub(crate) fn migrate_background(app: tauri::AppHandle) -> Reply<()> {
    #[cfg(windows)]
    {
        crate::tray::request_migration(&app);
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("Tray sync is Windows-only".into())
    }
}

fn desktop_updater(
    app: &tauri::AppHandle,
    settings: &core::updates::UpdateSettings,
) -> anyhow::Result<tauri_plugin_updater::Updater> {
    Ok(app
        .updater_builder()
        .pubkey(settings.tauri_public_key()?)
        .endpoints(vec![settings.endpoint().parse()?])?
        .timeout(std::time::Duration::from_secs(60))
        .build()?)
}

struct Nightly {
    pending: Option<(core::updates::UpdateSettings, tauri_plugin_updater::Update)>,
}
static NIGHTLY: Mutex<Nightly> = Mutex::new(Nightly { pending: None });

fn update_result<T>(context: &UpdateContext, result: anyhow::Result<T>) -> Reply<T> {
    let error = result
        .as_ref()
        .err()
        .map(|e| format!("Desktop update: {e:#}"));
    context
        .report_outcome(error.as_deref())
        .map_err(|notification| {
            format!(
                "{}; alert delivery failed: {notification:#}",
                error.as_deref().unwrap_or("Update completed")
            )
        })?;
    result.map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub(crate) async fn nightly_update(app: tauri::AppHandle, install: bool) -> Reply<bool> {
    automatic_update(app, install, false).await
}

#[tauri::command]
pub(crate) async fn startup_update(app: tauri::AppHandle) -> Reply<bool> {
    crate::update_scheduler::startup(app).await
}

pub(crate) async fn automatic_update(
    app: tauri::AppHandle,
    install: bool,
    launch: bool,
) -> Reply<bool> {
    tauri::async_runtime::spawn_blocking(move || {
        if install && !launch && crate::update_scheduler::busy(&app) {
            return Ok(false);
        }
        let _guard = if install {
            match MUTATION.try_lock() {
                Ok(guard) => Some(guard),
                Err(std::sync::TryLockError::WouldBlock) => return Ok(false),
                Err(error) => return Err(error.to_string()),
            }
        } else {
            None
        };
        let context = update_context(&app)?;
        let mut installed = false;
        let result = (|| -> anyhow::Result<_> {
            let mut nightly = NIGHTLY.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
            automatic_update_with(
                &context,
                &mut nightly.pending,
                chrono::Local::now().fixed_offset(),
                install,
                launch,
                |settings| {
                    tauri::async_runtime::block_on(desktop_updater(&app, settings)?.check())
                        .map_err(Into::into)
                },
                |update| {
                    // A download can outlast the idle period. Keep the checked
                    // update pending when editing begins during that request.
                    let bytes = tauri::async_runtime::block_on(update.download(|_, _| (), || ()))?;
                    if crate::update_scheduler::busy(&app)
                        || !context
                            .installation_allowed(chrono::Local::now().fixed_offset(), launch)?
                    {
                        return Ok(false);
                    }
                    install_downloaded_update(&app, update, bytes)?;
                    installed = true;
                    Ok(true)
                },
            )
        })();
        let report = result
            .as_ref()
            .map_or(true, |outcome| outcome.checked || outcome.installed);
        let result = result.map(|outcome| {
            if install {
                outcome.installed
            } else {
                outcome.available
            }
        });
        let result = if report {
            update_result(&context, result)
        } else {
            result.map_err(|e| format!("{e:#}"))
        };
        let restart = installed;
        // Installation succeeded even if alert recovery delivery failed; never lose that error.
        if restart {
            if let Err(error) = &result {
                eprintln!("Ratatoskr: {error}");
            }
            app.restart();
        }
        result
    })
    .await
    .map_err(|e| e.to_string())?
}

fn update_context(app: &tauri::AppHandle) -> Reply<UpdateContext> {
    let data_dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    UpdateContext::load(&config()?, &data_dir).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub(crate) fn read_update_settings(app: tauri::AppHandle) -> Reply<UpdateStatus> {
    let context = update_context(&app)?;
    let settings = context.settings();
    Ok(UpdateStatus {
        endpoint: settings.endpoint().into(),
        override_active: settings.override_active(),
        automatic: settings.automatic,
        time: settings.time.clone(),
    })
}

#[tauri::command]
pub(crate) async fn update_desktop(
    app: tauri::AppHandle,
    install: bool,
) -> Reply<crate::desktop_updates::DesktopUpdateResult> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
        let context = update_context(&app)?;
        let mut nightly = NIGHTLY.lock().map_err(|e| e.to_string())?;
        nightly.pending = None;
        let result = manual_update_with(
            &context,
            install,
            |settings| {
                tauri::async_runtime::block_on(desktop_updater(&app, settings)?.check())
                    .map_err(Into::into)
            },
            |update| update.version.clone(),
            |update| {
                install_desktop_update(&app, update)?;
                Ok(())
            },
        );
        update_result(&context, result)
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
pub(crate) fn save_update_schedule(automatic: bool, time: String) -> Reply<Snapshot> {
    mutate(|s| {
        let mut candidate = s.clone();
        candidate.updates.automatic = automatic;
        candidate.updates.time = time;
        core::save(&core::default_config_path()?, &candidate)
    })
}
#[tauri::command]
pub(crate) fn restart_desktop(app: tauri::AppHandle) -> Reply<()> {
    #[cfg(windows)]
    crate::tray::prepare_restart(&app).map_err(|e| format!("{e:#}"))?;
    app.restart();
}

#[tauri::command]
pub(crate) fn preview_conflict(
    path: PathBuf,
    action: core::ConflictAction,
) -> Reply<core::ResolutionPreview> {
    core::preview_resolution(&settings()?, &path, action).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub(crate) fn resolve_conflict(expected: String) -> Reply<String> {
    let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
    core::resolve_conflict(&settings()?, &expected)
        .map(|p| p.display().to_string())
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub(crate) fn preview_agent(target: core::AgentTarget) -> Reply<core::SourceDocument> {
    core::preview_agent_target(&settings()?, &target).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub(crate) async fn preview_skill(
    name: String,
    project: Option<PathBuf>,
    markdown: String,
    metadata: String,
    machine: String,
    harness: String,
) -> Reply<Vec<core::PreviewFile>> {
    tauri::async_runtime::spawn_blocking(move || {
        core::preview_skill(
            &settings()?,
            project.as_deref(),
            &name,
            &markdown,
            &metadata,
            &machine,
            &harness,
        )
        .map_err(|e| format!("{e:#}"))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) fn read_job_status() -> Reply<core::JobStatus> {
    core::job_status(&settings()?).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub(crate) fn login_enabled(app: tauri::AppHandle) -> Reply<bool> {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().map_err(|e| e.to_string())
}

#[tauri::command]
pub(crate) fn set_login_enabled(app: tauri::AppHandle, enabled: bool) -> Reply<bool> {
    use tauri_plugin_autostart::ManagerExt;
    let result = (|| -> Reply<bool> {
        let manager = app.autolaunch();
        if enabled {
            manager.enable()
        } else {
            manager.disable()
        }
        .map_err(|e| e.to_string())?;
        manager.is_enabled().map_err(|e| e.to_string())
    })();
    #[cfg(windows)]
    crate::tray::login_outcome(&app, &result).map_err(|e| format!("{e:#}"))?;
    result
}

#[tauri::command]
pub(crate) fn read_conflict_source(path: PathBuf) -> Reply<core::SourceDocument> {
    core::read_conflict_source(&settings()?, &path).map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub(crate) fn preview_source_edit(path: PathBuf, text: String) -> Reply<core::ResolutionPreview> {
    core::preview_source_edit(&settings()?, &path, &text).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub(crate) fn analyze_skill(
    name: String,
    markdown: String,
    machine: String,
    harness: String,
) -> Reply<core::SkillAnalysis> {
    core::analyze_skill(&settings()?, &name, &markdown, &machine, &harness)
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub(crate) fn list_agent_presets() -> Vec<core::AgentPreset> {
    core::agent_presets()
}
#[tauri::command]
pub(crate) fn list_preview_targets() -> Reply<Vec<core::PreviewTarget>> {
    core::preview_targets(&settings()?).map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub(crate) fn detect_helper() -> Reply<PathBuf> {
    detected_helper().map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub(crate) fn add_agent_target(
    target: core::AgentTarget,
    expected_hash: String,
) -> Reply<Snapshot> {
    mutate(|s| core::add_agent_target(s, &target, &expected_hash))
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
pub(crate) fn save_skill_draft(
    name: String,
    project: Option<PathBuf>,
    markdown: String,
    metadata: String,
    markdown_hash: String,
    metadata_hash: String,
) -> Reply<Snapshot> {
    mutate(|s| {
        core::save_skill_draft(
            s,
            project.as_deref(),
            &name,
            &markdown,
            &metadata,
            &markdown_hash,
            &metadata_hash,
        )
    })
}
#[tauri::command]
pub(crate) fn read_structured_source(file: String) -> Reply<core::StructuredSource> {
    core::structured_source(&settings()?, &file).map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub(crate) fn save_structured_source(
    file: String,
    hash: String,
    values: toml::Table,
) -> Reply<Snapshot> {
    mutate(|s| core::save_structured_source(s, &file, &hash, values))
}
#[tauri::command]
pub(crate) fn list_values(machine: String, harness: String) -> Reply<Vec<core::ValueOption>> {
    core::value_options(&settings()?, &machine, &harness).map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub(crate) fn list_skill_resources(
    name: String,
    project: Option<PathBuf>,
) -> Reply<Vec<core::Resource>> {
    core::skill_resources(&settings()?, project.as_deref(), &name).map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub(crate) fn add_skill_resource(
    name: String,
    project: Option<PathBuf>,
    file: String,
    from: PathBuf,
) -> Reply<Snapshot> {
    mutate(|s| core::add_skill_resource(s, project.as_deref(), &name, &file, &from))
}
#[tauri::command]
pub(crate) fn rename_source_skill(
    name: String,
    project: Option<PathBuf>,
    new_name: String,
    expected_hash: String,
) -> Reply<Snapshot> {
    mutate(|s| core::rename_skill(s, project.as_deref(), &name, &new_name, &expected_hash))
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
pub(crate) fn read_local_settings() -> Reply<core::LocalConfiguration> {
    core::local_configuration(&config()?).map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub(crate) fn save_local_settings(candidate: Settings, expected_hash: String) -> Reply<Snapshot> {
    mutate(|_| {
        core::save_local_configuration(&core::default_config_path()?, &candidate, &expected_hash)
    })
}

#[tauri::command]
pub(crate) fn preview_legacy_resolution(name: String) -> Reply<core::LegacyResolution> {
    core::preview_legacy_resolution(&settings()?, &name).map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub(crate) fn resolve_legacy(expected: String) -> Reply<PathBuf> {
    let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
    core::resolve_legacy(&settings()?, &expected).map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub(crate) fn skill_target_statuses()
-> Reply<std::collections::BTreeMap<String, std::collections::BTreeMap<String, bool>>> {
    core::skill_target_statuses(&settings()?).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
pub(crate) fn preview_install_path(
    name: String,
    project: Option<PathBuf>,
    machine: String,
    harness: String,
) -> Reply<PathBuf> {
    core::preview_install_path(&settings()?, project.as_deref(), &name, &machine, &harness)
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub(crate) fn create_desktop_skill(
    name: String,
    project: Option<PathBuf>,
    description: String,
) -> Reply<Snapshot> {
    mutate(|s| {
        core::create_skill(
            s,
            project.as_deref(),
            &name,
            &core::new_skill_markdown(&name, &description)?,
        )
    })
}

#[tauri::command]
pub(crate) fn different_preview_lines(text: String, versions: Vec<String>) -> Vec<usize> {
    core::different_preview_lines(&text, &versions)
}

#[tauri::command]
pub(crate) async fn choose_project_source(
    app: tauri::AppHandle,
    root: PathBuf,
) -> Reply<Option<String>> {
    use tauri_plugin_dialog::DialogExt;
    tauri::async_runtime::spawn_blocking(move || {
        let Some(selected) = app
            .dialog()
            .file()
            .set_directory(&root)
            .blocking_pick_folder()
        else {
            return Ok(None);
        };
        let selected = selected.into_path().map_err(|e| e.to_string())?;
        let relative = selected
            .strip_prefix(&root)
            .map_err(|_| "Choose a content folder inside the project repository".to_owned())?;
        let relative = relative
            .to_str()
            .ok_or("Content folder name is not UTF-8")?
            .replace('\\', "/");
        if relative.is_empty() {
            return Err(
                "Choose a content folder inside the repository, not the repository root".into(),
            );
        }
        Ok(Some(relative))
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) fn keep_legacy(expected: String) -> Reply<Snapshot> {
    mutate(|s| core::keep_legacy(s, &expected))
}
