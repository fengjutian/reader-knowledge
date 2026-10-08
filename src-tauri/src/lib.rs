mod ai;
mod commands;
pub mod database;
pub mod error;
mod http;
mod import;
mod rag;
mod models;
mod metadata;
mod sync;
mod weread;
pub mod wikipedia;

use std::sync::Arc;
use tauri::Manager;
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        // 只用于本地文件导入时弹出系统文件选择框；能力清单里只开放 open 权限。
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let path = app.path().app_data_dir()?.join("readflow.db");
            app.manage(
                database::Database::open(path)
                    .map_err(|e| Box::<dyn std::error::Error>::from(e))?,
            );
            app.manage(Arc::new(ai::stream::StreamRegistry::default()));
            Ok(())
        })
        .on_window_event(|window, event| {
            // 窗口关闭时中止所有还在读的网络流，否则 socket 会被一直持有到进程退出。
            if matches!(event, tauri::WindowEvent::Destroyed) {
                if let Some(registry) = window.app_handle().try_state::<Arc<ai::stream::StreamRegistry>>() {
                    registry.cancel_all();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_dashboard,
            commands::get_database_overview,
            commands::list_database_rows,
            commands::get_reading_stats,
            commands::get_book_recommendations,
            commands::get_book_recommendation_detail,
            commands::list_books,
            commands::list_books_page,
            commands::list_book_metadata,
            commands::get_book_metadata_details,
            commands::fetch_book_metadata,
            commands::fetch_douban_book_metadata,
            commands::fetch_books_metadata,
            commands::list_glossary_terms,
            commands::save_glossary_term,
            commands::delete_glossary_term,
            commands::glossary_import::create_glossary_import,
            commands::glossary_import::list_glossary_imports,
            commands::glossary_import::get_glossary_import,
            commands::glossary_import::pause_glossary_import,
            commands::glossary_import::resume_glossary_import,
            commands::glossary_import::cancel_glossary_import,
            commands::glossary_import::delete_glossary_import,
            commands::glossary_import::publish_glossary_import,
            commands::glossary_import::glossary_import_errors,
            commands::glossary_import::glossary_import_report,
            commands::glossary_import::cleanup_glossary_import,
            commands::glossary_import::list_glossary_review_queue,
            commands::glossary_import::set_glossary_term_status,
            commands::glossary_import::bulk_set_glossary_term_status,
            commands::search_wikipedia,
            commands::get_book,
            commands::list_book_notes,
            commands::open_book,
            commands::open_external_url,
            commands::list_notes,
            commands::search_notes,
            commands::global_search,
            commands::save_secret,
            commands::has_secret,
            commands::test_connection,
            commands::sync_weread,
            commands::get_ai_settings,
            commands::save_ai_settings,
            commands::test_ai,
            commands::ask_ai,
            commands::ask_ai_stream,
            commands::cancel_ai_stream,
            commands::analyze_book_relation,
            commands::get_cached_relation_analysis
            ,commands::get_embedding_settings
            ,commands::save_embedding_settings
            ,commands::test_embedding
            ,commands::get_reranker_settings
            ,commands::save_reranker_settings
            ,commands::test_reranker
            ,commands::list_concept_graph
            ,commands::get_concept_entity
            ,commands::correct_concept_entity
            ,commands::merge_concept_entities
            ,commands::clear_suggested_concepts
            ,commands::scan_concepts
            ,commands::preview_web_import
            ,commands::preview_file_import
            ,commands::confirm_import
            ,commands::list_library_sources
            ,commands::get_library_source
            ,commands::delete_library_source
            ,commands::purge_library_source
            ,commands::build_semantic_relations
            ,commands::build_metadata_relations
            ,commands::local_embedding_status
            ,commands::download_local_embedding
            ,commands::delete_local_embedding
        ])
        .run(tauri::generate_context!())
        .expect("error while running wereader")
}
