use crate::{
    database::Database, error::AppError, models::SyncProgress, weread::client::WeReadClient,
};
use rusqlite::{params, Transaction};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

struct BookNotes {
    book_id: String,
    book: Option<Value>,
    highlights: crate::weread::models::BookmarkListResponse,
    thoughts: Vec<Value>,
}

pub async fn run<F>(db: &Database, client: &WeReadClient, emit: F) -> Result<SyncProgress, AppError>
where
    F: Fn(&SyncProgress),
{
    let session_id = Uuid::new_v4().to_string();
    let started_at = now();
    db.connect()?.execute(
        "INSERT INTO sync_sessions(id,source,started_at,status) VALUES(?1,'weread',?2,'running')",
        params![session_id, started_at],
    )?;

    let result = async {
        let shelf = client.shelf().await?;
        let notebooks = client.all_notebooks().await?;
        let total = notebooks.len();
        let shelf_progress = persist(db, &session_id, started_at, &shelf, &[], false, true)?;
        emit(&SyncProgress {
            status: "processing".into(),
            progress: 10,
            books: shelf_progress.books,
            highlights: shelf_progress.highlights,
            thoughts: shelf_progress.thoughts,
            processed_books: 0,
            total_books: total,
        });

        for (index, notebook) in notebooks.into_iter().enumerate() {
            let highlights = client.highlights(&notebook.book_id).await?;
            let thoughts = client.all_thoughts(&notebook.book_id).await?;
            let item = BookNotes {
                book_id: notebook.book_id,
                book: notebook.book,
                highlights,
                thoughts,
            };
            let mut progress = persist(db, &session_id, started_at, &shelf, &[item], false, false)?;
            progress.status = "processing".into();
            progress.progress = if total == 0 { 90 } else { 10 + (((index + 1) * 80 / total) as i32) };
            progress.processed_books = index + 1;
            progress.total_books = total;
            emit(&progress);
        }

        let mut complete = persist(db, &session_id, started_at, &shelf, &[], true, false)?;
        complete.processed_books = total;
        complete.total_books = total;
        emit(&complete);
        Ok(complete)
    }
    .await;
    if let Err(error) = &result {
        mark_failed(db, &session_id, error);
    }
    result
}

fn persist(
    db: &Database,
    session_id: &str,
    started_at: i64,
    shelf: &crate::weread::models::ShelfResponse,
    notes: &[BookNotes],
    finalize: bool,
    write_shelf: bool,
) -> Result<SyncProgress, AppError> {
    let mut connection = db.connect()?;
    let tx = connection.transaction()?;

    if write_shelf {
        save_raw(
            &tx,
            "shelf",
            "current",
            &serde_json::to_value(&shelf)?,
            started_at,
        )?;
        for book in &shelf.books {
            tx.execute(
            "INSERT INTO books(book_id,title,author,cover,category,deep_link,read_update_time,finish_reading,update_time,created_at,synced_at,is_deleted,last_seen_sync_id)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10,0,?11)
             ON CONFLICT(book_id) DO UPDATE SET title=excluded.title,author=excluded.author,cover=excluded.cover,category=excluded.category,deep_link=excluded.deep_link,read_update_time=excluded.read_update_time,finish_reading=excluded.finish_reading,update_time=excluded.update_time,synced_at=excluded.synced_at,is_deleted=0,last_seen_sync_id=excluded.last_seen_sync_id",
            params![book.book_id,book.title,book.author,book.cover,book.category,book.deep_link,book.read_update_time,book.finish_reading,book.update_time,started_at,session_id],
            )?;
        }
    }

    for item in notes {
        if let Some(book) = &item.book {
            let title = string(book, "title").unwrap_or_else(|| "未命名书籍".into());
            tx.execute(
                "INSERT INTO books(book_id,title,author,cover,category,deep_link,created_at,synced_at,is_deleted,last_seen_sync_id)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?7,0,?8)
                 ON CONFLICT(book_id) DO UPDATE SET title=excluded.title,author=excluded.author,cover=excluded.cover,category=excluded.category,deep_link=coalesce(excluded.deep_link,books.deep_link),synced_at=excluded.synced_at,is_deleted=0,last_seen_sync_id=excluded.last_seen_sync_id",
                params![item.book_id,title,string(book,"author"),string(book,"cover"),string(book,"category"),string(book,"deepLink"),started_at,session_id],
            )?;
            save_raw(&tx, "notebook", &item.book_id, book, started_at)?;
        }
        save_raw(
            &tx,
            "highlights",
            &item.book_id,
            &serde_json::to_value(&item.highlights)?,
            started_at,
        )?;
        save_raw(
            &tx,
            "thoughts",
            &item.book_id,
            &Value::Array(item.thoughts.clone()),
            started_at,
        )?;
        let chapters = chapter_map(&item.highlights.chapters);
        for chapter in &item.highlights.chapters {
            upsert_chapter(&tx, &item.book_id, chapter, started_at)?;
        }
        for value in &item.highlights.updated {
            let Some(id) = string(value, "bookmarkId") else {
                continue;
            };
            let chapter_uid = integer(value, "chapterUid");
            let (chapter_idx, chapter_title) = chapter_uid
                .and_then(|id| chapters.get(&id).cloned())
                .unwrap_or((None, String::new()));
            let Some(content) = string(value, "markText") else {
                continue;
            };
            tx.execute(
                "INSERT INTO highlights(bookmark_id,book_id,chapter_uid,chapter_idx,chapter_title,mark_text,range_json,color_style,create_time,synced_at,is_deleted,last_seen_sync_id)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,0,?11)
                 ON CONFLICT(bookmark_id) DO UPDATE SET book_id=excluded.book_id,chapter_uid=excluded.chapter_uid,chapter_idx=excluded.chapter_idx,chapter_title=excluded.chapter_title,mark_text=excluded.mark_text,range_json=excluded.range_json,color_style=excluded.color_style,create_time=excluded.create_time,synced_at=excluded.synced_at,is_deleted=0,last_seen_sync_id=excluded.last_seen_sync_id",
                params![id,item.book_id,chapter_uid,chapter_idx,chapter_title,content,json_text(value.get("range")),string(value,"colorStyle"),integer(value,"createTime"),started_at,session_id],
            )?;
        }
        for wrapper in &item.thoughts {
            let value = wrapper.get("review").unwrap_or(wrapper);
            let Some(id) = string(value, "reviewId").or_else(|| string(wrapper, "reviewId")) else {
                continue;
            };
            let Some(content) = string(value, "content") else {
                continue;
            };
            tx.execute(
                "INSERT INTO thoughts(review_id,book_id,chapter_uid,chapter_idx,chapter_name,content,abstract,range_json,create_time,synced_at,is_deleted,last_seen_sync_id)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,0,?11)
                 ON CONFLICT(review_id) DO UPDATE SET book_id=excluded.book_id,chapter_uid=excluded.chapter_uid,chapter_idx=excluded.chapter_idx,chapter_name=excluded.chapter_name,content=excluded.content,abstract=excluded.abstract,range_json=excluded.range_json,create_time=excluded.create_time,synced_at=excluded.synced_at,is_deleted=0,last_seen_sync_id=excluded.last_seen_sync_id",
                params![id,item.book_id,integer(value,"chapterUid"),integer(value,"chapterIdx"),string(value,"chapterName"),content,string(value,"abstract"),json_text(value.get("range")),integer(value,"createTime"),started_at,session_id],
            )?;
        }
    }

    // 只有全部官方接口和分页成功后，才执行软删除。
    if finalize {
        tx.execute(
            "UPDATE books SET is_deleted=1 WHERE coalesce(last_seen_sync_id,'')<>?1",
            params![session_id],
        )?;
        tx.execute(
            "UPDATE highlights SET is_deleted=1 WHERE coalesce(last_seen_sync_id,'')<>?1",
            params![session_id],
        )?;
        tx.execute(
            "UPDATE thoughts SET is_deleted=1 WHERE coalesce(last_seen_sync_id,'')<>?1",
            params![session_id],
        )?;
    }
    if finalize {
        rebuild_fts(&tx)?;
    } else {
        for item in notes {
            rebuild_book_fts(&tx, &item.book_id)?;
        }
    }
    let finished_at = now();
    let book_count: i64 =
        tx.query_row("SELECT count(*) FROM books WHERE is_deleted=0", [], |row| {
            row.get(0)
        })?;
    let highlight_count: i64 = tx.query_row("SELECT count(*) FROM highlights WHERE is_deleted=0", [], |row| row.get(0))?;
    let thought_count: i64 = tx.query_row("SELECT count(*) FROM thoughts WHERE is_deleted=0", [], |row| row.get(0))?;
    if finalize {
        tx.execute("UPDATE sync_sessions SET finished_at=?2,status='success',books_fetched=?3,highlights_fetched=?4,thoughts_fetched=?5 WHERE id=?1", params![session_id,finished_at,book_count,highlight_count,thought_count])?;
        tx.execute("INSERT INTO sync_state(source,last_synced_at,last_successful_session) VALUES('weread',?1,?2) ON CONFLICT(source) DO UPDATE SET last_synced_at=excluded.last_synced_at,last_successful_session=excluded.last_successful_session", params![finished_at,session_id])?;
    }
    tx.commit()?;
    Ok(SyncProgress {
        status: if finalize { "complete" } else { "processing" }.into(),
        progress: if finalize { 100 } else { 0 },
        books: book_count,
        highlights: highlight_count,
        thoughts: thought_count,
        processed_books: 0,
        total_books: 0,
    })
}

fn rebuild_fts(tx: &Transaction<'_>) -> Result<(), AppError> {
    tx.execute("DELETE FROM notes_fts", [])?;
    tx.execute("INSERT INTO notes_fts(note_id,note_type,book_id,title,chapter_title,content) SELECT h.bookmark_id,'highlight',h.book_id,b.title,coalesce(h.chapter_title,''),h.mark_text FROM highlights h JOIN books b ON b.book_id=h.book_id WHERE h.is_deleted=0 AND b.is_deleted=0", [])?;
    tx.execute("INSERT INTO notes_fts(note_id,note_type,book_id,title,chapter_title,content) SELECT t.review_id,'thought',t.book_id,b.title,coalesce(t.chapter_name,''),t.content FROM thoughts t JOIN books b ON b.book_id=t.book_id WHERE t.is_deleted=0 AND b.is_deleted=0", [])?;
    Ok(())
}

fn rebuild_book_fts(tx: &Transaction<'_>, book_id: &str) -> Result<(), AppError> {
    tx.execute("DELETE FROM notes_fts WHERE book_id=?1", [book_id])?;
    tx.execute("INSERT INTO notes_fts(note_id,note_type,book_id,title,chapter_title,content) SELECT h.bookmark_id,'highlight',h.book_id,b.title,coalesce(h.chapter_title,''),h.mark_text FROM highlights h JOIN books b ON b.book_id=h.book_id WHERE h.book_id=?1 AND h.is_deleted=0 AND b.is_deleted=0", [book_id])?;
    tx.execute("INSERT INTO notes_fts(note_id,note_type,book_id,title,chapter_title,content) SELECT t.review_id,'thought',t.book_id,b.title,coalesce(t.chapter_name,''),t.content FROM thoughts t JOIN books b ON b.book_id=t.book_id WHERE t.book_id=?1 AND t.is_deleted=0 AND b.is_deleted=0", [book_id])?;
    Ok(())
}

fn save_raw(
    tx: &Transaction<'_>,
    kind: &str,
    id: &str,
    value: &Value,
    fetched_at: i64,
) -> Result<(), AppError> {
    tx.execute("INSERT INTO weread_raw(entity_type,entity_id,payload,fetched_at) VALUES(?1,?2,?3,?4) ON CONFLICT(entity_type,entity_id) DO UPDATE SET payload=excluded.payload,fetched_at=excluded.fetched_at", params![kind,id,serde_json::to_string(value)?,fetched_at])?;
    Ok(())
}
fn upsert_chapter(
    tx: &Transaction<'_>,
    book_id: &str,
    value: &Value,
    created_at: i64,
) -> Result<(), AppError> {
    let Some(uid) = integer(value, "chapterUid") else {
        return Ok(());
    };
    tx.execute("INSERT INTO chapters(book_id,chapter_uid,chapter_idx,title,created_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(book_id,chapter_uid) DO UPDATE SET chapter_idx=excluded.chapter_idx,title=excluded.title",params![book_id,uid,integer(value,"chapterIdx"),string(value,"title"),created_at])?;
    Ok(())
}
fn chapter_map(values: &[Value]) -> std::collections::HashMap<i64, (Option<i64>, String)> {
    values
        .iter()
        .filter_map(|v| {
            integer(v, "chapterUid").map(|id| {
                (
                    id,
                    (
                        integer(v, "chapterIdx"),
                        string(v, "title").unwrap_or_default(),
                    ),
                )
            })
        })
        .collect()
}
fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}
fn integer(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(Value::as_i64)
}
fn json_text(value: Option<&Value>) -> Option<String> {
    value.filter(|v| !v.is_null()).map(|v| {
        if let Some(s) = v.as_str() {
            s.to_owned()
        } else {
            v.to_string()
        }
    })
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn mark_failed(db: &Database, id: &str, error: &AppError) {
    let _ = db.connect().and_then(|c| {
        c.execute(
            "UPDATE sync_sessions SET finished_at=?2,status='failed',error_message=?3 WHERE id=?1",
            params![id, now(), error.to_string()],
        )?;
        Ok(c)
    });
}

#[cfg(test)]
mod tests {
    use super::{chapter_map, integer, json_text, string};
    use serde_json::json;

    #[test]
    fn maps_chapters_by_official_uid() {
        let chapters = chapter_map(&[
            json!({"chapterUid": 10, "chapterIdx": 2, "title": "第二章"}),
            json!({"chapterUid": 11, "title": "附录"}),
        ]);
        assert_eq!(chapters.get(&10), Some(&(Some(2), "第二章".into())));
        assert_eq!(chapters.get(&11), Some(&(None, "附录".into())));
    }

    #[test]
    fn extracts_only_matching_json_types() {
        let value = json!({"name":"内容","time":123,"range":{"start":1}});
        assert_eq!(string(&value, "name").as_deref(), Some("内容"));
        assert_eq!(integer(&value, "time"), Some(123));
        assert_eq!(string(&value, "time"), None);
        assert_eq!(
            json_text(value.get("range")).as_deref(),
            Some("{\"start\":1}")
        );
    }
}
