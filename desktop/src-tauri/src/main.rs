#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod desktop_updates;
#[cfg(windows)]
mod tray;

fn main() {
    let builder = tauri::Builder::default();
    #[cfg(windows)]
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, args, _| {
            if !args.iter().any(|arg| arg == "--background")
                && let Err(error) = tray::open(app)
            {
                tray::report(app, error);
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .setup(|app| tray::setup(app).map_err(Into::into));
    let autostart = tauri_plugin_autostart::Builder::new();
    #[cfg(windows)]
    let autostart = autostart.args(["--background"]);
    builder
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(autostart.build())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            commands::preview_legacy_resolution,
            commands::resolve_legacy,
            commands::keep_legacy,
            commands::skill_target_statuses,
            commands::preview_install_path,
            commands::create_desktop_skill,
            commands::different_preview_lines,
            commands::editor_view,
            commands::edit_skill_draft,
            commands::save_skill_draft,
            commands::read_structured_source,
            commands::save_structured_source,
            commands::list_values,
            commands::list_skill_resources,
            commands::add_skill_resource,
            commands::rename_source_skill,
            commands::choose_folder,
            commands::choose_project_source,
            commands::choose_resource,
            commands::read_local_settings,
            commands::save_local_settings,
            commands::read_conflict_source,
            commands::preview_source_edit,
            commands::preview_conflict,
            commands::resolve_conflict,
            commands::preview_agent,
            commands::preview_skill,
            commands::analyze_skill,
            commands::list_agent_presets,
            commands::list_preview_targets,
            commands::detect_helper,
            commands::add_agent_target,
            commands::read_job_status,
            commands::login_enabled,
            commands::set_login_enabled,
            commands::read_workspace,
            commands::connect_workspace,
            commands::apply_workspace,
            commands::save_configuration,
            commands::add_monitored_project,
            commands::remove_monitored_project,
            commands::read_source_configuration,
            commands::save_source_configuration,
            commands::read_source_skill,
            commands::save_source_skill,
            commands::new_source_skill,
            commands::set_skill_enabled,
            commands::archive_source_skill,
            commands::restore_workspace,
            commands::sync_workspace,
            commands::set_desktop_busy,
            commands::background_status,
            commands::set_background_preferences,
            commands::migrate_background,
            commands::preview_schedule,
            commands::enable_schedule,
            commands::disable_schedule,
            commands::read_update_settings,
            commands::update_desktop,
            commands::nightly_update,
            commands::startup_update,
            commands::save_update_schedule,
            commands::restart_desktop
        ])
        .run(tauri::generate_context!())
        .expect("Failed to run Ratatoskr Skill Sync");
}
