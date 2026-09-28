mod ai;
mod commands;
mod database;
mod error;
mod models;
mod weread;

use tauri::Manager;
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let path = app.path().app_data_dir()?.join("readflow.db");
            app.manage(
                database::Database::open(path)
                    .map_err(|e| Box::<dyn std::error::Error>::from(e))?,
            );
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_dashboard,
            commands::list_books,
            commands::list_notes,
            commands::search_notes,
            commands::save_secret,
            commands::test_connection,
            commands::sync_weread,
            commands::ask_ai
        ])
        .run(tauri::generate_context!())
        .expect("error while running ReadFlow")
}
