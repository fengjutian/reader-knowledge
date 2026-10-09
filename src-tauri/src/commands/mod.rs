use crate::ai::provider::AiProvider;
use crate::http::{limited_json, MAX_API_RESPONSE_BYTES};
use crate::{database::Database, error::AppError, models::*};
use rusqlite::OptionalExtension;
use serde_json::Value;
use std::{collections::{HashMap, HashSet}, future::Future, pin::Pin, sync::Arc, task::Poll, time::{Duration, SystemTime, UNIX_EPOCH}};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

pub mod glossary_import;

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
pub fn get_database_overview(db: State<'_, Database>) -> Result<DatabaseOverview, AppError> {
    let c = db.connect()?;
    let table_specs = [("books", "书籍"), ("chapters", "章节"), ("highlights", "划线"), ("thoughts", "想法"), ("weread_raw", "原始数据"), ("sync_sessions", "同步记录"), ("note_embeddings", "向量索引"), ("relation_analysis_cache", "关系分析缓存"), ("book_metadata_sources", "书籍元数据"), ("book_metadata_extras", "元数据扩展"), ("glossary_terms", "名词库")];
    let mut tables = Vec::with_capacity(table_specs.len());
    for (name, label) in table_specs {
        let rows = c.query_row(&format!("SELECT count(*) FROM {name}"), [], |row| row.get(0))?;
        tables.push(DatabaseTableStat { name: name.into(), label: label.into(), rows });
    }
    let mut query = c.prepare("SELECT CASE WHEN trim(coalesce(category,''))='' THEN '未分类' ELSE category END, count(*) FROM books WHERE is_deleted=0 GROUP BY 1 ORDER BY 2 DESC LIMIT 8")?;
    let categories = query.query_map([], |row| Ok(DatabaseCategoryStat { label: row.get(0)?, count: row.get(1)? }))?.collect::<Result<Vec<_>, _>>()?;
    // 表名与统计口径都写死在代码里，全部走参数绑定，外部无法传入任意 SQL。
    let metadata = MetadataOverview {
        weread: c.query_row("SELECT count(*) FROM book_metadata_sources WHERE source='weread'", [], |row| row.get(0))?,
        douban: c.query_row("SELECT count(*) FROM book_metadata_sources WHERE source='douban'", [], |row| row.get(0))?,
        missing: c.query_row("SELECT count(*) FROM books b WHERE b.is_deleted=0 AND NOT EXISTS(SELECT 1 FROM book_metadata_sources m WHERE m.book_id=b.book_id)", [], |row| row.get(0))?,
        vectors: c.query_row("SELECT count(*) FROM note_embeddings", [], |row| row.get(0))?,
        relation_cache: c.query_row("SELECT count(*) FROM relation_analysis_cache", [], |row| row.get(0))?,
    };
    let last_synced_at = c.query_row("SELECT datetime(last_synced_at,'unixepoch','localtime') FROM sync_state WHERE source='weread'", [], |row| row.get(0)).optional()?;
    Ok(DatabaseOverview { size_bytes: db.size_bytes(), tables, categories, metadata, last_synced_at })
}

/// 数据表浏览白名单：只有命中这里的数据表才会拼出 SQL，
/// 任何未登记的表名（含注入尝试）都拿不到语句，是唯一的入口防线。
pub(crate) fn database_row_query(table: &str) -> Option<(&'static str, &'static str)> {
    match table {
        "books" => Some((
            "SELECT count(*) FROM books WHERE is_deleted=0 AND (title LIKE ?1 OR coalesce(author,'') LIKE ?1 OR book_id LIKE ?1)",
            "SELECT book_id,title,coalesce(author,''),coalesce(category,''),coalesce(datetime(read_update_time,'unixepoch','localtime'),'') FROM books WHERE is_deleted=0 AND (title LIKE ?1 OR coalesce(author,'') LIKE ?1 OR book_id LIKE ?1) ORDER BY read_update_time DESC LIMIT ?2 OFFSET ?3",
        )),
        "highlights" => Some((
            "SELECT count(*) FROM highlights h LEFT JOIN books b ON b.book_id=h.book_id WHERE h.is_deleted=0 AND (h.mark_text LIKE ?1 OR coalesce(h.chapter_title,'') LIKE ?1 OR coalesce(b.title,'') LIKE ?1)",
            "SELECT h.bookmark_id,h.mark_text,coalesce(b.title,''),coalesce(h.chapter_title,''),coalesce(datetime(h.create_time,'unixepoch','localtime'),'') FROM highlights h LEFT JOIN books b ON b.book_id=h.book_id WHERE h.is_deleted=0 AND (h.mark_text LIKE ?1 OR coalesce(h.chapter_title,'') LIKE ?1 OR coalesce(b.title,'') LIKE ?1) ORDER BY h.create_time DESC LIMIT ?2 OFFSET ?3",
        )),
        "thoughts" => Some((
            "SELECT count(*) FROM thoughts t LEFT JOIN books b ON b.book_id=t.book_id WHERE t.is_deleted=0 AND (t.content LIKE ?1 OR coalesce(t.chapter_name,'') LIKE ?1 OR coalesce(b.title,'') LIKE ?1)",
            "SELECT t.review_id,t.content,coalesce(b.title,''),coalesce(t.chapter_name,''),coalesce(datetime(t.create_time,'unixepoch','localtime'),'') FROM thoughts t LEFT JOIN books b ON b.book_id=t.book_id WHERE t.is_deleted=0 AND (t.content LIKE ?1 OR coalesce(t.chapter_name,'') LIKE ?1 OR coalesce(b.title,'') LIKE ?1) ORDER BY t.create_time DESC LIMIT ?2 OFFSET ?3",
        )),
        "sync_sessions" => Some((
            "SELECT count(*) FROM sync_sessions WHERE source LIKE ?1 OR status LIKE ?1 OR coalesce(error_message,'') LIKE ?1",
            "SELECT id,status,source,printf('书籍 %d · 划线 %d · 想法 %d',books_fetched,highlights_fetched,thoughts_fetched),coalesce(datetime(started_at,'unixepoch','localtime'),'') FROM sync_sessions WHERE source LIKE ?1 OR status LIKE ?1 OR coalesce(error_message,'') LIKE ?1 ORDER BY started_at DESC LIMIT ?2 OFFSET ?3",
        )),
        // 元数据来源：书名从 books 联表取得，出版社为空时用作者数组兜底当详情展示。
        "book_metadata_sources" => Some((
            "SELECT count(*) FROM book_metadata_sources m LEFT JOIN books b ON b.book_id=m.book_id WHERE coalesce(b.title,'') LIKE ?1 OR m.source LIKE ?1 OR coalesce(m.publisher,'') LIKE ?1 OR coalesce(m.authors_json,'') LIKE ?1 OR m.book_id LIKE ?1",
            "SELECT m.book_id,coalesce(b.title,m.title,m.book_id),m.source,coalesce(nullif(m.publisher,''),m.authors_json,''),coalesce(datetime(m.fetched_at,'unixepoch','localtime'),'') FROM book_metadata_sources m LEFT JOIN books b ON b.book_id=m.book_id WHERE coalesce(b.title,'') LIKE ?1 OR m.source LIKE ?1 OR coalesce(m.publisher,'') LIKE ?1 OR coalesce(m.authors_json,'') LIKE ?1 OR m.book_id LIKE ?1 ORDER BY m.fetched_at DESC LIMIT ?2 OFFSET ?3",
        )),
        "glossary_terms" => Some((
            "SELECT count(*) FROM glossary_terms WHERE term LIKE ?1 OR canonical_name LIKE ?1 OR definition LIKE ?1",
            "SELECT cast(id AS TEXT),term,canonical_name,substr(definition,1,200),coalesce(datetime(updated_at,'unixepoch','localtime'),'') FROM glossary_terms WHERE term LIKE ?1 OR canonical_name LIKE ?1 OR definition LIKE ?1 ORDER BY updated_at DESC LIMIT ?2 OFFSET ?3",
        )),
        _ => None,
    }
}

#[tauri::command]
pub fn list_database_rows(db: State<'_, Database>, table: String, query: String, limit: i64, offset: i64) -> Result<DatabaseRows, AppError> {
    let c = db.connect()?;
    let pattern = format!("%{}%", query.trim());
    let limit = limit.clamp(1, 100);
    let offset = offset.max(0);
    let Some((count_sql, rows_sql)) = database_row_query(table.as_str()) else {
        return Err(AppError::Message("不支持浏览该数据表".into()));
    };
    let total = c.query_row(count_sql, [&pattern], |row| row.get(0))?;
    let mut statement = c.prepare(rows_sql)?;
    let rows = statement.query_map(rusqlite::params![pattern, limit, offset], |row| Ok(DatabaseRow { id: row.get(0)?, primary: row.get(1)?, secondary: row.get(2)?, detail: row.get(3)?, created_at: row.get(4)? }))?.collect::<Result<Vec<_>, _>>()?;
    Ok(DatabaseRows { total, rows })
}

#[tauri::command]
pub async fn get_reading_stats(mode: String) -> Result<serde_json::Value, AppError> {
    if !matches!(mode.as_str(), "weekly" | "monthly" | "annually" | "overall") {
        return Err(AppError::Message("不支持的阅读统计周期".into()));
    }
    let secret = keyring::Entry::new("ReadFlow", "weread")?
        .get_password()
        .map_err(|error| match error {
            keyring::Error::NoEntry => AppError::Message("请先在设置中填写微信读书 API Key".into()),
            other => AppError::Credential(other),
        })?;
    let client = crate::weread::client::WeReadClient::new(secret)?;
    let mut params = serde_json::Map::new();
    params.insert("mode".into(), serde_json::Value::String(mode));
    client.call("/readdata/detail", params).await
}

#[tauri::command]
pub async fn get_book_recommendations(count: i64, max_idx: i64) -> Result<serde_json::Value, AppError> {
    let secret = keyring::Entry::new("ReadFlow", "weread")?
        .get_password()
        .map_err(|error| match error {
            keyring::Error::NoEntry => AppError::Message("请先在设置中填写微信读书 API Key".into()),
            other => AppError::Credential(other),
        })?;
    let client = crate::weread::client::WeReadClient::new(secret)?;
    let mut params = serde_json::Map::new();
    params.insert("count".into(), serde_json::Value::from(count.clamp(1, 24)));
    params.insert("maxIdx".into(), serde_json::Value::from(max_idx.max(0)));
    let response = client.call("/book/recommend", params).await?;
    Ok(normalize_recommendations(response))
}

fn normalize_recommendations(mut response: Value) -> Value {
    let Some(books) = response.get_mut("books").and_then(Value::as_array_mut) else {
        return response;
    };
    const BOOK_FIELDS: &[&str] = &[
        "bookId", "deepLink", "title", "author", "cover", "intro", "category",
        "reason", "readingCount", "searchIdx", "newRating", "newRatingCount",
        "newRatingDetail",
    ];
    const NUMBER_FIELDS: &[&str] = &[
        "readingCount", "searchIdx", "newRating", "newRatingCount",
    ];

    for book in books {
        let nested = book
            .get("bookInfo")
            .or_else(|| book.pointer("/book/bookInfo"))
            .and_then(Value::as_object)
            .cloned();
        let Some(target) = book.as_object_mut() else { continue };

        if let Some(source) = nested {
            for field in BOOK_FIELDS {
                let missing = target.get(*field).is_none_or(Value::is_null);
                if missing {
                    if let Some(value) = source.get(*field) {
                        target.insert((*field).to_owned(), value.clone());
                    }
                }
            }
        }
        for field in NUMBER_FIELDS {
            if let Some(value) = target.get_mut(*field) {
                if let Some(parsed) = value.as_str().and_then(|text| text.trim().parse::<f64>().ok()) {
                    *value = Value::from(parsed);
                }
            }
        }
    }
    response
}

#[tauri::command]
pub async fn get_book_recommendation_detail(book_id: String, title: String) -> Result<Value, AppError> {
    let secret = keyring::Entry::new("ReadFlow", "weread")?
        .get_password()
        .map_err(|error| match error {
            keyring::Error::NoEntry => AppError::Message("请先在设置中填写微信读书 API Key".into()),
            other => AppError::Credential(other),
        })?;
    let client = crate::weread::client::WeReadClient::new(secret)?;
    let mut info_params = serde_json::Map::new();
    info_params.insert("bookId".into(), Value::String(book_id.clone()));
    let mut detail: Value = client.call("/book/info", info_params).await?;

    let mut search_params = serde_json::Map::new();
    search_params.insert("keyword".into(), Value::String(title));
    search_params.insert("scope".into(), Value::from(10));
    let search: Value = client.call("/store/search", search_params).await?;
    if let Some(found) = search
        .get("results").and_then(Value::as_array).into_iter().flatten()
        .filter_map(|group| group.get("books").and_then(Value::as_array)).flatten()
        .find(|item| item.pointer("/bookInfo/bookId").or_else(|| item.get("bookId")).is_some_and(|value| {
            value.as_str().map_or_else(|| value.to_string() == book_id, |value| value == book_id)
        }))
    {
        let nested = found.get("bookInfo").and_then(Value::as_object);
        if let Some(target) = detail.as_object_mut() {
            if let Some(source) = nested {
                for (key, value) in source { target.entry(key.clone()).or_insert_with(|| value.clone()); }
            }
            for field in ["readingCount", "newRating", "newRatingCount", "newRatingDetail"] {
                if let Some(value) = found.get(field) { target.insert(field.into(), value.clone()); }
            }
        }
    }
    let wrapper = serde_json::json!({ "books": [detail] });
    Ok(normalize_recommendations(wrapper)["books"][0].clone())
}

#[tauri::command]
pub fn list_books(db: State<'_, Database>) -> Result<Vec<Book>, AppError> {
    let c = db.connect()?;
    let mut q=c.prepare("SELECT b.book_id,b.title,coalesce(b.author,''),coalesce(b.category,''),coalesce(b.cover,''),coalesce(h.total,0),coalesce(t.total,0),CASE b.finish_reading WHEN 1 THEN 100 ELSE 0 END,coalesce(datetime(b.read_update_time,'unixepoch','localtime'),''),CASE WHEN b.finish_reading=1 THEN 'finished' WHEN coalesce(b.read_update_time,0)>0 OR coalesce(h.total,0)>0 OR coalesce(t.total,0)>0 THEN 'reading' ELSE 'unread' END FROM books b LEFT JOIN (SELECT book_id,count(*) total FROM highlights WHERE is_deleted=0 GROUP BY book_id) h ON h.book_id=b.book_id LEFT JOIN (SELECT book_id,count(*) total FROM thoughts WHERE is_deleted=0 GROUP BY book_id) t ON t.book_id=b.book_id WHERE b.is_deleted=0 ORDER BY b.read_update_time DESC")?;
    let books = q
        .query_map([], |r| {
            Ok(Book {
                id: r.get(0)?,
                title: r.get(1)?,
                author: r.get(2)?,
                category: r.get(3)?,
                cover: r.get(4)?,
                highlight_count: r.get(5)?,
                thought_count: r.get(6)?,
                progress: r.get(7)?,
                updated_at: r.get(8)?,
                reading_status: r.get(9)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(books)
}

#[tauri::command]
pub fn list_books_page(
    db: State<'_, Database>,
    query: String,
    limit: i64,
    offset: i64,
    category: String,
    reading_status: String,
    with_highlights: bool,
    with_thoughts: bool,
    sort_by: String,
) -> Result<BookPage, AppError> {
    let c = db.connect()?;
    let pattern = format!("%{}%", query.trim());
    let note_filter_disabled = !with_highlights && !with_thoughts;
    let filters = "b.is_deleted=0
        AND (b.title LIKE ?1 OR coalesce(b.author,'') LIKE ?1)
        AND (?2='all' OR coalesce(b.category,'')=?2)
        AND (?3='all' OR CASE WHEN b.finish_reading=1 THEN 'finished' WHEN coalesce(b.read_update_time,0)>0 OR coalesce(h.total,0)>0 OR coalesce(t.total,0)>0 THEN 'reading' ELSE 'unread' END=?3)
        AND (?4 OR (?5 AND coalesce(h.total,0)>0) OR (?6 AND coalesce(t.total,0)>0))";
    let total = c.query_row(
        &format!("SELECT count(*) FROM books b
            LEFT JOIN (SELECT book_id,count(*) total FROM highlights WHERE is_deleted=0 GROUP BY book_id) h ON h.book_id=b.book_id
            LEFT JOIN (SELECT book_id,count(*) total FROM thoughts WHERE is_deleted=0 GROUP BY book_id) t ON t.book_id=b.book_id
            WHERE {filters}"),
        rusqlite::params![pattern, category, reading_status, note_filter_disabled, with_highlights, with_thoughts],
        |row| row.get(0),
    )?;
    let mut statement = c.prepare(
        &format!("SELECT b.book_id,b.title,coalesce(b.author,''),coalesce(b.category,''),coalesce(b.cover,''),coalesce(h.total,0),coalesce(t.total,0),CASE b.finish_reading WHEN 1 THEN 100 ELSE 0 END,coalesce(datetime(b.read_update_time,'unixepoch','localtime'),''),CASE WHEN b.finish_reading=1 THEN 'finished' WHEN coalesce(b.read_update_time,0)>0 OR coalesce(h.total,0)>0 OR coalesce(t.total,0)>0 THEN 'reading' ELSE 'unread' END
         FROM books b
         LEFT JOIN (SELECT book_id,count(*) total FROM highlights WHERE is_deleted=0 GROUP BY book_id) h ON h.book_id=b.book_id
         LEFT JOIN (SELECT book_id,count(*) total FROM thoughts WHERE is_deleted=0 GROUP BY book_id) t ON t.book_id=b.book_id
         WHERE {filters}
         ORDER BY CASE WHEN ?7='highlights' THEN coalesce(h.total,0) END DESC,
                  CASE WHEN ?7='thoughts' THEN coalesce(t.total,0) END DESC,
                  CASE WHEN ?7='title' THEN b.title END COLLATE NOCASE ASC,
                  CASE WHEN ?7='recent' THEN coalesce(b.read_update_time,0) END DESC,
                  b.book_id
         LIMIT ?8 OFFSET ?9"),
    )?;
    let books = statement.query_map(rusqlite::params![pattern, category, reading_status, note_filter_disabled, with_highlights, with_thoughts, sort_by, limit.clamp(1, 500), offset.max(0)], |row| {
        Ok(Book {
            id: row.get(0)?, title: row.get(1)?, author: row.get(2)?, category: row.get(3)?, cover: row.get(4)?,
            highlight_count: row.get(5)?, thought_count: row.get(6)?, progress: row.get(7)?, updated_at: row.get(8)?, reading_status: row.get(9)?,
        })
    })?.collect::<Result<Vec<_>, _>>()?;
    let mut category_query = c.prepare("SELECT DISTINCT trim(category) FROM books WHERE is_deleted=0 AND trim(coalesce(category,''))<>'' ORDER BY trim(category) COLLATE NOCASE")?;
    let categories = category_query.query_map([], |row| row.get(0))?.collect::<Result<Vec<String>, _>>()?;
    Ok(BookPage { total, books, categories })
}

#[tauri::command]
pub fn list_book_metadata(db: State<'_, Database>, source: Option<String>) -> Result<Vec<BookMetadataRow>, AppError> {
    crate::metadata::list(&db, source.as_deref().unwrap_or("weread"))
}

#[tauri::command]
pub fn get_book_metadata_details(db: State<'_, Database>, book_id: String) -> Result<Vec<BookMetadataSourceDetail>, AppError> {
    crate::metadata::details(&db,&book_id)
}

#[tauri::command]
pub async fn fetch_book_metadata(db: State<'_, Database>, book_id: String, source: String, force: bool) -> Result<MetadataFetchResult, AppError> {
    crate::metadata::fetch(&db, &book_id, &source, force).await
}

#[tauri::command]
pub async fn fetch_douban_book_metadata(db: State<'_, Database>, book_id: String, url: String) -> Result<MetadataFetchResult, AppError> {
    crate::metadata::fetch_douban_url(&db, &book_id, &url).await
}

/// 单次批量补全的书籍数量上限。
pub(crate) const METADATA_BATCH_LIMIT: usize = 20;
/// 批量补全的最大并发路数，避免把上游接口打挂。
const METADATA_BATCH_CONCURRENCY: usize = 3;
/// 相邻两本之间的节流间隔。
const METADATA_BATCH_INTERVAL: Duration = Duration::from_millis(300);

pub(crate) type MetadataBatchFuture<'a> = Pin<Box<dyn Future<Output = Result<MetadataFetchResult, AppError>> + Send + 'a>>;

/// 逐本补全书籍元数据：有限并发 + 固定节流，单本失败只影响它自己，
/// 返回顺序始终与传入的 `book_ids` 一致。
pub(crate) async fn run_batched<'a, F>(
    book_ids: &[String],
    max_batch: usize,
    source: &'a str,
    interval: Duration,
    mut op: F,
) -> Result<Vec<MetadataFetchResult>, AppError>
where
    F: FnMut(String) -> MetadataBatchFuture<'a>,
{
    if book_ids.len() > max_batch {
        return Err(AppError::Message(format!("单次最多补全 {max_batch} 本书")));
    }
    let total = book_ids.len();
    let mut slots: Vec<Option<MetadataFetchResult>> = (0..total).map(|_| None).collect();
    let mut in_flight: Vec<(usize, MetadataBatchFuture<'a>)> = Vec::new();
    let mut next = 0usize;
    while next < total || !in_flight.is_empty() {
        while next < total && in_flight.len() < METADATA_BATCH_CONCURRENCY {
            if next > 0 && !interval.is_zero() {
                tokio::time::sleep(interval).await;
            }
            in_flight.push((next, op(book_ids[next].clone())));
            next += 1;
        }
        if in_flight.is_empty() {
            break;
        }
        // 就地驱动并发中的请求，谁先完成就先收谁，不额外引入 runtime 或新依赖。
        let (position, outcome) = std::future::poll_fn(|context| {
            for (position, (_, task)) in in_flight.iter_mut().enumerate() {
                if let Poll::Ready(outcome) = task.as_mut().poll(context) {
                    return Poll::Ready((position, outcome));
                }
            }
            Poll::Pending
        })
        .await;
        let (index, _) = in_flight.remove(position);
        slots[index] = Some(match outcome {
            Ok(result) => result,
            Err(error) => MetadataFetchResult {
                book_id: book_ids[index].clone(),
                source: source.to_owned(),
                status: "failed".into(),
                message: error.to_string(),
            },
        });
    }
    Ok(slots.into_iter().flatten().collect())
}

#[tauri::command]
pub async fn fetch_books_metadata(db: State<'_, Database>, book_ids: Vec<String>, source: String, force: bool) -> Result<Vec<MetadataFetchResult>, AppError> {
    let db = db.inner();
    run_batched(&book_ids, METADATA_BATCH_LIMIT, &source, METADATA_BATCH_INTERVAL, |book_id| -> MetadataBatchFuture<'_> {
        let source = source.clone();
        Box::pin(async move { crate::metadata::fetch(db, &book_id, &source, force).await })
    })
    .await
}

#[tauri::command]
pub fn list_glossary_terms(db:State<'_,Database>,query:Option<String>,source:Option<String>,status:Option<String>)->Result<Vec<GlossaryTerm>,AppError>{
    glossary_terms_for_review(db.inner(), query.as_deref(), source.as_deref(), status.as_deref())
}

/// 名词库列表。排序按需求 7.10：标准名称精确 > 别名精确 > 标题前缀 > 摘要包含。
/// 没有引入 FTS：项目现有 FTS 用 unicode61 分词，对中文整句只会切成一个 token，
/// 摘要全文匹配走 LIKE 反而更准，代价是名词规模变大后需要另加 trigram 索引。
pub(crate) fn glossary_terms_for_review(db:&Database,query:Option<&str>,source:Option<&str>,status:Option<&str>)->Result<Vec<GlossaryTerm>,AppError>{
    let trimmed=query.unwrap_or_default().trim().to_string();
    let pattern=format!("%{}%",trimmed);
    let prefix=format!("{}%",trimmed);
    let normalized=crate::wikipedia::title::normalize_search_key(&trimmed);
    let c=db.connect()?;
    let mut q=c.prepare("SELECT id,term,canonical_name,aliases_json,definition,source,coalesce(source_title,''),coalesce(source_url,''),coalesce(wikipedia_snapshot,''),status,updated_at,coalesce(external_page_id,0),coalesce(source_revision_id,0),coalesce(source_dump_version,''),coalesce(source_updated_at,0),coalesce(source_synced_at,0),coalesce(license_code,''),coalesce(manually_edited,0),coalesce(source_content_hash,''),coalesce(published_batch_id,'') FROM glossary_terms WHERE (term LIKE ?1 OR canonical_name LIKE ?1 OR definition LIKE ?1 OR EXISTS(SELECT 1 FROM glossary_term_aliases a WHERE a.term_id=glossary_terms.id AND a.normalized_alias LIKE ?1)) AND (?2='' OR source=?2) AND (?3='' OR status=?3) ORDER BY CASE WHEN canonical_name=?4 THEN 0 WHEN term=?4 THEN 1 WHEN EXISTS(SELECT 1 FROM glossary_term_aliases a WHERE a.term_id=glossary_terms.id AND a.normalized_alias=?5) THEN 2 WHEN term LIKE ?6 THEN 3 ELSE 4 END, updated_at DESC")?;
    let rows=q.query_map(rusqlite::params![pattern,source.unwrap_or_default(),status.unwrap_or_default(),trimmed,normalized,prefix],|r|Ok(GlossaryTerm{id:r.get(0)?,term:r.get(1)?,canonical_name:r.get(2)?,aliases:serde_json::from_str(&r.get::<_,String>(3)?).unwrap_or_default(),definition:r.get(4)?,source:r.get(5)?,source_title:r.get(6)?,source_url:r.get(7)?,wikipedia_snapshot:r.get(8)?,status:r.get(9)?,updated_at:r.get(10)?,external_page_id:r.get(11)?,source_revision_id:r.get(12)?,source_dump_version:r.get(13)?,source_updated_at:r.get(14)?,source_synced_at:r.get(15)?,license_code:r.get(16)?,manually_edited:r.get::<_,i64>(17)?!=0,source_content_hash:r.get(18)?,published_batch_id:r.get(19)?}))?.collect::<Result<Vec<_>,_>>()?;
    Ok(rows)
}

pub(crate) const GLOSSARY_STATUSES:[&str;5]=["pending","confirmed","ignored","conflict","source_missing"];

/// 单条审核：确认 / 忽略 / 退回待确认。只改审核状态，不碰来源与人工内容。
pub(crate) fn set_glossary_term_status(db:&Database,id:i64,status:&str)->Result<(),AppError>{
    if !GLOSSARY_STATUSES.contains(&status){return Err(AppError::Message(format!("未知的名词状态：{status}")));}
    let now=SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    let changed=db.connect()?.execute("UPDATE glossary_terms SET status=?2,updated_at=?3 WHERE id=?1",rusqlite::params![id,status,now])?;
    if changed==0{return Err(AppError::Message("名词不存在".into()));}
    Ok(())
}

pub(crate) fn bulk_set_glossary_term_status(db:&Database,ids:&[i64],status:&str)->Result<u64,AppError>{
    if !GLOSSARY_STATUSES.contains(&status){return Err(AppError::Message(format!("未知的名词状态：{status}")));}
    if ids.is_empty(){return Ok(0);}
    let now=SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    let list=ids.iter().map(|id|id.to_string()).collect::<Vec<_>>().join(",");
    let changed=db.connect()?.execute(&format!("UPDATE glossary_terms SET status=?1,updated_at=?2 WHERE id IN ({list})"),rusqlite::params![status,now])?;
    Ok(changed as u64)
}

#[tauri::command]
pub fn save_glossary_term(db:State<'_,Database>,term:GlossaryTerm)->Result<(),AppError>{
    save_glossary_term_impl(db.inner(), &term)
}

fn save_glossary_term_impl(db:&Database,term:&GlossaryTerm)->Result<(),AppError>{
    if term.term.trim().is_empty()||term.definition.trim().is_empty(){return Err(AppError::Message("名词和解释不能为空".into()));}
    if !term.status.is_empty()&&!GLOSSARY_STATUSES.contains(&term.status.as_str()){return Err(AppError::Message(format!("未知的名词状态：{}",term.status)));}
    let now=SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    let c=db.connect()?;
    // 维基来源的词条必须保留全部来源字段：人工编辑只改展示内容（需求 5.1 / 7.8.7）。
    // 定位已有行必须优先用 id：用户改了名词文本后，按 term 查会查不到，
    // 于是走 INSERT 分支并带上原 id，直接撞 `UNIQUE constraint failed: glossary_terms.id`。
    let trimmed=term.term.trim();
    let existing:Option<(i64,String,Option<i64>,String,String,i64,String)>=if term.id>0{
        c.query_row("SELECT id,source,external_page_id,definition,canonical_name,coalesce(manually_edited,0),term FROM glossary_terms WHERE id=?1",[term.id],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional()?
    }else{
        c.query_row("SELECT id,source,external_page_id,definition,canonical_name,coalesce(manually_edited,0),term FROM glossary_terms WHERE term=?1",[trimmed],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?))).optional()?
    };
    // 带 id 却查不到行：原词条已被删除，按新建处理，且不能再带 id（否则撞主键）。
    if term.id>0&&existing.is_none(){
        c.execute("INSERT INTO glossary_terms(term,canonical_name,aliases_json,definition,source,source_title,source_url,wikipedia_snapshot,status,updated_at,external_page_id,source_revision_id,license_code,manually_edited,normalized_term) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",rusqlite::params![trimmed,term.canonical_name.trim(),serde_json::to_string(&term.aliases)?,term.definition.trim(),if term.source.is_empty(){"manual".to_string()}else{term.source.clone()},term.source_title.clone(),term.source_url.clone(),term.wikipedia_snapshot.clone(),if term.status.is_empty(){"confirmed".to_string()}else{term.status.clone()},now,term.external_page_id,term.source_revision_id,term.license_code,if term.source=="wikipedia"&&term.external_page_id>0{0}else{1},crate::wikipedia::title::normalize_search_key(trimmed)])?;
        return Ok(());
    }
    let Some((row_id,stored_source,external_page_id,stored_definition,stored_canonical,manually_edited,stored_term))=existing else {
        c.execute("INSERT INTO glossary_terms(term,canonical_name,aliases_json,definition,source,source_title,source_url,wikipedia_snapshot,status,updated_at,external_page_id,source_revision_id,license_code,manually_edited,normalized_term) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15)",rusqlite::params![trimmed,term.canonical_name.trim(),serde_json::to_string(&term.aliases)?,term.definition.trim(),if term.source.is_empty(){"manual".to_string()}else{term.source.clone()},term.source_title.clone(),term.source_url.clone(),term.wikipedia_snapshot.clone(),if term.status.is_empty(){"confirmed".to_string()}else{term.status.clone()},now,term.external_page_id,term.source_revision_id,term.license_code,if term.source=="wikipedia"&&term.external_page_id>0{0}else{1},crate::wikipedia::title::normalize_search_key(trimmed)])?;
        return Ok(());
    };
    let definition=term.definition.trim();
    let canonical=term.canonical_name.trim();
    // 改名：term 与 normalized_term 必须一起更新，否则按新名词检索不到。
    let renamed=stored_term!=trimmed;
    if renamed{
        c.execute("UPDATE glossary_terms SET term=?2,normalized_term=?3 WHERE id=?1",rusqlite::params![row_id,trimmed,crate::wikipedia::title::normalize_search_key(trimmed)])?;
    }
    if stored_source=="wikipedia"{
        // 展示内容被人改过就标记 manually_edited，后续同步只更新来源快照。
        let edited=if manually_edited==1{1}else if definition!=stored_definition||canonical!=stored_canonical{1}else{0};
        c.execute("UPDATE glossary_terms SET canonical_name=?2,definition=?3,status=coalesce(nullif(?4,''),status),manually_edited=?5,updated_at=?6 WHERE id=?1",rusqlite::params![row_id,canonical,definition,term.status,edited,now])?;
        sync_human_aliases(&c,row_id,&term.aliases,now)?;
    } else {
        c.execute("UPDATE glossary_terms SET canonical_name=?2,definition=?3,aliases_json=?4,source=?5,source_title=?6,source_url=?7,wikipedia_snapshot=?8,status=coalesce(nullif(?9,''),status),manually_edited=1,updated_at=?10 WHERE id=?1",rusqlite::params![row_id,canonical,definition,serde_json::to_string(&term.aliases)?,if term.source.is_empty(){"manual".to_string()}else{term.source.clone()},term.source_title.clone(),term.source_url.clone(),term.wikipedia_snapshot.clone(),term.status,now])?;
    }
    let _=external_page_id;
    Ok(())
}

/// 人工在维基词条上加的别名进 alias 表（alias_type='manual'），
/// 并把 alias 表里的重定向别名并回 aliases_json，保证 AI 抽取还能看到别名。
fn sync_human_aliases(c:&rusqlite::Connection,term_id:i64,aliases:&[String],now:i64)->Result<(),AppError>{
    for alias in aliases{
        let alias=alias.trim();
        if alias.is_empty(){continue}
        let normalized=crate::wikipedia::title::normalize_search_key(alias);
        if normalized.is_empty(){continue}
        c.execute("INSERT INTO glossary_term_aliases(term_id,alias,normalized_alias,alias_type,source,created_at,updated_at) VALUES(?1,?2,?3,'manual','manual',?4,?4) ON CONFLICT(term_id,normalized_alias,alias_type) DO NOTHING",rusqlite::params![term_id,alias,normalized,now])?;
    }
    let mut statement=c.prepare("SELECT alias FROM glossary_term_aliases WHERE term_id=?1 ORDER BY alias")?;
    let merged:Vec<String>=statement.query_map([term_id],|r|r.get::<_,String>(0))?.collect::<Result<Vec<_>,_>>()?;
    drop(statement);
    c.execute("UPDATE glossary_terms SET aliases_json=?2 WHERE id=?1",rusqlite::params![term_id,serde_json::to_string(&merged)?])?;
    Ok(())
}

#[tauri::command]
pub fn delete_glossary_term(db:State<'_,Database>,id:i64)->Result<(),AppError>{db.connect()?.execute("DELETE FROM glossary_terms WHERE id=?1",[id])?;Ok(())}

#[tauri::command]
pub async fn search_wikipedia(term:String)->Result<Vec<WikipediaCandidate>,AppError>{
    let client=reqwest::Client::builder().user_agent("wereader/0.1 personal knowledge app").timeout(std::time::Duration::from_secs(12)).build()?;
    let response=client.get("https://zh.wikipedia.org/w/rest.php/v1/search/page").query(&[("q",term.as_str()),("limit","5")]).send().await?.error_for_status()?;
    let value:Value=limited_json(response,MAX_API_RESPONSE_BYTES,"维基百科").await?;
    let pages=value.get("pages").and_then(Value::as_array).cloned().unwrap_or_default();
    let mut candidates=pages.into_iter().filter_map(|page|{let title=page.get("title")?.as_str()?.to_owned();let description=page.get("description").and_then(Value::as_str).unwrap_or("").to_owned();let excerpt=page.get("excerpt").and_then(Value::as_str).unwrap_or("").replace("<span class=\"searchmatch\">","").replace("</span>","");let url=format!("https://zh.wikipedia.org/wiki/{}",title.replace(' ',"_"));Some(WikipediaCandidate{title,description,excerpt,url})}).collect::<Vec<_>>();
    for candidate in &mut candidates {
        let detail=client.get("https://zh.wikipedia.org/w/api.php").query(&[("action","query"),("format","json"),("formatversion","2"),("prop","extracts"),("explaintext","1"),("exsectionformat","plain"),("redirects","1"),("titles",candidate.title.as_str())]).send().await;
        let Ok(response)=detail else{continue};let Ok(response)=response.error_for_status() else{continue};let Ok(value)=limited_json::<Value>(response,MAX_API_RESPONSE_BYTES,"维基百科").await else{continue};
        if let Some(page)=value.pointer("/query/pages/0") {
            if let Some(title)=page.get("title").and_then(Value::as_str){candidate.title=title.to_owned();candidate.url=format!("https://zh.wikipedia.org/wiki/{}",title.replace(' ',"_"));}
            if let Some(extract)=page.get("extract").and_then(Value::as_str).filter(|value|!value.trim().is_empty()){candidate.excerpt=extract.chars().take(20_000).collect();}
        }
    }
    Ok(candidates)
}

#[tauri::command]
pub fn get_book(db: State<'_, Database>, book_id: String) -> Result<BookDetail, AppError> {
    let c = db.connect()?;
    c.query_row(
        "SELECT b.book_id,b.title,coalesce(b.author,''),coalesce(b.cover,''),
         (SELECT count(*) FROM highlights h WHERE h.book_id=b.book_id AND h.is_deleted=0),
         (SELECT count(*) FROM thoughts t WHERE t.book_id=b.book_id AND t.is_deleted=0),
         CASE b.finish_reading WHEN 1 THEN 100 ELSE 0 END,
         coalesce(datetime(b.read_update_time,'unixepoch','localtime'),''),coalesce(b.category,''),b.deep_link,b.finish_reading=1,
         CASE WHEN b.finish_reading=1 THEN 'finished' WHEN coalesce(b.read_update_time,0)>0 OR EXISTS(SELECT 1 FROM highlights h WHERE h.book_id=b.book_id AND h.is_deleted=0) OR EXISTS(SELECT 1 FROM thoughts t WHERE t.book_id=b.book_id AND t.is_deleted=0) THEN 'reading' ELSE 'unread' END
         FROM books b WHERE b.book_id=?1 AND b.is_deleted=0",
        [book_id],
        |r| Ok(BookDetail { book: Book { id:r.get(0)?,title:r.get(1)?,author:r.get(2)?,category:r.get(8)?,cover:r.get(3)?,highlight_count:r.get(4)?,thought_count:r.get(5)?,progress:r.get(6)?,updated_at:r.get(7)?,reading_status:r.get(11)? }, deep_link:r.get(9)?,finished:r.get(10)? }),
    ).map_err(AppError::from)
}

#[tauri::command]
pub fn list_book_notes(db: State<'_, Database>, book_id: String) -> Result<Vec<Note>, AppError> {
    let c = db.connect()?;
    let mut query = c.prepare(
        "SELECT h.bookmark_id,'highlight',h.book_id,b.title,coalesce(h.chapter_title,''),h.mark_text,coalesce(datetime(h.create_time,'unixepoch','localtime'),'')
         FROM highlights h JOIN books b ON b.book_id=h.book_id WHERE h.book_id=?1 AND h.is_deleted=0
         UNION ALL
         SELECT t.review_id,'thought',t.book_id,b.title,coalesce(t.chapter_name,''),t.content,coalesce(datetime(t.create_time,'unixepoch','localtime'),'')
         FROM thoughts t JOIN books b ON b.book_id=t.book_id WHERE t.book_id=?1 AND t.is_deleted=0
         ORDER BY 7 DESC",
    )?;
    let notes = query
        .query_map([book_id], |r| {
            Ok(Note {
                id: r.get(0)?,
                note_type: r.get(1)?,
                book_id: r.get(2)?,
                book_title: r.get(3)?,
                chapter: r.get(4)?,
                content: r.get(5)?,
                created_at: r.get(6)?,
            })
        })?
        .collect::<Result<_, _>>()?;
    Ok(notes)
}

#[tauri::command]
pub fn open_book(app: AppHandle, db: State<'_, Database>, book_id: String) -> Result<(), AppError> {
    let c = db.connect()?;
    let link: Option<String> = c
        .query_row(
            "SELECT deep_link FROM books WHERE book_id=?1 AND is_deleted=0",
            [&book_id],
            |row| row.get(0),
        )
        .optional()?;
    let reader_id = weread_reader_id(&book_id);
    let reader_path = if book_id.starts_with("MP_WXS_") {
        "https://weread.qq.com/web/mp/reader/"
    } else {
        "https://weread.qq.com/web/reader/"
    };
    let web_link = reqwest::Url::parse(reader_path)
        .ok()
        .and_then(|mut url| {
            url.path_segments_mut().ok()?.pop_if_empty().push(&reader_id);
            Some(url.to_string())
        });
    let link = web_link
        .or(link)
        .ok_or_else(|| AppError::Message("无法生成这本书的微信读书 Web 链接".into()))?;
    app.opener()
        .open_url(link, None::<String>)
        .map_err(|error| AppError::Message(format!("无法打开微信读书：{error}")))
}

#[tauri::command]
pub fn open_external_url(app: AppHandle, url: String) -> Result<(), AppError> {
    let parsed=reqwest::Url::parse(&url).map_err(|_|AppError::Message("无效的外部链接".into()))?;
    let host=parsed.host_str().unwrap_or_default();
    let allowed=parsed.scheme()=="https"&&matches!(host,"weread.qq.com"|"zh.wikipedia.org"|"book.douban.com"|"www.douban.com"|"openlibrary.org"|"books.google.com"|"books.google.cn"|"books.googleapis.com");
    if !allowed{return Err(AppError::Message("不允许打开该外部域名".into()));}
    app.opener().open_url(parsed.to_string(),None::<String>).map_err(|error|AppError::Message(format!("无法打开外部链接：{error}")))
}

/// Converts the book id returned by the WeRead API into the opaque id used by
/// the web reader. The API id itself is not a valid `/web/reader/` path.
fn weread_reader_id(book_id: &str) -> String {
    let digest = format!("{:x}", md5::compute(book_id.as_bytes()));
    let (kind, transformed): (char, Vec<String>) = if book_id.chars().all(|c| c.is_ascii_digit()) {
        (
            '3',
            book_id
                .as_bytes()
                .chunks(9)
                .map(|chunk| {
                    let value = std::str::from_utf8(chunk)
                        .expect("numeric book id is valid UTF-8")
                        .parse::<u64>()
                        .expect("numeric book id fits into u64");
                    format!("{value:x}")
                })
                .collect(),
        )
    } else {
        (
            '4',
            vec![book_id
                .chars()
                .map(|c| format!("{:x}", c as u32))
                .collect()],
        )
    };

    let mut result = format!("{}{}2{}", &digest[..3], kind, &digest[digest.len() - 2..]);
    for (index, part) in transformed.iter().enumerate() {
        result.push_str(&format!("{:02x}{part}", part.len()));
        if index + 1 < transformed.len() {
            result.push('g');
        }
    }
    if result.len() < 20 {
        result.push_str(&digest[..20 - result.len()]);
    }
    let checksum = format!("{:x}", md5::compute(result.as_bytes()));
    result.push_str(&checksum[..3]);
    result
}

#[tauri::command]
pub fn list_notes(
    db: State<'_, Database>,
    note_type: Option<String>,
) -> Result<Vec<Note>, AppError> {
    let c = db.connect()?;
    let mut query = c.prepare(
        "SELECT h.bookmark_id,'highlight',h.book_id,b.title,coalesce(h.chapter_title,''),h.mark_text,coalesce(datetime(h.create_time,'unixepoch','localtime'),'')
         FROM highlights h JOIN books b ON b.book_id=h.book_id
         WHERE h.is_deleted=0 AND b.is_deleted=0 AND (?1 IS NULL OR ?1='highlight')
         UNION ALL
         SELECT t.review_id,'thought',t.book_id,b.title,coalesce(t.chapter_name,''),t.content,coalesce(datetime(t.create_time,'unixepoch','localtime'),'')
         FROM thoughts t JOIN books b ON b.book_id=t.book_id
         WHERE t.is_deleted=0 AND b.is_deleted=0 AND (?1 IS NULL OR ?1='thought')
         ORDER BY 7 DESC",
    )?;
    let notes = query.query_map([note_type], |r| {
        Ok(Note { id:r.get(0)?,note_type:r.get(1)?,book_id:r.get(2)?,book_title:r.get(3)?,chapter:r.get(4)?,content:r.get(5)?,created_at:r.get(6)? })
    })?.collect::<Result<Vec<_>, _>>()?;
    Ok(notes)
}

#[tauri::command]
pub fn search_notes(
    db: State<'_, Database>,
    query: String,
    note_type: Option<String>,
) -> Result<Vec<SearchResult>, AppError> {
    if note_type
        .as_deref()
        .is_some_and(|value| value != "highlight" && value != "thought")
    {
        return Err(AppError::Message("不支持的笔记类型".into()));
    }
    search_impl(&db, &query, note_type)
}

const GLOBAL_SEARCH_MAX_LIMIT: i64 = 200;
const GLOBAL_SEARCH_MAX_OFFSET: i64 = 5_000;
const GLOBAL_SEARCH_SNIPPET_CHARS: usize = 160;

/// 构造 FTS5 的 MATCH 表达式，返回 `None` 表示没有任何可用词。
///
/// 关键点：
/// - 每个词都用双引号包起来并把内部 `"` 转义成 `""`，用户输入无法逃逸出字符串字面量。
/// - 纯符号输入（`***`、`(((`）规范化后为空，直接返回 `None` 避免生成空表达式。
/// - 空表达式会让 FTS5 报 `fts5: syntax error`，所以必须提前拦掉。
fn fts_match_expression(query: &str) -> Option<String> {
    let expression = search_terms(query)
        .into_iter()
        .filter(|term| !term.is_empty())
        .take(12)
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ");
    (!expression.is_empty()).then_some(expression)
}

/// 规范化搜索类型列表，去重并保留「全部」的语义（返回空列表）。
/// 测试用的默认请求：全部类型、无书籍过滤、首批 30 条。
#[cfg(test)]
fn request_default() -> GlobalSearchRequest {
    GlobalSearchRequest { query: String::new(), types: Vec::new(), book_id: None, limit: Some(30), offset: Some(0) }
}

fn normalized_search_types(types: &[String]) -> Result<Vec<String>, AppError> {
    let mut valid: Vec<String> = Vec::new();
    for kind in types.iter().filter(|value| !value.trim().is_empty()) {
        let kind = kind.trim();
        if !SEARCH_ENTITY_TYPES.contains(&kind) {
            return Err(AppError::Message("不支持的搜索类型".into()));
        }
        if !valid.iter().any(|item| item == kind) {
            valid.push(kind.to_owned());
        }
    }
    Ok(valid)
}

/// 摘要截断在后端完成，避免把超长正文整段送到前端。
fn snippet_of(text: &str) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= GLOBAL_SEARCH_SNIPPET_CHARS {
        return trimmed.to_owned();
    }
    let head: String = trimmed.chars().take(GLOBAL_SEARCH_SNIPPET_CHARS).collect();
    format!("{head}…")
}

/// bm25 返回负数，越小越相关；这里统一翻成「越大越相关」。
/// 读 FTS 分数时统一走这个函数，避免各处自己写符号判断。
fn relevance_from_bm25(bm25: f64) -> f64 {
    (-bm25).max(0.0)
}

/// 书籍结果的可解释加权：完全同名 > 前缀匹配 > 包含匹配。
fn book_title_boost(normalized_query: &str, title: &str, author: &str, category: &str, isbn: &str) -> f64 {
    if normalized_query.is_empty() {
        return 0.0;
    }
    if title == normalized_query {
        return 3.0;
    }
    if title.starts_with(normalized_query) {
        return 2.0;
    }
    let mut boost = 0.0;
    if title.contains(normalized_query) {
        boost += 1.0;
    }
    if author.contains(normalized_query) {
        boost += 0.8;
    }
    if category.contains(normalized_query) {
        boost += 0.4;
    }
    if !isbn.is_empty() && isbn.contains(normalized_query) {
        boost += 2.5;
    }
    boost
}

/// 归一化到 0–1，方便前端按分数排序展示。
fn normalize_scores(results: &mut [GlobalSearchResult]) {
    let peak = results.iter().map(|item| item.score).fold(0.0_f64, f64::max);
    if peak <= 0.0 {
        for item in results.iter_mut() {
            item.score = 0.0;
        }
        return;
    }
    for item in results.iter_mut() {
        item.score = (item.score / peak).clamp(0.0, 1.0);
    }
}

fn recent_book_results(
    db: &Database,
    book_id: Option<&String>,
    limit: i64,
) -> Result<Vec<GlobalSearchResult>, AppError> {
    let c = db.connect()?;
    let mut query = c.prepare(
        "SELECT book_id,title,coalesce(author,''),coalesce(category,''),
         coalesce(datetime(coalesce(update_time,read_update_time,synced_at),'unixepoch','localtime'),'')
         FROM books
         WHERE is_deleted=0 AND (?1 IS NULL OR book_id=?1)
         ORDER BY 5 DESC, book_id LIMIT ?2",
    )?;
    let rows = query
        .query_map(rusqlite::params![book_id, limit], |r| {
            let id: String = r.get(0)?;
            let author: String = r.get(2)?;
            let category: String = r.get(3)?;
            Ok(GlobalSearchResult {
                book_id: id.clone(),
                id,
                entity_type: "book".into(),
                title: r.get(1)?,
                subtitle: join_subtitle(&author, &category),
                snippet: String::new(),
                score: 0.0,
                updated_at: r.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 作者与分类拼成副标题，空的部分不留下多余分隔符。
fn join_subtitle(author: &str, category: &str) -> String {
    [author.trim(), category.trim()]
        .iter()
        .filter(|part| !part.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(" · ")
}

fn recent_note_results(
    db: &Database,
    types: &[String],
    book_id: Option<&String>,
    limit: i64,
) -> Result<Vec<GlobalSearchResult>, AppError> {
    let c = db.connect()?;
    let highlight_filter = types.iter().any(|kind| kind == "highlight");
    let thought_filter = types.iter().any(|kind| kind == "thought");
    let mut results = Vec::new();
    if highlight_filter {
        let mut query = c.prepare(
            "SELECT h.bookmark_id,h.book_id,b.title,coalesce(h.chapter_title,''),h.mark_text,
             coalesce(datetime(h.create_time,'unixepoch','localtime'),'')
             FROM highlights h JOIN books b ON b.book_id=h.book_id
             WHERE h.is_deleted=0 AND b.is_deleted=0 AND (?1 IS NULL OR h.book_id=?1)
             ORDER BY 6 DESC, h.bookmark_id LIMIT ?2",
        )?;
        for row in query.query_map(rusqlite::params![book_id, limit], |r| {
            let content: String = r.get(4)?;
            Ok(GlobalSearchResult {
                id: r.get(0)?,
                entity_type: "highlight".into(),
                book_id: r.get(1)?,
                title: r.get(2)?,
                subtitle: r.get(3)?,
                snippet: snippet_of(&content),
                score: 0.0,
                updated_at: r.get(5)?,
            })
        })? {
            results.push(row?);
        }
    }
    if thought_filter {
        let mut query = c.prepare(
            "SELECT t.review_id,t.book_id,b.title,coalesce(t.chapter_name,''),t.content,
             coalesce(datetime(t.create_time,'unixepoch','localtime'),'')
             FROM thoughts t JOIN books b ON b.book_id=t.book_id
             WHERE t.is_deleted=0 AND b.is_deleted=0 AND (?1 IS NULL OR t.book_id=?1)
             ORDER BY 6 DESC, t.review_id LIMIT ?2",
        )?;
        for row in query.query_map(rusqlite::params![book_id, limit], |r| {
            let content: String = r.get(4)?;
            Ok(GlobalSearchResult {
                id: r.get(0)?,
                entity_type: "thought".into(),
                book_id: r.get(1)?,
                title: r.get(2)?,
                subtitle: r.get(3)?,
                snippet: snippet_of(&content),
                score: 0.0,
                updated_at: r.get(5)?,
            })
        })? {
            results.push(row?);
        }
    }
    Ok(results)
}

/// 书籍打分：FTS 相关度加上可解释的标题 / 作者 / 分类 / ISBN 加权。
fn score_book(candidate: &BookCandidate, normalized_query: &str) -> f64 {
    let boost = book_title_boost(
        normalized_query,
        &normalize_search_text(&candidate.title),
        &normalize_search_text(&candidate.author),
        &normalize_search_text(&candidate.category),
        &candidate.isbn,
    );
    if normalized_query.is_empty() {
        return candidate.relevance;
    }
    let score = candidate.relevance + boost;
    // 兜底扫描进来的候选必须在 Rust 侧再筛一次，否则会把无关书籍也返回。
    (score > 0.0).then_some(score).unwrap_or(0.0)
}

/// 笔记候选行。
struct NoteCandidate {
    id: String,
    entity_type: String,
    book_id: String,
    title: String,
    chapter: String,
    content: String,
    updated_at: String,
    relevance: f64,
}

fn map_note_candidate(r: &rusqlite::Row<'_>) -> rusqlite::Result<NoteCandidate> {
    Ok(NoteCandidate {
        id: r.get(0)?,
        entity_type: r.get(1)?,
        book_id: r.get(2)?,
        title: r.get(3)?,
        chapter: r.get(4)?,
        content: r.get(5)?,
        updated_at: r.get(6)?,
        relevance: relevance_from_bm25(r.get(7)?),
    })
}

const NOTE_UPDATED_SQL: &str = "CASE note_type
    WHEN 'highlight' THEN coalesce((SELECT datetime(create_time,'unixepoch','localtime') FROM highlights WHERE bookmark_id=notes_fts.note_id),'')
    ELSE coalesce((SELECT datetime(create_time,'unixepoch','localtime') FROM thoughts WHERE review_id=notes_fts.note_id),'')
  END";

/// 笔记候选集。
///
/// `unicode61` 不切分中文，FTS 对中文查询必然落空。这里沿用现有 `hybrid_search`
/// 的做法：先用 FTS 收敛（拉丁文/ISBN 场景有效），落空时退回一次限量扫描，
/// 再由调用方用 Rust 侧包含匹配打分。
fn note_candidates(
    db: &Database,
    types: &[String],
    book_id: Option<&String>,
    match_query: Option<&str>,
    limit: i64,
) -> Result<Vec<NoteCandidate>, AppError> {
    let c = db.connect()?;
    let kind = if types.iter().any(|item| item == "highlight") && !types.iter().any(|item| item == "thought") {
        Some("highlight")
    } else if types.iter().any(|item| item == "thought") && !types.iter().any(|item| item == "highlight") {
        Some("thought")
    } else {
        None
    };
    if let Some(match_query) = match_query {
        let sql = format!(
            "SELECT note_id,note_type,book_id,title,chapter_title,content,{NOTE_UPDATED_SQL},bm25(notes_fts)
             FROM notes_fts
             WHERE notes_fts MATCH ?1 AND (?2 IS NULL OR note_type=?2) AND (?3 IS NULL OR book_id=?3)
             ORDER BY bm25(notes_fts) LIMIT ?4"
        );
        let mut query = c.prepare(&sql)?;
        let rows = query
            .query_map(rusqlite::params![match_query, kind, book_id, limit], map_note_candidate)?
            .collect::<Result<Vec<_>, _>>()?;
        if !rows.is_empty() {
            return Ok(rows);
        }
    }
    // 兜底：不限 MATCH 的限量扫描，中文靠上层包含匹配过滤。
    let sql = format!(
        "SELECT note_id,note_type,book_id,title,chapter_title,content,{NOTE_UPDATED_SQL},0.0
         FROM notes_fts
         WHERE (?1 IS NULL OR note_type=?1) AND (?2 IS NULL OR book_id=?2) LIMIT ?3"
    );
    let mut query = c.prepare(&sql)?;
    let rows = query
        .query_map(rusqlite::params![kind, book_id, limit], map_note_candidate)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 书籍候选行。
struct BookCandidate {
    book_id: String,
    title: String,
    author: String,
    category: String,
    isbn: String,
    updated_at: String,
    relevance: f64,
}

fn map_book_candidate(r: &rusqlite::Row<'_>) -> rusqlite::Result<BookCandidate> {
    Ok(BookCandidate {
        book_id: r.get(0)?,
        title: r.get(1)?,
        author: r.get(2)?,
        category: r.get(3)?,
        isbn: r.get(4)?,
        updated_at: r.get(5)?,
        relevance: relevance_from_bm25(r.get(6)?),
    })
}

const BOOK_UPDATED_SQL: &str = "coalesce(datetime(coalesce(b.update_time,b.read_update_time,b.synced_at),'unixepoch','localtime'),'')";

/// 书籍候选集，兜底策略与笔记一致。
fn book_candidates(
    db: &Database,
    book_id: Option<&String>,
    match_query: Option<&str>,
    limit: i64,
) -> Result<Vec<BookCandidate>, AppError> {
    let c = db.connect()?;
    if let Some(match_query) = match_query {
        // MATCH 里必须写表名本身：给 FTS 表起别名后 MATCH 会静默返回空结果。
        let sql = format!(
            "SELECT books_fts.book_id,books_fts.title,coalesce(books_fts.author,''),coalesce(books_fts.category,''),
             coalesce(books_fts.isbn,''),{BOOK_UPDATED_SQL},bm25(books_fts)
             FROM books_fts JOIN books b ON b.book_id=books_fts.book_id
             WHERE books_fts MATCH ?1 AND b.is_deleted=0 AND (?2 IS NULL OR books_fts.book_id=?2)
             ORDER BY bm25(books_fts) LIMIT ?3"
        );
        let mut query = c.prepare(&sql)?;
        let rows = query
            .query_map(rusqlite::params![match_query, book_id, limit], map_book_candidate)?
            .collect::<Result<Vec<_>, _>>()?;
        if !rows.is_empty() {
            return Ok(rows);
        }
    }
    let mut query = c.prepare(&format!(
        "SELECT books_fts.book_id,books_fts.title,coalesce(books_fts.author,''),coalesce(books_fts.category,''),
         coalesce(books_fts.isbn,''),{BOOK_UPDATED_SQL},0.0
         FROM books_fts JOIN books b ON b.book_id=books_fts.book_id
         WHERE b.is_deleted=0 AND (?1 IS NULL OR books_fts.book_id=?1) LIMIT ?2"
    ))?;
    let rows = query
        .query_map(rusqlite::params![book_id, limit], map_book_candidate)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 笔记打分：整段包含权重最高，其次标题、章节，最后按词命中次数累计。
/// 空查询下的最近导入资料。
fn recent_source_results(db: &Database, limit: i64) -> Result<Vec<GlobalSearchResult>, AppError> {
    let c = db.connect()?;
    let mut statement = c.prepare(
        "SELECT d.source_id,s.title,s.source_type,d.id,coalesce(d.heading,''),d.content,coalesce(d.locator_json,'{}')
         FROM source_documents d JOIN library_sources s ON s.id=d.source_id
         WHERE s.is_deleted=0 ORDER BY s.imported_at DESC, d.position LIMIT ?1",
    )?;
    let rows = statement
        .query_map([limit], |r| {
            Ok(SourceCandidate {
                source_id: r.get(0)?,
                source_title: r.get(1)?,
                source_type: r.get(2)?,
                doc_id: r.get(3)?,
                heading: r.get(4)?,
                content: r.get(5)?,
                locator: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
                relevance: 0.0,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows
        .into_iter()
        .map(|candidate| {
            let label = source_locator_label(&candidate.source_type, &candidate.locator);
            GlobalSearchResult {
                id: candidate.doc_id,
                entity_type: "source".into(),
                book_id: candidate.source_id,
                subtitle: if label.is_empty() { candidate.source_type.clone() } else { label },
                title: candidate.source_title,
                snippet: snippet_of(&candidate.content),
                score: 0.0,
                updated_at: String::new(),
            }
        })
        .collect())
}

/// 导入资料的候选块。
struct SourceCandidate {
    source_id: String,
    source_title: String,
    source_type: String,
    doc_id: String,
    heading: String,
    content: String,
    locator: crate::import::Locator,
    relevance: f64,
}

fn map_source_candidate(r: &rusqlite::Row<'_>) -> rusqlite::Result<SourceCandidate> {
    Ok(SourceCandidate {
        source_id: r.get(0)?,
        source_title: r.get(1)?,
        source_type: r.get(2)?,
        doc_id: r.get(3)?,
        heading: r.get(4)?,
        content: r.get(5)?,
        locator: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
        relevance: r.get(7)?,
    })
}

/// 导入资料的候选集，走 `source_docs_fts`。
///
/// 与笔记一样沿用「FTS 收敛 + 兜底扫描 + Rust 侧包含匹配」的双层策略：
/// `unicode61` 不切分中文，纯 FTS 对中文查询必然落空。
fn source_candidates(
    db: &Database,
    match_query: Option<&str>,
    source_id: Option<&String>,
    limit: i64,
) -> Result<Vec<SourceCandidate>, AppError> {
    let c = db.connect()?;
    if let Some(match_query) = match_query {
        // MATCH 里必须写表名本身：给 FTS 表起别名后 MATCH 会静默返回空结果。
        let mut statement = c.prepare(
            "SELECT source_docs_fts.source_id,coalesce(s.title,''),coalesce(s.source_type,''),source_docs_fts.doc_id,
                    coalesce(source_docs_fts.heading,''),source_docs_fts.content,coalesce(d.locator_json,'{}'),bm25(source_docs_fts)
             FROM source_docs_fts
             JOIN source_documents d ON d.id=source_docs_fts.doc_id
             JOIN library_sources s ON s.id=source_docs_fts.source_id
             WHERE source_docs_fts MATCH ?1 AND s.is_deleted=0 AND (?2 IS NULL OR source_docs_fts.source_id=?2)
             ORDER BY bm25(source_docs_fts) LIMIT ?3",
        )?;
        let rows = statement
            .query_map(rusqlite::params![match_query, source_id, limit], map_source_candidate)?
            .collect::<Result<Vec<_>, _>>()?;
        if !rows.is_empty() {
            return Ok(rows);
        }
    }
    let mut statement = c.prepare(
        "SELECT d.source_id,coalesce(s.title,''),coalesce(s.source_type,''),d.id,
                coalesce(d.heading,''),d.content,coalesce(d.locator_json,'{}'),0.0
         FROM source_documents d
         JOIN library_sources s ON s.id=d.source_id
         WHERE s.is_deleted=0 AND (?1 IS NULL OR d.source_id=?1) LIMIT ?2",
    )?;
    let rows = statement
        .query_map(rusqlite::params![source_id, limit], map_source_candidate)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// 导入资料的打分：正文整段包含权重最高，标题与章节名次之。
fn score_source(candidate: &SourceCandidate, normalized_query: &str, terms: &[String]) -> f64 {
    let content = normalize_search_text(&candidate.content);
    let title = normalize_search_text(&candidate.source_title);
    let heading = normalize_search_text(&candidate.heading);
    let mut score = candidate.relevance;
    if !normalized_query.is_empty() {
        if content.contains(normalized_query) {
            score += 12.0;
        } else if terms.iter().any(|term| content.contains(term)) {
            score += 4.0;
        } else {
            return 0.0;
        }
    }
    for term in terms {
        if title.contains(term) {
            score += 4.0;
        }
        if heading.contains(term) {
            score += 2.5;
        }
        score += content.match_indices(term).count().min(4) as f64;
    }
    score
}

/// 资料标题旁的来源标签，例如「第 12 页」「第 3 章」「网页」。
///
/// 优先看定位信息里有没有页码 / 章节号 —— 判据是「有没有」而不是
/// 「来源类型是不是 pdf」，这样 EPUB 的章节与 PDF 的页码都能正确显示。
fn source_locator_label(source_type: &str, locator: &crate::import::Locator) -> String {
    if let Some(page) = locator.page {
        return format!("第 {page} 页");
    }
    if let Some(chapter) = locator.chapter {
        return format!("第 {chapter} 章");
    }
    if source_type == "web" {
        return "网页".to_string();
    }
    locator.heading.clone().unwrap_or_default()
}

/// 把导入资料并入全局搜索结果。
fn source_search_results(
    db: &Database,
    active_types: &[String],
    book_id: Option<&String>,
    match_query: Option<&str>,
    terms: &[String],
    normalized_query: &str,
    limit: i64,
) -> Result<Vec<GlobalSearchResult>, AppError> {
    if !active_types.iter().any(|kind| kind == "source") {
        return Ok(Vec::new());
    }
    // 书籍过滤对导入资料没有意义：book_id 只在指定书籍时生效，这里直接跳过
    if book_id.is_some() {
        return Ok(Vec::new());
    }
    Ok(source_candidates(db, match_query, None, limit)?
        .into_iter()
        .filter_map(|candidate| {
            let score = score_source(&candidate, normalized_query, terms);
            if score <= 0.0 {
                return None;
            }
            let label = source_locator_label(&candidate.source_type, &candidate.locator);
            let subtitle = if label.is_empty() { candidate.source_type.clone() } else { label };
            Some(GlobalSearchResult {
                id: candidate.doc_id,
                entity_type: "source".into(),
                book_id: candidate.source_id,
                title: candidate.source_title,
                subtitle,
                snippet: snippet_of(&candidate.content),
                score,
                updated_at: String::new(),
            })
        })
        .collect())
}

fn score_note(candidate: &NoteCandidate, normalized_query: &str, terms: &[String]) -> f64 {
    let content = normalize_search_text(&candidate.content);
    let title = normalize_search_text(&candidate.title);
    let chapter = normalize_search_text(&candidate.chapter);
    let mut score = candidate.relevance;
    if !normalized_query.is_empty() {
        if content.contains(normalized_query) {
            score += 12.0;
        } else if terms.iter().any(|term| content.contains(term)) {
            score += 4.0;
        } else {
            return 0.0;
        }
    }
    for term in terms {
        if title.contains(term) {
            score += 5.0;
        }
        if chapter.contains(term) {
            score += 3.0;
        }
        score += content.match_indices(term).count().min(4) as f64;
    }
    score
}

/// 全局搜索：书籍 + 划线 + 想法，支持类型过滤与书籍过滤。
#[tauri::command]
pub fn global_search(db: State<'_, Database>, request: GlobalSearchRequest) -> Result<GlobalSearchPage, AppError> {
    global_search_impl(&db, &request)
}

fn global_search_impl(db: &Database, request: &GlobalSearchRequest) -> Result<GlobalSearchPage, AppError> {
    let types = normalized_search_types(&request.types)?;
    let active_types = if types.is_empty() {
        SEARCH_ENTITY_TYPES.iter().map(|kind| kind.to_string()).collect::<Vec<_>>()
    } else {
        types
    };
    let limit = request.limit.unwrap_or(30).clamp(1, GLOBAL_SEARCH_MAX_LIMIT);
    let offset = request.offset.unwrap_or(0).clamp(0, GLOBAL_SEARCH_MAX_OFFSET);
    let query = request.query.trim().to_owned();
    let book_id = request
        .book_id
        .clone()
        .filter(|value| !value.trim().is_empty());

    if query.is_empty() {
        // 空查询不跑 FTS，只返回最近更新的内容，数量同样受限。
        // 取数必须覆盖 offset+limit：只取 limit+1 的话，第二页永远取不到数据。
        let fetch = offset.saturating_add(limit).saturating_add(1);
        let mut results = recent_note_results(&db, &active_types, book_id.as_ref(), fetch)?;
        if active_types.iter().any(|kind| kind == "book") {
            results.extend(recent_book_results(&db, book_id.as_ref(), fetch)?);
        }
        // 导入资料：书籍过滤对它没有意义，只在未限定书籍时参与
        if active_types.iter().any(|kind| kind == "source") && book_id.is_none() {
            results.extend(recent_source_results(&db, fetch)?);
        }
        // 同分按更新时间倒序，书名稳定排序避免同毫秒抖动。
        results.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.title.cmp(&right.title))
                .then_with(|| left.id.cmp(&right.id))
        });
        // 先 skip 再 take：反过来的话 offset=limit 时会把结果全跳光，第二页恒空，
        // 而 has_more 仍为 true，前端「加载更多」会无限空转。
        let total = results.len();
        let page = results.into_iter().skip(offset as usize).take(limit as usize).collect::<Vec<_>>();
        let has_more = offset as usize + page.len() < total;
        return Ok(GlobalSearchPage { results: page, has_more });
    }

    // 中文沿用现有 search_terms 策略：整词 + bigram，书籍与笔记统一。
    let terms = search_terms(&query);
    let match_query = fts_match_expression(&query);
    let normalized_query = normalize_search_text(&query);

    // 候选集放大后再打分，最后才分页。纯符号输入没有可用词，跳过 FTS 直接走兜底。
    let candidate_limit = (limit + offset).saturating_mul(3).clamp(30, 500);
    let mut results = Vec::new();
    if active_types.iter().any(|kind| kind == "highlight" || kind == "thought") {
        for candidate in note_candidates(&db, &active_types, book_id.as_ref(), match_query.as_deref(), candidate_limit)? {
            let score = score_note(&candidate, &normalized_query, &terms);
            if score <= 0.0 {
                continue;
            }
            results.push(GlobalSearchResult {
                id: candidate.id,
                entity_type: candidate.entity_type,
                subtitle: candidate.chapter,
                book_id: candidate.book_id,
                title: candidate.title,
                snippet: snippet_of(&candidate.content),
                score,
                updated_at: candidate.updated_at,
            });
        }
    }
    // 导入资料：书籍过滤对它没有意义，只在未限定书籍时参与
    results.extend(source_search_results(
        &db,
        &active_types,
        book_id.as_ref(),
        match_query.as_deref(),
        &terms,
        &normalized_query,
        candidate_limit,
    )?);
    if active_types.iter().any(|kind| kind == "book") {
        for candidate in book_candidates(&db, book_id.as_ref(), match_query.as_deref(), candidate_limit)? {
            let score = score_book(&candidate, &normalized_query);
            if score <= 0.0 {
                continue;
            }
            results.push(GlobalSearchResult {
                id: candidate.book_id.clone(),
                entity_type: "book".into(),
                book_id: candidate.book_id,
                subtitle: join_subtitle(&candidate.author, &candidate.category),
                title: candidate.title,
                snippet: if candidate.isbn.is_empty() { String::new() } else { format!("ISBN {}", candidate.isbn) },
                score,
                updated_at: candidate.updated_at,
            });
        }
    }
    if results.is_empty() {
        return Ok(GlobalSearchPage { results: Vec::new(), has_more: false });
    }
    // 同分按更新时间倒序，保证分页稳定。
    results.sort_by(|left, right| {
        right
            .score
            .partial_cmp(&left.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| right.updated_at.cmp(&left.updated_at))
            .then_with(|| left.id.cmp(&right.id))
    });
    normalize_scores(&mut results);
    let has_more = results.len() > (limit + offset) as usize;
    let page = results
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect();
    Ok(GlobalSearchPage { results: page, has_more })
}

fn search_impl(
    db: &Database,
    query: &str,
    kind: Option<String>,
) -> Result<Vec<SearchResult>, AppError> {
    if !query.trim().is_empty() {
        return hybrid_search(db, query, kind.as_deref(), &[], 50);
    }
    let c = db.connect()?;
    let sql = if query.trim().is_empty() {
        "SELECT note_id,note_type,book_id,title,chapter_title,content,
         CASE note_type WHEN 'highlight' THEN coalesce(datetime((SELECT create_time FROM highlights WHERE bookmark_id=note_id),'unixepoch','localtime'),'') ELSE coalesce(datetime((SELECT create_time FROM thoughts WHERE review_id=note_id),'unixepoch','localtime'),'') END,
         0.0 FROM notes_fts WHERE (?1 IS NULL OR note_type=?1) ORDER BY 7 DESC LIMIT 100"
    } else {
        "SELECT note_id,note_type,book_id,title,chapter_title,content,
         CASE note_type WHEN 'highlight' THEN coalesce(datetime((SELECT create_time FROM highlights WHERE bookmark_id=note_id),'unixepoch','localtime'),'') ELSE coalesce(datetime((SELECT create_time FROM thoughts WHERE review_id=note_id),'unixepoch','localtime'),'') END,
         bm25(notes_fts) FROM notes_fts WHERE notes_fts MATCH ?2 AND (?1 IS NULL OR note_type=?1) ORDER BY bm25(notes_fts) LIMIT 20"
    };
    let mut q = c.prepare(sql)?;
    let rows = if query.trim().is_empty() {
        q.query_map(rusqlite::params![kind], map_note)?
    } else {
        q.query_map(rusqlite::params![kind, fts_query(query)], map_note)?
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
            created_at: r.get(6)?,
        },
        score: r.get(7)?,
    })
}

#[tauri::command]
pub fn save_secret(kind: String, value: String) -> Result<(), AppError> {
    validate_secret_kind(&kind)?;
    let entry = keyring::Entry::new("ReadFlow", &kind)?;
    entry.set_password(&value)?;
    let persisted = entry.get_password()?;
    if persisted != value {
        return Err(AppError::Message("密钥未能正确保存到系统凭据库，请重试".into()));
    }
    Ok(())
}

#[tauri::command]
pub fn has_secret(kind: String) -> Result<bool, AppError> {
    validate_secret_kind(&kind)?;
    match keyring::Entry::new("ReadFlow", &kind)?.get_password() {
        Ok(value) => Ok(!value.is_empty()),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(error) => Err(AppError::Credential(error)),
    }
}
/// Google Books 连接测试的判定逻辑，纯函数便于单测。
/// 返回的文案里只出现 HTTP 状态码，绝不包含 API Key。
pub(crate) fn classify_google_books_status(status: u16) -> Result<bool, String> {
    match status {
        200..=299 => Ok(true),
        401 | 403 => Err("Google Books API Key 无效或已被拒绝".into()),
        429 => Err("Google Books 请求已限流，请稍后重试或更换 API Key".into()),
        other => Err(format!("Google Books 接口返回异常状态码：{other}")),
    }
}

/// 用最小请求验证 Google Books Key 是否可用，只返回 HTTP 状态码。
/// 传输层错误统一改写成不含 URL 的文案，避免 API Key 随错误信息外泄。
async fn probe_google_books(secret: &str) -> Result<u16, AppError> {
    let client = reqwest::Client::builder().user_agent("ReadFlow/0.1 personal metadata client").timeout(Duration::from_secs(12)).build()?;
    let response = client
        .get("https://books.googleapis.com/books/v1/volumes")
        .query(&[("q", "flowers"), ("maxResults", "1"), ("key", secret)])
        .send()
        .await
        .map_err(|error| {
            if error.is_timeout() {
                AppError::Message("Google Books 连接测试超时，请稍后重试".into())
            } else if error.is_connect() {
                AppError::Message("无法连接 Google Books 接口，请检查网络".into())
            } else {
                AppError::Message("Google Books 连接测试失败".into())
            }
        })?;
    let status = response.status();
    if status.is_success() {
        // 成功时也要真正解析一次响应体，确认返回结构可用并限制响应大小。
        let value: Value = limited_json(response, MAX_API_RESPONSE_BYTES, "Google Books").await?;
        if value.get("totalItems").is_none() && value.get("items").is_none() {
            return Err(AppError::Message("Google Books 返回结构异常".into()));
        }
    }
    Ok(status.as_u16())
}

#[tauri::command]
pub async fn test_connection(kind: String, value: Option<String>) -> Result<bool, AppError> {
    validate_secret_kind(&kind)?;
    let secret = match value.filter(|value| !value.trim().is_empty()) {
        Some(value) => value,
        None => keyring::Entry::new("ReadFlow", &kind)?.get_password()?,
    };
    if kind == "weread" {
        crate::weread::client::WeReadClient::new(secret)?
            .test()
            .await?;
    } else if kind == "google_books" {
        // Google Books 必须真正请求一次，Key 错了要当场暴露，不能默认通过。
        let status = probe_google_books(&secret).await?;
        classify_google_books_status(status).map_err(AppError::Message)?;
    }
    Ok(true)
}

fn validate_secret_kind(kind: &str) -> Result<(), AppError> {
    let valid_provider_kind = ["ai:", "embedding:"].iter().any(|prefix| {
        kind.strip_prefix(prefix).is_some_and(|provider| {
            !provider.is_empty()
                && provider.len() <= 64
                && provider
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
        })
    });
    if matches!(kind, "weread" | "google_books") || valid_provider_kind {
        Ok(())
    } else {
        Err(AppError::Message("不支持的凭据类型".into()))
    }
}
#[tauri::command]
pub async fn sync_weread(app: AppHandle, db: State<'_, Database>) -> Result<SyncProgress, AppError> {
    let secret = keyring::Entry::new("ReadFlow", "weread")?
        .get_password()
        .map_err(|error| match error {
            keyring::Error::NoEntry => {
                AppError::Message("请先在设置中填写并保存微信读书 API Key".into())
            }
            other => AppError::Credential(other),
        })?;
    let client = crate::weread::client::WeReadClient::new(secret)?;
    crate::sync::run(&db, &client, |progress| {
        let _ = app.emit("sync-progress", progress);
    })
    .await
}

fn fts_query(input: &str) -> String {
    input
        .split_whitespace()
        .filter(|part| !part.is_empty())
        .map(|part| format!("\"{}\"", part.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

#[tauri::command]
pub fn get_ai_settings(db: State<'_, Database>) -> Result<Option<AiSettings>, AppError> {
    let c = db.connect()?;
    c.query_row(
        "SELECT provider,endpoint,model FROM ai_settings WHERE id=1",
        [],
        |r| {
            Ok(AiSettings {
                provider: r.get(0)?,
                endpoint: r.get(1)?,
                model: r.get(2)?,
            })
        },
    )
    .optional()
    .map_err(AppError::from)
}

#[tauri::command]
pub fn get_embedding_settings(db: State<'_, Database>) -> Result<Option<EmbeddingSettings>, AppError> {
    db.connect()?.query_row("SELECT provider,endpoint,model FROM embedding_settings WHERE id=1", [], |row| Ok(EmbeddingSettings { provider: row.get(0)?, endpoint: row.get(1)?, model: row.get(2)? })).optional().map_err(AppError::from)
}

fn reranker_settings_from_db(db: &Database) -> Result<Option<RerankerSettings>, AppError> {
    db.connect()?
        .query_row("SELECT provider,endpoint,model,top_n,enabled FROM reranker_settings WHERE id=1", [], |row| {
            Ok(RerankerSettings {
                provider: row.get(0)?,
                endpoint: row.get(1)?,
                model: row.get(2)?,
                top_n: row.get(3)?,
                enabled: row.get::<_, i64>(4)? != 0,
            })
        })
        .optional()
        .map_err(AppError::from)
}

#[tauri::command]
pub fn get_reranker_settings(db: State<'_, Database>) -> Result<Option<RerankerSettings>, AppError> {
    reranker_settings_from_db(&db)
}

#[tauri::command]
pub fn save_reranker_settings(
    db: State<'_, Database>,
    settings: RerankerSettings,
    api_key: Option<String>,
) -> Result<(), AppError> {
    save_reranker_settings_impl(&db, &settings, api_key)
}

fn save_reranker_settings_impl(
    db: &Database,
    settings: &RerankerSettings,
    api_key: Option<String>,
) -> Result<(), AppError> {
    let top_n_range = crate::ai::reranker::MIN_TOP_N..=crate::ai::reranker::MAX_TOP_N;
    // 关闭开关时只校验 Top N，这样用户能先关掉一个坏掉的配置。
    if settings.enabled {
        crate::ai::reranker::validate_settings(settings)?;
    } else if !top_n_range.contains(&settings.top_n) {
        return Err(AppError::Message(format!(
            "Top N 必须在 {} 到 {} 之间",
            crate::ai::reranker::MIN_TOP_N,
            crate::ai::reranker::MAX_TOP_N
        )));
    }
    if let Some(api_key) = api_key.filter(|value| !value.trim().is_empty()) {
        // 密钥只进系统凭据库，绝不写进 SQLite。
        let entry = keyring::Entry::new(
            "ReadFlow",
            &crate::ai::reranker::credential_name(&settings.provider),
        )?;
        entry.set_password(&api_key)?;
        if entry.get_password()? != api_key {
            return Err(AppError::Message(
                "Reranker Key 未能正确保存到系统凭据库，请重试".into(),
            ));
        }
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    db.connect()?.execute(
        "INSERT INTO reranker_settings(id,provider,endpoint,model,top_n,enabled,updated_at)
         VALUES(1,?1,?2,?3,?4,?5,?6)
         ON CONFLICT(id) DO UPDATE SET provider=excluded.provider,endpoint=excluded.endpoint,
           model=excluded.model,top_n=excluded.top_n,enabled=excluded.enabled,updated_at=excluded.updated_at",
        rusqlite::params![settings.provider, settings.endpoint, settings.model, settings.top_n, settings.enabled as i64, now],
    )?;
    Ok(())
}

// ---------- 概念级知识图谱 ----------

/// 一次扫描的单本书上下文。
struct BookNotes {
    book_id: String,
    /// (note_id, content)
    notes: Vec<(String, String)>,
}

/// 按书籍分批取出未删除的划线与想法。
fn notes_grouped_by_book(db: &Database, book_id: Option<&str>) -> Result<Vec<BookNotes>, AppError> {
    let c = db.connect()?;
    let mut books: std::collections::BTreeMap<String, Vec<(String, String)>> = std::collections::BTreeMap::new();
    let mut query = c.prepare(
        "SELECT h.book_id,h.bookmark_id,h.mark_text FROM highlights h JOIN books b ON b.book_id=h.book_id
         WHERE h.is_deleted=0 AND b.is_deleted=0 AND (?1 IS NULL OR h.book_id=?1)
         UNION ALL
         SELECT t.book_id,t.review_id,t.content FROM thoughts t JOIN books b ON b.book_id=t.book_id
         WHERE t.is_deleted=0 AND b.is_deleted=0 AND (?1 IS NULL OR t.book_id=?1)
         ORDER BY 1, 2",
    )?;
    let rows = query.query_map([book_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?;
    for row in rows {
        let (book, id, content) = row?;
        let content: String = content.chars().take(crate::ai::concepts::MAX_NOTE_CHARS).collect();
        books.entry(book).or_default().push((id, content));
    }
    Ok(books
        .into_iter()
        .map(|(book_id, notes)| BookNotes { book_id, notes })
        .collect())
}

/// 已确认的名词库条目，作为抽取时的候选提示。
fn glossary_candidates(db: &Database) -> Result<Vec<(String, String, Vec<String>)>, AppError> {
    let c = db.connect()?;
    let mut query = c.prepare("SELECT canonical_name,coalesce(definition,''),coalesce(aliases_json,'[]') FROM glossary_terms WHERE status='confirmed'")?;
    let rows = query.query_map([], |r| {
        let aliases: Vec<String> = serde_json::from_str(&r.get::<_, String>(2)?).unwrap_or_default();
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, aliases))
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// 库里已有实体的名字（含别名），用于校验关系两端。
fn known_entity_names(db: &Database) -> Result<std::collections::HashSet<String>, AppError> {
    let c = db.connect()?;
    let mut query = c.prepare("SELECT canonical_name,coalesce(aliases_json,'[]') FROM knowledge_entities")?;
    let rows = query.query_map([], |r| {
        let canonical = r.get::<_, String>(0)?;
        let aliases: Vec<String> = serde_json::from_str(&r.get::<_, String>(1)?).unwrap_or_default();
        Ok((canonical, aliases))
    })?;
    let mut names = std::collections::HashSet::new();
    for row in rows {
        let (canonical, aliases) = row?;
        for key in crate::ai::concepts::entity_lookup_keys(&canonical, &aliases) {
            names.insert(key);
        }
    }
    Ok(names)
}

/// 某本书里内容未变化的笔记（可以跳过重新抽取）。
fn unchanged_notes(db: &Database, notes: &[(String, String)]) -> Result<Vec<String>, AppError> {
    if notes.is_empty() {
        return Ok(Vec::new());
    }
    let c = db.connect()?;
    let mut query = c.prepare("SELECT note_id,content_hash FROM knowledge_note_state")?;
    let mut hashes: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for row in query.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (id, hash) = row?;
        hashes.insert(id, hash);
    }
    Ok(notes
        .iter()
        .filter(|(id, content)| {
            hashes
                .get(id)
                .map(|hash| *hash == content_hash(content))
                .unwrap_or(false)
        })
        .map(|(id, _)| id.clone())
        .collect())
}

fn content_hash(content: &str) -> String {
    format!("{:x}", md5::compute(content.as_bytes()))
}

/// 解析 AI 返回的 JSON。允许模型把 JSON 包在 ```json 代码块或散文里。
fn parse_extraction(text: &str) -> Result<crate::ai::concepts::RawExtraction, AppError> {
    let trimmed = text.trim();
    let without_fence = trimmed
        .strip_prefix("```json")
        .or_else(|| trimmed.strip_prefix("```"))
        .map(|value| value.trim_end_matches("```").trim())
        .unwrap_or(trimmed);
    let start = without_fence.find('{');
    let end = without_fence.rfind('}');
    let candidate = match (start, end) {
        (Some(start), Some(end)) if end > start => &without_fence[start..=end],
        _ => return Err(AppError::Message("AI 没有返回有效的 JSON".into())),
    };
    serde_json::from_str(candidate).map_err(|error| AppError::Message(format!("AI 返回的概念 JSON 无法解析：{error}")))
}

/// 把一批笔记的抽取结果写库。证据与关系两端都已经在 `validate_extraction` 里校验过。
fn persist_extraction(
    db: &Database,
    extraction: &crate::ai::concepts::ValidatedExtraction,
    scanned_notes: &[(String, String)],
) -> Result<(i64, i64), AppError> {
    let mut c = db.connect()?;
    let transaction = c.transaction()?;
    let now = now_timestamp() as i64;
    // 本次抽取产出的名字 → 实体 ID，关系落库时要用
    let mut name_to_id: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    {
        let mut query = transaction.prepare("SELECT id,canonical_name,coalesce(aliases_json,'[]') FROM knowledge_entities")?;
        let rows = query.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, serde_json::from_str::<Vec<String>>(&r.get::<_, String>(2)?).unwrap_or_default()))
        })?;
        for row in rows {
            let (id, canonical, aliases) = row?;
            for key in crate::ai::concepts::entity_lookup_keys(&canonical, &aliases) {
                name_to_id.insert(key, id.clone());
            }
        }
    }
    let mut entity_count = 0;
    for entity in &extraction.entities {
        let id = crate::ai::concepts::entity_id(&entity.kind, &entity.canonical_name);
        let key = entity.canonical_name.to_lowercase();
        // 已存在的实体只补证据，不覆盖用户改过的名字与状态
        if name_to_id.contains_key(&key) {
            let existing = name_to_id[&key].clone();
            {
                let mut query = transaction.prepare(
                    "INSERT INTO knowledge_entity_evidence(entity_id,note_id,book_id,quote,confidence)
                     VALUES(?1,?2,?3,?4,?5)
                     ON CONFLICT(entity_id,note_id) DO UPDATE SET quote=excluded.quote,confidence=excluded.confidence",
                )?;
                for evidence in &entity.evidence {
                    query.execute(rusqlite::params![existing, evidence.note_id, evidence.book_id, evidence.quote, evidence.confidence])?;
                }
            }
            continue;
        }
        transaction.execute(
            "INSERT INTO knowledge_entities(id,kind,canonical_name,description,aliases_json,status,source_hash,updated_at)
             VALUES(?1,?2,?3,?4,?5,'suggested',?6,?7)",
            rusqlite::params![id, entity.kind, entity.canonical_name, entity.description, serde_json::to_string(&entity.aliases).unwrap_or_else(|_| "[]".into()), entity.canonical_name.clone(), now],
        )?;
        {
            let mut query = transaction.prepare(
                "INSERT INTO knowledge_entity_evidence(entity_id,note_id,book_id,quote,confidence) VALUES(?1,?2,?3,?4,?5)",
            )?;
            for evidence in &entity.evidence {
                query.execute(rusqlite::params![id, evidence.note_id, evidence.book_id, evidence.quote, evidence.confidence])?;
            }
        }
        name_to_id.insert(key, id);
        entity_count += 1;
    }

    let mut relation_count = 0;
    for relation in &extraction.relations {
        let (Some(from), Some(to)) = (
            name_to_id.get(&relation.from_name.to_lowercase()).cloned(),
            name_to_id.get(&relation.to_name.to_lowercase()).cloned(),
        ) else {
            continue;
        };
        if from == to {
            continue;
        }
        let id = crate::ai::concepts::relation_id(&from, &to, &relation.relation);
        transaction.execute(
            "INSERT INTO knowledge_relations(id,from_entity_id,to_entity_id,relation,summary,confidence,evidence_json,input_hash,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)
             ON CONFLICT(id) DO UPDATE SET summary=excluded.summary,confidence=excluded.confidence,
               evidence_json=excluded.evidence_json,input_hash=excluded.input_hash,updated_at=excluded.updated_at",
            rusqlite::params![id, from, to, relation.relation, relation.summary, relation.confidence, serde_json::to_string(&relation.evidence).unwrap_or_else(|_| "[]".into()), format!("{}|{}", relation.from_name, relation.to_name), now],
        )?;
        relation_count += 1;
    }

    for (note_id, content) in scanned_notes {
        transaction.execute(
            "INSERT INTO knowledge_note_state(note_id,book_id,content_hash,scanned_at) VALUES(?1,'',?2,?3)
             ON CONFLICT(note_id) DO UPDATE SET content_hash=excluded.content_hash,scanned_at=excluded.scanned_at",
            rusqlite::params![note_id, content_hash(content), now],
        )?;
    }
    transaction.commit()?;
    Ok((entity_count, relation_count))
}

/// 概念图谱查询。大图默认只返回高置信度子图。
#[tauri::command]
pub fn list_concept_graph(db: State<'_, Database>, query: ConceptGraphQuery) -> Result<ConceptGraph, AppError> {
    list_concept_graph_impl(&db, &query)
}

fn list_concept_graph_impl(db: &Database, query: &ConceptGraphQuery) -> Result<ConceptGraph, AppError> {
    for kind in &query.kinds {
        if !ENTITY_KINDS.contains(&kind.as_str()) {
            return Err(AppError::Message("不支持的实体类型".into()));
        }
    }
    let min_confidence = query.min_confidence.unwrap_or(0.0).clamp(0.0, 1.0);
    // 超过这个数量只给高置信度子图，避免前端一次渲染上万节点
    const SUBGRAPH_THRESHOLD: i64 = 500;
    let limit = query.limit.unwrap_or(600).clamp(1, 2_000);
    let c = db.connect()?;

    // 中心实体模式：只看它的邻居
    if let Some(center_id) = query.center_id.as_ref().filter(|value| !value.is_empty()) {
        let mut query_stmt = c.prepare(
            "SELECT e.id,e.kind,e.canonical_name,e.description,e.aliases_json,e.status,e.updated_at,
                    (SELECT count(*) FROM knowledge_entity_evidence v WHERE v.entity_id=e.id)
             FROM knowledge_entities e
             WHERE e.id=?1 OR e.id IN (SELECT from_entity_id FROM knowledge_relations WHERE to_entity_id=?1
                                       UNION SELECT to_entity_id FROM knowledge_relations WHERE from_entity_id=?1)
             ORDER BY 4, 1 LIMIT ?2",
        )?;
        let entities = query_stmt
            .query_map(rusqlite::params![center_id, limit], map_entity_row)?
            .collect::<Result<Vec<_>, _>>()?;
        let total = entities.len() as i64;
        let ids: Vec<String> = entities.iter().map(|item| item.id.clone()).collect();
        let relations = relations_between(&c, &ids, min_confidence, query.include_hidden)?;
        let entities = entities
            .into_iter()
            .filter(|item| query.include_hidden || item.status != "hidden")
            .collect();
        return Ok(ConceptGraph { entities, relations, truncated: false, total_entities: total });
    }

    let mut count_query = c.prepare("SELECT count(*) FROM knowledge_entities WHERE (?1 IS NULL OR status<>'hidden')")?;
    let total_entities: i64 = count_query.query_row([if query.include_hidden { None } else { Some("x") }], |r| r.get(0)).unwrap_or(0);
    // 500 节点以上只加载高置信度子图
    let (subgraph_confidence, truncated) = if total_entities > SUBGRAPH_THRESHOLD {
        (min_confidence.max(0.45), true)
    } else {
        (min_confidence, false)
    };

    let kind_filter: Vec<String> = query.kinds.clone();
    let mut stmt = c.prepare(
        "SELECT e.id,e.kind,e.canonical_name,e.description,e.aliases_json,e.status,e.updated_at,
                (SELECT count(*) FROM knowledge_entity_evidence v WHERE v.entity_id=e.id)
         FROM knowledge_entities e
         WHERE (?1 IS NULL OR e.status<>'hidden')
           AND (?2 IS NULL OR e.kind IN (SELECT value FROM json_each(?2)))
           AND (?3 IS NULL OR e.id IN (SELECT entity_id FROM knowledge_entity_evidence WHERE book_id=?3))
           AND EXISTS(SELECT 1 FROM knowledge_entity_evidence v WHERE v.entity_id=e.id AND v.confidence>=?4)
         ORDER BY 7 DESC LIMIT ?5",
    )?;
    let kinds_json = if kind_filter.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&kind_filter).unwrap_or_default())
    };
    let entities: Vec<KnowledgeEntity> = stmt
        .query_map(
            rusqlite::params![
                if query.include_hidden { None } else { Some("x") },
                kinds_json,
                query.book_id.as_ref().filter(|value| !value.is_empty()),
                subgraph_confidence,
                limit
            ],
            map_entity_row,
        )?
        .collect::<Result<Vec<_>, _>>()?;
    let ids: Vec<String> = entities.iter().map(|item| item.id.clone()).collect();
    let relations = relations_between(&c, &ids, subgraph_confidence, query.include_hidden)?;
    Ok(ConceptGraph { entities, relations, truncated, total_entities })
}

fn map_entity_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<KnowledgeEntity> {
    let updated: i64 = r.get(6)?;
    Ok(KnowledgeEntity {
        id: r.get(0)?,
        kind: r.get(1)?,
        canonical_name: r.get(2)?,
        description: r.get(3)?,
        aliases: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
        status: r.get(5)?,
        evidence_count: r.get(7)?,
        updated_at: if updated > 0 {
            rusqlite::Connection::open_in_memory()
                .ok()
                .and_then(|c| c.query_row("SELECT coalesce(datetime(?1,'unixepoch','localtime'),'')", rusqlite::params![updated], |r| r.get::<_, String>(0)).ok())
                .unwrap_or_default()
        } else {
            String::new()
        },
        evidence: Vec::new(),
    })
}

/// 只保留两端都在给定集合内的关系。
fn relations_between(
    c: &rusqlite::Connection,
    entity_ids: &[String],
    min_confidence: f64,
    include_hidden: bool,
) -> Result<Vec<KnowledgeRelation>, AppError> {
    if entity_ids.len() < 2 {
        return Ok(Vec::new());
    }
    let json = serde_json::to_string(entity_ids).unwrap_or_default();
    // 用 json_each 展开实体 ID 集合，避免拼接大量占位符。
    let mut stmt = c.prepare(
        "SELECT r.id,r.from_entity_id,r.to_entity_id,r.relation,r.summary,r.confidence,r.evidence_json
         FROM knowledge_relations r
         WHERE r.confidence>=?1
           AND r.from_entity_id IN (SELECT value FROM json_each(?2))
           AND r.to_entity_id IN (SELECT value FROM json_each(?2))
           AND (?3 OR r.from_entity_id NOT IN (SELECT id FROM knowledge_entities WHERE status='hidden')
                  AND r.to_entity_id NOT IN (SELECT id FROM knowledge_entities WHERE status='hidden'))
         ORDER BY r.confidence DESC",
    )?;
    let rows = stmt.query_map(rusqlite::params![min_confidence, json, include_hidden], |r| {
        Ok(KnowledgeRelation {
            id: r.get(0)?,
            from_entity_id: r.get(1)?,
            to_entity_id: r.get(2)?,
            relation: r.get(3)?,
            summary: r.get(4)?,
            confidence: r.get(5)?,
            evidence: serde_json::from_str(&r.get::<_, String>(6)?).unwrap_or_default(),
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// 概念详情：带上证据笔记。
fn concept_entity_with_evidence(db: &Database, entity_id: &str) -> Result<KnowledgeEntity, AppError> {
    let c = db.connect()?;
    let mut stmt = c.prepare(
        "SELECT e.id,e.kind,e.canonical_name,e.description,e.aliases_json,e.status,e.updated_at,
                (SELECT count(*) FROM knowledge_entity_evidence v WHERE v.entity_id=e.id)
         FROM knowledge_entities e WHERE e.id=?1",
    )?;
    let mut entity = stmt
        .query_row([&entity_id], map_entity_row)
        .optional()?
        .ok_or_else(|| AppError::Message("概念不存在".into()))?;
    let mut evidence_query = c.prepare("SELECT note_id,book_id,quote,confidence FROM knowledge_entity_evidence WHERE entity_id=?1 ORDER BY confidence DESC")?;
    entity.evidence = evidence_query
        .query_map([entity_id], |r| {
            Ok(ConceptEvidence { note_id: r.get(0)?, book_id: r.get(1)?, quote: r.get(2)?, confidence: r.get(3)? })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(entity)
}

/// 概念详情：带上证据笔记。
#[tauri::command]
pub fn get_concept_entity(db: State<'_, Database>, entity_id: String) -> Result<KnowledgeEntity, AppError> {
    concept_entity_with_evidence(&db, &entity_id)
}

/// 用户校正：确认 / 隐藏 / 重命名 / 加别名。
#[tauri::command]
pub fn correct_concept_entity(db: State<'_, Database>, correction: EntityCorrection) -> Result<KnowledgeEntity, AppError> {
    correct_entity_impl(&db, &correction)
}

fn correct_entity_impl(db: &Database, correction: &EntityCorrection) -> Result<KnowledgeEntity, AppError> {
    let c = db.connect()?;
    let mut stmt = c.prepare("SELECT id,canonical_name,description,aliases_json,status FROM knowledge_entities WHERE id=?1")?;
    let (id, mut name, mut description, aliases_json, status): (String, String, String, String, String) = stmt
        .query_row([&correction.id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .optional()?
        .ok_or_else(|| AppError::Message("概念不存在".into()))?;
    if let Some(next) = correction.canonical_name.as_deref() {
        let normalized = crate::ai::concepts::normalize_name(next).ok_or_else(|| AppError::Message("概念名称不能为空".into()))?;
        name = normalized;
    }
    if let Some(next) = correction.description.as_deref() {
        description = next.chars().take(320).collect();
    }
    let mut status = status;
    if let Some(next) = correction.status.as_deref() {
        if !ENTITY_STATUSES.contains(&next) {
            return Err(AppError::Message("不支持的概念状态".into()));
        }
        status = next.to_owned();
    }
    let mut aliases: Vec<String> = serde_json::from_str(&aliases_json).unwrap_or_default();
    for alias in correction.aliases.iter().filter_map(|value| crate::ai::concepts::normalize_name(value)) {
        if alias != name && !aliases.contains(&alias) {
            aliases.push(alias);
        }
    }
    c.execute(
        "UPDATE knowledge_entities SET canonical_name=?1,description=?2,aliases_json=?3,status=?4,updated_at=?5 WHERE id=?6",
        rusqlite::params![name, description, serde_json::to_string(&aliases).unwrap_or_else(|_| "[]".into()), status, now_timestamp() as i64, id],
    )?;
    drop(stmt);
    drop(c);
    concept_entity_with_evidence(&db, &correction.id)
}

/// 合并两个实体。整个过程在一个事务里完成，关系两端统一改写并去重。
#[tauri::command]
pub fn merge_concept_entities(db: State<'_, Database>, request: MergeEntitiesRequest) -> Result<(), AppError> {
    merge_entities_impl(&db, &request.source_id, &request.target_id)
}

fn merge_entities_impl(db: &Database, source_id: &str, target_id: &str) -> Result<(), AppError> {
    if source_id == target_id {
        return Err(AppError::Message("不能把概念合并到它自己".into()));
    }
    let mut c = db.connect()?;
    let transaction = c.transaction()?;
    let mut stmt = transaction.prepare("SELECT canonical_name,description,coalesce(aliases_json,'[]') FROM knowledge_entities WHERE id=?1")?;
    let (source_name, source_description, source_aliases): (String, String, String) = stmt
        .query_row([source_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?
        .ok_or_else(|| AppError::Message("待合并的概念不存在".into()))?;
    let (target_name, _target_description, target_aliases): (String, String, String) = stmt
        .query_row([target_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()?
        .ok_or_else(|| AppError::Message("目标概念不存在".into()))?;
    drop(stmt);

    // 证据搬到目标实体：主键冲突时保留更高置信度的一条
    transaction.execute(
        "INSERT INTO knowledge_entity_evidence(entity_id,note_id,book_id,quote,confidence)
         SELECT ?1,note_id,book_id,quote,confidence FROM knowledge_entity_evidence WHERE entity_id=?2
         ON CONFLICT(entity_id,note_id) DO UPDATE SET quote=excluded.quote,
           confidence=max(knowledge_entity_evidence.confidence,excluded.confidence)",
        rusqlite::params![target_id, source_id],
    )?;
    transaction.execute("DELETE FROM knowledge_entity_evidence WHERE entity_id=?1", [source_id])?;

    // 关系两端改写到目标，并处理合并后出现的自环与重复边
    transaction.execute("UPDATE OR IGNORE knowledge_relations SET from_entity_id=?1 WHERE from_entity_id=?2", rusqlite::params![target_id, source_id])?;
    transaction.execute("UPDATE OR IGNORE knowledge_relations SET to_entity_id=?1 WHERE to_entity_id=?2", rusqlite::params![target_id, source_id])?;
    transaction.execute("DELETE FROM knowledge_relations WHERE from_entity_id=to_entity_id", [])?;
    // 同一对实体同一关系类型只保留一条。
    // 端点要先归一化：合并后可能出现 2→3 与 3→2 两条反向重复边。
    transaction.execute(
        "DELETE FROM knowledge_relations WHERE rowid NOT IN (
           SELECT min(rowid) FROM knowledge_relations GROUP BY
             CASE WHEN from_entity_id < to_entity_id THEN from_entity_id ELSE to_entity_id END,
             CASE WHEN from_entity_id < to_entity_id THEN to_entity_id ELSE from_entity_id END,
             relation
         )",
        [],
    )?;

    // 目标实体的别名吸收来源的名称与别名
    let mut aliases: Vec<String> = serde_json::from_str(&target_aliases).unwrap_or_default();
    for alias in std::iter::once(source_name.clone()).chain(serde_json::from_str::<Vec<String>>(&source_aliases).unwrap_or_default()) {
        let alias = alias.trim().to_owned();
        if !alias.is_empty() && alias != target_name && !aliases.contains(&alias) {
            aliases.push(alias);
        }
    }
    transaction.execute(
        "UPDATE knowledge_entities SET aliases_json=?1,description=coalesce(nullif(?2,''),description),updated_at=?3 WHERE id=?4",
        rusqlite::params![serde_json::to_string(&aliases).unwrap_or_else(|_| "[]".into()), source_description, now_timestamp() as i64, target_id],
    )?;
    transaction.execute("DELETE FROM knowledge_entities WHERE id=?1", [source_id])?;
    transaction.commit()?;
    Ok(())
}

/// 清理所有 suggested 状态的实体（用户主动触发）。
#[tauri::command]
pub fn clear_suggested_concepts(db: State<'_, Database>) -> Result<i64, AppError> {
    clear_suggested_concepts_impl(&db)
}

fn clear_suggested_concepts_impl(db: &Database) -> Result<i64, AppError> {
    let mut connection = db.connect()?;
    let transaction = connection.transaction()?;
    // knowledge_relations 没有指向 knowledge_entities 的外键，
    // 只删实体会把关系行留下来；下次扫描时同 id 实体重建，这些陈旧关系
    // 会带着旧的 summary/confidence 重新挂回图谱。必须先删两端待清理实体的关系。

    transaction.execute(
        "DELETE FROM knowledge_relations WHERE from_entity_id IN (SELECT id FROM knowledge_entities WHERE status='suggested')
            OR to_entity_id IN (SELECT id FROM knowledge_entities WHERE status='suggested')",
        [],
    )?;
    let removed = transaction.execute("DELETE FROM knowledge_entities WHERE status='suggested'", [])?;
    transaction.commit()?;
    Ok(removed as i64)
}

/// 扫描全部 / 仅变化内容，抽取概念与关系。
#[tauri::command]
pub async fn scan_concepts(db: State<'_, Database>, request: ConceptScanRequest) -> Result<ConceptScanResult, AppError> {
    let provider = ai_provider(&db)?;
    let books = notes_grouped_by_book(&db, request.book_id.as_deref().filter(|value| !value.trim().is_empty()))?;
    let glossary = glossary_candidates(&db)?;
    let known = known_entity_names(&db)?;
    let mut result = ConceptScanResult {
        books_scanned: 0, books_failed: 0, notes_scanned: 0, notes_skipped: 0,
        entities_created: 0, relations_created: 0, rejected: 0, failures: Vec::new(),
    };
    if books.is_empty() {
        return Ok(result);
    }
    for book in books {
        // 每本书独立处理：任何一本书失败都不影响其他书，也不回滚已完成的
        let skip = match unchanged_notes(&db, &book.notes) {
            Ok(skip) => skip,
            Err(error) => {
                result.books_failed += 1;
                result.failures.push(format!("《{}》读取抽取缓存失败：{error}", book.book_id));
                continue;
            }
        };
        let pending: Vec<(String, String)> = if request.changed_only {
            book.notes.iter().filter(|(id, _)| !skip.contains(id)).cloned().collect()
        } else {
            book.notes.clone()
        };
        result.notes_skipped += skip.len() as i64;
        if pending.is_empty() {
            continue;
        }
        let candidates = crate::ai::concepts::extract_candidates(
            &pending.iter().map(|(id, content)| (id.as_str(), content.as_str())).collect::<Vec<_>>(),
            &glossary,
            60,
        );
        // 证据校验用的真实笔记集合：note_id, book_id, book_title, content
        let real_notes: Vec<(String, String, String, String)> = pending
            .iter()
            .map(|(id, content)| (id.clone(), book.book_id.clone(), book.book_id.clone(), content.clone()))
            .collect();
        let system = "你是阅读笔记的概念抽取器。只输出严格 JSON，不要任何解释或 Markdown。格式：{\"entities\":[{\"kind\":\"concept|topic|idea\",\"canonical_name\":\"概念名\",\"description\":\"一句话定义\",\"aliases\":[\"同义词\"],\"confidence\":0.0-1.0,\"evidence_note_ids\":[\"笔记ID\"]}],\"relations\":[{\"from\":\"概念名\",\"to\":\"概念名\",\"relation\":\"broader|narrower|related|supports|conflicts|causes|applies\",\"summary\":\"一句话\",\"confidence\":0.0-1.0,\"evidence_note_ids\":[\"笔记ID\"]}]}。每个实体至少给一条真实存在的笔记 ID 作为证据；不要编造 ID。";
        let mut entities_created = 0;
        let mut relations_created = 0;
        for batch in crate::ai::concepts::batch_notes(&pending) {
            let body = batch
                .iter()
                .map(|(id, content)| format!("[{}] {}", id, content.chars().take(crate::ai::concepts::MAX_NOTE_CHARS).collect::<String>()))
                .collect::<Vec<_>>()
                .join("\n");
            // 名词库条目单独列出，让 AI 优先复用已有规范名而不是另起一个。
            let glossary_hint = if candidates.glossary_terms.is_empty() {
                "（名词库为空）".to_string()
            } else {
                candidates
                    .glossary_terms
                    .values()
                    .cloned()
                    .collect::<std::collections::HashSet<_>>()
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join("、")
            };
            let prompt = format!(
                "已确认名词库条目（优先复用这些规范名）：\n{glossary_hint}\n\n高频候选词（只作参考，不要强行全用）：\n{}\n\n以下是笔记：\n{}\n\n请抽取概念、主题与用户观点，以及它们之间的关系。",
                candidates.frequent_phrases.join("、"),
                body
            );
            let messages = vec![ChatMessage { role: "system".into(), content: system.into() }, ChatMessage { role: "user".into(), content: prompt }];
            let response = match provider.chat(&messages).await {
                Ok(response) => response,
                Err(error) => {
                    result.books_failed += 1;
                    result.failures.push(format!("《{}》：{error}", book.book_id));
                    break;
                }
            };
            let raw = match parse_extraction(&response) {
                Ok(raw) => raw,
                Err(error) => {
                    result.books_failed += 1;
                    result.failures.push(format!("《{}》：{error}", book.book_id));
                    break;
                }
            };
            let validated = crate::ai::concepts::validate_extraction(&raw, &real_notes, &known);
            result.rejected += (validated.rejected_without_evidence + validated.rejected_unknown_note + validated.rejected_unknown_endpoint + validated.rejected_bad_kind) as i64;
            if validated.entities.is_empty() && validated.relations.is_empty() {
                continue;
            }
            let scanned: Vec<(String, String)> = batch
                .iter()
                .filter(|(id, _)| real_notes.iter().any(|(note_id, ..)| note_id == id))
                .cloned()
                .collect();
            match persist_extraction(&db, &validated, &scanned) {
                Ok((entities, relations)) => {
                    entities_created += entities;
                    relations_created += relations;
                    result.notes_scanned += batch.len() as i64;
                }
                Err(error) => result.failures.push(format!("《{}》写入失败：{error}", book.book_id)),
            }
        }
        result.entities_created += entities_created;
        result.relations_created += relations_created;
        result.books_scanned += 1;
    }
    Ok(result)
}

#[tauri::command]
pub async fn test_reranker(
    settings: RerankerSettings,
    api_key: Option<String>,
) -> Result<bool, AppError> {
    let api_key = match api_key.filter(|value| !value.trim().is_empty()) {
        Some(value) => value,
        None => {
            keyring::Entry::new("ReadFlow", &crate::ai::reranker::credential_name(&settings.provider))?
                .get_password()?
        }
    };
    crate::ai::reranker::test_connection(&reqwest::Client::new(), &settings, &api_key).await
}

#[tauri::command]
pub fn save_embedding_settings(db: State<'_, Database>, settings: EmbeddingSettings, api_key: Option<String>) -> Result<(), AppError> {
    let url = reqwest::Url::parse(&settings.endpoint).map_err(|_| AppError::Message("Embedding Endpoint 格式无效".into()))?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if url.scheme() != "https" && !(local && url.scheme() == "http") { return Err(AppError::Message("Embedding Endpoint 必须使用 HTTPS".into())); }
    if settings.provider.trim().is_empty() || settings.model.trim().is_empty() { return Err(AppError::Message("Embedding Provider 和模型不能为空".into())); }
    if let Some(value) = api_key.filter(|value| !value.trim().is_empty()) { keyring::Entry::new("ReadFlow", &format!("embedding:{}", settings.provider))?.set_password(&value)?; }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    db.connect()?.execute("INSERT INTO embedding_settings(id,provider,endpoint,model,updated_at) VALUES(1,?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET provider=excluded.provider,endpoint=excluded.endpoint,model=excluded.model,updated_at=excluded.updated_at", rusqlite::params![settings.provider,settings.endpoint,settings.model,now])?;
    Ok(())
}

#[tauri::command]
pub async fn test_embedding(db: State<'_, Database>) -> Result<bool, AppError> {
    let settings = embedding_settings(&db)?;
    let provider = ai_provider_for_name(&db, &settings.provider)?;
    provider.embed(&settings.endpoint, &settings.model, &["测试文本".into()]).await?;
    Ok(true)
}

fn embedding_settings(db: &Database) -> Result<EmbeddingSettings, AppError> {
    db.connect()?.query_row("SELECT provider,endpoint,model FROM embedding_settings WHERE id=1", [], |row| Ok(EmbeddingSettings { provider: row.get(0)?, endpoint: row.get(1)?, model: row.get(2)? })).optional()?.ok_or_else(|| AppError::Message("请先在设置中配置远程 Embedding 服务".into()))
}

fn ai_provider_for_name(_db: &Database, provider_name: &str) -> Result<crate::ai::providers::OpenAiCompatibleProvider, AppError> {
    let api_key = keyring::Entry::new("ReadFlow", &format!("embedding:{provider_name}")).and_then(|entry| entry.get_password()).or_else(|_| keyring::Entry::new("ReadFlow", &format!("ai:{provider_name}")).and_then(|entry| entry.get_password())).map_err(|_| AppError::Message("未找到 Embedding API Key，请在设置中填写并保存".into()))?;
    Ok(crate::ai::providers::OpenAiCompatibleProvider { endpoint: String::new(), model: String::new(), api_key, http: reqwest::Client::new() })
}

fn cosine(left: &[f32], right: &[f32]) -> f64 {
    if left.len() != right.len() || left.is_empty() { return 0.0; }
    let mut dot = 0.0f64; let mut a = 0.0f64; let mut b = 0.0f64;
    for (x, y) in left.iter().zip(right) { let x = *x as f64; let y = *y as f64; dot += x * y; a += x * x; b += y * y; }
    if a == 0.0 || b == 0.0 { 0.0 } else { dot / (a.sqrt() * b.sqrt()) }
}

fn local_model_dir(app: &AppHandle) -> Result<std::path::PathBuf, AppError> {
    Ok(app.path().app_data_dir().map_err(|error| AppError::Message(error.to_string()))?.join("models").join("bge-small-zh-v1.5"))
}

fn directory_size(path: &std::path::Path) -> u64 {
    std::fs::read_dir(path).ok().into_iter().flatten().flatten().map(|entry| { let path = entry.path(); if path.is_dir() { directory_size(&path) } else { entry.metadata().map(|value| value.len()).unwrap_or(0) } }).sum()
}

#[tauri::command]
pub fn local_embedding_status(app: AppHandle) -> Result<LocalModelStatus, AppError> {
    let path = local_model_dir(&app)?; let size_bytes = directory_size(&path);
    Ok(LocalModelStatus { installed: size_bytes > 10_000_000, size_bytes, model: "BAAI/bge-small-zh-v1.5".into() })
}

#[tauri::command]
pub async fn download_local_embedding(app: AppHandle) -> Result<LocalModelStatus, AppError> {
    let path = local_model_dir(&app)?; std::fs::create_dir_all(&path).map_err(|error| AppError::Message(error.to_string()))?;
    app.emit("local-embedding-progress", serde_json::json!({"stage":"downloading","source":"国内镜像"})).ok();
    let download_path = path.clone();
    let result = tokio::task::spawn_blocking(move || {
        std::env::set_var("HF_ENDPOINT", "https://hf-mirror.com");
        fastembed::TextEmbedding::try_new(fastembed::TextInitOptions::new(fastembed::EmbeddingModel::BGESmallZHV15).with_cache_dir(download_path.clone()).with_show_download_progress(false))
            .or_else(|_| { std::env::remove_var("HF_ENDPOINT"); fastembed::TextEmbedding::try_new(fastembed::TextInitOptions::new(fastembed::EmbeddingModel::BGESmallZHV15).with_cache_dir(download_path).with_show_download_progress(false)) })
            .map(|_| ()).map_err(|error| error.to_string())
    }).await.map_err(|error| AppError::Message(error.to_string()))?;
    result.map_err(|error| AppError::Message(format!("本地模型下载失败：{error}")))?;
    app.emit("local-embedding-progress", serde_json::json!({"stage":"complete"})).ok();
    local_embedding_status(app)
}

#[tauri::command]
pub fn delete_local_embedding(app: AppHandle) -> Result<(), AppError> {
    let path = local_model_dir(&app)?;
    if path.exists() { std::fs::remove_dir_all(&path).map_err(|error| AppError::Message(format!("删除模型失败：{error}")))?; }
    Ok(())
}

#[tauri::command]
pub async fn build_semantic_relations(app: AppHandle, db: State<'_, Database>) -> Result<Vec<SemanticRelation>, AppError> {
    let local_path = local_model_dir(&app)?;
    let use_local = directory_size(&local_path) > 10_000_000;
    let settings = if use_local { None } else { Some(embedding_settings(&db)?) };
    let provider = settings.as_ref().map(|value| ai_provider_for_name(&db, &value.provider)).transpose()?;
    let model_name = settings.as_ref().map(|value| value.model.as_str()).unwrap_or("local:bge-small-zh-v1.5");
    let notes = {
        let c = db.connect()?;
        let mut query = c.prepare("SELECT h.bookmark_id,'highlight',h.book_id,b.title,coalesce(h.chapter_title,''),h.mark_text,'' FROM highlights h JOIN books b ON b.book_id=h.book_id WHERE h.is_deleted=0 AND b.is_deleted=0 UNION ALL SELECT t.review_id,'thought',t.book_id,b.title,coalesce(t.chapter_name,''),t.content,'' FROM thoughts t JOIN books b ON b.book_id=t.book_id WHERE t.is_deleted=0 AND b.is_deleted=0")?;
        let values = query.query_map([], |r| Ok(Note { id:r.get(0)?,note_type:r.get(1)?,book_id:r.get(2)?,book_title:r.get(3)?,chapter:r.get(4)?,content:r.get(5)?,created_at:r.get(6)? }))?.collect::<Result<Vec<_>,_>>()?;
        values
    };
    if notes.len() < 2 { return Ok(Vec::new()); }
    let mut vectors: HashMap<String, Vec<f32>> = HashMap::new();
    let mut missing = Vec::new();
    {
        let c = db.connect()?;
        for note in &notes {
            let text = format!("书名：{}\n章节：{}\n{}", note.book_title, note.chapter, note.content.chars().take(1600).collect::<String>());
            let hash = format!("{:x}", md5::compute(format!("{}|{}", model_name, text).as_bytes()));
            let cached: Option<String> = c.query_row("SELECT vector_json FROM note_embeddings WHERE note_id=?1 AND content_hash=?2 AND model=?3", rusqlite::params![note.id,hash,model_name], |row| row.get(0)).optional()?;
            if let Some(json) = cached { vectors.insert(note.id.clone(), serde_json::from_str(&json)?); } else { missing.push((note.id.clone(), note.book_id.clone(), hash, text)); }
        }
    }
    let local_vectors = if use_local && !missing.is_empty() {
        let input = missing.iter().map(|item| item.3.clone()).collect::<Vec<_>>(); let path = local_path.clone();
        Some(tokio::task::spawn_blocking(move || { let mut model = fastembed::TextEmbedding::try_new(fastembed::TextInitOptions::new(fastembed::EmbeddingModel::BGESmallZHV15).with_cache_dir(path).with_show_download_progress(false)).map_err(|error| error.to_string())?; model.embed(input, Some(32)).map_err(|error| error.to_string()) }).await.map_err(|error| AppError::Message(error.to_string()))?.map_err(|error| AppError::Message(format!("本地 Embedding 失败：{error}")))?)
    } else { None };
    for (batch_index, batch) in missing.chunks(32).enumerate() {
        let embedded = if let Some(values) = &local_vectors { values[batch_index * 32..(batch_index * 32 + batch.len())].to_vec() } else { let input = batch.iter().map(|item| item.3.clone()).collect::<Vec<_>>(); let settings = settings.as_ref().expect("remote settings"); provider.as_ref().expect("remote provider").embed(&settings.endpoint, &settings.model, &input).await? };
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
        let mut c = db.connect()?; let tx = c.transaction()?;
        for ((id, book_id, hash, _), vector) in batch.iter().zip(embedded) {
            tx.execute("INSERT INTO note_embeddings(note_id,book_id,content_hash,model,vector_json,updated_at) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(note_id) DO UPDATE SET book_id=excluded.book_id,content_hash=excluded.content_hash,model=excluded.model,vector_json=excluded.vector_json,updated_at=excluded.updated_at", rusqlite::params![id,book_id,hash,model_name,serde_json::to_string(&vector)?,now])?;
            vectors.insert(id.clone(), vector);
        }
        tx.commit()?;
    }
    let mut by_book: HashMap<String, Vec<&Note>> = HashMap::new();
    for note in &notes { if vectors.contains_key(&note.id) { by_book.entry(note.book_id.clone()).or_default().push(note); } }
    let mut book_vectors: HashMap<String, Vec<f32>> = HashMap::new();
    for (book_id, items) in &by_book {
        let Some(first) = items.first().and_then(|note| vectors.get(&note.id)) else { continue; };
        let mut mean = vec![0.0f32; first.len()];
        for note in items { if let Some(vector) = vectors.get(&note.id) { for (value, component) in mean.iter_mut().zip(vector) { *value += component; } } }
        for value in &mut mean { *value /= items.len() as f32; }
        book_vectors.insert(book_id.clone(), mean);
    }
    let ids = book_vectors.keys().cloned().collect::<Vec<_>>(); let mut candidates = Vec::new();
    for a in 0..ids.len() { for b in a+1..ids.len() {
        let score = cosine(&book_vectors[&ids[a]], &book_vectors[&ids[b]]); if score < 0.35 { continue; }
        let best = |book_id: &String, target: &Vec<f32>| by_book[book_id].iter().max_by(|x,y| cosine(&vectors[&x.id],target).total_cmp(&cosine(&vectors[&y.id],target))).copied();
        let evidence = [best(&ids[a], &book_vectors[&ids[b]]), best(&ids[b], &book_vectors[&ids[a]])].into_iter().flatten().map(|note| SemanticEvidence { book_id:note.book_id.clone(),note_id:note.id.clone(),text:note.content.chars().take(180).collect() }).collect();
        candidates.push(SemanticRelation { id:format!("semantic:{}:{}",ids[a],ids[b]),from:ids[a].clone(),to:ids[b].clone(),score,keywords:Vec::new(),relation:if score >= 0.72 { "高度语义相关".into() } else if score >= 0.52 { "语义相关".into() } else { "潜在语义关联".into() },evidence });
    }}
    let mut thought_vectors: HashMap<String, Vec<f32>> = HashMap::new();
    for (book_id, items) in &by_book {
        let thoughts = items.iter().filter(|note| note.note_type == "thought").collect::<Vec<_>>();
        let Some(first) = thoughts.first().and_then(|note| vectors.get(&note.id)) else { continue; };
        let mut mean = vec![0.0f32; first.len()];
        for note in &thoughts { if let Some(vector) = vectors.get(&note.id) { for (value, component) in mean.iter_mut().zip(vector) { *value += component; } } }
        for value in &mut mean { *value /= thoughts.len() as f32; }
        thought_vectors.insert(book_id.clone(), mean);
    }
    let thought_ids = thought_vectors.keys().cloned().collect::<Vec<_>>();
    for a in 0..thought_ids.len() { for b in a+1..thought_ids.len() {
        let left=&thought_ids[a]; let right=&thought_ids[b]; let score=cosine(&thought_vectors[left],&thought_vectors[right]); if score<0.48 { continue; }
        let best=|book_id:&String,target:&Vec<f32>| by_book[book_id].iter().filter(|note|note.note_type=="thought").max_by(|x,y|cosine(&vectors[&x.id],target).total_cmp(&cosine(&vectors[&y.id],target))).copied();
        let evidence=[best(left,&thought_vectors[right]),best(right,&thought_vectors[left])].into_iter().flatten().map(|note|SemanticEvidence{book_id:note.book_id.clone(),note_id:note.id.clone(),text:note.content.chars().take(180).collect()}).collect();
        candidates.push(SemanticRelation{id:format!("semantic:thought:{left}:{right}"),from:left.clone(),to:right.clone(),score,keywords:Vec::new(),relation:"用户观点关联".into(),evidence});
    }}
    candidates.sort_by(|a,b| b.score.total_cmp(&a.score)); let mut degree: HashMap<String,usize> = HashMap::new();
    Ok(candidates.into_iter().filter(|edge| { if degree.get(&edge.from).copied().unwrap_or(0)>=12 || degree.get(&edge.to).copied().unwrap_or(0)>=12 { return false; } *degree.entry(edge.from.clone()).or_default()+=1; *degree.entry(edge.to.clone()).or_default()+=1; true }).collect())
}

#[tauri::command]
pub fn build_metadata_relations(db: State<'_, Database>, source: String) -> Result<Vec<SemanticRelation>, AppError> {
    if !matches!(source.as_str(), "weread" | "douban") {
        return Err(AppError::Message("仅支持微信读书或豆瓣元数据关系".into()));
    }
    let c = db.connect()?;
    let mut query = c.prepare("SELECT book_id,coalesce(authors_json,'[]'),coalesce(subjects_json,'[]'),coalesce(raw_json,'{}') FROM book_metadata_sources WHERE source=?1")?;
    let rows = query.query_map([&source], |row| {
        let authors: String = row.get(1)?;
        let subjects: String = row.get(2)?;
        let raw: String = row.get(3)?;
        let raw: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
        let translators: Vec<String> = raw.get("translator").map(|value| if let Some(values)=value.as_array(){values.iter().filter_map(Value::as_str).map(str::to_owned).collect()}else{value.as_str().map(|text|text.split(['、',',','/']).map(str::trim).filter(|part|!part.is_empty()).map(str::to_owned).collect()).unwrap_or_default()}).unwrap_or_default();
        Ok((row.get::<_, String>(0)?, serde_json::from_str::<Vec<String>>(&authors).unwrap_or_default(), serde_json::from_str::<Vec<String>>(&subjects).unwrap_or_default(), translators))
    })?.collect::<Result<Vec<_>, _>>()?;
    let mut postings: HashMap<String, Vec<String>> = HashMap::new();
    for (book_id, authors, subjects, translators) in rows {
        for author in authors.into_iter().filter(|value| !value.trim().is_empty()) {
            postings.entry(format!("author:{}", author.trim().to_lowercase())).or_default().push(book_id.clone());
        }
        for subject in subjects.into_iter().filter(|value| !value.trim().is_empty()) {
            postings.entry(format!("subject:{}", subject.trim().to_lowercase())).or_default().push(book_id.clone());
        }
        for translator in translators.into_iter().filter(|value| !value.trim().is_empty()) {
            postings.entry(format!("translator:{}", translator.trim().to_lowercase())).or_default().push(book_id.clone());
        }
    }
    let mut pairs: HashMap<(String, String), (Vec<String>, Vec<String>, Vec<String>)> = HashMap::new();
    for (term, mut ids) in postings {
        ids.sort(); ids.dedup();
        if ids.len() < 2 || ids.len() > 80 { continue; }
        let is_author = term.starts_with("author:");
        let is_translator = term.starts_with("translator:");
        let label = term.split_once(':').map(|(_, value)| value.to_owned()).unwrap_or(term);
        for left in 0..ids.len() { for right in left + 1..ids.len() {
            let entry = pairs.entry((ids[left].clone(), ids[right].clone())).or_insert_with(|| (Vec::new(), Vec::new(), Vec::new()));
            let values = if is_author { &mut entry.1 } else if is_translator { &mut entry.2 } else { &mut entry.0 };
            if values.len() < 6 && !values.contains(&label) { values.push(label.clone()); }
        }}
    }
    let source_label = if source == "weread" { "微信读书" } else { "豆瓣" };
    let mut relations = pairs.into_iter().flat_map(|((from, to), (subjects, authors, translators))| {
        [("author", "共同作者", authors, 0.94), ("translator", "共同译者", translators, 0.9), ("subject", "共同主题", subjects.clone(), (0.48 + subjects.len() as f64 * 0.08).min(0.88))]
            .into_iter().filter(|(_, _, keywords, _)| !keywords.is_empty()).map(|(kind, label, keywords, score)| {
                let evidence_text = format!("来源：{source_label}；{label}：{}", keywords.join("、"));
                SemanticRelation { id: format!("metadata:{source}:{kind}:{from}:{to}"), from: from.clone(), to: to.clone(), score, keywords: keywords.clone(), relation: format!("{label} · {source_label}"), evidence: vec![SemanticEvidence { book_id: from.clone(), note_id: String::new(), text: evidence_text.clone() }, SemanticEvidence { book_id: to.clone(), note_id: String::new(), text: evidence_text }] }
            }).collect::<Vec<_>>()
    }).collect::<Vec<_>>();
    let mut version_query=c.prepare("SELECT book_id,coalesce(title,''),coalesce(isbn13,isbn10,'') FROM book_metadata_sources WHERE source=?1")?;
    let versions=version_query.query_map([&source],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?)))?.collect::<Result<Vec<_>,_>>()?;
    let mut works:HashMap<String,Vec<(String,String,String)>>=HashMap::new();
    for (book_id,title,isbn) in versions { let key=title.to_lowercase().replace(['《','》',' '],"").split(['（','(',':','：']).next().unwrap_or("").to_owned(); if key.chars().count()>=3 { works.entry(key).or_default().push((book_id,title,isbn)); } }
    for items in works.into_values().filter(|items|items.len()>1&&items.len()<=12) { for left in 0..items.len(){for right in left+1..items.len(){
        if !items[left].2.is_empty()&&items[left].2==items[right].2 { continue; }
        let keywords=vec![items[left].1.clone(),items[right].1.clone()]; let evidence_text=format!("来源：{source_label}；同一作品的不同版本");
        relations.push(SemanticRelation{id:format!("metadata:{source}:version:{}:{}",items[left].0,items[right].0),from:items[left].0.clone(),to:items[right].0.clone(),score:0.86,keywords,relation:format!("同系列／不同版本 · {source_label}"),evidence:vec![SemanticEvidence{book_id:items[left].0.clone(),note_id:String::new(),text:evidence_text.clone()},SemanticEvidence{book_id:items[right].0.clone(),note_id:String::new(),text:evidence_text.clone()}]});
    }}}
    let mut intro_query=c.prepare("SELECT book_id,coalesce(description,'') FROM book_metadata_sources WHERE source=?1 AND length(coalesce(description,''))>=40")?;
    let intros=intro_query.query_map([&source],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?)))?.collect::<Result<Vec<_>,_>>()?;
    let mut intro_lengths:HashMap<String,usize>=HashMap::new(); let mut intro_postings:HashMap<String,Vec<String>>=HashMap::new();
    for (book_id,intro) in intros { let terms=semantic_bigrams(&intro).into_iter().collect::<HashSet<_>>(); intro_lengths.insert(book_id.clone(),terms.len()); for term in terms { intro_postings.entry(term).or_default().push(book_id.clone()); } }
    let mut intro_pairs:HashMap<(String,String),usize>=HashMap::new();
    for ids in intro_postings.into_values().filter(|ids|ids.len()>1&&ids.len()<=60) { for left in 0..ids.len(){for right in left+1..ids.len(){let pair=if ids[left]<ids[right]{(ids[left].clone(),ids[right].clone())}else{(ids[right].clone(),ids[left].clone())};*intro_pairs.entry(pair).or_default()+=1;}}}
    for ((from,to),shared) in intro_pairs { if shared<4 {continue;} let score=shared as f64/((intro_lengths[&from]*intro_lengths[&to]) as f64).sqrt(); if score<0.18 {continue;} let evidence_text=format!("来源：{source_label}；两本书的简介语义相似度 {}%",(score*100.0).round()); relations.push(SemanticRelation{id:format!("metadata:{source}:intro:{from}:{to}"),from:from.clone(),to:to.clone(),score:score.min(0.92),keywords:vec!["简介语义相似".into()],relation:format!("简介语义相似 · {source_label}"),evidence:vec![SemanticEvidence{book_id:from,note_id:String::new(),text:evidence_text.clone()},SemanticEvidence{book_id:to,note_id:String::new(),text:evidence_text}]}); }
    relations.sort_by(|left, right| right.score.total_cmp(&left.score));
    Ok(relations)
}

fn metadata_context(db: &Database, book_ids: &[String]) -> Result<String, AppError> {
    let c = db.connect()?;
    let mut query = c.prepare("SELECT source,coalesce(title,''),coalesce(authors_json,'[]'),coalesce(publisher,''),coalesce(published_date,''),coalesce(subjects_json,'[]'),coalesce(description,'') FROM book_metadata_sources WHERE book_id=?1 ORDER BY CASE source WHEN 'weread' THEN 0 WHEN 'douban' THEN 1 ELSE 9 END")?;
    let mut context = String::new();
    for book_id in book_ids.iter().take(20) {
        let rows = query.query_map([book_id], |row| Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,String>(3)?,row.get::<_,String>(4)?,row.get::<_,String>(5)?,row.get::<_,String>(6)?)))?;
        for row in rows {
            let (source,title,authors_json,publisher,date,subjects_json,description)=row?;
            if !matches!(source.as_str(), "weread" | "douban") { continue; }
            let source_label=if source=="weread"{"微信读书"}else{"豆瓣"};
            let authors=serde_json::from_str::<Vec<String>>(&authors_json).unwrap_or_default().join("、");
            let subjects=serde_json::from_str::<Vec<String>>(&subjects_json).unwrap_or_default().join("、");
            let intro=description.chars().take(600).collect::<String>();
            context.push_str(&format!("[元数据来源：{source_label}]\n书名：{title}\n作者：{authors}\n出版社：{publisher}\n出版时间：{date}\n主题：{subjects}\n简介：{intro}\n\n"));
        }
    }
    Ok(context)
}

fn glossary_context(db:&Database,text:&str)->Result<(String,Vec<GlossaryCitation>),AppError>{
    let c=db.connect()?;let mut q=c.prepare("SELECT term,canonical_name,aliases_json,definition,source,coalesce(source_url,'') FROM glossary_terms WHERE status='confirmed' ORDER BY updated_at DESC")?;
    let rows=q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?)))?;
    let normalized_text=text.to_lowercase();let mut result=String::new();let mut citations=Vec::new();let mut index=1;
    for row in rows {let(term,name,aliases,definition,source,url)=row?;let aliases=serde_json::from_str::<Vec<String>>(&aliases).unwrap_or_default();let matched=[term.as_str(),name.as_str()].into_iter().chain(aliases.iter().map(String::as_str)).filter(|value|!value.trim().is_empty()).any(|value|normalized_text.contains(&value.to_lowercase()));if !matched{continue;}let display_name=if name.is_empty(){term.clone()}else{name};let source_label=if source=="wikipedia"{"中文维基百科"}else{"人工编辑"};let ai_definition=definition.chars().take(8_000).collect::<String>();result.push_str(&format!("[W{index}] 名词：{}\n解释：{}\n来源：{}{}\n\n",display_name,ai_definition,source_label,if url.is_empty(){String::new()}else{format!("（{url}）")}));citations.push(GlossaryCitation{index,term:display_name,definition,source:source_label.into(),source_url:url});index+=1;if index>8{break;}}
    Ok((result,citations))
}

#[tauri::command]
pub fn save_ai_settings(
    db: State<'_, Database>,
    settings: AiSettings,
    api_key: Option<String>,
) -> Result<(), AppError> {
    validate_ai_settings(&settings)?;
    if let Some(api_key) = api_key.filter(|value| !value.trim().is_empty()) {
        let entry = keyring::Entry::new("ReadFlow", &format!("ai:{}", settings.provider))?;
        entry.set_password(&api_key)?;
        if entry.get_password()? != api_key {
            return Err(AppError::Message("AI Key 未能正确保存到系统凭据库，请重试".into()));
        }
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    db.connect()?.execute("INSERT INTO ai_settings(id,provider,endpoint,model,updated_at) VALUES(1,?1,?2,?3,?4) ON CONFLICT(id) DO UPDATE SET provider=excluded.provider,endpoint=excluded.endpoint,model=excluded.model,updated_at=excluded.updated_at", rusqlite::params![settings.provider,settings.endpoint,settings.model,now])?;
    Ok(())
}

#[tauri::command]
pub async fn test_ai(db: State<'_, Database>, api_key: Option<String>) -> Result<bool, AppError> {
    let provider = ai_provider_with_key(&db, api_key)?;
    provider
        .chat(&[ChatMessage {
            role: "user".into(),
            content: "请只回复 OK".into(),
        }])
        .await?;
    Ok(true)
}

/// RAG 检索 + Prompt 组装的结果。
/// `ask_ai` 与 `ask_ai_stream` 共用这一份准备逻辑，保证两条路径的证据完全一致。
struct PreparedAsk {
    messages: Vec<ChatMessage>,
    results: Vec<SearchResult>,
    glossary_matches: Vec<GlossaryCitation>,
    /// 重排降级时的非阻断提示；启用并成功时为空。
    rerank_note: String,
    /// 导入资料里的证据，编号接在笔记之后。
    source_evidence: Vec<crate::rag::Evidence>,
}

async fn prepare_ask(db: &Database, request: &AiRequest) -> Result<PreparedAsk, AppError> {
    prepare_ask_with_cancel(db, request, &std::sync::atomic::AtomicBool::new(false)).await
}

async fn prepare_ask_with_cancel(
    db: &Database,
    request: &AiRequest,
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<PreparedAsk, AppError> {
    let question = &request.question;
    if !matches!(request.mode.as_str(), "ask" | "summary" | "compare") {
        return Err(AppError::Message("不支持的 AI 分析模式".into()));
    }
    if request.mode == "summary" && request.book_ids.len() != 1 {
        return Err(AppError::Message("单书总结必须选择一本书".into()));
    }
    if request.mode == "compare" && request.book_ids.len() < 2 {
        return Err(AppError::Message("跨书分析至少选择两本书".into()));
    }
    if request.mode == "compare" && request.book_ids.len() > 100 {
        return Err(AppError::Message("跨书分析最多选择 100 本书".into()));
    }
    if question.trim().is_empty() {
        return Err(AppError::Message("问题不能为空".into()));
    }
    let (results, rerank_note) = rag_search(db, question, &request.mode, &request.book_ids, cancelled).await?;
    let mut context = String::new();
    for (index, result) in results.iter().enumerate() {
        let source = if result.note.note_type == "thought" {
            "用户想法"
        } else {
            "书籍原文划线"
        };
        let content_limit = if request.mode == "ask" && results.len() > 100 {
            220
        } else if request.mode == "ask" && results.len() > 30 {
            700
        } else if request.mode == "compare" && request.book_ids.len() > 30 {
            420
        } else if request.mode == "compare" {
            700
        } else {
            1200
        };
        let content: String = result.note.content.chars().take(content_limit).collect();
        context.push_str(&format!(
            "[{}]\ntype: {}\nbook: 《{}》\nchapter: {}\ncontent: {}\n\n",
            index + 1,
            source,
            result.note.book_title,
            result.note.chapter,
            content
        ));
    }
    // 导入资料并入证据：编号接着笔记往后排，AI 的 [n] 引用不会与笔记混淆。
    let source_evidence = crate::rag::source_evidence(
        db,
        question,
        &search_terms(question),
        &request.book_ids,
        if request.mode == "ask" { 12 } else { 6 },
    )?;
    let source_start = results.len();
    for (offset, evidence) in source_evidence.iter().enumerate() {
        let content_limit = if request.mode == "ask" { 700 } else { 420 };
        context.push_str(&evidence.prompt_text(source_start + offset + 1, content_limit));
        context.push('\n');
    }
    let metadata_book_ids = if request.book_ids.is_empty() { results.iter().map(|result| result.note.book_id.clone()).collect::<HashSet<_>>().into_iter().collect::<Vec<_>>() } else { request.book_ids.clone() };
    let metadata = metadata_context(db, &metadata_book_ids)?;
    let (glossary, glossary_matches) = glossary_context(db,&format!("{}\n{}",question,context))?;
    // 笔记、导入资料、名词解释三者都为空才算没有依据
    if results.is_empty() && source_evidence.is_empty() && glossary.is_empty() {
        return Err(AppError::Message(
            "没有检索到相关笔记、导入资料或名词解释，无法生成有依据的回答".into(),
        ));
    }
    let system = "你是 wereader 的个人阅读知识助手。阅读笔记是观点回答的唯一证据；书籍元数据只能作为背景信息。必须区分书籍原文划线、用户自己的想法和元数据；不得把简介或主题当成用户观点或书中论证；不同元数据来源不得合并成一个事实。每个重要观点结论使用 [数字] 标注笔记来源；证据不足时必须明确说明。";
    let task = match request.mode.as_str() {
        "summary" => "任务类型：单书总结。提炼主题、核心观点和用户想法，不要逐条复述。",
        "compare" => "任务类型：跨书分析。明确列出各书的共识、分歧与可互相补充之处。",
        _ => "任务类型：基于阅读知识库回答问题。对于宽泛主题，应综合尽可能多的相关书籍与笔记，先说明知识库覆盖范围，再按主题组织回答，避免只围绕单本书展开。",
    };
    let prompt = format!(
        "{}\n\n以下是已确认的名词解释（仅作背景；引用格式为 [W数字]）：\n\n{}\n以下是按来源独立提供的书籍背景（不可作为观点证据）：\n\n{}\n以下是检索到的笔记证据：\n\n{}\n用户问题：{}",
        task,
        glossary,
        metadata,
        context,
        question.trim()
    );
    let mut messages = vec![ChatMessage { role: "system".into(), content: system.into() }];
    for turn in request.history.iter().rev().take(8).rev() {
        messages.push(ChatMessage { role: "user".into(), content: turn.question.chars().take(1200).collect() });
        messages.push(ChatMessage { role: "assistant".into(), content: turn.answer.chars().take(5000).collect() });
    }
    messages.push(ChatMessage { role: "user".into(), content: prompt });
    Ok(PreparedAsk { messages, results, glossary_matches, rerank_note, source_evidence })
}

/// 正文固定下来之后再决定引用哪些证据，所以两条路径共用这一个收尾函数。
fn finalize_answer(prepared: PreparedAsk, content: String) -> AiAnswer {
    let PreparedAsk { results, glossary_matches, rerank_note, source_evidence, .. } = prepared;
    let sources_considered = results.len();
    // 导入资料的引用编号接在笔记之后，与 prompt 中给出的编号保持一致
    let source_start = results.len();
    let citations = results
        .into_iter()
        .enumerate()
        .filter(|(index, _)| content.contains(&format!("[{}]", index + 1)))
        .map(|(index, result)| Citation {
            index: index + 1,
            note: result.note,
        })
        .collect();
    let glossary_citations = glossary_matches
        .into_iter()
        .filter(|citation| content.contains(&format!("[W{}]", citation.index)))
        .collect();
    // 导入资料的引用编号接在笔记之后，编号与 prompt 里给出的完全一致
    let source_citations = source_evidence
        .into_iter()
        .enumerate()
        .filter(|(offset, _)| content.contains(&format!("[{}]", source_start + offset + 1)))
        .map(|(offset, evidence)| SourceCitation {
            index: source_start + offset + 1,
            source_id: evidence.book_id().to_owned(),
            source_type: match &evidence {
                crate::rag::Evidence::Source { source_type, .. } => source_type.clone(),
                crate::rag::Evidence::Note { .. } => "note".to_owned(),
            },
            title: match &evidence {
                crate::rag::Evidence::Source { source_title, .. } => source_title.clone(),
                crate::rag::Evidence::Note { book_title, .. } => book_title.clone(),
            },
            locator: match &evidence {
                crate::rag::Evidence::Source { locator, .. } => locator.clone(),
                crate::rag::Evidence::Note { .. } => crate::import::Locator::default(),
            },
            quote: evidence.content().chars().take(200).collect(),
        })
        .collect();
    AiAnswer {
        content,
        citations,
        glossary_citations,
        sources_considered,
        rerank_note,
        source_citations,
    }
}

#[tauri::command]
pub async fn ask_ai(db: State<'_, Database>, request: AiRequest) -> Result<AiAnswer, AppError> {
    let prepared = prepare_ask(&db, &request).await?;
    let provider = ai_provider(&db)?;
    let content = provider.chat(&prepared.messages).await?;
    Ok(finalize_answer(prepared, content))
}

// ---------- 导入资料 ----------

/// 应用数据目录，用来存 EPUB 封面等副本。
fn app_data_dir(app: &AppHandle) -> Result<std::path::PathBuf, AppError> {
    app.path().app_data_dir().map_err(|error| AppError::Message(error.to_string()))
}

fn cover_file_name(source_id: &str, extension: &str) -> String {
    // 文件名带 source id：所有 EPUB 的封面都叫 cover.jpg 的话，
    // 删掉任意一份资料都会连带删掉别人的封面。
    let suffix: String = source_id.chars().filter(|c| c.is_ascii_alphanumeric()).take(36).collect();
    format!("cover-{}.{}", if suffix.is_empty() { "unknown".to_string() } else { suffix }, extension.trim_start_matches('.').to_lowercase())
}

/// 把解析结果写库。
fn persist_source(
    db: &Database,
    app: &AppHandle,
    parsed: &crate::import::ParsedSource,
) -> Result<String, AppError> {
    let kind = parsed.kind.ok_or_else(|| AppError::Message("无法识别资料类型".into()))?;
    let body: String = parsed.chunks.iter().map(|chunk| chunk.content.as_str()).collect::<Vec<_>>().join("\n");
    let hash = crate::import::content_hash(kind, &parsed.title, &body);
    let now = now_timestamp() as i64;
    let source_id = uuid::Uuid::new_v4().to_string();

    // 封面复制进应用数据目录，不依赖原文件持续存在
    let cover_path = match (parsed.cover.as_ref(), parsed.cover_extension.as_ref()) {
        (Some(bytes), Some(extension)) => {
            let directory = app_data_dir(app)?.join("imports");
            std::fs::create_dir_all(&directory).map_err(|error| AppError::Message(error.to_string()))?;
            let name = cover_file_name(&source_id, extension);
            std::fs::write(directory.join(&name), bytes).map_err(|error| AppError::Message(format!("保存封面失败：{error}")))?;
            Some(name)
        }
        _ => None,
    };

    let mut c = db.connect()?;
    let transaction = c.transaction()?;
    transaction.execute(
        "INSERT INTO library_sources(id,source_type,title,author,origin,local_path,content_hash,page_count,cover_path,imported_at,updated_at,is_deleted)
         VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?10,0)",
        rusqlite::params![source_id, kind.as_str(), parsed.title, parsed.author, parsed.origin, parsed.origin, hash, parsed.page_count as i64, cover_path, now],
    )?;
    {
        let mut statement = transaction.prepare("INSERT INTO source_documents(id,source_id,position,heading,content,locator_json) VALUES(?1,?2,?3,?4,?5,?6)")?;
        for chunk in &parsed.chunks {
            statement.execute(rusqlite::params![
                uuid::Uuid::new_v4().to_string(),
                source_id,
                chunk.position as i64,
                chunk.heading,
                chunk.content,
                serde_json::to_string(&chunk.locator).unwrap_or_else(|_| "{}".into()),
            ])?;
        }
    }
    transaction.commit()?;
    Ok(source_id)
}

/// 找出内容 hash 相同且未删除的既有资料。
fn find_duplicate(db: &Database, kind: crate::import::SourceKind, title: &str, body: &str) -> Result<Option<String>, AppError> {
    let hash = crate::import::content_hash(kind, title, body);
    db.connect()?
        .query_row("SELECT id FROM library_sources WHERE content_hash=?1 AND is_deleted=0 LIMIT 1", [hash], |r| r.get(0))
        .optional()
        .map_err(AppError::from)
}

fn preview_chunks(parsed: &crate::import::ParsedSource) -> Vec<SourceDocumentItem> {
    parsed
        .chunks
        .iter()
        .take(5)
        .map(|chunk| SourceDocumentItem {
            id: String::new(),
            position: chunk.position as i64,
            heading: chunk.heading.clone(),
            content: chunk.content.chars().take(400).collect(),
            locator: chunk.locator.clone(),
        })
        .collect()
}

fn parsed_body(parsed: &crate::import::ParsedSource) -> String {
    parsed.chunks.iter().map(|chunk| chunk.content.as_str()).collect::<Vec<_>>().join("\n")
}

/// 按 URL 抓取网页并返回预览。
///
/// 重复内容在这里就查出来：用户点确认之前就能看到「这份已经导入过」。
#[tauri::command]
pub async fn preview_web_import(db: State<'_, Database>, request: PreviewWebRequest) -> Result<ImportPreview, AppError> {
    let parsed = fetch_web_source(&request.url).await?;
    let duplicate_of = find_duplicate(&db, crate::import::SourceKind::Web, &parsed.title, &parsed_body(&parsed))?;
    Ok(ImportPreview {
        source_type: "web".into(),
        title: parsed.title.clone(),
        author: parsed.author.clone(),
        origin: parsed.origin.clone(),
        page_count: 0,
        sample: preview_chunks(&parsed),
        document_count: parsed.chunks.len() as i64,
        warnings: parsed.warnings.clone(),
        duplicate: duplicate_of.is_some(),
        duplicate_of,
        file_fingerprint: None,
    })
}

/// 抓取网页。每次重定向都重新校验目标地址，挡「公网 302 到内网」。
async fn fetch_web_source(input: &str) -> Result<crate::import::ParsedSource, AppError> {
    let mut url = crate::import::web::validate_url(input)?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(crate::import::web::WEB_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| AppError::Message(format!("网络客户端初始化失败：{error}")))?;

    // 自己处理重定向：reqwest 的自动跟随不会把目标地址交回来校验
    for _ in 0..=crate::import::web::MAX_REDIRECTS {
        let response = client
            .get(url.clone())
            .header("User-Agent", "wereader/0.2 (personal reading library)")
            .header("Accept", "text/html,application/xhtml+xml")
            .send()
            .await
            .map_err(|error| AppError::Message(format!("抓取失败：{error}")))?;

        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| AppError::Message("重定向响应没有 Location".into()))?;
            let next = url
                .join(location)
                .map_err(|_| AppError::Message("重定向地址无效".into()))?;
            crate::import::web::validate_redirect(&url, &next)?;
            // 下一轮仍然要走 HTTPS + 内网检查
            url = crate::import::web::validate_url(next.as_str())?;
            continue;
        }

        let status = response.status();
        if !status.is_success() {
            // 404 / 无结果不是系统错误，给出明确文案
            return Err(AppError::Message(match status.as_u16() {
                404 => "这个页面不存在（404）".into(),
                403 => "对方拒绝了抓取（403）".into(),
                429 => "请求过于频繁（429），请稍后再试".into(),
                code => format!("抓取失败（HTTP {code}）"),
            }));
        }
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if !crate::import::web::is_html_content_type(content_type) {
            return Err(AppError::Message(format!("这个链接不是网页（{content_type}），暂不支持该格式")));
        }
        // 先看 Content-Length，再边读边限制，避免大响应打爆内存
        if let Some(length) = response.content_length() {
            if length > crate::import::web::MAX_HTML_BYTES as u64 {
                return Err(AppError::Message("页面体积超过导入上限".into()));
            }
        }
        // reqwest 的 Response 没有实现 Read，用 chunk() 逐段读并累加限制。
        let mut body = Vec::new();
        let mut stream = response;
        while let Some(chunk) = stream.chunk().await.map_err(|error| AppError::Message(format!("读取页面失败：{error}")))? {
            if body.len() + chunk.len() > crate::import::web::MAX_HTML_BYTES {
                return Err(AppError::Message("页面体积超过导入上限".into()));
            }
            body.extend_from_slice(&chunk);
        }
        if body.len() >= crate::import::web::MAX_HTML_BYTES {
            return Err(AppError::Message("页面体积超过导入上限".into()));
        }
        let html = String::from_utf8_lossy(&body).to_string();
        let parsed = crate::import::web::build_parsed_source(&url, &html);
        crate::import::validate_size(crate::import::SourceKind::Web, &parsed.chunks)?;
        return Ok(parsed);
    }
    Err(AppError::Message("重定向次数过多，已中止抓取".into()))
}

/// 预览本地文件（PDF / EPUB）。先校验再解析。
#[tauri::command]
pub fn preview_file_import(db: State<'_, Database>, request: PreviewFileRequest) -> Result<ImportPreview, AppError> {
    let (parsed, fingerprint) = read_local_file(&request.path)?;
    let duplicate_of = find_duplicate(&db, parsed.kind.unwrap_or(crate::import::SourceKind::Web), &parsed.title, &parsed_body(&parsed))?;
    Ok(ImportPreview {
        source_type: parsed.kind.map(|kind| kind.as_str().to_owned()).unwrap_or_default(),
        title: parsed.title.clone(),
        author: parsed.author.clone(),
        origin: parsed.origin.clone(),
        page_count: parsed.page_count as i64,
        sample: preview_chunks(&parsed),
        document_count: parsed.chunks.len() as i64,
        warnings: parsed.warnings.clone(),
        duplicate: duplicate_of.is_some(),
        duplicate_of,
        file_fingerprint: Some(fingerprint),
    })
}

/// 本地文件（压缩后）的体积上限。解压后的规模另算，见 `epub::MAX_TOTAL_UNCOMPRESSED`。
pub const MAX_LOCAL_FILE_BYTES: u64 = 200 * 1024 * 1024;

/// 读本地文件并按类型分派，返回解析结果与内容指纹。
///
/// 校验顺序是刻意的：先看扩展名决定解析器 → 但真正能不能读，要等
/// 「存在 / 是普通文件 / 体积超限 / 内容签名对得上」全部过一遍才解析。
/// 大小检查放在 `read` 之前，否则一个 2 GB 的文件会先被整个读进内存。
fn read_local_file(input: &str) -> Result<(crate::import::ParsedSource, String), AppError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(AppError::Message("请选择要导入的 PDF 或 EPUB 文件".into()));
    }
    let path = std::path::Path::new(trimmed);

    let metadata = std::fs::metadata(path).map_err(|error| {
        // 不把原始 io 错误抛给用户，也不泄漏调用栈
        let _ = error;
        AppError::Message("文件不存在或无法读取，请重新选择".into())
    })?;
    if !metadata.is_file() {
        return Err(AppError::Message("这是一个文件夹，请选择具体的 PDF 或 EPUB 文件".into()));
    }

    let kind = path
        .extension()
        .and_then(|value| value.to_str())
        .and_then(crate::import::kind_from_extension)
        .ok_or_else(|| AppError::Message("请选择 PDF 或 EPUB 文件".into()))?;

    if metadata.len() > MAX_LOCAL_FILE_BYTES {
        return Err(AppError::Message(format!(
            "{} 体积超过 {} MB 的导入上限",
            kind.label(),
            MAX_LOCAL_FILE_BYTES / 1024 / 1024
        )));
    }

    let bytes = std::fs::read(path).map_err(|_| AppError::Message("文件不存在或无法读取，请重新选择".into()))?;
    // 读进来之后再确认一次大小：metadata 可能是符号链接或被换过的文件
    if bytes.len() as u64 > MAX_LOCAL_FILE_BYTES {
        return Err(AppError::Message(format!(
            "{} 体积超过 {} MB 的导入上限",
            kind.label(),
            MAX_LOCAL_FILE_BYTES / 1024 / 1024
        )));
    }
    crate::import::check_file_signature(kind, &bytes)?;

    let fingerprint = crate::import::file_fingerprint(&bytes);
    let mut parsed = match kind {
        crate::import::SourceKind::Pdf => crate::import::pdf::parse(bytes)?,
        crate::import::SourceKind::Epub => crate::import::epub::parse(bytes)?,
        crate::import::SourceKind::Web => return Err(AppError::Message("网页不能作为本地文件导入".into())),
    };
    parsed.origin = Some(path.to_string_lossy().to_string());
    crate::import::validate_size(kind, &parsed.chunks)?;
    Ok((parsed, fingerprint))
}

/// 用户确认后正式入库。网页会重新抓一次（本地文件会重新读一次），
/// 预览与确认之间内容可能已经变了，所以每次确认都重新解析、重新去重。
#[tauri::command]
pub async fn confirm_import(app: AppHandle, db: State<'_, Database>, request: ConfirmImportRequest) -> Result<ImportPreview, AppError> {
    let mut fingerprint: Option<String> = None;
    let parsed = match request.source_type.as_str() {
        "web" => {
            let url = request.url.clone().ok_or_else(|| AppError::Message("缺少网页地址".into()))?;
            let mut parsed = fetch_web_source(&url).await?;
            parsed.title = request.title.clone();
            parsed.author = request.author.clone();
            parsed.origin = request.origin_hint();
            parsed
        }
        "pdf" | "epub" => {
            let path = request.path.clone().ok_or_else(|| AppError::Message("缺少文件路径".into()))?;
            let (mut parsed, current) = read_local_file(&path)?;
            // 预览时的指纹对不上：文件在预览之后被换掉或改写了，
            // 不能拿旧预览的标题把新内容入库。
            if let Some(expected) = request.file_fingerprint.as_deref() {
                if !expected.is_empty() && expected != current {
                    return Err(AppError::Message("这个文件在预览之后发生了变化，请重新预览后再导入".into()));
                }
            }
            fingerprint = Some(current);
            parsed.title = request.title.clone();
            parsed.author = request.author.clone();
            // 用户改了标题时 origin 仍按实际来源记录
            parsed.origin = request.origin_hint();
            parsed
        }
        other => return Err(AppError::Message(format!("不支持的来源类型：{other}"))),
    };
    let kind = parsed.kind.ok_or_else(|| AppError::Message("无法识别资料类型".into()))?;
    let body = parsed_body(&parsed);
    if let Some(existing) = find_duplicate(&db, kind, &parsed.title, &body)? {
        // 重复内容不重复入库，把已有资料指回去
        return Ok(ImportPreview {
            source_type: kind.as_str().to_owned(),
            title: parsed.title.clone(),
            author: parsed.author.clone(),
            origin: parsed.origin.clone(),
            page_count: parsed.page_count as i64,
            sample: preview_chunks(&parsed),
            document_count: parsed.chunks.len() as i64,
            warnings: parsed.warnings,
            duplicate: true,
            duplicate_of: Some(existing),
            file_fingerprint: fingerprint,
        });
    }
    let id = persist_source(&db, &app, &parsed)?;
    Ok(ImportPreview {
        source_type: kind.as_str().to_owned(),
        title: parsed.title.clone(),
        author: parsed.author.clone(),
        origin: parsed.origin.clone(),
        page_count: parsed.page_count as i64,
        sample: preview_chunks(&parsed),
        document_count: parsed.chunks.len() as i64,
        warnings: parsed.warnings,
        duplicate: false,
        duplicate_of: Some(id),
        file_fingerprint: fingerprint,
    })
}

fn list_sources_impl(db: &Database, include_deleted: bool) -> Result<Vec<LibrarySource>, AppError> {
    let c = db.connect()?;
    let mut statement = c.prepare(
        "SELECT s.id,s.source_type,s.title,s.author,s.origin,s.page_count,s.cover_path,
                (SELECT count(*) FROM source_documents d WHERE d.source_id=s.id),s.imported_at,s.is_deleted
         FROM library_sources s
         WHERE (?1=1 OR s.is_deleted=0)
         ORDER BY s.imported_at DESC",
    )?;
    let rows = statement.query_map([include_deleted as i64], |r| {
        Ok(LibrarySource {
            id: r.get(0)?,
            source_type: r.get(1)?,
            title: r.get(2)?,
            author: r.get(3)?,
            origin: r.get(4)?,
            page_count: r.get(5)?,
            cover: r.get(6)?,
            document_count: r.get(7)?,
            imported_at: r.get::<_, i64>(8)?.to_string(),
            deleted: r.get::<_, i64>(9)? != 0,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// 导入资料列表。
#[tauri::command]
pub fn list_library_sources(db: State<'_, Database>, include_deleted: Option<bool>) -> Result<Vec<LibrarySource>, AppError> {
    list_sources_impl(&db, include_deleted.unwrap_or(false))
}

fn source_detail_impl(db: &Database, source_id: &str) -> Result<SourceDetail, AppError> {
    let c = db.connect()?;
    let mut statement = c.prepare(
        "SELECT s.id,s.source_type,s.title,s.author,s.origin,s.page_count,s.cover_path,
                (SELECT count(*) FROM source_documents d WHERE d.source_id=s.id),s.imported_at,s.is_deleted
         FROM library_sources s WHERE s.id=?1",
    )?;
    let source = statement
        .query_row([source_id], |r| {
            Ok(LibrarySource {
                id: r.get(0)?,
                source_type: r.get(1)?,
                title: r.get(2)?,
                author: r.get(3)?,
                origin: r.get(4)?,
                page_count: r.get(5)?,
                cover: r.get(6)?,
                document_count: r.get(7)?,
                imported_at: r.get::<_, i64>(8)?.to_string(),
                deleted: r.get::<_, i64>(9)? != 0,
            })
        })
        .optional()?
        .ok_or_else(|| AppError::Message("资料不存在".into()))?;
    let mut documents = c.prepare("SELECT id,position,coalesce(heading,''),content,locator_json FROM source_documents WHERE source_id=?1 ORDER BY position")?;
    let items = documents
        .query_map([source_id], |r| {
            Ok(SourceDocumentItem {
                id: r.get(0)?,
                position: r.get(1)?,
                heading: r.get(2)?,
                content: r.get(3)?,
                locator: serde_json::from_str(&r.get::<_, String>(4)?).unwrap_or_default(),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SourceDetail { source, documents: items })
}

/// 资料详情（含全部文档块）。
#[tauri::command]
pub fn get_library_source(db: State<'_, Database>, source_id: String) -> Result<SourceDetail, AppError> {
    source_detail_impl(&db, &source_id)
}

/// 软删除：FTS 由触发器同步清理，列表里不再出现但数据还在。
#[tauri::command]
pub fn delete_library_source(db: State<'_, Database>, source_id: String) -> Result<(), AppError> {
    let changed = db
        .connect()?
        .execute("UPDATE library_sources SET is_deleted=1,updated_at=?1 WHERE id=?2", rusqlite::params![now_timestamp() as i64, source_id])?;
    if changed == 0 {
        return Err(AppError::Message("资料不存在".into()));
    }
    Ok(())
}

/// 彻底删除本地副本。删表由 CASCADE 完成，封面文件也要一起清掉。
///
/// 顺序是「先删库、再删文件」：数据库失败时提前 return，不会出现
/// 文件已经没了但记录还在的中间态。封面文件不存在（比如用户手动清过）
/// 不算失败 —— 数据已经删掉了，文件清理只是尽力而为。
#[tauri::command]
pub fn purge_library_source(app: AppHandle, db: State<'_, Database>, source_id: String) -> Result<(), AppError> {
    let cover: Option<String> = db
        .connect()?
        .query_row("SELECT cover_path FROM library_sources WHERE id=?1", [&source_id], |r| r.get(0))
        .optional()?;
    let changed = db.connect()?.execute("DELETE FROM library_sources WHERE id=?1", [&source_id])?;
    if changed == 0 {
        return Err(AppError::Message("资料不存在".into()));
    }
    if let Some(name) = cover.filter(|value| !value.is_empty()) {
        let directory = app_data_dir(&app)?.join("imports");
        // 只删除我们自己生成的文件名：数据库里的 cover_path 绝不能被当成任意路径使用
        if is_own_cover_file(&name) {
            let _ = std::fs::remove_file(directory.join(&name));
        }
    }
    Ok(())
}

/// cover_path 是否是我们自己写进去的文件名。
///
/// 拒绝绝对路径、盘符、`..`、子目录分隔符：即使数据库被改坏，
/// 也只能删到 `imports/` 目录下的那一个文件。
fn is_own_cover_file(name: &str) -> bool {
    if name.is_empty() || name.len() > 128 {
        return false;
    }
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return false;
    }
    if name.contains(':') {
        return false;
    }
    name.starts_with("cover-")
}

/// 流式问答。命令立刻返回 `requestId`，正文通过 `ai-stream` 事件推送。
///
/// 参数校验和 RAG 检索在返回前完成，检索失败直接当错误返回；
/// 之后的网络与解析失败都通过 `failed` 事件上报，不会被当成正常结束。
#[tauri::command]
pub async fn ask_ai_stream(
    app: AppHandle,
    db: State<'_, Database>,
    registry: State<'_, Arc<crate::ai::stream::StreamRegistry>>,
    request: AiRequest,
    request_id: Option<String>,
) -> Result<String, AppError> {
    let request_id = request_id
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    if request_id.len() > 128 {
        return Err(AppError::Message("请求标识过长".into()));
    }
    let prepared = prepare_ask_with_cancel(&db, &request, &std::sync::atomic::AtomicBool::new(false)).await?;
    let provider = ai_provider(&db)?;
    let token = registry.register(&request_id)?;
    let cancel_token = Arc::clone(&token);

    let emit_app = app.clone();
    let task_id = request_id.clone();
    let task_registry = registry.inner().clone();
    let mut handle = tokio::spawn(async move {
        let emit = |event: crate::ai::provider::AiStreamEvent| {
            let _ = emit_app.emit(crate::ai::provider::AI_STREAM_EVENT, event);
        };
        emit(crate::ai::provider::AiStreamEvent::started(&task_id));
        let outcome = provider
            .chat_stream(&prepared.messages, &cancel_token, |delta| {
                emit(crate::ai::provider::AiStreamEvent::delta(&task_id, delta));
            })
            .await;
        match outcome {
            Ok((crate::ai::providers::StreamOutcome::Completed(content), notice)) => {
                // 服务不支持流式的回退说明优先于 rerank 提示；两者都是非阻断的。
                let mut answer = finalize_answer(prepared, content);
                if let Some(notice) = notice {
                    answer.rerank_note = notice;
                }
                emit(crate::ai::provider::AiStreamEvent::completed(&task_id, answer));
            }
            Ok((crate::ai::providers::StreamOutcome::Cancelled, _)) => {
                emit(crate::ai::provider::AiStreamEvent::cancelled(&task_id));
            }
            Err(error) => {
                emit(crate::ai::provider::AiStreamEvent::failed(&task_id, error.to_string()));
            }
        }
        task_registry.finish(&task_id);
    });

    // 用户取消时直接中止整个读取任务：响应体在 future 被丢弃时关闭连接，
    // 不需要处理「半读状态」。
    tokio::select! {
        _ = &mut handle => {}
        _ = wait_for_cancel(&token) => {
            handle.abort();
            registry.finish(&request_id);
            let _ = app.emit(
                crate::ai::provider::AI_STREAM_EVENT,
                crate::ai::provider::AiStreamEvent::cancelled(&request_id),
            );
        }
    }
    Ok(request_id)
}

/// 每 60ms 采样一次取消标记。
async fn wait_for_cancel(token: &Arc<std::sync::atomic::AtomicBool>) {
    loop {
        if token.load(std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
}

/// 取消指定 `requestId` 的流式请求。请求已结束或从未存在时返回 `false`。
#[tauri::command]
pub fn cancel_ai_stream(
    app: AppHandle,
    registry: State<'_, Arc<crate::ai::stream::StreamRegistry>>,
    request_id: String,
) -> Result<bool, AppError> {
    let cancelled = registry.cancel(&request_id);
    if cancelled {
        let _ = app.emit(
            crate::ai::provider::AI_STREAM_EVENT,
            crate::ai::provider::AiStreamEvent::cancelled(&request_id),
        );
    }
    Ok(cancelled)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawRelationClaim {
    relation: String,
    summary: String,
    confidence: f64,
    evidence_ids: Vec<String>,
}

#[derive(serde::Deserialize)]
struct RawRelationAnalysis {
    relation: String,
    concepts: Vec<String>,
    summary: String,
    confidence: f64,
    claims: Vec<RawRelationClaim>,
}

#[tauri::command]
pub fn get_cached_relation_analysis(
    db: State<'_, Database>,
    request: RelationAnalysisRequest,
) -> Result<Option<RelationAnalysis>, AppError> {
    let model: Option<String> = db.connect()?.query_row("SELECT model FROM ai_settings WHERE id=1", [], |row| row.get(0)).optional()?;
    let Some(model) = model else { return Ok(None); };
    let (_, _, notes) = relation_context(&db, &request.left_book_id, &request.right_book_id, &request.keywords)?;
    let pair_key = format!("{}:{}", request.left_book_id, request.right_book_id);
    let input_hash = relation_signature(&model, &pair_key, &request.keywords, &notes);
    let cached: Option<String> = db.connect()?.query_row(
        "SELECT result_json FROM relation_analysis_cache WHERE pair_key=?1 AND input_hash=?2",
        rusqlite::params![pair_key, input_hash],
        |row| row.get(0),
    ).optional()?;
    cached.map(|json| {
        let mut result: RelationAnalysis = serde_json::from_str(&json)?;
        result.cached = true;
        Ok(result)
    }).transpose()
}

#[tauri::command]
pub async fn analyze_book_relation(
    db: State<'_, Database>,
    request: RelationAnalysisRequest,
) -> Result<RelationAnalysis, AppError> {
    if request.left_book_id == request.right_book_id {
        return Err(AppError::Message("请选择两本不同的书".into()));
    }
    let (left_title, right_title, mut notes) = relation_context(
        &db,
        &request.left_book_id,
        &request.right_book_id,
        &request.keywords,
    )?;
    let missing: Vec<&str> = [(&request.left_book_id, left_title.as_str()), (&request.right_book_id, right_title.as_str())]
        .into_iter()
        .filter_map(|(id, title)| (!notes.iter().any(|note| note.book_id == *id)).then_some(title))
        .collect();
    if !missing.is_empty() {
        return Err(AppError::Message(format!("《{}》暂无划线或想法，不能进行有证据的观点分析", missing.join("》《"))));
    }

    let provider = ai_provider(&db)?;
    let pair_key = format!("{}:{}", request.left_book_id, request.right_book_id);
    let input_hash = relation_signature(&provider.model, &pair_key, &request.keywords, &notes);
    if !request.refresh {
        let cached: Option<String> = db.connect()?.query_row(
            "SELECT result_json FROM relation_analysis_cache WHERE pair_key=?1 AND input_hash=?2",
            rusqlite::params![pair_key, input_hash],
            |row| row.get(0),
        ).optional()?;
        if let Some(json) = cached {
            let mut result: RelationAnalysis = serde_json::from_str(&json)?;
            result.cached = true;
            return Ok(result);
        }
    }

    let mut labels = HashMap::<String, Note>::new();
    let mut context = String::new();
    for (index, note) in notes.drain(..).enumerate() {
        let side = if note.book_id == request.left_book_id { "A" } else { "B" };
        let label = format!("{}{}", side, index + 1);
        let source = if note.note_type == "thought" { "读者想法" } else { "书籍原文划线" };
        let content: String = note.content.chars().take(700).collect();
        context.push_str(&format!("[{label}] {source}｜{}｜{}\n{}\n\n", note.book_title, note.chapter, content));
        labels.insert(label, note);
    }
    let allowed = "same_concept, agreement, conflict, complementary, causal, application, uncertain";
    let system = "你是严谨的跨书观点分析器。只能根据提供的笔记判断，不能依赖书名常识补充结论。必须区分书籍原文划线和读者想法；读者想法不能当作作者立场。只有两侧证据都存在时才能判断一致、冲突、因果或应用关系。输出纯 JSON，不要 Markdown。";
    let prompt = format!(
        "比较 A《{left_title}》与 B《{right_title}》。候选共同词：{}。\n\n笔记：\n{context}\n输出结构：{{\"relation\":\"类型\",\"concepts\":[\"规范化概念\"],\"summary\":\"一句话结论\",\"confidence\":0到1,\"claims\":[{{\"relation\":\"类型\",\"summary\":\"原子观点关系\",\"confidence\":0到1,\"evidenceIds\":[\"A编号\",\"B编号\"]}}]}}。类型只能是 {allowed}。concepts 最多5项，claims 最多3项；每个 claim 必须同时引用 A、B 证据，否则省略。证据不足时 relation=uncertain。",
        request.keywords.iter().take(8).cloned().collect::<Vec<_>>().join("、")
    );
    let response = provider.chat(&[
        ChatMessage { role: "system".into(), content: system.into() },
        ChatMessage { role: "user".into(), content: prompt },
    ]).await?;
    let raw = match parse_relation_analysis(&response) {
        Ok(raw) => raw,
        Err(_) => {
            // Models occasionally wrap JSON in prose or emit a nearly-correct object. Give the
            // model one cheap opportunity to normalize its own output instead of exposing a
            // serde parser error to the user.
            let repair_prompt = format!(
                "请把下面内容修正为严格有效的 JSON。不要增删事实，不要输出 Markdown 或解释，只返回 JSON 对象：\n\n{}",
                response.chars().take(4_000).collect::<String>()
            );
            let repaired = provider.chat(&[
                ChatMessage {
                    role: "system".into(),
                    content: "你是 JSON 格式修复器。输出必须是可被标准 JSON 解析器直接解析的单个对象。".into(),
                },
                ChatMessage { role: "user".into(), content: repair_prompt },
            ]).await?;
            parse_relation_analysis(&repaired).map_err(|_| {
                AppError::Message("AI 返回的数据格式不完整，请点击“重新分析”再试一次".into())
            })?
        }
    };
    let allowed: HashSet<&str> = allowed.split(", ").collect();
    let mut claims = Vec::new();
    for raw_claim in raw.claims.into_iter().take(3) {
        if !allowed.contains(raw_claim.relation.as_str()) { continue; }
        let mut evidence = Vec::new();
        let mut sides = HashSet::new();
        for id in raw_claim.evidence_ids.into_iter().take(4) {
            if let Some(note) = labels.get(&id) {
                sides.insert(if note.book_id == request.left_book_id { "A" } else { "B" });
                evidence.push(RelationEvidence {
                    book_id: note.book_id.clone(),
                    note_id: note.id.clone(),
                    note_type: note.note_type.clone(),
                    text: note.content.chars().take(180).collect(),
                });
            }
        }
        if sides.len() == 2 {
            claims.push(RelationClaim {
                relation: raw_claim.relation,
                summary: raw_claim.summary.chars().take(240).collect(),
                confidence: raw_claim.confidence.clamp(0.0, 1.0),
                evidence,
            });
        }
    }
    let relation = if allowed.contains(raw.relation.as_str()) && (!claims.is_empty() || raw.relation == "uncertain") { raw.relation } else { "uncertain".into() };
    let result = RelationAnalysis {
        relation,
        concepts: raw.concepts.into_iter().filter(|value| !value.trim().is_empty()).take(5).map(|value| value.chars().take(30).collect()).collect(),
        summary: raw.summary.chars().take(300).collect(),
        confidence: raw.confidence.clamp(0.0, 1.0),
        claims,
        cached: false,
    };
    let result_json = serde_json::to_string(&result)?;
    db.connect()?.execute(
        "INSERT INTO relation_analysis_cache(pair_key,input_hash,result_json,updated_at) VALUES(?1,?2,?3,?4) ON CONFLICT(pair_key) DO UPDATE SET input_hash=excluded.input_hash,result_json=excluded.result_json,updated_at=excluded.updated_at",
        rusqlite::params![pair_key, input_hash, result_json, now_timestamp()],
    )?;
    Ok(result)
}

fn relation_context(db: &Database, left_id: &str, right_id: &str, keywords: &[String]) -> Result<(String, String, Vec<Note>), AppError> {
    let connection = db.connect()?;
    let title = |id: &str| -> Result<String, AppError> {
        connection.query_row("SELECT title FROM books WHERE book_id=?1 AND is_deleted=0", [id], |row| row.get(0)).map_err(AppError::from)
    };
    let left_title = title(left_id)?;
    let right_title = title(right_id)?;
    let mut query = connection.prepare(
        "SELECT h.bookmark_id,'highlight',h.book_id,b.title,coalesce(h.chapter_title,''),h.mark_text,coalesce(datetime(h.create_time,'unixepoch','localtime'),'') FROM highlights h JOIN books b ON b.book_id=h.book_id WHERE h.book_id IN (?1,?2) AND h.is_deleted=0
         UNION ALL SELECT t.review_id,'thought',t.book_id,b.title,coalesce(t.chapter_name,''),t.content,coalesce(datetime(t.create_time,'unixepoch','localtime'),'') FROM thoughts t JOIN books b ON b.book_id=t.book_id WHERE t.book_id IN (?1,?2) AND t.is_deleted=0"
    )?;
    let all = query.query_map(rusqlite::params![left_id, right_id], |row| Ok(Note { id:row.get(0)?,note_type:row.get(1)?,book_id:row.get(2)?,book_title:row.get(3)?,chapter:row.get(4)?,content:row.get(5)?,created_at:row.get(6)? }))?.collect::<Result<Vec<_>, _>>()?;
    let mut selected = Vec::new();
    for id in [left_id, right_id] {
        let mut book_notes: Vec<(usize, Note)> = all.iter().filter(|note| note.book_id == id).cloned().map(|note| {
            let hits = keywords.iter().filter(|word| !word.is_empty() && (note.content.contains(word.as_str()) || note.chapter.contains(word.as_str()))).count();
            let score = hits * 10 + usize::from(note.note_type == "thought");
            (score, note)
        }).collect();
        book_notes.sort_by(|a, b| b.0.cmp(&a.0));
        selected.extend(book_notes.into_iter().take(12).map(|(_, note)| note));
    }
    Ok((left_title, right_title, selected))
}

fn relation_signature(model: &str, pair_key: &str, keywords: &[String], notes: &[Note]) -> String {
    let source = format!("v1|{}|{}|{}|{}", model, pair_key, keywords.join("|"), notes.iter().map(|note| format!("{}:{}", note.id, note.content)).collect::<Vec<_>>().join("|"));
    format!("{:x}", md5::compute(source.as_bytes()))
}

fn json_object(value: &str) -> Option<&str> {
    let start = value.find('{')?;
    let end = value.rfind('}')?;
    (end >= start).then_some(&value[start..=end])
}

fn parse_relation_analysis(value: &str) -> Result<RawRelationAnalysis, serde_json::Error> {
    // Accept a plain object as well as the common ```json ... ``` response shape.
    // The returned error is deliberately handled at the command boundary so raw parser details
    // never become the user-facing error message.
    let candidate = json_object(value).unwrap_or(value.trim());
    serde_json::from_str(candidate)
}

fn now_timestamp() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

fn ai_provider(db: &Database) -> Result<crate::ai::providers::OpenAiCompatibleProvider, AppError> {
    ai_provider_with_key(db, None)
}

fn ai_provider_with_key(
    db: &Database,
    api_key: Option<String>,
) -> Result<crate::ai::providers::OpenAiCompatibleProvider, AppError> {
    let c = db.connect()?;
    let settings = c
        .query_row(
            "SELECT provider,endpoint,model FROM ai_settings WHERE id=1",
            [],
            |r| {
                Ok(AiSettings {
                    provider: r.get(0)?,
                    endpoint: r.get(1)?,
                    model: r.get(2)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| AppError::Message("请先配置 AI Provider".into()))?;
    validate_ai_settings(&settings)?;
    let api_key = match api_key.filter(|value| !value.trim().is_empty()) {
        Some(value) => value,
        None => {
            keyring::Entry::new("ReadFlow", &format!("ai:{}", settings.provider))?.get_password()?
        }
    };
    Ok(crate::ai::providers::OpenAiCompatibleProvider {
        endpoint: settings.endpoint,
        model: settings.model,
        api_key,
        http: reqwest::Client::new(),
    })
}

fn validate_ai_settings(settings: &AiSettings) -> Result<(), AppError> {
    if settings.provider.trim().is_empty() || settings.model.trim().is_empty() {
        return Err(AppError::Message("Provider 和模型不能为空".into()));
    }
    let url = reqwest::Url::parse(&settings.endpoint)
        .map_err(|_| AppError::Message("AI Endpoint 格式无效".into()))?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if url.scheme() != "https" && !(local && url.scheme() == "http") {
        return Err(AppError::Message(
            "AI Endpoint 必须使用 HTTPS；仅本机地址允许 HTTP".into(),
        ));
    }
    Ok(())
}

async fn rag_search(
    db: &Database,
    question: &str,
    mode: &str,
    book_ids: &[String],
    cancelled: &std::sync::atomic::AtomicBool,
) -> Result<(Vec<SearchResult>, String), AppError> {
    let search_input = if mode == "ask" { question } else { "" };
    if book_ids.len() > 1 {
        // 小范围比较保留更多证据；大范围分析保证每本书至少有一个样本，
        // 避免固定总上限导致排在后面的书完全没有进入上下文。
        let per_book = if book_ids.len() <= 12 {
            5
        } else if book_ids.len() <= 30 {
            3
        } else {
            1
        };
        let total_limit = book_ids.len() * per_book;
        let mut combined = Vec::new();
        for book_id in book_ids {
            combined.extend(hybrid_search(
                db,
                search_input,
                None,
                std::slice::from_ref(book_id),
                per_book,
            )?);
        }
        combined.truncate(total_limit);
        return Ok(maybe_rerank(db, question, mode, book_ids, combined, cancelled).await);
    }
    let candidates = hybrid_search(db, search_input, None, book_ids, 200)?;
    Ok(maybe_rerank(db, question, mode, book_ids, candidates, cancelled).await)
}

/// Hybrid Search 之后可选地做一次 rerank。
///
/// 返回 `(候选, 降级提示)`。任何一步失败都只是把提示填上，候选本身原样返回，
/// 所以 AI 永远不会因为重排不可用而失败。
async fn maybe_rerank(
    db: &Database,
    question: &str,
    mode: &str,
    book_ids: &[String],
    candidates: Vec<SearchResult>,
    cancelled: &std::sync::atomic::AtomicBool,
) -> (Vec<SearchResult>, String) {
    let Ok(Some(settings)) = reranker_settings_from_db(db) else {
        return (candidates, String::new());
    };
    if !crate::ai::reranker::is_usable(&settings) {
        return (candidates, String::new());
    }
    // 只在全库提问时重排：summary / compare 的检索输入是空串，没有 query 可用。
    if question.trim().is_empty() || mode != "ask" {
        return (candidates, String::new());
    }
    let Ok(api_key) = keyring::Entry::new(
        "ReadFlow",
        &crate::ai::reranker::credential_name(&settings.provider),
    )
    .and_then(|entry| entry.get_password())
    else {
        return (
            candidates,
            "未配置 Reranker API Key，本次沿用本地排序".into(),
        );
    };
    let documents: Vec<String> = candidates
        .iter()
        .map(|item| item.note.content.clone())
        .collect();
    let outcome = crate::ai::reranker::rerank(
        &reqwest::Client::new(),
        &settings,
        &api_key,
        question.trim(),
        &documents,
        cancelled,
    )
    .await;
    // 只记录耗时与候选数，不记录问题正文、笔记正文或密钥。
    eprintln!(
        "[rerank] applied={} candidates={} elapsed_ms={}",
        outcome.applied, outcome.candidate_count, outcome.elapsed_ms
    );
    if !outcome.applied || outcome.original_order.is_empty() {
        let warning = if outcome.warning.is_empty() {
            "重排未生效，本次沿用本地排序".to_string()
        } else {
            outcome.warning
        };
        return (candidates, warning);
    }
    let top_n = (settings.top_n as usize).clamp(1, candidates.len());
    // 调试用途：打印每条候选的排序来源与分项分数，不记录正文。
    crate::ai::reranker::log_score_breakdown(
        &outcome,
        &candidates.iter().map(|item| -item.score).collect::<Vec<_>>(),
        |index| {
            candidates
                .get(index)
                .map(|item| item.note.id.clone())
                .unwrap_or_default()
        },
    );
    let mut reranked: Vec<SearchResult> = outcome
        .original_order
        .iter()
        .filter_map(|index| candidates.get(*index).cloned())
        .collect();
    if reranked.is_empty() {
        return (candidates, "重排结果为空，本次沿用本地排序".into());
    }
    // 跨书分析要保住选中书籍的证据覆盖，不能被单本书刷屏。
    if book_ids.len() > 1 {
        let book_ids_of: Vec<String> = reranked
            .iter()
            .map(|item| item.note.book_id.clone())
            .collect();
        let selected = crate::ai::reranker::apply_book_coverage(
            &(0..reranked.len()).collect::<Vec<usize>>(),
            &book_ids_of,
            top_n,
            2,
        );
        reranked = selected
            .into_iter()
            .filter_map(|index| reranked.get(index).cloned())
            .collect();
    }
    reranked.truncate(top_n);
    (reranked, String::new())
}

fn hybrid_search(
    db: &Database,
    input: &str,
    kind: Option<&str>,
    book_ids: &[String],
    limit: usize,
) -> Result<Vec<SearchResult>, AppError> {
    let c = db.connect()?;
    let base = "SELECT note_id,note_type,book_id,title,chapter_title,content,
        CASE note_type WHEN 'highlight' THEN coalesce(datetime((SELECT create_time FROM highlights WHERE bookmark_id=note_id),'unixepoch','localtime'),'') ELSE coalesce(datetime((SELECT create_time FROM thoughts WHERE review_id=note_id),'unixepoch','localtime'),'') END, 0.0
        FROM notes_fts";
    let notes = if input.trim().is_empty() {
        let book_id = book_ids.first().map(String::as_str);
        let sql = format!("{base} WHERE (?1 IS NULL OR note_type=?1) AND (?2 IS NULL OR book_id=?2) LIMIT ?3");
        let mut query = c.prepare(&sql)?;
        let rows = query
            .query_map(rusqlite::params![kind, book_id, limit as i64], map_note)?
            .collect::<Result<Vec<_>, _>>()?;
        rows
    } else {
        let match_query = search_terms(input)
            .into_iter()
            .take(12)
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR ");
        let sql = format!("{base} WHERE notes_fts MATCH ?1 AND (?2 IS NULL OR note_type=?2) ORDER BY bm25(notes_fts) LIMIT 5000");
        let mut query = c.prepare(&sql)?;
        let matched = query
            .query_map(rusqlite::params![match_query, kind], map_note)?
            .collect::<Result<Vec<_>, _>>()?;
        if matched.is_empty() {
            let fallback_sql = format!("{base} WHERE (?1 IS NULL OR note_type=?1) LIMIT 800");
            let mut fallback = c.prepare(&fallback_sql)?;
            let rows = fallback
                .query_map([kind], map_note)?
                .collect::<Result<Vec<_>, _>>()?;
            rows
        } else {
            matched
        }
    };
    let normalized = normalize_search_text(input);
    let terms = search_terms(input);
    let mut ranked: Vec<SearchResult> = notes
        .into_iter()
        .filter(|item| book_ids.is_empty() || book_ids.iter().any(|id| id == &item.note.book_id))
        .filter_map(|mut item| {
            let title = normalize_search_text(&item.note.book_title);
            let chapter = normalize_search_text(&item.note.chapter);
            let content = normalize_search_text(&item.note.content);
            let mut score = if normalized.is_empty() {
                1.0
            } else if content.contains(&normalized) {
                12.0
            } else {
                0.0
            };
            for term in &terms {
                if title.contains(term) {
                    score += 5.0;
                }
                if chapter.contains(term) {
                    score += 3.0;
                }
                score += content.match_indices(term).count().min(4) as f64;
            }
            (score > 0.0).then(|| {
                item.score = -score;
                item
            })
        })
        .collect();
    ranked.sort_by(|a, b| {
        a.score
            .partial_cmp(&b.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if book_ids.is_empty() && limit > 20 {
        let mut diverse = Vec::with_capacity(limit);
        let mut overflow = Vec::new();
        let mut per_book = std::collections::HashMap::<String, usize>::new();
        for item in ranked {
            let count = per_book.entry(item.note.book_id.clone()).or_default();
            if *count < 8 {
                *count += 1;
                diverse.push(item);
            } else {
                overflow.push(item);
            }
        }
        if diverse.len() < limit {
            diverse.extend(overflow.into_iter().take(limit - diverse.len()));
        }
        diverse.truncate(limit);
        return Ok(diverse);
    }
    ranked.truncate(limit);
    Ok(ranked)
}

/// 规范化搜索文本：转小写并只保留字母数字。
///
/// 抽成 `pub(crate)` 是为了让 `rag` 模块用同一套规则给导入资料打分，
/// 否则笔记与资料的匹配口径会不一致。
pub(crate) fn normalize_search_text(input: &str) -> String {
    input
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn search_terms(input: &str) -> Vec<String> {
    let mut terms: Vec<String> = input
        .split_whitespace()
        .map(normalize_search_text)
        .filter(|term| !term.is_empty())
        .collect();
    terms.extend(semantic_bigrams(input));
    let normalized = normalize_search_text(input);
    if normalized.contains("清朝") || normalized.contains("清代") || normalized.contains("清史") {
        terms.extend(["清朝", "清代", "清史", "大清", "满清"].map(str::to_string));
    }
    terms.sort_by(|a, b| {
        b.chars()
            .count()
            .cmp(&a.chars().count())
            .then_with(|| a.cmp(b))
    });
    terms.dedup();
    terms
}

fn semantic_bigrams(input: &str) -> Vec<String> {
    let normalized: Vec<char> = input
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect();
    let mut terms: Vec<String> = normalized
        .windows(2)
        .map(|pair| pair.iter().collect())
        .collect();
    terms.sort();
    terms.dedup();
    terms
}

#[cfg(test)]
mod tests {
    use super::{
        book_title_boost, classify_google_books_status, clear_suggested_concepts_impl, concept_entity_with_evidence,
        content_hash, correct_entity_impl, database_row_query, fts_match_expression, fts_query, global_search_impl,
        known_entity_names, list_concept_graph_impl, maybe_rerank, merge_entities_impl, normalize_recommendations,
        normalize_scores, notes_grouped_by_book, parse_extraction, parse_relation_analysis, persist_extraction,
        cover_file_name, find_duplicate, is_own_cover_file, list_sources_impl, read_local_file,
        relevance_from_bm25, reranker_settings_from_db, request_default, save_reranker_settings_impl, search_terms,
        snippet_of, source_candidates, source_detail_impl, score_note, unchanged_notes, validate_secret_kind,
        weread_reader_id, NoteCandidate, glossary_terms_for_review, save_glossary_term_impl,
        MAX_LOCAL_FILE_BYTES,
        MetadataBatchFuture, MetadataFetchResult, AppError, GLOBAL_SEARCH_SNIPPET_CHARS,
    };
    use crate::{
        database::Database,
        models::{
            ConceptGraphQuery, EntityCorrection, GlobalSearchPage, GlobalSearchRequest, GlobalSearchResult, GlossaryTerm, Note,
            RerankerSettings, SearchResult,
        },
    };
    use std::{
        future::Future,
        sync::{atomic::AtomicBool, Arc, Mutex},
        task::{Context, Poll, Waker},
        time::Duration,
    };

    /// 跑一个真正的异步函数。
    ///
    /// 与 `block_on` 不同，这里起的是 tokio runtime：重排降级路径会读系统凭据库，
    /// 而凭据库调用可能真的挂起，用无 waker 的忙等会死循环。
    fn run_async<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("创建 tokio runtime 失败")
            .block_on(future)
    }

    /// 最小执行器：tokio 没有开启 macros feature，用不了 `#[tokio::test]`；
    /// 这里喂进去的假请求都是立刻就绪的 future，直接轮询即可。
    fn block_on<F: Future>(future: F) -> F::Output {
        let waker = Waker::noop();
        let mut context = Context::from_waker(&waker);
        let mut future = std::pin::pin!(future);
        loop {
            if let Poll::Ready(output) = future.as_mut().poll(&mut context) {
                return output;
            }
            std::thread::yield_now();
        }
    }

    fn updated_result(book_id: String) -> MetadataFetchResult {
        MetadataFetchResult {
            book_id,
            source: "weread".into(),
            status: "updated".into(),
            message: "已更新".into(),
        }
    }

    #[test]
    fn batched_metadata_fetch_keeps_other_results_when_one_book_fails() {
        let book_ids = vec!["bk-1".to_string(), "bk-2".to_string(), "bk-3".to_string()];
        let results = block_on(super::run_batched(
            &book_ids,
            super::METADATA_BATCH_LIMIT,
            "weread",
            Duration::ZERO,
            |book_id| -> MetadataBatchFuture<'static> {
                Box::pin(async move {
                    if book_id == "bk-2" {
                        return Err(AppError::Message("这一本抓取失败".into()));
                    }
                    Ok(updated_result(book_id))
                })
            },
        ))
        .unwrap();

        // 单本失败不能吞掉其他结果，且顺序必须与传入的 book_ids 一致
        assert_eq!(results.len(), 3);
        assert_eq!(
            results
                .iter()
                .map(|result| result.book_id.as_str())
                .collect::<Vec<_>>(),
            ["bk-1", "bk-2", "bk-3"]
        );
        assert_eq!(results[0].status, "updated");
        assert_eq!(results[1].status, "failed");
        assert_eq!(results[1].book_id, "bk-2");
        assert_eq!(results[1].source, "weread");
        assert!(results[1].message.contains("这一本抓取失败"));
        assert_eq!(results[2].status, "updated");
    }

    #[test]
    fn batched_metadata_fetch_keeps_input_order_when_requests_finish_out_of_order() {
        let book_ids = vec!["bk-1".to_string(), "bk-2".to_string(), "bk-3".to_string()];
        let finished = Arc::new(Mutex::new(Vec::new()));
        let recorder = finished.clone();
        let results = block_on(super::run_batched(
            &book_ids,
            super::METADATA_BATCH_LIMIT,
            "weread",
            Duration::ZERO,
            move |book_id| -> MetadataBatchFuture<'static> {
                let recorder = recorder.clone();
                Box::pin(async move {
                    if book_id == "bk-1" {
                        // 让出两轮再就绪，模拟最后完成的那一个
                        let mut pending = 2u8;
                        std::future::poll_fn(|context| {
                            if pending > 0 {
                                pending -= 1;
                                context.waker().wake_by_ref();
                                Poll::Pending
                            } else {
                                Poll::Ready(())
                            }
                        })
                        .await;
                    }
                    recorder.lock().expect("完成顺序记录锁可用").push(book_id.clone());
                    Ok(updated_result(book_id))
                })
            },
        ))
        .unwrap();

        assert_eq!(
            finished.lock().expect("完成顺序记录锁可用").as_slice(),
            ["bk-2", "bk-3", "bk-1"]
        );
        assert_eq!(
            results
                .iter()
                .map(|result| result.book_id.as_str())
                .collect::<Vec<_>>(),
            ["bk-1", "bk-2", "bk-3"]
        );
    }

    #[test]
    fn batched_metadata_fetch_rejects_more_than_twenty_books() {
        assert_eq!(super::METADATA_BATCH_LIMIT, 20);
        let book_ids = (0..21).map(|index| format!("bk-{index}")).collect::<Vec<_>>();
        let outcome = block_on(super::run_batched(
            &book_ids,
            super::METADATA_BATCH_LIMIT,
            "weread",
            Duration::ZERO,
            |book_id| -> MetadataBatchFuture<'static> { Box::pin(async move { Ok(updated_result(book_id)) }) },
        ));
        let error = match outcome {
            Ok(results) => panic!("超过 20 本时应当被拒绝，实际返回 {} 条结果", results.len()),
            Err(error) => error,
        };
        assert!(error.to_string().contains("单次最多补全 20 本书"));
    }

    #[test]
    fn google_books_connection_test_fails_on_bad_status() {
        assert_eq!(classify_google_books_status(200), Ok(true));
        assert_eq!(classify_google_books_status(206), Ok(true));
        assert!(classify_google_books_status(401).is_err());
        assert!(classify_google_books_status(401).unwrap_err().contains("无效"));
        assert!(classify_google_books_status(403).unwrap_err().contains("无效"));
        assert!(classify_google_books_status(429).unwrap_err().contains("限流"));
        assert!(classify_google_books_status(500).is_err());
        assert!(classify_google_books_status(404).unwrap_err().contains("404"));
    }

    #[test]
    fn database_row_browser_only_accepts_whitelisted_tables() {
        for table in [
            "books",
            "highlights",
            "thoughts",
            "sync_sessions",
            "book_metadata_sources",
            "glossary_terms",
        ] {
            let (count_sql, rows_sql) = database_row_query(table)
                .unwrap_or_else(|| panic!("{table} 应当在白名单内"));
            assert!(count_sql.contains("count(*)"));
            // 行语句必须按 id/主/次/详情/时间五列返回
            assert!(rows_sql.contains("LIMIT ?2 OFFSET ?3"));
        }
        for table in [
            "sqlite_master",
            "book_metadata_sources; DROP TABLE books",
            "glossary_terms'",
            "books--",
            "",
        ] {
            assert!(
                database_row_query(table).is_none(),
                "{table} 不应当拿到任何 SQL"
            );
        }
    }

    #[test]
    fn normalizes_nested_recommendation_metrics_and_numeric_strings() {
        let response = serde_json::json!({
            "books": [{
                "reason": "recommended",
                "bookInfo": {
                    "bookId": "123",
                    "title": "Example",
                    "readingCount": "4567",
                    "newRating": "82",
                    "newRatingCount": "90"
                }
            }]
        });
        let normalized = normalize_recommendations(response);
        let book = &normalized["books"][0];
        assert_eq!(book["bookId"], "123");
        assert_eq!(book["title"], "Example");
        assert_eq!(book["readingCount"], 4567.0);
        assert_eq!(book["newRating"], 82.0);
        assert_eq!(book["newRatingCount"], 90.0);
    }


    #[test]
    fn restricts_credential_names_to_known_namespaces() {
        assert!(validate_secret_kind("weread").is_ok());
        assert!(validate_secret_kind("google_books").is_ok());
        assert!(validate_secret_kind("ai:deepseek").is_ok());
        assert!(validate_secret_kind("embedding:openai-compatible").is_ok());
        assert!(validate_secret_kind("arbitrary-secret").is_err());
        assert!(validate_secret_kind("ai:").is_err());
        assert!(validate_secret_kind("ai:../other").is_err());
    }

    #[test]
    fn parses_relation_json_wrapped_in_markdown() {
        let response = r#"```json
{"relation":"agreement","concepts":["制度"],"summary":"观点一致","confidence":0.8,"claims":[]}
```"#;
        let parsed = parse_relation_analysis(response).unwrap();
        assert_eq!(parsed.relation, "agreement");
        assert_eq!(parsed.concepts, ["制度"]);
    }

    #[test]
    fn rejects_malformed_relation_json_before_retrying() {
        let response = r#"{"relation":"agreement" "concepts":[]}"#;
        assert!(parse_relation_analysis(response).is_err());
    }

    #[test]
    fn builds_the_real_web_reader_id_for_a_numeric_book_id() {
        assert_eq!(
            weread_reader_id("3300220342"),
            "8d2321e0813abbc92g012963"
        );

        let reader_id = weread_reader_id("3300220342");
        let mut url = reqwest::Url::parse("https://weread.qq.com/web/reader/").unwrap();
        url.path_segments_mut()
            .unwrap()
            .pop_if_empty()
            .push(&reader_id);
        assert_eq!(
            url.as_str(),
            "https://weread.qq.com/web/reader/8d2321e0813abbc92g012963"
        );
    }

    #[test]
    fn fts_query_uses_and_for_multiple_terms() {
        assert_eq!(fts_query("组织 管理"), "\"组织\" AND \"管理\"");
    }

    #[test]
    fn fts_query_escapes_quotes() {
        assert_eq!(fts_query("a\"b"), "\"a\"\"b\"");
    }

    #[test]
    fn chinese_questions_produce_search_bigrams() {
        assert!(super::semantic_bigrams("我对组织有什么想法？").contains(&"组织".into()));
    }

    #[test]
    fn chinese_search_combines_phrase_and_bigrams() {
        let terms = search_terms("组织管理");
        assert!(terms.contains(&"组织管理".into()));
        assert!(terms.contains(&"组织".into()));
        assert!(terms.contains(&"管理".into()));
    }

    // ---------- 全局搜索 ----------

    /// 建一个只含 schema.sql 的临时库，不碰用户数据。
    fn search_test_db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().expect("创建临时目录失败");
        let db = Database::open(dir.path().join("test.db")).expect("打开临时数据库失败");
        (dir, db)
    }

    fn seed_book(db: &Database, book_id: &str, title: &str, author: &str, category: &str) {
        db.connect()
            .unwrap()
            .execute(
                "INSERT INTO books(book_id,title,author,category,is_deleted,created_at,synced_at,update_time)
                 VALUES(?1,?2,?3,?4,0,0,0,1700000000)",
                rusqlite::params![book_id, title, author, category],
            )
            .unwrap();
    }

    fn seed_highlight(db: &Database, id: &str, book_id: &str, chapter: &str, text: &str) {
        db.connect()
            .unwrap()
            .execute(
                "INSERT INTO highlights(bookmark_id,book_id,chapter_title,mark_text,is_deleted,synced_at,create_time)
                 VALUES(?1,?2,?3,?4,0,0,1700000000)",
                rusqlite::params![id, book_id, chapter, text],
            )
            .unwrap();
    }

    fn seed_thought(db: &Database, id: &str, book_id: &str, chapter: &str, text: &str) {
        db.connect()
            .unwrap()
            .execute(
                "INSERT INTO thoughts(review_id,book_id,chapter_name,content,is_deleted,synced_at,create_time)
                 VALUES(?1,?2,?3,?4,0,0,1700000000)",
                rusqlite::params![id, book_id, chapter, text],
            )
            .unwrap();
    }

    fn search(db: &Database, request: GlobalSearchRequest) -> GlobalSearchPage {
        // command 签名带 State，直接测内部实现。
        global_search_impl(db, &request).expect("全局搜索不应报错")
    }

    fn titles(page: &GlobalSearchPage) -> Vec<String> {
        page.results.iter().map(|item| item.title.clone()).collect()
    }

    #[test]
    fn global_search_matches_title_author_and_isbn() {
        let (_dir, db) = search_test_db();
        seed_book(&db, "b1", "置身事内", "兰小欢", "社科");
        seed_book(&db, "b2", "中国历代政治得失", "钱穆", "历史");
        db.connect()
            .unwrap()
            .execute(
                "INSERT INTO book_metadata_sources(book_id,source,source_id,raw_json,fetched_at,isbn13)
                 VALUES('b2','douban','x','{}',0,'9787508660752')",
                [],
            )
            .unwrap();

        // 书名精确命中
        assert!(titles(&search(&db, GlobalSearchRequest { query: "置身事内".into(), ..request_default() })).contains(&"置身事内".to_string()));
        // 作者命中
        assert!(titles(&search(&db, GlobalSearchRequest { query: "钱穆".into(), ..request_default() })).contains(&"中国历代政治得失".to_string()));
        // ISBN 命中
        assert!(titles(&search(&db, GlobalSearchRequest { query: "9787508660752".into(), ..request_default() })).contains(&"中国历代政治得失".to_string()));
        // 分类命中
        assert!(titles(&search(&db, GlobalSearchRequest { query: "社科".into(), ..request_default() })).contains(&"置身事内".to_string()));
    }

    #[test]
    fn global_search_matches_highlights_and_thoughts() {
        let (_dir, db) = search_test_db();
        seed_book(&db, "b1", "置身事内", "兰小欢", "社科");
        seed_highlight(&db, "h1", "b1", "第一章", "地方政府的债务问题");
        seed_thought(&db, "t1", "b1", "第二章", "我对地方融资的想法");

        let page = search(&db, GlobalSearchRequest { query: "地方政府".into(), ..request_default() });
        assert!(page.results.iter().any(|item| item.entity_type == "highlight" && item.id == "h1"));

        let page = search(&db, GlobalSearchRequest { query: "地方融资".into(), ..request_default() });
        assert!(page.results.iter().any(|item| item.entity_type == "thought" && item.id == "t1"));
    }

    #[test]
    fn global_search_chinese_phrase_hits_notes() {
        let (_dir, db) = search_test_db();
        seed_book(&db, "b1", "书", "作者", "分类");
        seed_highlight(&db, "h1", "b1", "第一章", "组织管理非常重要");
        let page = search(&db, GlobalSearchRequest { query: "组织管理".into(), ..request_default() });
        assert!(page.results.iter().any(|item| item.id == "h1"));
    }

    #[test]
    fn global_search_type_and_book_filters_combine() {
        let (_dir, db) = search_test_db();
        seed_book(&db, "b1", "甲书", "作者甲", "社科");
        seed_book(&db, "b2", "乙书", "作者乙", "历史");
        seed_highlight(&db, "h1", "b1", "第一章", "共同关键词内容");
        seed_highlight(&db, "h2", "b2", "第一章", "共同关键词内容");
        seed_thought(&db, "t1", "b1", "第二章", "共同关键词内容");

        // 只搜划线：想法不出现
        let page = search(&db, GlobalSearchRequest { query: "共同关键词".into(), types: vec!["highlight".into()], ..request_default() });
        assert!(page.results.iter().all(|item| item.entity_type == "highlight"));
        assert_eq!(page.results.len(), 2);

        // 只搜想法
        let page = search(&db, GlobalSearchRequest { query: "共同关键词".into(), types: vec!["thought".into()], ..request_default() });
        assert_eq!(page.results.len(), 1);
        assert_eq!(page.results[0].id, "t1");

        // 类型 + 书籍组合
        let page = search(&db, GlobalSearchRequest { query: "共同关键词".into(), types: vec!["highlight".into()], book_id: Some("b2".into()), ..request_default() });
        assert_eq!(page.results.len(), 1);
        assert_eq!(page.results[0].id, "h2");

        // 只要书籍类型时不应返回笔记
        let page = search(&db, GlobalSearchRequest { query: "甲书".into(), types: vec!["book".into()], ..request_default() });
        assert!(page.results.iter().all(|item| item.entity_type == "book"));
    }

    #[test]
    fn global_search_skips_soft_deleted_books_and_notes() {
        let (_dir, db) = search_test_db();
        seed_book(&db, "b1", "活着的书", "作者", "分类");
        seed_book(&db, "b2", "已删的书", "作者", "分类");
        seed_highlight(&db, "h1", "b1", "第一章", "命中关键词");
        seed_highlight(&db, "h2", "b2", "第一章", "命中关键词");
        db.connect().unwrap().execute("UPDATE books SET is_deleted=1 WHERE book_id='b2'", []).unwrap();
        db.connect().unwrap().execute("UPDATE highlights SET is_deleted=1 WHERE bookmark_id='h2'", []).unwrap();

        let page = search(&db, GlobalSearchRequest { query: "命中关键词".into(), ..request_default() });
        assert!(page.results.iter().all(|item| item.book_id == "b1"));
        assert!(!page.results.iter().any(|item| item.id == "h2"));

        let page = search(&db, GlobalSearchRequest { query: "书".into(), types: vec!["book".into()], ..request_default() });
        assert!(!titles(&page).contains(&"已删的书".to_string()));
    }

    #[test]
    fn global_search_renamed_book_stops_matching_old_title() {
        let (_dir, db) = search_test_db();
        seed_book(&db, "b1", "旧书名", "作者", "分类");
        assert!(titles(&search(&db, GlobalSearchRequest { query: "旧书名".into(), ..request_default() })).contains(&"旧书名".to_string()));

        db.connect().unwrap().execute("UPDATE books SET title='新书名' WHERE book_id='b1'", []).unwrap();

        let page = search(&db, GlobalSearchRequest { query: "旧书名".into(), ..request_default() });
        assert!(!titles(&page).contains(&"旧书名".to_string()));
        assert!(titles(&search(&db, GlobalSearchRequest { query: "新书名".into(), ..request_default() })).contains(&"新书名".to_string()));
    }

    #[test]
    fn global_search_survives_quotes_and_sql_injection_attempts() {
        let (_dir, db) = search_test_db();
        seed_book(&db, "b1", "正常书名", "作者", "分类");
        seed_highlight(&db, "h1", "b1", "第一章", "正常内容");

        for hostile in [
            "\" OR 1=1 --",
            "'; DROP TABLE books; --",
            "a\"b",
            "NEAR(",
            "*",
            "(((((",
            "正向内容\"\"\"",
            "^abc",
            "a:b(c)",
            "***",
            "x\".x",
        ] {
            // 关键点是不能 panic、不能返回数据库错误，更不能把表删掉。
            // FTS5 语法错误会直接冒泡成 Err，所以这一条同时覆盖了 MATCH 转义。
            if let Err(error) = global_search_impl(&db, &GlobalSearchRequest { query: hostile.into(), ..request_default() }) {
                panic!("输入 {hostile:?} 触发了错误：{error}");
            }
        }
        // 表结构与数据都还在
        assert_eq!(
            db.connect().unwrap().query_row("SELECT count(*) FROM books", [], |r| r.get::<_, i64>(0)).unwrap(),
            1
        );
        assert_eq!(
            db.connect().unwrap().query_row("SELECT count(*) FROM notes_fts", [], |r| r.get::<_, i64>(0)).unwrap(),
            1
        );
    }

    #[test]
    fn fts_match_expression_quotes_every_term_and_rejects_symbols_only() {
        // 引号在规范化阶段被剔除，剩下纯字母词；关键是结果里每个词都被双引号包住
        assert_eq!(fts_match_expression("a\"b").as_deref(), Some("\"ab\""));
        // 纯符号没有可用词，必须返回 None，否则空表达式会让 FTS5 报语法错误
        assert_eq!(fts_match_expression("***"), None);
        assert_eq!(fts_match_expression("((("), None);
        assert_eq!(fts_match_expression("   "), None);
        // 中文仍然走 search_terms 的整词 + bigram
        let expression = fts_match_expression("组织管理").expect("中文查询应当有可用词");
        assert!(expression.contains("\"组织管理\""));
        assert!(expression.contains("\"组织\""));
        assert!(expression.contains("\"管理\""));
    }

    #[test]
    fn global_search_paginates_without_duplicating_rows() {
        let (_dir, db) = search_test_db();
        seed_book(&db, "b1", "书", "作者", "分类");
        for index in 0..12 {
            seed_highlight(&db, &format!("h{index}"), "b1", "第一章", &format!("共同关键词第 {index} 条"));
        }
        let all = search(&db, GlobalSearchRequest { query: "共同关键词".into(), limit: Some(30), ..request_default() });
        let mut ids = all.results.iter().map(|item| item.id.clone()).collect::<Vec<_>>();
        let total = ids.len();
        ids.sort();

        let mut paged = Vec::new();
        for offset in [0, 5, 10] {
            let page = search(&db, GlobalSearchRequest { query: "共同关键词".into(), limit: Some(5), offset: Some(offset), ..request_default() });
            paged.extend(page.results.iter().map(|item| item.id.clone()));
        }
        paged.sort();
        assert_eq!(paged, ids, "分页结果应当与一次性取全量一致");
        assert_eq!(paged.len(), total);
    }

    #[test]
    fn global_search_empty_query_returns_recent_items() {
        let (_dir, db) = search_test_db();
        seed_book(&db, "b1", "甲书", "作者甲", "社科");
        seed_highlight(&db, "h1", "b1", "第一章", "最近的一条划线");

        let page = search(&db, request_default());
        assert!(!page.results.is_empty());
        // 空查询不跑 FTS，但仍然有明确上限
        let page = search(&db, GlobalSearchRequest { limit: Some(1), ..request_default() });
        assert_eq!(page.results.len(), 1);
    }

    #[test]
    fn global_search_rejects_unknown_type() {
        let (_dir, db) = search_test_db();
        let error = global_search_impl(&db, &GlobalSearchRequest { types: vec!["chapter".into()], ..request_default() }).unwrap_err();
        assert!(error.to_string().contains("不支持的搜索类型"));
    }

    #[test]
    fn global_search_snippet_is_truncated_in_backend() {
        let long = "字".repeat(GLOBAL_SEARCH_SNIPPET_CHARS + 100);
        let snippet = snippet_of(&long);
        assert!(snippet.chars().count() <= GLOBAL_SEARCH_SNIPPET_CHARS + 1);
        assert!(snippet.ends_with('…'));
        assert_eq!(snippet_of("  短句  "), "短句");
    }

    #[test]
    fn bm25_sign_is_flipped_into_larger_is_better() {
        // FTS5 的 bm25 返回负数，越小越相关；统一翻成「越大越相关」。
        assert!(relevance_from_bm25(-12.0) > relevance_from_bm25(-2.0));
        assert_eq!(relevance_from_bm25(3.0), 0.0, "异常正数不应变成负分");
        // 兜底扫描写 0.0，翻转后仍必须是 0，不能变成 -0.0 之外的数。
        assert_eq!(relevance_from_bm25(0.0), 0.0);
    }

    #[test]
    fn fts_命中最强项不会被零分阈值丢弃() {
        // 真实缺陷：relevance 直接用 bm25 负数时，FTS 命中的强结果 score 会变成负数，
        // 被 global_search_impl 的 `score <= 0.0` 过滤掉，反而不如兜底扫描。
        // 这里只验证打分链路：relevance 必须已经翻正，score 恒为正。
        let strong = NoteCandidate {
            id: "n1".into(),
            entity_type: "highlight".into(),
            book_id: "b1".into(),
            title: "地方债".into(),
            chapter: String::new(),
            content: "地方债的风险".into(),
            updated_at: String::new(),
            relevance: relevance_from_bm25(-12.0),
        };
        let weak = NoteCandidate {
            id: "n2".into(),
            entity_type: "highlight".into(),
            book_id: "b1".into(),
            title: "地方债".into(),
            chapter: String::new(),
            content: "地方债的风险".into(),
            updated_at: String::new(),
            relevance: relevance_from_bm25(-1.0),
        };
        let terms = vec!["地方债".to_string()];
        let strong_score = score_note(&strong, "地方债", &terms);
        let weak_score = score_note(&weak, "地方债", &terms);
        assert!(strong_score > 0.0, "FTS 最强命中必须能通过 score > 0 的筛选");
        assert!(strong_score > weak_score, "bm25 更负的候选分数必须更高");
        // 未翻转成负数时这里会得到 -12+12+5+1 = 6 之外的负值并被 continue 丢弃。
        assert!(strong_score >= 12.0, "relevance 必须以正数参与累加，实际 {strong_score}");
    }

    #[test]
    fn global_search_normalizes_scores_into_zero_to_one() {
        let mut results = vec![
            GlobalSearchResult { id: "a".into(), entity_type: "highlight".into(), book_id: "b".into(), title: "t".into(), subtitle: String::new(), snippet: String::new(), score: 10.0, updated_at: String::new() },
            GlobalSearchResult { id: "b".into(), entity_type: "highlight".into(), book_id: "b".into(), title: "t".into(), subtitle: String::new(), snippet: String::new(), score: 5.0, updated_at: String::new() },
        ];
        normalize_scores(&mut results);
        assert!((results[0].score - 1.0).abs() < 1e-9);
        assert!((results[1].score - 0.5).abs() < 1e-9);
    }

    #[test]
    fn book_title_boost_prefers_exact_then_prefix_then_contains() {
        let exact = book_title_boost("置身事内", "置身事内", "", "", "");
        let prefix = book_title_boost("置身事", "置身事内", "", "", "");
        let contains = book_title_boost("事内", "置身事内", "", "", "");
        assert!(exact > prefix && prefix > contains);
        assert_eq!(contains, 1.0);
        // ISBN 命中比标题包含更值钱
        assert!(book_title_boost("9787", "无关书名", "", "", "9787508660752") > contains);
    }

    #[test]
    fn books_fts_backfill_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        {
            let db = Database::open(&path).unwrap();
            seed_book(&db, "b1", "甲书", "作者甲", "社科");
            seed_book(&db, "b2", "乙书", "作者乙", "历史");
        }
        let count_indexed = |db: &Database| -> i64 {
            db.connect().unwrap().query_row("SELECT count(*) FROM books_fts", [], |r| r.get(0)).unwrap()
        };
        // 再次打开（模拟重复启动）不会重复插入
        for _ in 0..3 {
            Database::open(&path).unwrap();
        }
        let db = Database::open(&path).unwrap();
        assert_eq!(count_indexed(&db), 2);
    }

    // ---------- Reranker ----------

    fn seed_reranker(db: &Database, settings: &RerankerSettings) {
        db.connect()
            .unwrap()
            .execute(
                "INSERT INTO reranker_settings(id,provider,endpoint,model,top_n,enabled,updated_at)
                 VALUES(1,?1,?2,?3,?4,?5,0)
                 ON CONFLICT(id) DO UPDATE SET provider=excluded.provider,endpoint=excluded.endpoint,
                   model=excluded.model,top_n=excluded.top_n,enabled=excluded.enabled",
                rusqlite::params![settings.provider, settings.endpoint, settings.model, settings.top_n, settings.enabled as i64],
            )
            .unwrap();
    }

    fn usable_reranker() -> RerankerSettings {
        RerankerSettings {
            provider: "Cohere".into(),
            endpoint: "https://api.cohere.com/v1/rerank".into(),
            model: "rerank-multilingual-v3.0".into(),
            top_n: 8,
            enabled: true,
        }
    }

    fn notes_of(results: Vec<SearchResult>) -> Vec<String> {
        results.into_iter().map(|item| item.note.id).collect()
    }

    fn candidates(db: &Database, book_id: &str, count: usize) -> Vec<SearchResult> {
        seed_book(db, book_id, "书", "作者", "分类");
        (0..count)
            .map(|index| {
                seed_highlight(db, &format!("h{index}"), book_id, "第一章", &format!("候选正文 {index}"));
                SearchResult {
                    note: Note {
                        id: format!("h{index}"),
                        note_type: "highlight".into(),
                        book_id: book_id.into(),
                        book_title: "书".into(),
                        chapter: "第一章".into(),
                        content: format!("候选正文 {index}"),
                        created_at: String::new(),
                    },
                    // hybrid_search 用负分表示名次，越小越靠前。
                    score: -(index as f64),
                }
            })
            .collect()
    }

    #[test]
    fn rerank_未配置时原样返回候选() {
        let (_dir, db) = search_test_db();
        let local = candidates(&db, "b1", 5);
        let (results, warning) = run_async(maybe_rerank(&db, "问题", "ask", &[], local.clone(), &AtomicBool::new(false)));
        assert!(warning.is_empty());
        assert_eq!(notes_of(results), notes_of(local));
    }

    #[test]
    fn rerank_关闭时原样返回候选() {
        let (_dir, db) = search_test_db();
        let mut settings = usable_reranker();
        settings.enabled = false;
        seed_reranker(&db, &settings);
        let local = candidates(&db, "b1", 4);
        let (results, warning) = run_async(maybe_rerank(&db, "问题", "ask", &[], local.clone(), &AtomicBool::new(false)));
        assert!(warning.is_empty());
        assert_eq!(notes_of(results), notes_of(local));
    }

    #[test]
    fn rerank_summary_与_compare_模式不重排() {
        let (_dir, db) = search_test_db();
        seed_reranker(&db, &usable_reranker());
        // 这两种模式的检索输入是空串，重排没有 query 可用
        let local = candidates(&db, "b1", 3);
        let (results, warning) = run_async(maybe_rerank(&db, "问题", "compare", &[], local.clone(), &AtomicBool::new(false)));
        assert!(warning.is_empty());
        assert_eq!(notes_of(results), notes_of(local));
    }

    #[test]
    fn rerank_缺少_api_key_时降级并给出提示() {
        let (_dir, db) = search_test_db();
        seed_reranker(&db, &usable_reranker());
        let local = candidates(&db, "b1", 3);
        let (results, warning) = run_async(maybe_rerank(&db, "问题", "ask", &[], local.clone(), &AtomicBool::new(false)));
        // 没有 key 也不能让 AI 失败：候选原样返回，附带一条提示
        assert!(!warning.is_empty(), "应当给出降级提示");
        assert_eq!(notes_of(results), notes_of(local), "候选必须原样返回");
    }

    #[test]
    fn rerank_端点无效时降级而不是报错() {
        let (_dir, db) = search_test_db();
        let mut settings = usable_reranker();
        // 指向一个必定拒绝连接的本地端口
        settings.endpoint = "http://127.0.0.1:9/rerank".into();
        seed_reranker(&db, &settings);
        let local = candidates(&db, "b1", 3);
        // 所有降级路径都必须给出非空 warning 且不丢候选
        let (results, warning) = run_async(maybe_rerank(&db, "问题", "ask", &[], local.clone(), &AtomicBool::new(false)));
        assert!(!warning.is_empty());
        assert_eq!(results.len(), local.len());
    }

    #[test]
    fn rerank_不会发送候选集以外的数据() {
        let (_dir, db) = search_test_db();
        seed_reranker(&db, &usable_reranker());
        // 库里还有别的笔记，但只有传入的候选会进入重排
        seed_book(&db, "b2", "另一本书", "另一位作者", "分类");
        seed_highlight(&db, "other", "b2", "第二章", "不该被发送的正文");
        let local = candidates(&db, "b1", 3);
        let (results, _warning) = run_async(maybe_rerank(&db, "问题", "ask", &[], local, &AtomicBool::new(false)));
        assert!(!results.iter().any(|item| item.note.id == "other"));
    }

    #[test]
    fn rerank_配置校验覆盖开关打开与关闭两种情况() {
        let (_dir, db) = search_test_db();
        // 关闭时允许 endpoint 非法，用户才能先关掉坏配置
        let mut broken = usable_reranker();
        broken.enabled = false;
        broken.endpoint = "http://insecure.example.com/rerank".into();
        save_reranker_settings_impl(&db, &broken, None).expect("关闭状态下不应因 endpoint 报错");
        let saved = reranker_settings_from_db(&db).unwrap().expect("配置应当被保存");
        assert!(!saved.enabled);

        // 打开时必须校验
        broken.enabled = true;
        assert!(save_reranker_settings_impl(&db, &broken, None).is_err());
    }

    #[test]
    fn rerank_top_n_越界被拒绝() {
        let (_dir, db) = search_test_db();
        let mut settings = usable_reranker();
        settings.enabled = false;
        settings.top_n = 999;
        assert!(save_reranker_settings_impl(&db, &settings, None).unwrap_err().to_string().contains("Top N"));
    }

    #[test]
    fn rerank_开关与_top_n_能正确往返数据库() {
        let (_dir, db) = search_test_db();
        let mut settings = usable_reranker();
        settings.top_n = 12;
        settings.enabled = false;
        save_reranker_settings_impl(&db, &settings, None).unwrap();
        let saved = reranker_settings_from_db(&db).unwrap().unwrap();
        assert_eq!(saved.top_n, 12);
        assert!(!saved.enabled);
        assert_eq!(saved.provider, "Cohere");

        // 再存一次应当是覆盖而不是插第二行
        settings.enabled = true;
        save_reranker_settings_impl(&db, &settings, None).unwrap();
        let count = db.connect().unwrap().query_row("SELECT count(*) FROM reranker_settings", [], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!(count, 1);
    }

    // ---------- 导入资料 ----------

    fn seed_source(db: &Database, source_id: &str, source_type: &str, title: &str, deleted: bool) {
        db.connect()
            .unwrap()
            .execute(
                "INSERT INTO library_sources(id,source_type,title,content_hash,page_count,imported_at,updated_at,is_deleted)
                 VALUES(?1,?2,?3,?4,1,1700000000,1700000000,?5)",
                rusqlite::params![source_id, source_type, title, format!("hash-{source_id}"), deleted as i64],
            )
            .unwrap();
    }

    fn seed_source_doc(db: &Database, doc_id: &str, source_id: &str, heading: &str, content: &str, locator: crate::import::Locator) {
        db.connect()
            .unwrap()
            .execute(
                "INSERT INTO source_documents(id,source_id,position,heading,content,locator_json) VALUES(?1,?2,0,?3,?4,?5)",
                rusqlite::params![doc_id, source_id, heading, content, serde_json::to_string(&locator).unwrap()],
            )
            .unwrap();
    }

    #[test]
    fn 导入资料_按内容哈希判重() {
        let (_dir, db) = search_test_db();
        let chunks = vec![crate::import::DocumentChunk {
            position: 0,
            heading: "第一章".into(),
            content: "一些正文".into(),
            locator: crate::import::Locator::default(),
        }];
        let parsed = crate::import::ParsedSource {
            title: "测试资料".into(),
            kind: Some(crate::import::SourceKind::Epub),
            chunks,
            ..Default::default()
        };
        crate::import::validate_size(crate::import::SourceKind::Epub, &parsed.chunks).unwrap();
        assert!(find_duplicate(&db, crate::import::SourceKind::Epub, "测试资料", "一些正文").unwrap().is_none());

        // 直接插一条同 hash 的记录
        let hash = crate::import::content_hash(crate::import::SourceKind::Epub, "测试资料", "一些正文");
        db.connect()
            .unwrap()
            .execute(
                "INSERT INTO library_sources(id,source_type,title,content_hash,page_count,imported_at,updated_at,is_deleted)
                 VALUES('s1','epub','测试资料',?1,1,0,0,0)",
                [&hash],
            )
            .unwrap();
        assert_eq!(find_duplicate(&db, crate::import::SourceKind::Epub, "测试资料", "一些正文").unwrap().as_deref(), Some("s1"));
    }

    #[test]
    fn 导入资料_软删除后不再算重复() {
        let (_dir, db) = search_test_db();
        let hash = crate::import::content_hash(crate::import::SourceKind::Web, "网页", "正文");
        db.connect()
            .unwrap()
            .execute(
                "INSERT INTO library_sources(id,source_type,title,content_hash,page_count,imported_at,updated_at,is_deleted)
                 VALUES('s1','web','网页',?1,0,0,0,1)",
                [&hash],
            )
            .unwrap();
        assert!(find_duplicate(&db, crate::import::SourceKind::Web, "网页", "正文").unwrap().is_none());
    }

    #[test]
    fn 导入资料_软删除后不出现在列表() {
        let (_dir, db) = search_test_db();
        seed_source(&db, "s1", "pdf", "在架资料", false);
        seed_source(&db, "s2", "epub", "已删资料", true);
        let visible = list_sources_impl(&db, false).unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].title, "在架资料");
        let all = list_sources_impl(&db, true).unwrap();
        assert_eq!(all.len(), 2);
    }

    #[test]
    fn 导入资料_删除后从搜索里消失() {
        let (_dir, db) = search_test_db();
        seed_source(&db, "s1", "pdf", "导入的PDF", false);
        seed_source_doc(&db, "d1", "s1", "第 1 页", "独特关键词内容", crate::import::Locator { page: Some(1), chapter: None, heading: None });

        let page = search(&db, GlobalSearchRequest { query: "独特关键词".into(), ..request_default() });
        assert!(page.results.iter().any(|item| item.entity_type == "source" && item.id == "d1"));

        db.connect().unwrap().execute("UPDATE library_sources SET is_deleted=1 WHERE id='s1'", []).unwrap();
        let page = search(&db, GlobalSearchRequest { query: "独特关键词".into(), ..request_default() });
        assert!(!page.results.iter().any(|item| item.entity_type == "source"));
    }

    #[test]
    fn 导入资料_搜索结果带页码或章节定位() {
        let (_dir, db) = search_test_db();
        seed_source(&db, "s1", "pdf", "一本PDF", false);
        seed_source_doc(&db, "d1", "s1", "第 12 页", "关键词甲", crate::import::Locator { page: Some(12), chapter: None, heading: None });
        seed_source(&db, "s2", "epub", "一本EPUB", false);
        seed_source_doc(&db, "d2", "s2", "第三章", "关键词甲", crate::import::Locator { page: None, chapter: Some(3), heading: None });

        let page = search(&db, GlobalSearchRequest { query: "关键词甲".into(), ..request_default() });
        let labels: Vec<String> = page.results.iter().filter(|item| item.entity_type == "source").map(|item| item.subtitle.clone()).collect();
        assert!(labels.contains(&"第 12 页".to_string()), "PDF 结果应带页码，实际 {labels:?}");
        assert!(labels.contains(&"第 3 章".to_string()), "EPUB 结果应带章节号，实际 {labels:?}");
    }

    #[test]
    fn 导入资料_限定书籍时不参与搜索() {
        let (_dir, db) = search_test_db();
        seed_book(&db, "b1", "书", "作者", "分类");
        seed_source(&db, "s1", "pdf", "导入的PDF", false);
        seed_source_doc(&db, "d1", "s1", "第 1 页", "关键词乙", crate::import::Locator { page: Some(1), chapter: None, heading: None });

        let page = search(&db, GlobalSearchRequest { query: "关键词乙".into(), book_id: Some("b1".into()), ..request_default() });
        assert!(!page.results.iter().any(|item| item.entity_type == "source"), "限定书籍时导入资料不应出现");
    }

    #[test]
    fn 导入资料_文档块增删改会同步_fts() {
        let (_dir, db) = search_test_db();
        seed_source(&db, "s1", "pdf", "导入的PDF", false);
        seed_source_doc(&db, "d1", "s1", "第 1 页", "原始唯一词汇", crate::import::Locator { page: Some(1), chapter: None, heading: None });

        let count = |db: &Database| -> i64 { db.connect().unwrap().query_row("SELECT count(*) FROM source_docs_fts", [], |r| r.get(0)).unwrap() };
        assert_eq!(count(&db), 1);

        // 改内容：旧词应当消失
        db.connect().unwrap().execute("UPDATE source_documents SET content='修改后的新词汇' WHERE id='d1'", []).unwrap();
        assert_eq!(count(&db), 1, "更新不应产生重复行");
        let old_hit: i64 = db
            .connect()
            .unwrap()
            .query_row("SELECT count(*) FROM source_docs_fts WHERE source_docs_fts MATCH '\"原始唯一词汇\"'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(old_hit, 0, "旧内容必须从 FTS 移除");

        // 删文档：FTS 同步清理
        db.connect().unwrap().execute("DELETE FROM source_documents WHERE id='d1'", []).unwrap();
        assert_eq!(count(&db), 0, "删除文档必须同步清理 FTS");
    }

    #[test]
    fn 导入资料_删除来源会级联清理文档与_fts() {
        let (_dir, db) = search_test_db();
        seed_source(&db, "s1", "epub", "导入的EPUB", false);
        seed_source_doc(&db, "d1", "s1", "第一章", "内容甲", crate::import::Locator::default());
        db.connect().unwrap().execute("DELETE FROM library_sources WHERE id='s1'", []).unwrap();
        let docs: i64 = db.connect().unwrap().query_row("SELECT count(*) FROM source_documents", [], |r| r.get(0)).unwrap();
        let fts: i64 = db.connect().unwrap().query_row("SELECT count(*) FROM source_docs_fts", [], |r| r.get(0)).unwrap();
        assert_eq!(docs, 0, "文档应当级联删除");
        assert_eq!(fts, 0, "FTS 应当同步清理");
    }

    #[test]
    fn 导入资料_资料详情返回全部文档块() {
        let (_dir, db) = search_test_db();
        seed_source(&db, "s1", "pdf", "导入的PDF", false);
        seed_source_doc(&db, "d1", "s1", "第 1 页", "第一段", crate::import::Locator { page: Some(1), chapter: None, heading: None });
        seed_source_doc(&db, "d2", "s1", "第 2 页", "第二段", crate::import::Locator { page: Some(2), chapter: None, heading: None });

        let detail = source_detail_impl(&db, "s1").unwrap();
        assert_eq!(detail.source.title, "导入的PDF");
        assert_eq!(detail.documents.len(), 2);
        assert_eq!(detail.documents[1].locator.page, Some(2));
        assert!(source_detail_impl(&db, "missing").is_err());
    }

    #[test]
    fn 导入资料_内容_hash_唯一索引拦住重复入库() {
        let (_dir, db) = search_test_db();
        seed_source(&db, "s1", "pdf", "书", false);
        let second = db
            .connect()
            .unwrap()
            .execute(
                "INSERT INTO library_sources(id,source_type,title,content_hash,page_count,imported_at,updated_at,is_deleted)
                 VALUES('s2','pdf','书','hash-s1',1,0,0,0)",
                [],
            );
        assert!(second.is_err(), "相同内容 hash 重复入库必须被唯一索引拒绝");
    }

    #[test]
    fn 导入资料_封面文件名必须清洗() {
        // 只允许自己生成的文件名，避免 purge 时被传入任意路径
        assert_eq!(cover_file_name("s1", "JPG"), "cover-s1.jpg");
        assert_eq!(cover_file_name("s1", ".png"), "cover-s1.png");
        // 不同资料的封面不能重名，否则删一份会带走别人的封面
        assert_ne!(cover_file_name("s1", "jpg"), cover_file_name("s2", "jpg"));
    }

    #[test]
    fn 导入资料_恶意_cover_path_不能通过校验() {
        for evil in [
            "../../secret.txt",
            "..\\..\\secret.txt",
            "/etc/passwd",
            "C:\\Windows\\System32\\drivers\\etc\\hosts",
            "imports/cover-s1.jpg",
            "",
            "other-file.jpg",
        ] {
            assert!(!is_own_cover_file(evil), "不该被放行：{evil}");
        }
        assert!(is_own_cover_file("cover-abc123.jpg"));
        assert!(is_own_cover_file("cover-abc123.png"));
    }

    // ---------- 本地文件校验 ----------

    /// 写一个临时文件，返回其路径。
    fn write_temp(dir: &std::path::Path, name: &str, bytes: &[u8]) -> std::path::PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, bytes).unwrap();
        path
    }

    /// 最小可解析 PDF：单页、一行正文、带标题元数据。
    ///
    /// 用 lopdf 自己写出来，而不是手拼字节 —— 手写的 xref 偏移错一位就
    /// 会得到一个「结构损坏」的文件，测的就不是导入器而是我的字节拼接了。
    fn sample_pdf_bytes() -> Vec<u8> {
        use lopdf::{dictionary, Document, Object, Stream};

        let mut doc = Document::with_version("1.4");
        let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let pages_id = doc.new_object_id();
        let stream = doc.add_object(Stream::new(dictionary! {}, b"BT /F1 12 Tf 72 720 Td (Hello imported body) Tj ET".to_vec()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => stream,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        });
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Count" => 1, "Kids" => vec![page.into()] }));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        let info = doc.add_object(dictionary! { "Title" => Object::string_literal("Sample Doc"), "Producer" => Object::string_literal("test") });
        doc.trailer.set("Info", info);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("写出样本 PDF 失败");
        bytes
    }

    #[test]
    fn 导入资料_空路径被拒绝() {
        assert!(read_local_file("   ").unwrap_err().to_string().contains("请选择"));
    }

    #[test]
    fn 导入资料_不存在的路径被拒绝() {
        let (dir, _db) = search_test_db();
        let path = dir.path().join("nope.pdf");
        let error = read_local_file(path.to_str().unwrap()).unwrap_err().to_string();
        assert!(error.contains("文件不存在或无法读取"), "实际：{error}");
    }

    #[test]
    fn 导入资料_目录路径被拒绝() {
        let (dir, _db) = search_test_db();
        let folder = dir.path().join("folder.pdf");
        std::fs::create_dir_all(&folder).unwrap();
        let error = read_local_file(folder.to_str().unwrap()).unwrap_err().to_string();
        assert!(error.contains("文件夹"), "实际：{error}");
    }

    #[test]
    fn 导入资料_不支持的扩展名被拒绝() {
        let (dir, _db) = search_test_db();
        for name in ["book.txt", "book.mobi", "book", "book.pdf.zip"] {
            let path = write_temp(dir.path(), name, b"whatever");
            let error = read_local_file(path.to_str().unwrap()).unwrap_err().to_string();
            assert!(error.contains("请选择 PDF 或 EPUB 文件"), "{name} 实际：{error}");
        }
    }

    #[test]
    fn 导入资料_大写扩展名可以识别() {
        let (dir, _db) = search_test_db();
        for name in ["book.PDF", "book.Pdf"] {
            let path = write_temp(dir.path(), name, &sample_pdf_bytes());
            let (parsed, fingerprint) = read_local_file(path.to_str().unwrap()).unwrap();
            assert_eq!(parsed.kind, Some(crate::import::SourceKind::Pdf), "{name} 应当识别为 PDF");
            assert!(!fingerprint.is_empty());
        }
    }

    #[test]
    fn 导入资料_改名的假文件被拒绝() {
        let (dir, _db) = search_test_db();
        // ZIP 改名 .pdf
        let path = write_temp(dir.path(), "fake.pdf", b"PK\x03\x04\x14\x00\x00\x00rest");
        let error = read_local_file(path.to_str().unwrap()).unwrap_err().to_string();
        assert!(error.contains("扩展名是 PDF"), "实际：{error}");

        // HTML 改名 .pdf
        let path = write_temp(dir.path(), "page.pdf", b"<!DOCTYPE html><html><body>hi</body></html>");
        let error = read_local_file(path.to_str().unwrap()).unwrap_err().to_string();
        assert!(error.contains("扩展名是 PDF"), "实际：{error}");

        // PDF 改名 .epub
        let path = write_temp(dir.path(), "book.epub", &sample_pdf_bytes());
        let error = read_local_file(path.to_str().unwrap()).unwrap_err().to_string();
        assert!(error.contains("扩展名是 EPUB"), "实际：{error}");
    }

    #[test]
    fn 导入资料_中文与空格路径可以读取() {
        let (dir, _db) = search_test_db();
        let path = write_temp(dir.path(), "我的 资料/测试 书.pdf", &sample_pdf_bytes());
        let (parsed, _) = read_local_file(path.to_str().unwrap()).unwrap();
        assert_eq!(parsed.title, "Sample Doc");
        assert_eq!(parsed.origin.as_deref(), Some(path.to_str().unwrap()));
    }

    #[test]
    fn 导入资料_超限文件在完整读取前被拒绝() {
        let (dir, _db) = search_test_db();
        let path = dir.path().join("huge.pdf");
        // set_len 只是把文件长度标大，不写内容：能验证「先看 metadata 再读」
        let file = std::fs::File::create(&path).unwrap();
        file.set_len(MAX_LOCAL_FILE_BYTES + 1).unwrap();
        drop(file);
        let error = read_local_file(path.to_str().unwrap()).unwrap_err().to_string();
        assert!(error.contains("体积超过"), "实际：{error}");
    }

    #[test]
    fn 导入资料_指纹只跟内容有关() {
        let (dir, _db) = search_test_db();
        let bytes = sample_pdf_bytes();
        let first = write_temp(dir.path(), "a.pdf", &bytes);
        let second = write_temp(dir.path(), "b.pdf", &bytes);
        let (_, fingerprint_a) = read_local_file(first.to_str().unwrap()).unwrap();
        let (_, fingerprint_b) = read_local_file(second.to_str().unwrap()).unwrap();
        assert_eq!(fingerprint_a, fingerprint_b, "同一份内容在不同路径下指纹必须相同");

        let mut changed = bytes.clone();
        changed.extend_from_slice(b"\n% edited\n");
        let third = write_temp(dir.path(), "c.pdf", &changed);
        let (_, fingerprint_c) = read_local_file(third.to_str().unwrap()).unwrap();
        assert_ne!(fingerprint_a, fingerprint_c, "内容变了指纹必须变");
    }

    // ---------- 删除与清理 ----------

    #[test]
    fn 导入资料_软删除保留正文但不参与搜索() {
        let (_dir, db) = search_test_db();
        seed_source(&db, "s1", "pdf", "可被搜到的原始词汇", false);
        seed_source_doc(&db, "d1", "s1", "第 1 页", "可被搜到的原始词汇", crate::import::Locator { page: Some(1), chapter: None, heading: None });

        let changed = db
            .connect()
            .unwrap()
            .execute("UPDATE library_sources SET is_deleted=1 WHERE id='s1'", [])
            .unwrap();
        assert_eq!(changed, 1);
        // 数据库正文还在
        let docs: i64 = db.connect().unwrap().query_row("SELECT count(*) FROM source_documents WHERE source_id='s1'", [], |r| r.get(0)).unwrap();
        assert_eq!(docs, 1, "软删除不能删正文");
        // 但列表与搜索都看不到
        assert!(list_sources_impl(&db, false).unwrap().is_empty());
        let hits = source_candidates(&db, Some("原始词汇"), None, 10).unwrap();
        assert!(hits.is_empty(), "软删除的资料不该出现在搜索里");
    }

    #[test]
    fn 导入资料_永久删除清掉文档与索引() {
        let (_dir, db) = search_test_db();
        seed_source(&db, "s1", "pdf", "待彻底删除的词汇", false);
        seed_source_doc(&db, "d1", "s1", "第 1 页", "待彻底删除的词汇", crate::import::Locator { page: Some(1), chapter: None, heading: None });
        assert_eq!(source_candidates(&db, Some("待彻底删除"), None, 10).unwrap().len(), 1);

        let removed = db.connect().unwrap().execute("DELETE FROM library_sources WHERE id='s1'", []).unwrap();
        assert_eq!(removed, 1);

        let exists: i64 = db.connect().unwrap().query_row("SELECT count(*) FROM library_sources WHERE id='s1'", [], |r| r.get(0)).unwrap();
        assert_eq!(exists, 0);
        let docs: i64 = db.connect().unwrap().query_row("SELECT count(*) FROM source_documents WHERE source_id='s1'", [], |r| r.get(0)).unwrap();
        assert_eq!(docs, 0, "外键级联必须清掉文档块");
        let fts: i64 = db.connect().unwrap().query_row("SELECT count(*) FROM source_docs_fts", [], |r| r.get(0)).unwrap();
        assert_eq!(fts, 0, "FTS 索引必须清掉");
        assert!(source_candidates(&db, Some("待彻底删除"), None, 10).unwrap().is_empty());
    }

    #[test]
    fn 导入资料_软删除默认不出现在列表里() {
        let (_dir, db) = search_test_db();
        seed_source(&db, "s1", "pdf", "书", false);
        assert_eq!(list_sources_impl(&db, false).unwrap().len(), 1);
        assert_eq!(list_sources_impl(&db, true).unwrap().len(), 1);
        db.connect().unwrap().execute("UPDATE library_sources SET is_deleted=1 WHERE id='s1'", []).unwrap();
        assert!(list_sources_impl(&db, false).unwrap().is_empty());
        assert_eq!(list_sources_impl(&db, true).unwrap().len(), 1, "显式请求时仍能看到已移除的资料");
    }

    // ---------- 概念级知识图谱 ----------

    fn seed_entity(db: &Database, id: &str, kind: &str, name: &str, status: &str, confidence: f64) {
        let c = db.connect().unwrap();
        c.execute(
            "INSERT INTO knowledge_entities(id,kind,canonical_name,description,aliases_json,status,source_hash,updated_at)
             VALUES(?1,?2,?3,'','[]',?4,'',0)",
            rusqlite::params![id, kind, name, status],
        )
        .unwrap();
        c.execute(
            "INSERT INTO knowledge_entity_evidence(entity_id,note_id,book_id,quote,confidence) VALUES(?1,'n1','b1','引文',?2)",
            rusqlite::params![id, confidence],
        )
        .unwrap();
    }

    fn seed_concept_relation(db: &Database, id: &str, from: &str, to: &str, confidence: f64) {
        db.connect()
            .unwrap()
            .execute(
                "INSERT INTO knowledge_relations(id,from_entity_id,to_entity_id,relation,summary,confidence,evidence_json,input_hash,updated_at)
                 VALUES(?1,?2,?3,'related','关系说明',?4,'[]','',0)",
                rusqlite::params![id, from, to, confidence],
            )
            .unwrap();
    }

    #[test]
    fn 概念图谱_空库返回空图() {
        let (_dir, db) = search_test_db();
        let graph = list_concept_graph_impl(&db, &ConceptGraphQuery::default()).unwrap();
        assert!(graph.entities.is_empty());
        assert!(graph.relations.is_empty());
        assert!(!graph.truncated);
    }

    #[test]
    fn 概念图谱_按类型过滤() {
        let (_dir, db) = search_test_db();
        seed_entity(&db, "concept:1", "concept", "组织", "confirmed", 0.9);
        seed_entity(&db, "topic:1", "topic", "效率", "confirmed", 0.9);
        seed_entity(&db, "idea:1", "idea", "我的判断", "confirmed", 0.9);

        let graph = list_concept_graph_impl(&db, &ConceptGraphQuery { kinds: vec!["topic".into()], ..Default::default() }).unwrap();
        assert_eq!(graph.entities.len(), 1);
        assert_eq!(graph.entities[0].kind, "topic");
    }

    #[test]
    fn 概念图谱_按书籍过滤() {
        let (_dir, db) = search_test_db();
        seed_entity(&db, "concept:a", "concept", "甲", "confirmed", 0.9);
        db.connect().unwrap().execute(
            "INSERT INTO knowledge_entity_evidence(entity_id,note_id,book_id,quote,confidence) VALUES('concept:a','n2','b2','引文',0.9)", []).unwrap();
        seed_entity(&db, "concept:b", "concept", "乙", "confirmed", 0.9);
        db.connect().unwrap().execute(
            "INSERT INTO knowledge_entity_evidence(entity_id,note_id,book_id,quote,confidence) VALUES('concept:b','n2','b2','引文',0.9)", []).unwrap();
        seed_entity(&db, "concept:c", "concept", "丙", "confirmed", 0.9);

        let graph = list_concept_graph_impl(&db, &ConceptGraphQuery { book_id: Some("b2".into()), ..Default::default() }).unwrap();
        assert_eq!(graph.entities.len(), 2, "限定书籍后只返回有该书证据的概念");
        assert!(!graph.entities.iter().any(|item| item.canonical_name == "丙"));
    }

    #[test]
    fn 概念图谱_默认不返回隐藏实体() {
        let (_dir, db) = search_test_db();
        seed_entity(&db, "concept:1", "concept", "可见", "confirmed", 0.9);
        seed_entity(&db, "concept:2", "concept", "隐藏的", "hidden", 0.9);

        let graph = list_concept_graph_impl(&db, &ConceptGraphQuery::default()).unwrap();
        assert_eq!(graph.entities.len(), 1);
        assert_eq!(graph.entities[0].canonical_name, "可见");

        let graph = list_concept_graph_impl(&db, &ConceptGraphQuery { include_hidden: true, ..Default::default() }).unwrap();
        assert_eq!(graph.entities.len(), 2);
    }

    #[test]
    fn 概念图谱_拒绝未知类型() {
        let (_dir, db) = search_test_db();
        let error = list_concept_graph_impl(&db, &ConceptGraphQuery { kinds: vec!["paragraph".into()], ..Default::default() }).unwrap_err();
        assert!(error.to_string().contains("不支持的实体类型"));
    }

    #[test]
    fn 关系两端都在结果集里才返回() {
        let (_dir, db) = search_test_db();
        seed_entity(&db, "concept:1", "concept", "甲", "confirmed", 0.9);
        seed_entity(&db, "concept:2", "concept", "乙", "confirmed", 0.9);
        seed_entity(&db, "concept:3", "concept", "丙", "confirmed", 0.9);
        // 丙只和乙有关系，和甲没有关系
        seed_concept_relation(&db, "r1", "concept:1", "concept:2", 0.9);
        seed_concept_relation(&db, "r2", "concept:2", "concept:3", 0.9);

        let graph = list_concept_graph_impl(&db, &ConceptGraphQuery { center_id: Some("concept:1".into()), ..Default::default() }).unwrap();
        let ids: Vec<String> = graph.entities.iter().map(|item| item.id.clone()).collect();
        assert!(!ids.contains(&"concept:3".to_string()), "无连接的实体不应出现");
        for relation in &graph.relations {
            assert!(ids.contains(&relation.from_entity_id) && ids.contains(&relation.to_entity_id), "关系两端必须都在返回的节点里");
        }
    }

    #[test]
    fn 关系置信度低于阈值时被过滤() {
        let (_dir, db) = search_test_db();
        seed_entity(&db, "concept:1", "concept", "甲", "confirmed", 0.9);
        seed_entity(&db, "concept:2", "concept", "乙", "confirmed", 0.9);
        seed_concept_relation(&db, "r1", "concept:1", "concept:2", 0.2);

        let graph = list_concept_graph_impl(&db, &ConceptGraphQuery { min_confidence: Some(0.5), ..Default::default() }).unwrap();
        assert!(graph.relations.is_empty());
    }

    #[test]
    fn 大图会标记为截断并只返回高置信度子图() {
        let (_dir, db) = search_test_db();
        let c = db.connect().unwrap();
        {
            let mut insert = c.prepare("INSERT INTO knowledge_entities(id,kind,canonical_name,description,aliases_json,status,source_hash,updated_at) VALUES(?1,'concept',?2,'','[]','confirmed','',0)").unwrap();
            let mut evidence = c.prepare("INSERT INTO knowledge_entity_evidence(entity_id,note_id,book_id,quote,confidence) VALUES(?1,?2,'b1','引文',?3)").unwrap();
            for index in 0..520 {
                let id = format!("concept:{index}");
                insert.execute(rusqlite::params![id, format!("概念{index}")]).unwrap();
                // 置信度都低于截断阈值 0.45
                evidence.execute(rusqlite::params![id, format!("n{index}"), 0.3]).unwrap();
            }
        }
        let graph = list_concept_graph_impl(&db, &ConceptGraphQuery::default()).unwrap();
        assert!(graph.truncated, "超过 500 节点应标记截断");
        assert_eq!(graph.total_entities, 520);
        assert!(graph.entities.is_empty(), "低置信度实体应被高置信度门槛挡掉");
    }

    #[test]
    fn 中心实体扩展只返回邻居() {
        let (_dir, db) = search_test_db();
        seed_entity(&db, "concept:1", "concept", "中心", "confirmed", 0.9);
        seed_entity(&db, "concept:2", "concept", "邻居", "confirmed", 0.9);
        seed_entity(&db, "concept:3", "concept", "无关", "confirmed", 0.9);
        seed_concept_relation(&db, "r1", "concept:1", "concept:2", 0.9);

        let graph = list_concept_graph_impl(&db, &ConceptGraphQuery { center_id: Some("concept:1".into()), ..Default::default() }).unwrap();
        let ids: Vec<String> = graph.entities.iter().map(|item| item.id.clone()).collect();
        assert!(ids.contains(&"concept:1".to_string()));
        assert!(ids.contains(&"concept:2".to_string()));
        assert!(!ids.contains(&"concept:3".to_string()));
        assert_eq!(graph.relations.len(), 1);
    }

    #[test]
    fn 内容未变化的笔记会被跳过() {
        let (_dir, db) = search_test_db();
        let notes = vec![("n1".to_string(), "同一段内容".to_string()), ("n2".to_string(), "另一段内容".to_string())];
        assert!(unchanged_notes(&db, &notes).unwrap().is_empty());

        db.connect().unwrap().execute(
            "INSERT INTO knowledge_note_state(note_id,book_id,content_hash,scanned_at) VALUES('n1','b1',?1,0)",
            [content_hash("同一段内容")],
        ).unwrap();

        let skipped = unchanged_notes(&db, &notes).unwrap();
        assert_eq!(skipped, vec!["n1".to_string()], "只有内容未变化的笔记才跳过");

        // 内容变了就不再跳过
        let changed = vec![("n1".to_string(), "改过的内容".to_string())];
        assert!(unchanged_notes(&db, &changed).unwrap().is_empty());
    }

    #[test]
    fn 写入后能读回实体与证据() {
        let (_dir, db) = search_test_db();
        let notes = vec![("n1".to_string(), "b1".to_string(), "书一".to_string(), "地方债务".to_string())];
        let raw = crate::ai::concepts::RawExtraction {
            entities: vec![crate::ai::concepts::RawEntity {
                kind: "concept".into(),
                canonical_name: "地方债务".into(),
                description: "定义".into(),
                aliases: vec!["政府债务".into()],
                confidence: Some(0.8),
                evidence_note_ids: vec!["n1".into()],
            }],
            relations: vec![],
        };
        let validated = crate::ai::concepts::validate_extraction(&raw, &notes, &std::collections::HashSet::new());
        let (created, _) = persist_extraction(&db, &validated, &[("n1".to_string(), "地方债务".to_string())]).unwrap();
        assert_eq!(created, 1);

        let graph = list_concept_graph_impl(&db, &ConceptGraphQuery::default()).unwrap();
        assert_eq!(graph.entities.len(), 1);
        assert_eq!(graph.entities[0].canonical_name, "地方债务");
        assert_eq!(graph.entities[0].status, "suggested", "新抽取的实体默认是待确认");
        assert_eq!(graph.entities[0].aliases, vec!["政府债务".to_string()]);

        // 再次写入同一实体不应重复建节点，只补证据
        let (created_again, _) = persist_extraction(&db, &validated, &[]).unwrap();
        assert_eq!(created_again, 0);
        assert_eq!(list_concept_graph_impl(&db, &ConceptGraphQuery::default()).unwrap().entities.len(), 1);
    }

    #[test]
    fn 伪造的_note_id_不会入库() {
        let (_dir, db) = search_test_db();
        let notes = vec![("n1".to_string(), "b1".to_string(), "书一".to_string(), "真实内容".to_string())];
        let raw = crate::ai::concepts::RawExtraction {
            entities: vec![crate::ai::concepts::RawEntity {
                kind: "concept".into(),
                canonical_name: "伪造概念".into(),
                description: String::new(),
                aliases: vec![],
                confidence: Some(0.9),
                evidence_note_ids: vec!["n999".into()],
            }],
            relations: vec![],
        };
        let validated = crate::ai::concepts::validate_extraction(&raw, &notes, &std::collections::HashSet::new());
        persist_extraction(&db, &validated, &[]).unwrap();
        let graph = list_concept_graph_impl(&db, &ConceptGraphQuery::default()).unwrap();
        assert!(graph.entities.is_empty(), "伪造证据的概念不得出现在图谱里");
    }

    #[test]
    fn 重命名与别名可以往返数据库() {
        let (_dir, db) = search_test_db();
        seed_entity(&db, "concept:1", "concept", "旧名字", "suggested", 0.9);
        let updated = correct_entity_impl(&db, &EntityCorrection {
            id: "concept:1".into(),
            canonical_name: Some("新名字".into()),
            description: Some("说明".into()),
            aliases: vec!["别名一".into(), "别名一".into()],
            status: Some("confirmed".into()),
        })
        .unwrap();
        assert_eq!(updated.canonical_name, "新名字");
        assert_eq!(updated.description, "说明");
        assert_eq!(updated.aliases, vec!["别名一".to_string()], "重复别名要去重");
        assert_eq!(updated.status, "confirmed");

        // 重启后仍在
        let reloaded = concept_entity_with_evidence(&db, "concept:1").unwrap();
        assert_eq!(reloaded.canonical_name, "新名字");
        assert_eq!(reloaded.status, "confirmed");
        assert_eq!(reloaded.aliases, vec!["别名一".to_string()]);
    }

    #[test]
    fn 拒绝非法状态与空名称() {
        let (_dir, db) = search_test_db();
        seed_entity(&db, "concept:1", "concept", "原名", "suggested", 0.9);
        assert!(correct_entity_impl(&db, &EntityCorrection { id: "concept:1".into(), status: Some("deleted".into()), ..Default::default() }).is_err());
        assert!(correct_entity_impl(&db, &EntityCorrection { id: "concept:1".into(), canonical_name: Some("   ".into()), ..Default::default() }).is_err());
    }

    #[test]
    fn 合并会搬运证据并删掉自环() {
        let (_dir, db) = search_test_db();
        seed_entity(&db, "concept:1", "concept", "甲", "confirmed", 0.9);
        seed_entity(&db, "concept:2", "concept", "乙", "confirmed", 0.9);
        seed_entity(&db, "concept:3", "concept", "丙", "confirmed", 0.9);
        seed_concept_relation(&db, "r1", "concept:1", "concept:2", 0.9);
        seed_concept_relation(&db, "r2", "concept:2", "concept:3", 0.9);
        db.connect().unwrap().execute(
            "INSERT INTO knowledge_entity_evidence(entity_id,note_id,book_id,quote,confidence) VALUES('concept:1','n2','b1','甲的证据',0.4)", []).unwrap();

        merge_entities_impl(&db, "concept:1", "concept:2").unwrap();

        let graph = list_concept_graph_impl(&db, &ConceptGraphQuery::default()).unwrap();
        assert_eq!(graph.entities.len(), 2, "合并后只剩两个实体");
        assert!(!graph.entities.iter().any(|item| item.id == "concept:1"));

        let target = concept_entity_with_evidence(&db, "concept:2").unwrap();
        assert_eq!(target.evidence.len(), 2, "来源实体的证据要搬过来");
        assert!(target.aliases.contains(&"甲".to_string()), "来源实体的名称要成为别名");
        assert!(!graph.relations.iter().any(|item| item.from_entity_id == item.to_entity_id), "合并后不允许自环");
    }

    #[test]
    fn 合并时重复关系只保留一条() {
        let (_dir, db) = search_test_db();
        seed_entity(&db, "concept:1", "concept", "甲", "confirmed", 0.9);
        seed_entity(&db, "concept:2", "concept", "乙", "confirmed", 0.9);
        seed_entity(&db, "concept:3", "concept", "丙", "confirmed", 0.9);
        // concept:1 ↔ concept:3 有两条方向不同的重复边
        seed_concept_relation(&db, "r1", "concept:1", "concept:3", 0.9);
        seed_concept_relation(&db, "r2", "concept:3", "concept:1", 0.8);

        merge_entities_impl(&db, "concept:1", "concept:2").unwrap();

        let graph = list_concept_graph_impl(&db, &ConceptGraphQuery::default()).unwrap();
        let involving_target = graph
            .relations
            .iter()
            .filter(|item| item.from_entity_id == "concept:2" || item.to_entity_id == "concept:2")
            .count();
        assert_eq!(involving_target, 1, "重复边必须去重");
    }

    #[test]
    fn 合并时保留更高的证据置信度() {
        let (_dir, db) = search_test_db();
        seed_entity(&db, "concept:1", "concept", "甲", "confirmed", 0.9);
        seed_entity(&db, "concept:2", "concept", "乙", "confirmed", 0.9);
        // seed_entity 已经给两者都建了同一条 n1 证据，这里只调整甲的置信度更低
        db.connect().unwrap().execute(
            "UPDATE knowledge_entity_evidence SET confidence=0.2,quote='低置信' WHERE entity_id='concept:1'", []).unwrap();

        merge_entities_impl(&db, "concept:1", "concept:2").unwrap();

        let target = concept_entity_with_evidence(&db, "concept:2").unwrap();
        assert_eq!(target.evidence.len(), 1, "同一笔记不应重复");
        assert_eq!(target.evidence[0].confidence, 0.9, "同一笔记冲突时保留更高置信度");
    }

    #[test]
    fn 拒绝合并到自身或不存在的实体() {
        let (_dir, db) = search_test_db();
        seed_entity(&db, "concept:1", "concept", "甲", "confirmed", 0.9);
        assert!(merge_entities_impl(&db, "concept:1", "concept:1").is_err());
        assert!(merge_entities_impl(&db, "concept:1", "concept:missing").is_err());
        assert!(merge_entities_impl(&db, "concept:missing", "concept:1").is_err());
    }

    #[test]
    fn 清理建议项只删_suggested() {
        let (_dir, db) = search_test_db();
        seed_entity(&db, "concept:1", "concept", "待确认", "suggested", 0.9);
        seed_entity(&db, "concept:2", "concept", "已确认", "confirmed", 0.9);
        seed_entity(&db, "concept:3", "concept", "已隐藏", "hidden", 0.9);

        let removed = clear_suggested_concepts_impl(&db).unwrap();
        assert_eq!(removed, 1);
        let graph = list_concept_graph_impl(&db, &ConceptGraphQuery { include_hidden: true, ..Default::default() }).unwrap();
        assert_eq!(graph.entities.len(), 2);
        assert!(!graph.entities.iter().any(|item| item.canonical_name == "待确认"));
    }

    #[test]
    fn 空查询翻页不会拿到空页() {
        // 真实缺陷：空查询分支先 truncate(limit) 再 skip(offset)，
        // offset=limit 时把结果全跳光，第二页恒空，而 has_more 仍为 true，
        // 前端「加载更多」无限空转。
        let (_dir, db) = search_test_db();
        seed_book(&db, "b1", "甲书", "作者甲", "社科");
        for index in 0..7 {
            seed_highlight(&db, &format!("h{index}"), "b1", "第一章", &format!("划线正文 {index}"));
        }

        let request = |offset: i64| GlobalSearchRequest {
            query: "".into(),
            types: vec!["highlight".into()],
            book_id: None,
            limit: Some(3),
            offset: Some(offset),
        };

        let first = global_search_impl(&db, &request(0)).unwrap();
        assert_eq!(first.results.len(), 3, "第一页应满页");
        assert!(first.has_more);

        let second = global_search_impl(&db, &request(3)).unwrap();
        assert_eq!(second.results.len(), 3, "第二页不能是空的");
        assert!(second.has_more);

        let third = global_search_impl(&db, &request(6)).unwrap();
        assert_eq!(third.results.len(), 1, "最后一页应只剩 1 条");
        assert!(!third.has_more, "取完之后 has_more 必须为 false，否则前端继续空转");

        // 三页合起来必须是全部 7 条且互不重复。
        let ids: Vec<String> = first
            .results
            .iter()
            .chain(second.results.iter())
            .chain(third.results.iter())
            .map(|item| item.id.clone())
            .collect();
        assert_eq!(ids.len(), 7);
        let unique: std::collections::HashSet<&String> = ids.iter().collect();
        assert_eq!(unique.len(), 7, "翻页不得重复");
    }

    #[test]
    fn 清理建议项不会留下孤儿关系() {
        // 真实缺陷：knowledge_relations 没有指向 knowledge_entities 的外键，
        // 删实体只级联清 evidence，关系行会残留；重扫时同 id 实体重建，
        // 这些陈旧关系带着旧 summary/confidence 重新出现在图谱里。
        let (_dir, db) = search_test_db();
        seed_entity(&db, "concept:1", "concept", "待确认甲", "suggested", 0.9);
        seed_entity(&db, "concept:2", "concept", "已确认乙", "confirmed", 0.9);
        seed_entity(&db, "concept:3", "concept", "已确认丙", "confirmed", 0.9);
        db.connect()
            .unwrap()
            .execute(
                "INSERT INTO knowledge_relations(id,from_entity_id,to_entity_id,relation,summary,confidence,updated_at)
                 VALUES('r1','concept:1','concept:2','supports','陈旧关系',0.9,1)",
                [],
            )
            .unwrap();
        db.connect()
            .unwrap()
            .execute(
                "INSERT INTO knowledge_relations(id,from_entity_id,to_entity_id,relation,summary,confidence,updated_at)
                 VALUES('r2','concept:2','concept:3','supports','正常关系',0.9,1)",
                [],
            )
            .unwrap();

        clear_suggested_concepts_impl(&db).unwrap();

        let left: Vec<String> = db
            .connect()
            .unwrap()
            .prepare("SELECT id FROM knowledge_relations ORDER BY id")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        // 只应剩 r2；r1 两端之一被删，必须一起消失。
        assert_eq!(left, vec!["r2".to_string()], "孤儿关系必须随实体一起清理");
    }

    #[test]
    fn 名词改名不会撞主键且能按新名检索() {
        // 真实缺陷：save_glossary_term 按 term 文本查已有行，改名后查不到 →
        // 走 INSERT 分支并带上原 id → UNIQUE constraint failed: glossary_terms.id，
        // 名词库改名功能完全不可用。
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("test.db")).unwrap();
        let mut term = GlossaryTerm {
            id: 0,
            term: "人工智能".into(),
            canonical_name: "人工智能".into(),
            aliases: vec![],
            definition: "研究让机器具有智能的学科".into(),
            source: String::new(),
            source_title: String::new(),
            source_url: String::new(),
            wikipedia_snapshot: String::new(),
            status: String::new(),
            updated_at: 0,
            external_page_id: 0,
            source_revision_id: 0,
            source_dump_version: String::new(),
            source_updated_at: 0,
            source_synced_at: 0,
            license_code: String::new(),
            manually_edited: false,
            source_content_hash: String::new(),
            published_batch_id: String::new(),
        };
        save_glossary_term_impl(&db, &term).unwrap();
        let created_id: i64 = db
            .connect()
            .unwrap()
            .query_row("SELECT id FROM glossary_terms WHERE term='人工智能'", [], |r| r.get(0))
            .unwrap();

        // 改名后保存：必须更新原行，而不是插入撞主键。
        term.id = created_id;
        term.term = "机器学习".into();
        term.canonical_name = "机器学习".into();
        save_glossary_term_impl(&db, &term).unwrap();

        let total: i64 = db
            .connect()
            .unwrap()
            .query_row("SELECT count(*) FROM glossary_terms", [], |r| r.get(0))
            .unwrap();
        assert_eq!(total, 1, "改名不应产生新行");
        let stored: (String, i64) = db
            .connect()
            .unwrap()
            .query_row("SELECT term,id FROM glossary_terms WHERE id=?1", [created_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        assert_eq!(stored, ("机器学习".to_string(), created_id), "原行应被就地改名");
        // normalized_term 必须一起更新，否则按新名词检索不到。
        let by_new_name = glossary_terms_for_review(&db, Some("机器学习"), None, None).unwrap();
        assert_eq!(by_new_name.len(), 1, "改名后必须能按新名词检索到");
        let by_old_name = glossary_terms_for_review(&db, Some("人工智能"), None, None).unwrap();
        assert!(by_old_name.is_empty(), "旧名词不应再命中");
    }

    #[test]
    fn 概念四张表迁移可重复执行() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.db");
        for _ in 0..3 {
            Database::open(&path).unwrap();
        }
        let c = Database::open(&path).unwrap().connect().unwrap();
        for table in ["knowledge_entities", "knowledge_entity_evidence", "knowledge_relations", "knowledge_note_state"] {
            let count: i64 = c
                .query_row("SELECT count(*) FROM sqlite_master WHERE name=?1", [table], |r| r.get(0))
                .unwrap();
            assert_eq!(count, 1, "{table} 应当存在且只有一份");
        }
    }

    #[test]
    fn 解析_ai_返回的_json_容忍代码块与散文() {
        let clean: crate::ai::concepts::RawExtraction = parse_extraction(r#"{"entities":[],"relations":[]}"#).unwrap();
        assert!(clean.entities.is_empty());

        let fenced = parse_extraction("```json\n{\"entities\":[{\"kind\":\"concept\",\"canonical_name\":\"甲\"}]}\n```").unwrap();
        assert_eq!(fenced.entities.len(), 1);

        let wrapped = parse_extraction("好的，结果如下：{\"entities\":[{\"kind\":\"concept\",\"canonical_name\":\"甲\"}]} 以上。").unwrap();
        assert_eq!(wrapped.entities[0].canonical_name, "甲");

        // 没有 JSON 时明确报错
        assert!(parse_extraction("完全没有 JSON").is_err());
        assert!(parse_extraction("").is_err());
    }

    #[test]
    fn 按书籍分批取笔记且过滤已删除内容() {
        let (_dir, db) = search_test_db();
        seed_book(&db, "b1", "书一", "作者", "分类");
        seed_book(&db, "b2", "书二", "作者", "分类");
        seed_highlight(&db, "h1", "b1", "第一章", "划线一");
        seed_highlight(&db, "h2", "b1", "第一章", "划线二");
        seed_thought(&db, "t1", "b1", "第二章", "想法一");
        seed_highlight(&db, "h3", "b2", "第一章", "另一本书");
        db.connect().unwrap().execute("UPDATE highlights SET is_deleted=1 WHERE bookmark_id='h2'", []).unwrap();

        let groups = notes_grouped_by_book(&db, None).unwrap();
        assert_eq!(groups.len(), 2, "两本书各一组");
        let first = groups.iter().find(|group| group.book_id == "b1").unwrap();
        assert_eq!(first.notes.len(), 2, "已删除的划线不应出现");

        let only_b2 = notes_grouped_by_book(&db, Some("b2")).unwrap();
        assert_eq!(only_b2.len(), 1);
        assert_eq!(only_b2[0].book_id, "b2");
    }

    #[test]
    fn 已知实体名包含别名() {
        let (_dir, db) = search_test_db();
        db.connect().unwrap().execute(
            "INSERT INTO knowledge_entities(id,kind,canonical_name,description,aliases_json,status,source_hash,updated_at)
             VALUES('concept:1','concept','组织管理','定义','[\"组织\"]','confirmed','',0)",
            [],
        ).unwrap();
        let names = known_entity_names(&db).unwrap();
        assert!(names.contains("组织管理"));
        assert!(names.contains("组织"), "别名也要算已知名字");
    }
}
