use crate::{
    database::Database, error::AppError, models::SyncProgress, weread::client::WeReadClient,
};
use rusqlite::{params, Connection, Transaction};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

struct BookNotes {
    book_id: String,
    book: Option<Value>,
    highlights: crate::weread::models::BookmarkListResponse,
    thoughts: Vec<Value>,
    /// `/user/notebooks` 报告的划线条数。`/book/bookmarklist` 没有分页参数，
    /// 拿到的条数少于这个基准就说明本次没拿全。
    expected_highlights: i64,
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
        let shelf_progress = persist(db, &session_id, started_at, &shelf, &[], false, true, &[])?;
        emit(&SyncProgress {
            status: "processing".into(),
            progress: 10,
            books: shelf_progress.books,
            highlights: shelf_progress.highlights,
            thoughts: shelf_progress.thoughts,
            processed_books: 0,
            total_books: total,
        });

        // 划线被截断的书：接口没有分页参数，拿不全时绝不能对它们做软删除，
        // 否则超出部分的划线会在每次同步时被反复标记删除（真实数据丢失）。
        let mut truncated_books: Vec<String> = Vec::new();

        for (index, notebook) in notebooks.into_iter().enumerate() {
            let highlights = client.highlights(&notebook.book_id).await?;
            let thoughts = client.all_thoughts(&notebook.book_id).await?;
            let item = BookNotes {
                expected_highlights: notebook.note_count,
                book_id: notebook.book_id,
                book: notebook.book,
                highlights,
                thoughts,
            };
            if item.expected_highlights > item.highlights.updated.len() as i64 {
                truncated_books.push(item.book_id.clone());
            }
            let mut progress = persist(db, &session_id, started_at, &shelf, &[item], false, false, &[])?;
            progress.status = "processing".into();
            progress.progress = if total == 0 { 90 } else { 10 + (((index + 1) * 80 / total) as i32) };
            progress.processed_books = index + 1;
            progress.total_books = total;
            emit(&progress);
        }

        let mut complete = persist(db, &session_id, started_at, &shelf, &[], true, false, &truncated_books)?;
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
    truncated_books: &[String],
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
            let raw = serde_json::to_value(book)?;
            upsert_weread_metadata(&tx, &book.book_id, &raw, started_at)?;
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
            upsert_weread_metadata(&tx, &item.book_id, book, started_at)?;
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
        // `/book/bookmarklist` 没有分页参数（官方只声明 bookId 入参），拿不全时
        // 对该书做软删除会把超出部分的划线永远标记为删除。所以截断的书整本跳过。
        let skip_highlights = if truncated_books.is_empty() {
            // 无截断时不加任何额外条件。
            String::new()
        } else {
            let list = truncated_books
                .iter()
                .map(|id| format!("'{}'", id.replace('\'', "''")))
                .collect::<Vec<_>>()
                .join(",");
            format!(" AND book_id NOT IN ({list})")
        };
        let affected = tx.execute(
            &format!(
                "UPDATE highlights SET is_deleted=1
                  WHERE coalesce(last_seen_sync_id,'')<>?1{skip_highlights}"
            ),
            params![session_id],
        )?;
        let _ = affected;
        tx.execute(
            "UPDATE thoughts SET is_deleted=1 WHERE coalesce(last_seen_sync_id,'')<>?1",
            params![session_id],
        )?;
        if !truncated_books.is_empty() {
            tx.execute(
                "UPDATE sync_sessions SET error_message=?2 WHERE id=?1",
                params![
                    session_id,
                    format!(
                        "有 {} 本书的划线未能完整获取（接口无分页），已跳过这些书的划线软删除：{}",
                        truncated_books.len(),
                        truncated_books.join("、")
                    )
                ],
            )?;
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

/// 微信读书书籍元数据的解析结果，只包含落库前的纯数据，不接触数据库和网络。
#[derive(Debug, Default, PartialEq)]
pub(crate) struct WereadMetadataFields {
    pub title: Option<String>,
    pub authors: Vec<String>,
    pub subjects: Vec<String>,
    pub isbn10: Option<String>,
    pub isbn13: Option<String>,
    pub publisher: Option<String>,
    pub published_date: Option<String>,
    pub page_count: Option<i64>,
    pub cover_url: Option<String>,
    pub description: Option<String>,
    pub rating: Option<f64>,
    pub rating_count: Option<i64>,
    pub source_url: Option<String>,
    pub raw_json: String,
}

/// 从微信读书返回的书籍 JSON 中提取元数据字段，便于脱离同步流程单独测试。
pub(crate) fn weread_metadata_fields(value: &Value) -> Result<WereadMetadataFields, AppError> {
    let author = string(value, "author").unwrap_or_default();
    let authors = if author.trim().is_empty() {
        Vec::new()
    } else {
        vec![author]
    };
    let category = string(value, "category").unwrap_or_default();
    let subjects = category
        .split(['/', ',', '、', '·'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let isbn = string(value, "isbn").unwrap_or_default();
    let (isbn10, isbn13) = match isbn.len() {
        10 => (Some(isbn), None),
        13 => (None, Some(isbn)),
        _ => (None, None),
    };
    Ok(WereadMetadataFields {
        title: string(value, "title"),
        authors,
        subjects,
        isbn10,
        isbn13,
        publisher: string(value, "publisher"),
        published_date: string(value, "publishTime"),
        page_count: integer(value, "pageCount"),
        cover_url: string(value, "cover"),
        description: string(value, "intro"),
        rating: number(value, "newRating"),
        rating_count: integer(value, "newRatingCount"),
        source_url: string(value, "deepLink"),
        raw_json: serde_json::to_string(value)?,
    })
}

/// 写入微信读书元数据。`coalesce` 与 `CASE WHEN ...='[]'` 保证精简的同步结果
/// 不会覆盖已有的详细字段，因此重复同步始终只保留一行。
fn upsert_weread_metadata(
    conn: &Connection,
    book_id: &str,
    value: &Value,
    fetched_at: i64,
) -> Result<(), AppError> {
    let fields = weread_metadata_fields(value)?;
    conn.execute(
        "INSERT INTO book_metadata_sources(
            book_id,source,source_id,source_url,isbn10,isbn13,title,authors_json,
            publisher,published_date,page_count,subjects_json,cover_url,description,
            rating,rating_count,raw_json,fetched_at
         ) VALUES(?1,'weread',?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16)
         ON CONFLICT(book_id,source) DO UPDATE SET
            source_id=excluded.source_id,
            source_url=coalesce(excluded.source_url,book_metadata_sources.source_url),
            isbn10=coalesce(excluded.isbn10,book_metadata_sources.isbn10),
            isbn13=coalesce(excluded.isbn13,book_metadata_sources.isbn13),
            title=coalesce(excluded.title,book_metadata_sources.title),
            authors_json=CASE WHEN excluded.authors_json='[]' THEN book_metadata_sources.authors_json ELSE excluded.authors_json END,
            publisher=coalesce(excluded.publisher,book_metadata_sources.publisher),
            published_date=coalesce(excluded.published_date,book_metadata_sources.published_date),
            page_count=coalesce(excluded.page_count,book_metadata_sources.page_count),
            subjects_json=CASE WHEN excluded.subjects_json='[]' THEN book_metadata_sources.subjects_json ELSE excluded.subjects_json END,
            cover_url=coalesce(excluded.cover_url,book_metadata_sources.cover_url),
            description=coalesce(excluded.description,book_metadata_sources.description),
            rating=coalesce(excluded.rating,book_metadata_sources.rating),
            rating_count=coalesce(excluded.rating_count,book_metadata_sources.rating_count),
            raw_json=excluded.raw_json,
            fetched_at=excluded.fetched_at",
        params![
            book_id,
            fields.source_url,
            fields.isbn10,
            fields.isbn13,
            fields.title,
            serde_json::to_string(&fields.authors)?,
            fields.publisher,
            fields.published_date,
            fields.page_count,
            serde_json::to_string(&fields.subjects)?,
            fields.cover_url,
            fields.description,
            fields.rating,
            fields.rating_count,
            fields.raw_json,
            fetched_at,
        ],
    )?;
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
fn number(value: &Value, key: &str) -> Option<f64> {
    value.get(key).and_then(Value::as_f64)
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
    use super::{
        chapter_map, integer, json_text, persist, string, upsert_weread_metadata, weread_metadata_fields,
    };
    use rusqlite::{params, Connection};
    use serde_json::json;

    /// 内存库：跑同一份建表脚本，既覆盖外键约束也避免写出真实数据文件。
    fn test_connection() -> Connection {
        let connection = Connection::open_in_memory().expect("应能创建内存数据库");
        connection
            .execute_batch(include_str!("../database/schema.sql"))
            .expect("建表脚本应能在内存库执行");
        connection
    }

    fn insert_book(connection: &Connection, book_id: &str, title: &str, deleted: i64) {
        connection
            .execute(
                "INSERT INTO books(book_id,title,author,cover,category,deep_link,created_at,synced_at,is_deleted,last_seen_sync_id)
                 VALUES(?1,?2,'尤瓦尔·赫拉利','https://cover/1.jpg','历史/文化','wxlink://book',1000,1000,?3,'sync-1')",
                params![book_id, title, deleted],
            )
            .expect("书籍应能插入");
    }

    fn weread_book_json() -> serde_json::Value {
        json!({
            "bookId": "bk-1",
            "title": "人类简史",
            "author": "尤瓦尔·赫拉利",
            "category": "历史 / 文化、随笔",
            "isbn": "9787508647357",
            "publisher": "中信出版社",
            "publishTime": "2014-11",
            "pageCount": 440,
            "cover": "https://cover/1.jpg",
            "intro": "人类从哪里来",
            "newRating": 9.1,
            "newRatingCount": 12345,
            "deepLink": "wxlink://book/bk-1"
        })
    }

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

    #[test]
    fn extracts_weread_fields_without_touching_the_database() {
        let fields = weread_metadata_fields(&weread_book_json()).unwrap();
        assert_eq!(fields.title.as_deref(), Some("人类简史"));
        assert_eq!(fields.authors, vec!["尤瓦尔·赫拉利".to_string()]);
        assert_eq!(
            fields.subjects,
            vec!["历史".to_string(), "文化".to_string(), "随笔".to_string()]
        );
        // 13 位 ISBN 归位到 isbn13，10 位归位到 isbn10
        assert_eq!(fields.isbn13.as_deref(), Some("9787508647357"));
        assert_eq!(fields.isbn10, None);
        assert_eq!(fields.publisher.as_deref(), Some("中信出版社"));
        assert_eq!(fields.published_date.as_deref(), Some("2014-11"));
        assert_eq!(fields.page_count, Some(440));
        assert_eq!(fields.rating, Some(9.1));
        assert_eq!(fields.rating_count, Some(12345));
        assert_eq!(fields.source_url.as_deref(), Some("wxlink://book/bk-1"));

        let ten = weread_metadata_fields(&json!({"isbn":"7508647357","author":"  "})).unwrap();
        assert_eq!(ten.isbn10.as_deref(), Some("7508647357"));
        assert!(ten.authors.is_empty());
        assert!(ten.subjects.is_empty());
    }

    #[test]
    fn first_sync_writes_one_weread_metadata_row() {
        let connection = test_connection();
        insert_book(&connection, "bk-1", "人类简史", 0);
        upsert_weread_metadata(&connection, "bk-1", &weread_book_json(), 1_700_000_000).unwrap();

        let total: i64 = connection
            .query_row(
                "SELECT count(*) FROM book_metadata_sources WHERE source='weread'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(total, 1);

        let (title, authors_json, subjects_json, cover_url, page_count) = connection
            .query_row(
                "SELECT title,authors_json,subjects_json,cover_url,page_count FROM book_metadata_sources WHERE book_id='bk-1' AND source='weread'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(title, "人类简史");
        assert_eq!(cover_url, "https://cover/1.jpg");
        assert_eq!(page_count, 440);
        // authors_json / subjects_json 必须是能解析成数组的 JSON 文本，而不是裸字符串
        let authors: Vec<String> = serde_json::from_str(&authors_json).unwrap();
        assert_eq!(authors, vec!["尤瓦尔·赫拉利".to_string()]);
        let subjects: Vec<String> = serde_json::from_str(&subjects_json).unwrap();
        assert_eq!(
            subjects,
            vec!["历史".to_string(), "文化".to_string(), "随笔".to_string()]
        );
    }

    #[test]
    fn repeated_sync_does_not_duplicate_metadata_rows() {
        let connection = test_connection();
        insert_book(&connection, "bk-1", "人类简史", 0);
        upsert_weread_metadata(&connection, "bk-1", &weread_book_json(), 1_700_000_000).unwrap();
        upsert_weread_metadata(&connection, "bk-1", &weread_book_json(), 1_700_000_500).unwrap();

        let total: i64 = connection
            .query_row(
                "SELECT count(*) FROM book_metadata_sources WHERE book_id='bk-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(total, 1);
    }

    #[test]
    fn slim_shelf_data_keeps_existing_detailed_fields() {
        let connection = test_connection();
        insert_book(&connection, "bk-1", "人类简史", 0);
        upsert_weread_metadata(&connection, "bk-1", &weread_book_json(), 1_000).unwrap();
        // 第二次同步只带回书名和作者
        upsert_weread_metadata(
            &connection,
            "bk-1",
            &json!({"title": "人类简史", "author": "尤瓦尔·赫拉利"}),
            2_000,
        )
        .unwrap();

        let (publisher, page_count, rating, fetched_at) = connection
            .query_row(
                "SELECT publisher,page_count,rating,fetched_at FROM book_metadata_sources WHERE book_id='bk-1' AND source='weread'",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, f64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(publisher, "中信出版社");
        assert_eq!(page_count, 440);
        assert_eq!(rating, 9.1);
        // 抓取时间仍然要推进，界面才能看出数据是新的
        assert_eq!(fetched_at, 2_000);
    }

    #[test]
    fn legacy_database_backfill_is_idempotent() {
        let connection = test_connection();
        insert_book(&connection, "bk-1", "人类简史", 0);
        insert_book(&connection, "bk-2", "未来简史", 0);
        insert_book(&connection, "bk-3", "已删除的书", 1);
        let active: i64 = connection
            .query_row("SELECT count(*) FROM books WHERE is_deleted=0", [], |row| {
                row.get(0)
            })
            .unwrap();

        // 老库启动时会重跑建表脚本，其中的回填语句应为首次补齐
        connection
            .execute_batch(include_str!("../database/schema.sql"))
            .unwrap();
        let after_first: i64 = connection
            .query_row("SELECT count(*) FROM book_metadata_sources", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(after_first, active);

        // 再次执行不应产生新行
        connection
            .execute_batch(include_str!("../database/schema.sql"))
            .unwrap();
        let after_second: i64 = connection
            .query_row("SELECT count(*) FROM book_metadata_sources", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(after_second, active);

        let (title, authors_json) = connection
            .query_row(
                "SELECT title,authors_json FROM book_metadata_sources WHERE book_id='bk-1'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();
        assert_eq!(title, "人类简史");
        let authors: Vec<String> = serde_json::from_str(&authors_json).unwrap();
        assert_eq!(authors, vec!["尤瓦尔·赫拉利".to_string()]);
    }

    #[test]
    fn douban_metadata_survives_weread_sync() {
        let connection = test_connection();
        insert_book(&connection, "bk-1", "人类简史", 0);
        connection
            .execute(
                "INSERT INTO book_metadata_sources(book_id,source,source_id,title,authors_json,publisher,raw_json,fetched_at)
                 VALUES('bk-1','douban','25976985','人类简史： unimaginable','[\"尤瓦尔·赫拉利\"]','江苏凤凰文艺出版社','{}',500)",
                [],
            )
            .unwrap();
        upsert_weread_metadata(&connection, "bk-1", &weread_book_json(), 2_000).unwrap();

        let (title, publisher, raw_json, fetched_at): (String, String, String, i64) = connection
            .query_row(
                "SELECT title,publisher,raw_json,fetched_at FROM book_metadata_sources WHERE book_id='bk-1' AND source='douban'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(title, "人类简史： unimaginable");
        assert_eq!(publisher, "江苏凤凰文艺出版社");
        assert_eq!(raw_json, "{}");
        assert_eq!(fetched_at, 500);

        let weread_rows: i64 = connection
            .query_row(
                "SELECT count(*) FROM book_metadata_sources WHERE book_id='bk-1' AND source='weread'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(weread_rows, 1);
    }

    #[test]
    fn 划线被截断的书不会被软删除() {
        // 真实数据丢失缺陷：`/book/bookmarklist` 没有分页参数，划线多的书拿不全；
        // 而 finalize 无条件执行 `UPDATE highlights SET is_deleted=1 WHERE last_seen_sync_id<>session_id`，
        // 导致超出部分的划线在每次同步都被标记删除。
        let dir = tempfile::tempdir().unwrap();
        let db = crate::database::Database::open(dir.path().join("test.db")).unwrap();
        let shelf = crate::weread::models::ShelfResponse { books: vec![], albums: vec![], mp: None };
        let session = "sync-truncated";

        // 预置两条上一轮同步留下的划线，本轮都没有再出现。
        db.connect()
            .unwrap()
            .execute(
                "INSERT INTO books(book_id,title,created_at,synced_at,is_deleted,last_seen_sync_id)
                 VALUES('bk-trunc','截断书',1,1,0,'sync-old')",
                [],
            )
            .unwrap();
        for id in ["h1", "h2"] {
            db.connect()
                .unwrap()
                .execute(
                    "INSERT INTO highlights(bookmark_id,book_id,mark_text,create_time,synced_at,is_deleted,last_seen_sync_id)
                     VALUES(?1,'bk-trunc','旧划线',1,1,0,'sync-old')",
                    params![id],
                )
                .unwrap();
        }

        // 正常情况：所有书都拿全了，旧划线应被软删除。
        let before: i64 = db
            .connect()
            .unwrap()
            .query_row("SELECT count(*) FROM highlights WHERE book_id='bk-trunc'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(before, 2, "预置的两条划线不应凭空消失");
        persist(&db, session, 1, &shelf, &[], true, false, &[]).unwrap();
        let deleted: i64 = db
            .connect()
            .unwrap()
            .query_row("SELECT count(*) FROM highlights WHERE book_id='bk-trunc' AND is_deleted=1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(deleted, 2, "拿全时仍应正常软删除");

        // 截断情况：该书在 truncated_books 里，划线一条都不能被标记删除。
        db.connect()
            .unwrap()
            .execute("UPDATE highlights SET is_deleted=0,last_seen_sync_id='sync-old' WHERE book_id='bk-trunc'", [])
            .unwrap();
        // 且要在同步记录里留下可追溯的原因。
        db.connect()
            .unwrap()
            .execute(
                "INSERT INTO sync_sessions(id,source,started_at,status) VALUES('sync-2','weread',1,'running')",
                [],
            )
            .unwrap();
        persist(&db, "sync-2", 1, &shelf, &[], true, false, &["bk-trunc".to_string()]).unwrap();
        let deleted: i64 = db
            .connect()
            .unwrap()
            .query_row("SELECT count(*) FROM highlights WHERE book_id='bk-trunc' AND is_deleted=1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(deleted, 0, "划线未完整获取时绝不能软删除，否则超页划线会永久丢失");
        let message: Option<String> = db
            .connect()
            .unwrap()
            .query_row("SELECT error_message FROM sync_sessions WHERE id='sync-2'", [], |r| r.get(0))
            .unwrap();
        let message = message.expect("截断时应记录原因");
        assert!(message.contains("bk-trunc"), "{message}");
    }

    #[test]
    fn 划线截断的书名含引号不会破坏_sql() {
        // book_id 来自接口，拼进 IN 列表前必须转义，否则会拼出非法 SQL。
        let dir = tempfile::tempdir().unwrap();
        let db = crate::database::Database::open(dir.path().join("test.db")).unwrap();
        let shelf = crate::weread::models::ShelfResponse { books: vec![], albums: vec![], mp: None };
        persist(&db, "sync-quote", 1, &shelf, &[], true, false, &["bk'; DROP TABLE highlights;--".to_string()]).unwrap();
        let table_exists: i64 = db
            .connect()
            .unwrap()
            .query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name='highlights'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(table_exists, 1, "表必须还在");
    }
}
