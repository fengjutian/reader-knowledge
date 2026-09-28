use crate::{database::Database, error::AppError, models::*};
use rusqlite::OptionalExtension;
use serde::Serialize;
use tauri::State;

#[tauri::command]
pub fn get_dashboard(db: State<'_, Database>) -> Result<DashboardStats, AppError> {
    let c = db.connect()?;
    Ok(DashboardStats {
        books: c.query_row("SELECT count(*) FROM books WHERE is_deleted=0", [], |r| {
            r.get(0)
        })?,
        highlights: c.query_row(
            "SELECT count(*) FROM highlights WHERE is_deleted=0",
            [],
            |r| r.get(0),
        )?,
        thoughts: c.query_row(
            "SELECT count(*) FROM thoughts WHERE is_deleted=0",
            [],
            |r| r.get(0),
        )?,
        last_synced_at: c
            .query_row(
                "SELECT datetime(last_synced_at,'unixepoch','localtime') FROM sync_state WHERE source='weread'",
                [],
                |row| row.get(0),
            )
            .optional()?,
    })
}

#[tauri::command]
pub fn list_books(db: State<'_, Database>) -> Result<Vec<Book>, AppError> {
    let c = db.connect()?;
    let mut q=c.prepare("SELECT b.book_id,b.title,coalesce(b.author,''),coalesce(b.cover,''),(SELECT count(*) FROM highlights h WHERE h.book_id=b.book_id AND h.is_deleted=0),(SELECT count(*) FROM thoughts t WHERE t.book_id=b.book_id AND t.is_deleted=0),CASE b.finish_reading WHEN 1 THEN 100 ELSE 0 END,coalesce(datetime(b.read_update_time,'unixepoch','localtime'),'') FROM books b WHERE b.is_deleted=0 ORDER BY b.read_update_time DESC")?;
    let books = q
        .query_map([], |r| {
            Ok(Book {
                id: r.get(0)?,
                title: r.get(1)?,
                author: r.get(2)?,
                cover: r.get(3)?,
                highlight_count: r.get(4)?,
                thought_count: r.get(5)?,
                progress: r.get(6)?,
                updated_at: r.get(7)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(books)
}

#[tauri::command]
pub fn list_notes(
    db: State<'_, Database>,
    note_type: Option<String>,
) -> Result<Vec<Note>, AppError> {
    search_impl(&db, "", note_type).map(|v| v.into_iter().map(|r| r.note).collect())
}

#[tauri::command]
pub fn search_notes(db: State<'_, Database>, query: String) -> Result<Vec<SearchResult>, AppError> {
    search_impl(&db, &query, None)
}

fn search_impl(
    db: &Database,
    query: &str,
    kind: Option<String>,
) -> Result<Vec<SearchResult>, AppError> {
    let c = db.connect()?;
    let sql = if query.trim().is_empty() {
        "SELECT note_id,note_type,book_id,title,chapter_title,content,0.0 FROM notes_fts WHERE (?1 IS NULL OR note_type=?1) LIMIT 100"
    } else {
        "SELECT note_id,note_type,book_id,title,chapter_title,content,bm25(notes_fts) FROM notes_fts WHERE notes_fts MATCH ?2 AND (?1 IS NULL OR note_type=?1) ORDER BY bm25(notes_fts) LIMIT 20"
    };
    let mut q = c.prepare(sql)?;
    let rows = if query.trim().is_empty() {
        q.query_map(rusqlite::params![kind], map_note)?
    } else {
        q.query_map(
            rusqlite::params![kind, format!("\"{}\"", query.replace('"', "\"\""))],
            map_note,
        )?
    };
    Ok(rows.collect::<Result<_, _>>()?)
}
fn map_note(r: &rusqlite::Row<'_>) -> rusqlite::Result<SearchResult> {
    Ok(SearchResult {
        note: Note {
            id: r.get(0)?,
            note_type: r.get(1)?,
            book_id: r.get(2)?,
            book_title: r.get(3)?,
            chapter: r.get(4)?,
            content: r.get(5)?,
            created_at: String::new(),
        },
        score: r.get(6)?,
    })
}

#[tauri::command]
pub fn save_secret(kind: String, value: String) -> Result<(), AppError> {
    keyring::Entry::new("ReadFlow", &kind)?.set_password(&value)?;
    Ok(())
}
#[tauri::command]
pub async fn test_connection(kind: String) -> Result<bool, AppError> {
    let secret = keyring::Entry::new("ReadFlow", &kind)?.get_password()?;
    if kind == "weread" {
        crate::weread::client::WeReadClient::new(secret)?
            .test()
            .await?;
    }
    Ok(true)
}
#[tauri::command]
pub async fn sync_weread(db: State<'_, Database>) -> Result<SyncProgress, AppError> {
    let secret = keyring::Entry::new("ReadFlow", "weread")?.get_password()?;
    let client = crate::weread::client::WeReadClient::new(secret)?;
    crate::sync::run(&db, &client).await
}

#[derive(Serialize)]
pub struct Citation {
    index: i32,
    note: Note,
}
#[derive(Serialize)]
pub struct AiAnswer {
    content: String,
    citations: Vec<Citation>,
}
#[tauri::command]
pub fn ask_ai(_db: State<'_, Database>, _question: String) -> Result<AiAnswer, AppError> {
    Err(AppError::Message("AI 服务尚未配置，不能生成回答".into()))
}
