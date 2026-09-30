use crate::ai::provider::AiProvider;
use crate::{database::Database, error::AppError, models::*};
use rusqlite::OptionalExtension;
use serde_json::Value;
use std::{collections::{HashMap, HashSet}, time::{SystemTime, UNIX_EPOCH}};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;

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
    let table_specs = [("books", "书籍"), ("chapters", "章节"), ("highlights", "划线"), ("thoughts", "想法"), ("weread_raw", "原始数据"), ("sync_sessions", "同步记录"), ("note_embeddings", "向量索引"), ("relation_analysis_cache", "关系分析缓存")];
    let mut tables = Vec::with_capacity(table_specs.len());
    for (name, label) in table_specs {
        let rows = c.query_row(&format!("SELECT count(*) FROM {name}"), [], |row| row.get(0))?;
        tables.push(DatabaseTableStat { name: name.into(), label: label.into(), rows });
    }
    let mut query = c.prepare("SELECT CASE WHEN trim(coalesce(category,''))='' THEN '未分类' ELSE category END, count(*) FROM books WHERE is_deleted=0 GROUP BY 1 ORDER BY 2 DESC LIMIT 8")?;
    let categories = query.query_map([], |row| Ok(DatabaseCategoryStat { label: row.get(0)?, count: row.get(1)? }))?.collect::<Result<Vec<_>, _>>()?;
    let last_synced_at = c.query_row("SELECT datetime(last_synced_at,'unixepoch','localtime') FROM sync_state WHERE source='weread'", [], |row| row.get(0)).optional()?;
    Ok(DatabaseOverview { size_bytes: db.size_bytes(), tables, categories, last_synced_at })
}

#[tauri::command]
pub fn list_database_rows(db: State<'_, Database>, table: String, query: String, limit: i64, offset: i64) -> Result<DatabaseRows, AppError> {
    let c = db.connect()?;
    let pattern = format!("%{}%", query.trim());
    let limit = limit.clamp(1, 100);
    let offset = offset.max(0);
    let (count_sql, rows_sql) = match table.as_str() {
        "books" => ("SELECT count(*) FROM books WHERE is_deleted=0 AND (title LIKE ?1 OR coalesce(author,'') LIKE ?1 OR book_id LIKE ?1)", "SELECT book_id,title,coalesce(author,''),coalesce(category,''),coalesce(datetime(read_update_time,'unixepoch','localtime'),'') FROM books WHERE is_deleted=0 AND (title LIKE ?1 OR coalesce(author,'') LIKE ?1 OR book_id LIKE ?1) ORDER BY read_update_time DESC LIMIT ?2 OFFSET ?3"),
        "highlights" => ("SELECT count(*) FROM highlights h LEFT JOIN books b ON b.book_id=h.book_id WHERE h.is_deleted=0 AND (h.mark_text LIKE ?1 OR coalesce(h.chapter_title,'') LIKE ?1 OR coalesce(b.title,'') LIKE ?1)", "SELECT h.bookmark_id,h.mark_text,coalesce(b.title,''),coalesce(h.chapter_title,''),coalesce(datetime(h.create_time,'unixepoch','localtime'),'') FROM highlights h LEFT JOIN books b ON b.book_id=h.book_id WHERE h.is_deleted=0 AND (h.mark_text LIKE ?1 OR coalesce(h.chapter_title,'') LIKE ?1 OR coalesce(b.title,'') LIKE ?1) ORDER BY h.create_time DESC LIMIT ?2 OFFSET ?3"),
        "thoughts" => ("SELECT count(*) FROM thoughts t LEFT JOIN books b ON b.book_id=t.book_id WHERE t.is_deleted=0 AND (t.content LIKE ?1 OR coalesce(t.chapter_name,'') LIKE ?1 OR coalesce(b.title,'') LIKE ?1)", "SELECT t.review_id,t.content,coalesce(b.title,''),coalesce(t.chapter_name,''),coalesce(datetime(t.create_time,'unixepoch','localtime'),'') FROM thoughts t LEFT JOIN books b ON b.book_id=t.book_id WHERE t.is_deleted=0 AND (t.content LIKE ?1 OR coalesce(t.chapter_name,'') LIKE ?1 OR coalesce(b.title,'') LIKE ?1) ORDER BY t.create_time DESC LIMIT ?2 OFFSET ?3"),
        "sync_sessions" => ("SELECT count(*) FROM sync_sessions WHERE source LIKE ?1 OR status LIKE ?1 OR coalesce(error_message,'') LIKE ?1", "SELECT id,status,source,printf('书籍 %d · 划线 %d · 想法 %d',books_fetched,highlights_fetched,thoughts_fetched),coalesce(datetime(started_at,'unixepoch','localtime'),'') FROM sync_sessions WHERE source LIKE ?1 OR status LIKE ?1 OR coalesce(error_message,'') LIKE ?1 ORDER BY started_at DESC LIMIT ?2 OFFSET ?3"),
        _ => return Err(AppError::Message("不支持浏览该数据表".into())),
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
    client.call("/book/recommend", params).await
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
) -> Result<BookPage, AppError> {
    let c = db.connect()?;
    let pattern = format!("%{}%", query.trim());
    let total = c.query_row(
        "SELECT count(*) FROM books WHERE is_deleted=0 AND (title LIKE ?1 OR coalesce(author,'') LIKE ?1)",
        [&pattern],
        |row| row.get(0),
    )?;
    let mut statement = c.prepare(
        "SELECT b.book_id,b.title,coalesce(b.author,''),coalesce(b.category,''),coalesce(b.cover,''),coalesce(h.total,0),coalesce(t.total,0),CASE b.finish_reading WHEN 1 THEN 100 ELSE 0 END,coalesce(datetime(b.read_update_time,'unixepoch','localtime'),''),CASE WHEN b.finish_reading=1 THEN 'finished' WHEN coalesce(b.read_update_time,0)>0 OR coalesce(h.total,0)>0 OR coalesce(t.total,0)>0 THEN 'reading' ELSE 'unread' END
         FROM books b
         LEFT JOIN (SELECT book_id,count(*) total FROM highlights WHERE is_deleted=0 GROUP BY book_id) h ON h.book_id=b.book_id
         LEFT JOIN (SELECT book_id,count(*) total FROM thoughts WHERE is_deleted=0 GROUP BY book_id) t ON t.book_id=b.book_id
         WHERE b.is_deleted=0 AND (b.title LIKE ?1 OR coalesce(b.author,'') LIKE ?1)
         ORDER BY b.read_update_time DESC,b.book_id
         LIMIT ?2 OFFSET ?3",
    )?;
    let books = statement.query_map(rusqlite::params![pattern, limit.clamp(1, 500), offset.max(0)], |row| {
        Ok(Book {
            id: row.get(0)?, title: row.get(1)?, author: row.get(2)?, category: row.get(3)?, cover: row.get(4)?,
            highlight_count: row.get(5)?, thought_count: row.get(6)?, progress: row.get(7)?, updated_at: row.get(8)?, reading_status: row.get(9)?,
        })
    })?.collect::<Result<Vec<_>, _>>()?;
    Ok(BookPage { total, books })
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

#[tauri::command]
pub async fn fetch_books_metadata(db: State<'_, Database>, book_ids: Vec<String>, source: String, force: bool) -> Result<Vec<MetadataFetchResult>, AppError> {
    if book_ids.len() > 100 { return Err(AppError::Message("单次最多补全 100 本书".into())); }
    let mut results=Vec::with_capacity(book_ids.len());
    for (index,book_id) in book_ids.iter().enumerate() {
        match crate::metadata::fetch(&db,book_id,&source,force).await {
            Ok(value)=>results.push(value),
            Err(error)=>results.push(MetadataFetchResult{book_id:book_id.clone(),source:source.clone(),status:"failed".into(),message:error.to_string()}),
        }
        if index+1<book_ids.len(){tokio::time::sleep(std::time::Duration::from_millis(1200)).await;}
    }
    Ok(results)
}

#[tauri::command]
pub fn list_glossary_terms(db:State<'_,Database>,query:Option<String>)->Result<Vec<GlossaryTerm>,AppError>{
    let pattern=format!("%{}%",query.unwrap_or_default()); let c=db.connect()?;
    let mut q=c.prepare("SELECT id,term,canonical_name,aliases_json,definition,source,coalesce(source_title,''),coalesce(source_url,''),coalesce(wikipedia_snapshot,''),status,updated_at FROM glossary_terms WHERE term LIKE ?1 OR canonical_name LIKE ?1 OR definition LIKE ?1 ORDER BY updated_at DESC")?;
    let rows=q.query_map([pattern],|r|Ok(GlossaryTerm{id:r.get(0)?,term:r.get(1)?,canonical_name:r.get(2)?,aliases:serde_json::from_str(&r.get::<_,String>(3)?).unwrap_or_default(),definition:r.get(4)?,source:r.get(5)?,source_title:r.get(6)?,source_url:r.get(7)?,wikipedia_snapshot:r.get(8)?,status:r.get(9)?,updated_at:r.get(10)?}))?.collect::<Result<Vec<_>,_>>()?;
    Ok(rows)
}

#[tauri::command]
pub fn save_glossary_term(db:State<'_,Database>,term:GlossaryTerm)->Result<(),AppError>{
    if term.term.trim().is_empty()||term.definition.trim().is_empty(){return Err(AppError::Message("名词和解释不能为空".into()));}
    let now=SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64;
    db.connect()?.execute("INSERT INTO glossary_terms(id,term,canonical_name,aliases_json,definition,source,source_title,source_url,wikipedia_snapshot,status,updated_at) VALUES(nullif(?1,0),?2,?3,?4,?5,?6,?7,?8,?9,?10,?11) ON CONFLICT(term) DO UPDATE SET canonical_name=excluded.canonical_name,aliases_json=excluded.aliases_json,definition=excluded.definition,source=excluded.source,source_title=excluded.source_title,source_url=excluded.source_url,wikipedia_snapshot=excluded.wikipedia_snapshot,status=excluded.status,updated_at=excluded.updated_at",rusqlite::params![term.id,term.term.trim(),term.canonical_name.trim(),serde_json::to_string(&term.aliases)?,term.definition.trim(),term.source,term.source_title,term.source_url,term.wikipedia_snapshot,term.status,now])?; Ok(())
}

#[tauri::command]
pub fn delete_glossary_term(db:State<'_,Database>,id:i64)->Result<(),AppError>{db.connect()?.execute("DELETE FROM glossary_terms WHERE id=?1",[id])?;Ok(())}

#[tauri::command]
pub async fn search_wikipedia(term:String)->Result<Vec<WikipediaCandidate>,AppError>{
    let client=reqwest::Client::builder().user_agent("wereader/0.1 personal knowledge app").timeout(std::time::Duration::from_secs(12)).build()?;
    let value:Value=client.get("https://zh.wikipedia.org/w/rest.php/v1/search/page").query(&[("q",term.as_str()),("limit","5")]).send().await?.error_for_status()?.json().await?;
    let pages=value.get("pages").and_then(Value::as_array).cloned().unwrap_or_default();
    Ok(pages.into_iter().filter_map(|page|{let title=page.get("title")?.as_str()?.to_owned();let description=page.get("description").and_then(Value::as_str).unwrap_or("").to_owned();let excerpt=page.get("excerpt").and_then(Value::as_str).unwrap_or("").replace("<span class=\"searchmatch\">","").replace("</span>","");let url=format!("https://zh.wikipedia.org/wiki/{}",title.replace(' ',"_"));Some(WikipediaCandidate{title,description,excerpt,url})}).collect())
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
    let allowed=parsed.scheme()=="https"&&matches!(host,"book.douban.com"|"www.douban.com"|"openlibrary.org"|"books.google.com"|"books.google.cn"|"books.googleapis.com");
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
    match keyring::Entry::new("ReadFlow", &kind)?.get_password() {
        Ok(value) => Ok(!value.is_empty()),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(error) => Err(AppError::Credential(error)),
    }
}
#[tauri::command]
pub async fn test_connection(kind: String, value: Option<String>) -> Result<bool, AppError> {
    let secret = match value.filter(|value| !value.trim().is_empty()) {
        Some(value) => value,
        None => keyring::Entry::new("ReadFlow", &kind)?.get_password()?,
    };
    if kind == "weread" {
        crate::weread::client::WeReadClient::new(secret)?
            .test()
            .await?;
    }
    Ok(true)
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
        candidates.push(SemanticRelation { id:format!("{}:{}",ids[a],ids[b]),from:ids[a].clone(),to:ids[b].clone(),score,keywords:Vec::new(),relation:if score >= 0.72 { "高度语义相关".into() } else if score >= 0.52 { "语义相关".into() } else { "潜在语义关联".into() },evidence });
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
        candidates.push(SemanticRelation{id:format!("thought:{left}:{right}"),from:left.clone(),to:right.clone(),score,keywords:Vec::new(),relation:"用户观点关联".into(),evidence});
    }}
    candidates.sort_by(|a,b| b.score.total_cmp(&a.score)); let mut degree: HashMap<String,usize> = HashMap::new();
    Ok(candidates.into_iter().filter(|edge| { if degree.get(&edge.from).copied().unwrap_or(0)>=12 && degree.get(&edge.to).copied().unwrap_or(0)>=12 { return false; } *degree.entry(edge.from.clone()).or_default()+=1; *degree.entry(edge.to.clone()).or_default()+=1; true }).collect())
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

fn glossary_context(db:&Database,text:&str)->Result<String,AppError>{
    let c=db.connect()?;let mut q=c.prepare("SELECT term,canonical_name,aliases_json,definition,source,coalesce(source_url,'') FROM glossary_terms WHERE status='confirmed' ORDER BY updated_at DESC")?;
    let rows=q.query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,String>(3)?,r.get::<_,String>(4)?,r.get::<_,String>(5)?)))?;
    let normalized_text=text.to_lowercase();let mut result=String::new();let mut index=1;
    for row in rows {let(term,name,aliases,definition,source,url)=row?;let aliases=serde_json::from_str::<Vec<String>>(&aliases).unwrap_or_default();let matched=[term.as_str(),name.as_str()].into_iter().chain(aliases.iter().map(String::as_str)).filter(|value|!value.trim().is_empty()).any(|value|normalized_text.contains(&value.to_lowercase()));if !matched{continue;}result.push_str(&format!("[W{index}] 名词：{}\n解释：{}\n来源：{}{}\n\n",if name.is_empty(){&term}else{&name},definition,if source=="wikipedia"{"中文维基百科"}else{"人工编辑"},if url.is_empty(){String::new()}else{format!("（{url}）")}));index+=1;if index>8{break;}}
    Ok(result)
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

#[tauri::command]
pub async fn ask_ai(db: State<'_, Database>, request: AiRequest) -> Result<AiAnswer, AppError> {
    let question = request.question;
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
    let results = rag_search(&db, &question, &request.mode, &request.book_ids)?;
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
    let metadata_book_ids = if request.book_ids.is_empty() { results.iter().map(|result| result.note.book_id.clone()).collect::<HashSet<_>>().into_iter().collect::<Vec<_>>() } else { request.book_ids.clone() };
    let metadata = metadata_context(&db, &metadata_book_ids)?;
    let glossary = glossary_context(&db,&format!("{}\n{}",question,context))?;
    if results.is_empty() && glossary.is_empty() {
        return Err(AppError::Message(
            "没有检索到相关笔记或名词解释，无法生成有依据的回答".into(),
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
    let provider = ai_provider(&db)?;
    let mut messages = vec![ChatMessage { role: "system".into(), content: system.into() }];
    for turn in request.history.iter().rev().take(8).rev() {
        messages.push(ChatMessage { role: "user".into(), content: turn.question.chars().take(1200).collect() });
        messages.push(ChatMessage { role: "assistant".into(), content: turn.answer.chars().take(5000).collect() });
    }
    messages.push(ChatMessage { role: "user".into(), content: prompt });
    let content = provider.chat(&messages).await?;
    let sources_considered = results.len();
    let citations = results
        .into_iter()
        .enumerate()
        .filter(|(index, _)| content.contains(&format!("[{}]", index + 1)))
        .map(|(index, result)| Citation {
            index: index + 1,
            note: result.note,
        })
        .collect();
    Ok(AiAnswer {
        content,
        citations,
        sources_considered,
    })
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

fn rag_search(
    db: &Database,
    question: &str,
    mode: &str,
    book_ids: &[String],
) -> Result<Vec<SearchResult>, AppError> {
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
        return Ok(combined);
    }
    hybrid_search(db, search_input, None, book_ids, 200)
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

fn normalize_search_text(input: &str) -> String {
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
    use super::{fts_query, parse_relation_analysis, search_terms, weread_reader_id};

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
}
