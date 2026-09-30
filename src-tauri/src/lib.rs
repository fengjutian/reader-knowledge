mod ai;
mod commands;
mod database;
mod error;
mod models;
mod metadata;
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
            commands::get_database_overview,
            commands::list_database_rows,
            commands::get_reading_stats,
            commands::list_books,
            commands::list_book_metadata,
            commands::get_book_metadata_details,
            commands::fetch_book_metadata,
            commands::fetch_douban_book_metadata,
            commands::fetch_books_metadata,
            commands::get_book,
            commands::list_book_notes,
            commands::open_book,
            commands::open_external_url,
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
            ,commands::local_embedding_status
            ,commands::download_local_embedding
            ,commands::delete_local_embedding
        ])
        .run(tauri::generate_context!())
        .expect("error while running wereader")
}
