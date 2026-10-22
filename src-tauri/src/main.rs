// Sin consola extra en Windows en las builds de release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod state;

use tauri::Manager;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,micra=debug,micra_core=debug".into()),
        )
        .init();

    let state = state::AppState::new().expect("no se pudo inicializar el estado de la app");

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::hardware_info,
            commands::minecraft_versions,
            commands::list_java_runtimes,
            commands::search_modrinth,
            commands::modrinth_versions,
            commands::curseforge_enabled,
            commands::search_curseforge,
            commands::curseforge_files,
            commands::list_servers,
            commands::create_server,
            commands::start_server,
            commands::stop_server,
            commands::send_command,
        ])
        .build(tauri::generate_context!())
        .expect("error al construir la app")
        .run(|app, event| {
            // Al cerrar la app, apagar los servidores con `stop` para no
            // corromper mundos.
            if let tauri::RunEvent::Exit = event {
                let state = app.state::<state::AppState>();
                tauri::async_runtime::block_on(state.stop_all());
            }
        });
}
