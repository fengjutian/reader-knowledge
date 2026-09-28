use crate::ai::provider::AiProvider;
use crate::{database::Database, error::AppError, models::*};
use rusqlite::OptionalExtension;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, State};
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
pub fn list_books(db: State<'_, Database>) -> Result<Vec<Book>, AppError> {
    let c = db.connect()?;
    let mut q=c.prepare("SELECT b.book_id,b.title,coalesce(b.author,''),coalesce(b.cover,''),coalesce(h.total,0),coalesce(t.total,0),CASE b.finish_reading WHEN 1 THEN 100 ELSE 0 END,coalesce(datetime(b.read_update_time,'unixepoch','localtime'),'') FROM books b LEFT JOIN (SELECT book_id,count(*) total FROM highlights WHERE is_deleted=0 GROUP BY book_id) h ON h.book_id=b.book_id LEFT JOIN (SELECT book_id,count(*) total FROM thoughts WHERE is_deleted=0 GROUP BY book_id) t ON t.book_id=b.book_id WHERE b.is_deleted=0 ORDER BY b.read_update_time DESC")?;
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
pub fn get_book(db: State<'_, Database>, book_id: String) -> Result<BookDetail, AppError> {
    let c = db.connect()?;
    c.query_row(
        "SELECT b.book_id,b.title,coalesce(b.author,''),coalesce(b.cover,''),
         (SELECT count(*) FROM highlights h WHERE h.book_id=b.book_id AND h.is_deleted=0),
         (SELECT count(*) FROM thoughts t WHERE t.book_id=b.book_id AND t.is_deleted=0),
         CASE b.finish_reading WHEN 1 THEN 100 ELSE 0 END,
         coalesce(datetime(b.read_update_time,'unixepoch','localtime'),''),coalesce(b.category,''),b.deep_link,b.finish_reading=1
         FROM books b WHERE b.book_id=?1 AND b.is_deleted=0",
        [book_id],
        |r| Ok(BookDetail { book: Book { id:r.get(0)?,title:r.get(1)?,author:r.get(2)?,cover:r.get(3)?,highlight_count:r.get(4)?,thought_count:r.get(5)?,progress:r.get(6)?,updated_at:r.get(7)? }, category:r.get(8)?,deep_link:r.get(9)?,finished:r.get(10)? }),
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
            [book_id],
            |row| row.get(0),
        )
        .optional()?;
    let web_link = reqwest::Url::parse("https://weread.qq.com/web/reader/")
        .ok()
        .and_then(|mut url| {
            url.path_segments_mut().ok()?.push(&book_id);
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
         ORDER BY 7 DESC LIMIT 500",
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
    if question.trim().is_empty() {
        return Err(AppError::Message("问题不能为空".into()));
    }
    let results = rag_search(&db, &question, &request.mode, &request.book_ids)?;
    if results.is_empty() {
        return Err(AppError::Message(
            "没有检索到相关笔记，无法生成有依据的回答".into(),
        ));
    }
    let mut context = String::new();
    for (index, result) in results.iter().enumerate() {
        let source = if result.note.note_type == "thought" {
            "用户想法"
        } else {
            "书籍原文划线"
        };
        let content: String = result.note.content.chars().take(1200).collect();
        context.push_str(&format!(
            "[{}]\ntype: {}\nbook: 《{}》\nchapter: {}\ncontent: {}\n\n",
            index + 1,
            source,
            result.note.book_title,
            result.note.chapter,
            content
        ));
    }
    let system = "你是 ReadFlow 的个人阅读知识助手。只能依据提供的阅读笔记回答；必须区分书籍原文划线与用户自己的想法；每个重要结论使用 [数字] 标注来源；证据不足时必须明确说明；不得把作者观点描述成用户观点。";
    let task = match request.mode.as_str() {
        "summary" => "任务类型：单书总结。提炼主题、核心观点和用户想法，不要逐条复述。",
        "compare" => "任务类型：跨书分析。明确列出各书的共识、分歧与可互相补充之处。",
        _ => "任务类型：基于阅读知识库回答问题。",
    };
    let prompt = format!(
        "{}\n\n以下是检索到的笔记：\n\n{}\n用户问题：{}",
        task,
        context,
        question.trim()
    );
    let provider = ai_provider(&db)?;
    let content = provider
        .chat(&[
            ChatMessage {
                role: "system".into(),
                content: system.into(),
            },
            ChatMessage {
                role: "user".into(),
                content: prompt,
            },
        ])
        .await?;
    let citations = results
        .into_iter()
        .enumerate()
        .filter(|(index, _)| content.contains(&format!("[{}]", index + 1)))
        .map(|(index, result)| Citation {
            index: index + 1,
            note: result.note,
        })
        .collect();
    Ok(AiAnswer { content, citations })
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
        let per_book = (20 / book_ids.len()).max(3);
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
        combined.truncate(20);
        return Ok(combined);
    }
    hybrid_search(db, search_input, None, book_ids, 20)
}

fn hybrid_search(
    db: &Database,
    input: &str,
    kind: Option<&str>,
    book_ids: &[String],
    limit: usize,
) -> Result<Vec<SearchResult>, AppError> {
    let c = db.connect()?;
    let mut query = c.prepare(
        "SELECT note_id,note_type,book_id,title,chapter_title,content,
         CASE note_type WHEN 'highlight' THEN coalesce(datetime((SELECT create_time FROM highlights WHERE bookmark_id=note_id),'unixepoch','localtime'),'') ELSE coalesce(datetime((SELECT create_time FROM thoughts WHERE review_id=note_id),'unixepoch','localtime'),'') END, 0.0
         FROM notes_fts WHERE (?1 IS NULL OR note_type=?1)"
    )?;
    let notes = query
        .query_map([kind], map_note)?
        .collect::<Result<Vec<_>, _>>()?;
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
    use super::{fts_query, search_terms};

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
