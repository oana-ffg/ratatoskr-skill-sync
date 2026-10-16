#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
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
            commands::preview_schedule,
            commands::enable_schedule,
            commands::disable_schedule,
            commands::update_desktop,
            commands::nightly_update,
            commands::startup_update,
            commands::save_update_schedule,
            commands::restart_desktop
        ])
        .run(tauri::generate_context!())
        .expect("Failed to run Ratatoskr Skill Sync");
}
