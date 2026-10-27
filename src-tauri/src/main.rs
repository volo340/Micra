// Sin consola extra en Windows en las builds de release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod events;
mod notifier;
mod state;
mod supervisor;
mod tunnel;

use std::sync::Arc;

use tauri::Manager;

use commands::*;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,micra=debug,micra_core=debug".into()),
        )
        .init();

    let state = tauri::async_runtime::block_on(state::AppState::new())
        .expect("no se pudo inicializar el estado de la app");
    // Lo que quedó de un servidor que no se terminó de borrar.
    tauri::async_runtime::block_on(micra_core::provision::purge_deleted(&state.paths));

    tauri::Builder::default()
        // Una sola Micra a la vez: dos copias se pelearían por los servidores.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(Arc::new(state))
        .invoke_handler(tauri::generate_handler![
            app::app_info,
            app::hardware_info,
            app::network_info,
            app::list_java_runtimes,
            app::open_data_folder,
            app::get_settings,
            app::save_settings,
            app::set_secret,
            app::delete_secret,
            app::test_notification,
            app::check_app_update,
            app::install_app_update,
            content::minecraft_versions,
            content::search_content,
            content::loader_game_versions,
            content::modrinth_versions,
            content::curseforge_enabled,
            content::curseforge_files,
            servers::list_servers,
            servers::create_server,
            servers::delete_server,
            servers::start_server,
            servers::stop_server,
            servers::restart_server,
            servers::send_command,
            servers::open_server_folder,
            servers::update_server_settings,
            players::player_lists,
            players::player_action,
            backups::list_backups,
            backups::create_backup,
            backups::restore_backup,
            backups::delete_backup,
            backups::open_backups_folder,
            mods::list_mods,
            mods::set_mod_enabled,
            mods::remove_mod,
            mods::check_mod_updates,
            mods::update_mod,
            mods::search_mods,
            mods::install_mod,
            mods::install_missing_mod,
            mods::scan_mods,
            mods::quarantine_mod,
            mods::check_pack_update,
            mods::update_pack,
            mods::dismiss_crash,
            mods::open_crash_report,
            properties::get_properties,
            properties::save_properties,
            stats::server_stats,
            network::public_ip,
            network::check_reachable,
            network::playit_claim_start,
            network::playit_claim_poll,
            import::pick_folder,
            import::analyze_import,
            import::import_server,
            map::enable_map,
            map::disable_map,
            map::accept_map_download,
            map::open_map,
        ])
        .build(tauri::generate_context!())
        .expect("error al construir la app")
        .run(|app, event| {
            // Al cerrar la app, apagar los servidores con `stop` para no
            // dañar los mundos, y cerrar los puertos abiertos en el router.
            if let tauri::RunEvent::Exit = event {
                let state = app.state::<state::Shared>();
                tauri::async_runtime::block_on(state.stop_all());
            }
        });
}
