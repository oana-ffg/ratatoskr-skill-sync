#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let handle = app.handle().clone();
            if ratatoskr_core::default_config_path()
                .ok()
                .and_then(|p| ratatoskr_core::load(&p).ok())
                .and_then(|s| s.updates)
                .is_some_and(|u| u.automatic)
            {
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = commands::update_desktop(handle, true).await {
                        eprintln!("Ratatoskr desktop auto-update: {error}");
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
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
            commands::adopt_workspace,
            commands::restore_workspace,
            commands::sync_workspace,
            commands::preview_schedule,
            commands::enable_schedule,
            commands::disable_schedule,
            commands::update_desktop,
            commands::restart_desktop
        ])
        .run(tauri::generate_context!())
        .expect("Failed to run Ratatoskr Skill Sync");
}
