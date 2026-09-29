mod ai;
mod commands;
mod database;
mod error;
mod models;
mod sync;
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
            commands::get_book,
            commands::list_book_notes,
            commands::open_book,
            commands::list_notes,
            commands::search_notes,
            commands::save_secret,
            commands::has_secret,
            commands::test_connection,
            commands::sync_weread,
            commands::get_ai_settings,
            commands::save_ai_settings,
            commands::test_ai,
            commands::ask_ai,
            commands::analyze_book_relation,
            commands::get_cached_relation_analysis
            ,commands::get_embedding_settings
            ,commands::save_embedding_settings
            ,commands::test_embedding
            ,commands::build_semantic_relations
        ])
        .run(tauri::generate_context!())
        .expect("error while running wereader")
}
