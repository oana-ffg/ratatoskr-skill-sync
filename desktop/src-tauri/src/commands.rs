mod editing;
mod updates;
pub(crate) use editing::*;
use ratatoskr_core::{
    self as core, DesktopSourceEdit as Edit, Plan, Settings, SkillDocument, SkillEntry,
};
use serde::Serialize;
use std::cell::RefCell;
use std::{
    fs,
    path::PathBuf,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
};
use tauri::ipc::Channel;
pub(crate) use updates::*;

static MUTATION: Mutex<()> = Mutex::new(());
static APP_ERROR_LISTENER: AtomicBool = AtomicBool::new(false);

#[tauri::command]
pub(crate) fn runtime_platform() -> &'static str {
    std::env::consts::OS
}

#[tauri::command]
pub(crate) fn set_app_error_listener(active: bool) {
    APP_ERROR_LISTENER.store(active, Ordering::Release);
}

#[cfg(target_os = "macos")]
pub(crate) fn app_error_listener_ready() -> bool {
    APP_ERROR_LISTENER.load(Ordering::Acquire)
}

type Reply<T> = Result<T, String>;

#[derive(Serialize)]
pub(crate) struct Snapshot {
    config_path: PathBuf,
    config_text: String,
    plan: Plan,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_edit: Option<core::SourceEditOutcome>,
    skills: Vec<SkillEntry>,
    skills_error: Option<String>,
    helper: Option<PathBuf>,
    helper_error: Option<String>,
    #[cfg(target_os = "macos")]
    schedule_installed: bool,
    jobs: core::JobStatus,
    presets: Vec<core::AgentPreset>,
    preview_targets: Vec<core::PreviewTarget>,
    source_files: Vec<String>,
    legacy: Vec<core::LegacySkill>,
    updates: UpdateStatus,
}

#[derive(Serialize)]
pub(crate) struct SavedPath {
    path: PathBuf,
    workspace: Snapshot,
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
    snapshot_with(None)
}
/// `current` is a plan the caller just computed, so it is not rendered a second time.
fn snapshot_with(current: Option<Plan>) -> Reply<Snapshot> {
    let path = config()?;
    let settings = settings()?;
    let planned = match current {
        Some(mut plan) => {
            // Reviewed Apply compares this plan with its own preflight, which has no counts.
            plan.applied = None;
            Ok(plan)
        }
        None => core::plan(&settings),
    };
    let mut plan = planned.unwrap_or_else(|error| Plan {
        content_status: core::content_status(&settings),
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
        stale_targets: vec![],
        revision: String::new(),
        applied: None,
        projects: settings
            .projects
            .iter()
            .map(|root| core::project_status(root))
            .collect(),
    });
    let (skills, skills_error) = match core::list_skills(&settings) {
        Ok(skills) => (skills, None),
        Err(error) => {
            let message = format!("Cannot list source skills: {error:#}");
            plan.conflicts.push(message.clone());
            (vec![], Some(message))
        }
    };
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
        source_edit: None,
        skills,
        skills_error,
        helper: helper.as_ref().ok().cloned(),
        helper_error: helper.err().map(|e| format!("{e:#}")),
        #[cfg(target_os = "macos")]
        schedule_installed: core::schedule_installed(&settings).map_err(|e| format!("{e:#}"))?,
        jobs: core::job_status(&settings).map_err(|e| format!("{e:#}"))?,
        presets: core::agent_presets(),
        preview_targets,
        source_files,
        legacy: core::legacy_duplicates(&settings).map_err(|e| format!("{e:#}"))?,
    })
}
// A channel belongs to one invocation, so a finished command retains no sender.
// Progress is advisory: a closed window must never interrupt publication.
struct MutationProgress {
    channel: Option<Channel<String>>,
    // A saved edit is distinct from a command failure. Keep its typed outcomes
    // alongside progress until the refreshed snapshot is returned to the editor.
    source_edit: RefCell<Option<core::SourceEditOutcome>>,
}

impl MutationProgress {
    fn new(channel: Option<Channel<String>>) -> Self {
        Self {
            channel,
            source_edit: RefCell::new(None),
        }
    }

    fn snapshot(&self) -> Reply<Snapshot> {
        let mut workspace = snapshot()?;
        workspace.source_edit = self.source_edit.borrow_mut().take();
        Ok(workspace)
    }

    fn send(&self, message: String) {
        if let Some(channel) = &self.channel
            && let Err(error) = channel.send(message)
        {
            eprintln!("Ratatoskr: cannot show save progress: {error:#}");
        }
    }

    fn stage(&self, stage: core::SourceEditStage) {
        self.send(match stage {
            core::SourceEditStage::Fetching => "Getting the latest content…".into(),
            core::SourceEditStage::Writing => "Checking and saving your changes…".into(),
            core::SourceEditStage::Committing => "Committing…".into(),
            core::SourceEditStage::Installing => "Installing on this machine…".into(),
            core::SourceEditStage::Pushing { remote, branch } => {
                format!("Pushing to {remote}/{branch}…")
            }
        });
    }

    fn edit(
        &self,
        settings: &Settings,
        project: Option<&std::path::Path>,
        edit: Edit<'_>,
    ) -> anyhow::Result<Option<PathBuf>> {
        let saved = core::desktop_source_edit_with_progress(settings, project, edit, &|stage| {
            self.stage(stage)
        })?;
        *self.source_edit.borrow_mut() = Some(saved.outcome);
        Ok(saved.path)
    }
}

/// A mutation that ends in an up-to-date plan, which the snapshot reuses.
fn mutate_with_plan(
    progress: &MutationProgress,
    action: impl FnOnce(&Settings) -> anyhow::Result<Plan>,
) -> Reply<Snapshot> {
    let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
    let plan = action(&settings()?).map_err(|e| format!("{e:#}"))?;
    progress.send("Refreshing…".into());
    snapshot_with(Some(plan))
}
fn mutate(action: impl FnOnce(&Settings) -> anyhow::Result<()>) -> Reply<Snapshot> {
    mutate_with_progress(&MutationProgress::new(None), action)
}
fn mutate_with_progress(
    progress: &MutationProgress,
    action: impl FnOnce(&Settings) -> anyhow::Result<()>,
) -> Reply<Snapshot> {
    let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
    action(&settings()?).map_err(|e| format!("{e:#}"))?;
    progress.send("Refreshing…".into());
    progress.snapshot()
}

// Synchronous Tauri commands run on the main thread, so anything that touches the disk, a
// process or the network would freeze the window. MUTATION and the core operation locks are
// acquired inside the worker. `main_thread_commands_stay_trivial` enforces this.
async fn off_main_thread<T: Send + 'static>(
    action: impl FnOnce() -> Reply<T> + Send + 'static,
) -> Reply<T> {
    tauri::async_runtime::spawn_blocking(action)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub(crate) async fn read_workspace() -> Reply<Option<Snapshot>> {
    off_main_thread(move || {
        if !config()?.try_exists().map_err(|e| e.to_string())? {
            return Ok(None);
        }
        snapshot().map(Some)
    })
    .await
}

#[tauri::command]
pub(crate) async fn connect_workspace(
    source: String,
    machine: String,
    create: bool,
    remote: Option<String>,
    clone_name: Option<String>,
) -> Reply<Snapshot> {
    off_main_thread(move || {
        let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
        let path = config()?;
        if create {
            core::create_content(&path, &PathBuf::from(source), &machine, None)
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
            core::clone_content(&path, &remote, &parent.join(leaf), &machine, None)
        } else {
            core::initialize(&path, &PathBuf::from(source), &machine, None)
        }
        .map_err(|e| format!("{e:#}"))?;
        snapshot()
    })
    .await
}

#[tauri::command]
pub(crate) async fn read_job_log() -> Reply<String> {
    off_main_thread(|| core::job_log(&settings()?).map_err(|e| format!("{e:#}"))).await
}
#[tauri::command]
pub(crate) async fn read_content_status() -> Reply<core::ContentStatus> {
    off_main_thread(move || Ok(core::content_status(&settings()?))).await
}

#[tauri::command]
pub(crate) async fn push_content(on_progress: Channel<String>) -> Reply<Snapshot> {
    off_main_thread(move || {
        let progress = MutationProgress::new(Some(on_progress));
        mutate_with_progress(&progress, |s| {
            core::push_content_with_progress(s, &|stage| progress.stage(stage))
        })
    })
    .await
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ApplyReply {
    Applied { workspace: Snapshot },
    PlanChanged { workspace: Snapshot },
}

#[tauri::command]
pub(crate) async fn apply_workspace(
    on_progress: Channel<String>,
    expected_plan: String,
) -> Reply<ApplyReply> {
    off_main_thread(move || {
        let progress = MutationProgress::new(Some(on_progress));
        let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
        progress.send("Applying changes…".into());
        match core::apply_reviewed(&settings()?, &expected_plan) {
            Ok(plan) => Ok(ApplyReply::Applied {
                workspace: snapshot_with(Some(plan))?,
            }),
            Err(error) => match error.downcast::<core::PlanChanged>() {
                Ok(changed) => Ok(ApplyReply::PlanChanged {
                    workspace: snapshot_with(Some(*changed.plan))?,
                }),
                Err(error) => Err(format!("{error:#}")),
            },
        }
    })
    .await
}

#[tauri::command]
pub(crate) async fn add_monitored_project(root: String, source: String) -> Reply<Snapshot> {
    off_main_thread(move || {
        mutate(|_| {
            core::add_project(&core::default_config_path()?, &PathBuf::from(root), &source)
                .map(|_| ())
        })
    })
    .await
}
#[tauri::command]
pub(crate) async fn remove_monitored_project(root: String) -> Reply<Snapshot> {
    off_main_thread(move || {
        mutate(|_| core::remove_project(&core::default_config_path()?, &PathBuf::from(root)))
    })
    .await
}
#[tauri::command]
pub(crate) async fn restore_workspace(
    on_progress: Channel<String>,
    backup: PathBuf,
) -> Reply<Snapshot> {
    off_main_thread(move || {
        let progress = MutationProgress::new(Some(on_progress));
        mutate_with_progress(&progress, |s| {
            progress
                .edit(s, None, Edit::RestoreBackup { directory: &backup })
                .map(|_| ())
        })
    })
    .await
}
#[tauri::command]
pub(crate) async fn sync_workspace(
    app: tauri::AppHandle,
    on_progress: Channel<String>,
) -> Reply<Snapshot> {
    #[cfg(windows)]
    {
        let _ = on_progress;
        crate::tray::request_sync(&app);
        off_main_thread(snapshot).await
    }
    #[cfg(not(windows))]
    let _ = app;
    #[cfg(not(windows))]
    off_main_thread(move || {
        let progress = MutationProgress::new(Some(on_progress));
        mutate_with_plan(&progress, |s| {
            progress.send("Syncing…".into());
            core::sync(s)
        })
    })
    .await
}
#[tauri::command]
pub(crate) async fn preview_schedule(seconds: u32) -> Reply<core::Schedule> {
    off_main_thread(move || {
        let helper = detected_helper().map_err(|e| format!("{e:#}"))?;
        core::schedule_plan(&settings()?, &config()?, &helper, seconds)
            .map_err(|e| format!("{e:#}"))
    })
    .await
}
#[tauri::command]
pub(crate) async fn enable_schedule(seconds: u32) -> Reply<String> {
    off_main_thread(move || {
        let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
        let settings = settings()?;
        let helper = detected_helper().map_err(|e| format!("{e:#}"))?;
        let plan = core::schedule_plan(&settings, &config()?, &helper, seconds)
            .map_err(|e| format!("{e:#}"))?;
        core::install_schedule(&settings, &plan).map_err(|e| format!("{e:#}"))?;
        Ok(format!("Installed {}", plan.label))
    })
    .await
}
#[tauri::command]
pub(crate) async fn disable_schedule() -> Reply<String> {
    off_main_thread(move || {
        let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
        core::remove_schedule(&settings()?).map_err(|e| format!("{e:#}"))?;
        Ok("Background schedule removed.".into())
    })
    .await
}

#[tauri::command]
pub(crate) fn set_desktop_busy(app: tauri::AppHandle, busy: bool) {
    crate::update_scheduler::set_busy(&app, busy);
}
#[tauri::command]
pub(crate) async fn background_status(app: tauri::AppHandle) -> Reply<Option<serde_json::Value>> {
    off_main_thread(move || {
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
    })
    .await
}
#[tauri::command]
pub(crate) async fn set_background_preferences(
    app: tauri::AppHandle,
    paused: Option<bool>,
    interval_seconds: Option<u32>,
) -> Reply<()> {
    off_main_thread(move || {
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
    })
    .await
}
#[tauri::command]
pub(crate) async fn migrate_background(app: tauri::AppHandle) -> Reply<()> {
    off_main_thread(move || {
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
    })
    .await
}

#[tauri::command]
pub(crate) async fn preview_conflict(
    path: PathBuf,
    action: core::ConflictAction,
) -> Reply<core::ResolutionPreview> {
    off_main_thread(move || {
        core::preview_resolution(&settings()?, &path, action).map_err(|e| format!("{e:#}"))
    })
    .await
}

#[tauri::command]
pub(crate) async fn resolve_conflict(
    on_progress: Channel<String>,
    expected: String,
) -> Reply<SavedPath> {
    off_main_thread(move || {
        let progress = MutationProgress::new(Some(on_progress));
        let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
        let path = progress
            .edit(
                &settings()?,
                None,
                Edit::ResolveConflict {
                    expected: &expected,
                },
            )
            .and_then(|p| {
                p.ok_or_else(|| anyhow::anyhow!("Conflict resolution returned no backup path"))
            })
            .map_err(|e| format!("{e:#}"))?;
        Ok(SavedPath {
            path,
            workspace: progress.snapshot()?,
        })
    })
    .await
}

#[tauri::command]
pub(crate) async fn preview_agent(target: core::AgentTarget) -> Reply<core::SourceDocument> {
    off_main_thread(move || {
        core::preview_agent_target(&settings()?, &target).map_err(|e| format!("{e:#}"))
    })
    .await
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
pub(crate) async fn login_enabled(app: tauri::AppHandle) -> Reply<bool> {
    off_main_thread(move || {
        use tauri_plugin_autostart::ManagerExt;
        app.autolaunch().is_enabled().map_err(|e| e.to_string())
    })
    .await
}

#[tauri::command]
pub(crate) async fn set_login_enabled(app: tauri::AppHandle, enabled: bool) -> Reply<bool> {
    off_main_thread(move || {
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
    })
    .await
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
pub(crate) async fn keep_legacy(expected: String) -> Reply<Snapshot> {
    off_main_thread(move || mutate(|s| core::keep_legacy(s, &expected))).await
}

#[tauri::command]
pub(crate) async fn preview_stale_target(
    path: PathBuf,
    action: core::StaleAction,
) -> Reply<core::StalePreview> {
    off_main_thread(move || {
        core::preview_stale_target(&settings()?, &path, action).map_err(|e| format!("{e:#}"))
    })
    .await
}

#[tauri::command]
pub(crate) async fn resolve_stale_target(expected: String) -> Reply<Option<PathBuf>> {
    off_main_thread(move || {
        let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
        core::resolve_stale_target(&settings()?, &expected).map_err(|e| format!("{e:#}"))
    })
    .await
}

#[cfg(test)]
mod command_tests;
