use ratatoskr_core::{self as core, Plan, Settings, SkillDocument, SkillEntry};
use serde::Serialize;
use std::{fs, path::PathBuf, sync::Mutex};
use tauri_plugin_updater::UpdaterExt;

static MUTATION: Mutex<()> = Mutex::new(());
type Reply<T> = Result<T, String>;

#[derive(Serialize)]
pub(crate) struct Snapshot {
    config_path: PathBuf,
    config_text: String,
    plan: Plan,
    skills: Vec<SkillEntry>,
    helper: PathBuf,
    source_files: Vec<String>,
    updates: UpdateStatus,
}

#[derive(Serialize)]
pub(crate) struct UpdateStatus {
    endpoint: String,
    override_active: bool,
    automatic: bool,
    time: String,
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
        changes: vec![],
        conflicts: vec![format!("Preview failed: {error:#}")],
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
        helper: std::env::current_exe()
            .and_then(|path| path.canonicalize())
            .map_err(|e| e.to_string())?
            .with_file_name(if cfg!(windows) {
                "ratatoskr.exe"
            } else {
                "ratatoskr"
            }),
        source_files,
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
) -> Reply<Snapshot> {
    let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
    let path = config()?;
    if create {
        core::create_content(&path, &PathBuf::from(source), &machine)
    } else if let Some(remote) = remote.filter(|r| !r.trim().is_empty()) {
        core::clone_content(&path, &remote, &PathBuf::from(source), &machine)
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
pub(crate) fn adopt_workspace() -> Reply<String> {
    let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
    core::adopt(&settings()?)
        .map(|p| p.display().to_string())
        .map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub(crate) fn restore_workspace(backup: PathBuf) -> Reply<Snapshot> {
    mutate(|s| core::restore_backup(s, &backup).map(|_| ()))
}
#[tauri::command]
pub(crate) async fn sync_workspace() -> Reply<Snapshot> {
    tauri::async_runtime::spawn_blocking(|| mutate(|s| core::sync(s).map(|_| ())))
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
pub(crate) fn preview_schedule(helper: PathBuf, seconds: u32) -> Reply<core::Schedule> {
    core::schedule_plan(&settings()?, &config()?, &helper, seconds).map_err(|e| format!("{e:#}"))
}
#[tauri::command]
pub(crate) fn enable_schedule(helper: PathBuf, seconds: u32) -> Reply<String> {
    let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
    let settings = settings()?;
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

fn desktop_updater(
    app: &tauri::AppHandle,
    settings: &Settings,
) -> anyhow::Result<tauri_plugin_updater::Updater> {
    Ok(app
        .updater_builder()
        .pubkey(settings.updates.tauri_public_key()?)
        .endpoints(vec![settings.updates.endpoint().parse()?])?
        .timeout(std::time::Duration::from_secs(60))
        .build()?)
}

struct Nightly {
    pending: Option<(String, tauri_plugin_updater::Update)>,
}
static NIGHTLY: Mutex<Nightly> = Mutex::new(Nightly { pending: None });

fn update_result<T>(settings: &Settings, result: anyhow::Result<T>) -> Reply<T> {
    let error = result
        .as_ref()
        .err()
        .map(|e| format!("Desktop update: {e:#}"));
    core::updates::report_outcome(
        settings,
        error.as_deref(),
        core::updates::UpdateKind::Desktop,
    )
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
    if !config()?.try_exists().map_err(|e| e.to_string())? {
        return Ok(false);
    }
    automatic_update(app, true, true).await
}

async fn automatic_update(app: tauri::AppHandle, install: bool, launch: bool) -> Reply<bool> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = if install {
            match MUTATION.try_lock() {
                Ok(guard) => Some(guard),
                Err(std::sync::TryLockError::WouldBlock) => return Ok(false),
                Err(error) => return Err(error.to_string()),
            }
        } else {
            None
        };
        let settings = settings()?;
        let mut attempted = false;
        let mut installed = false;
        let result = (|| -> anyhow::Result<bool> {
            let mut nightly = NIGHTLY.lock().map_err(|e| anyhow::anyhow!(e.to_string()))?;
            let now = chrono::Local::now().fixed_offset();
            let fingerprint = toml::to_string(&settings)?;
            if !settings.updates.automatic
                || (!launch && !core::updates::desktop_window(&settings.updates, now)?)
            {
                nightly.pending = None;
                return Ok(false);
            }
            if nightly
                .pending
                .as_ref()
                .is_some_and(|(saved, _)| saved != &fingerprint)
            {
                nightly.pending = None;
            }
            let mut state = core::updates::DesktopUpdateState::load(&settings)?;
            if (launch || !install) && state.due(&settings.updates, now, launch)? {
                attempted = true;
                state.record_attempt(&settings, now)?;
                nightly.pending = None;
                nightly.pending =
                    tauri::async_runtime::block_on(desktop_updater(&app, &settings)?.check())?
                        .map(|update| (fingerprint, update));
                state.record_available(&settings, nightly.pending.is_some())?;
            }
            if install && let Some((_, update)) = nightly.pending.take() {
                let Some(_idle) = core::updates::idle_guard(&settings, launch)? else {
                    nightly.pending = Some((toml::to_string(&settings)?, update));
                    return Ok(false);
                };
                attempted = true;
                tauri::async_runtime::block_on(update.download_and_install(|_, _| (), || ()))?;
                installed = true;
                // The installed version will recheck on the next window, without a restart loop.
                state.record_available(&settings, false)?;
                return Ok(true);
            }
            Ok(!install && nightly.pending.is_some())
        })();
        let restart = install && installed;
        let result = if attempted || result.is_err() {
            update_result(&settings, result)
        } else {
            result.map_err(|e| format!("{e:#}"))
        };
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

#[tauri::command]
pub(crate) async fn update_desktop(app: tauri::AppHandle, install: bool) -> Reply<String> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = MUTATION.lock().map_err(|e| e.to_string())?;
        let settings = settings()?;
        let result = (|| -> anyhow::Result<String> {
            match tauri::async_runtime::block_on(desktop_updater(&app, &settings)?.check())? {
                Some(update) if install => {
                    let _idle = core::updates::idle_guard(&settings, false)?.ok_or_else(|| {
                        anyhow::anyhow!(
                            "Apply, sync, or a schedule change is running; retry when idle"
                        )
                    })?;
                    tauri::async_runtime::block_on(update.download_and_install(|_, _| (), || ()))?;
                    Ok("Signed desktop update installed. Restart to use it.".into())
                }
                Some(update) => Ok(format!("Desktop update available: {}", update.version)),
                None => Ok("No newer desktop release available.".into()),
            }
        })();
        update_result(&settings, result)
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
pub(crate) fn restart_desktop(app: tauri::AppHandle) {
    app.restart();
}
