use crate::{database::Database, error::AppError, models::{BookMetadataRow, MetadataFetchResult}};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

pub async fn fetch_douban_url(db: &Database, book_id: &str, input_url: &str) -> Result<MetadataFetchResult, AppError> {
    let url = reqwest::Url::parse(input_url.trim()).map_err(|_| AppError::Message("请输入有效的豆瓣图书链接".into()))?;
    if url.scheme() != "https" || url.host_str() != Some("book.douban.com") || !url.path().starts_with("/subject/") {
        return Err(AppError::Message("仅支持 https://book.douban.com/subject/... 链接".into()));
    }
    let source_id = url.path_segments().and_then(|mut parts| { parts.next(); parts.next().map(str::to_owned) }).filter(|v| !v.is_empty()).ok_or_else(|| AppError::Message("豆瓣链接缺少条目 ID".into()))?;
    db.connect()?.query_row("SELECT 1 FROM books WHERE book_id=?1 AND is_deleted=0", [book_id], |_| Ok(()))?;
    let client = reqwest::Client::builder().timeout(Duration::from_secs(12)).user_agent("Mozilla/5.0 (compatible; ReadFlow/0.1; personal metadata lookup)").redirect(reqwest::redirect::Policy::limited(3)).build()?;
    let response = client.get(url.clone()).send().await?.error_for_status()?;
    if response.url().host_str() != Some("book.douban.com") {
        return Err(AppError::Message("豆瓣要求安全验证，本次拉取已停止".into()));
    }
    let html = response.text().await?;
    if html.contains("sec.douban.com") || html.contains("captcha") || html.contains("安全验证") {
        return Err(AppError::Message("豆瓣要求安全验证，本次拉取已停止".into()));
    }
    let data = parse_douban_html(&html, &source_id, url.as_str())?;
    save(db, book_id, "douban", &data)?;
    Ok(MetadataFetchResult { book_id:book_id.into(), source:"douban".into(), status:"updated".into(), message:"豆瓣公开条目信息已保存到本地".into() })
}

fn parse_douban_html(html: &str, source_id: &str, source_url: &str) -> Result<Value, AppError> {
    let json_ld = html.find("<script type=\"application/ld+json\">")
        .and_then(|start| { let body=&html[start+35..]; body.find("</script>").map(|end| &body[..end]) })
        .and_then(|body| serde_json::from_str::<Value>(body.trim()).ok());
    let meta = |property: &str| -> Option<String> {
        let marker=format!("property=\"{property}\"");
        let start=html.find(&marker)?;
        let tag_start=html[..start].rfind('<')?;
        let tag_end=html[start..].find('>')?+start;
        let tag=&html[tag_start..=tag_end];
        let content=tag.find("content=\"")?+9;
        let end=tag[content..].find('\"')?+content;
        Some(tag[content..end].replace("&amp;", "&").replace("&quot;", "\""))
    };
    let title=json_ld.as_ref().and_then(|v|v.get("name")).and_then(Value::as_str).map(str::to_owned).or_else(||meta("og:title")).ok_or_else(||AppError::Message("无法从豆瓣页面识别书名，页面结构可能已变化".into()))?;
    let authors=json_ld.as_ref().and_then(|v|v.get("author")).and_then(Value::as_array).map(|values|values.iter().filter_map(|a|a.get("name").and_then(Value::as_str).map(str::to_owned)).collect::<Vec<_>>()).unwrap_or_default();
    let isbn=json_ld.as_ref().and_then(|v|v.get("isbn")).and_then(Value::as_str).map(|v|vec![v.to_owned()]).unwrap_or_default();
    let image=json_ld.as_ref().and_then(|v|v.get("image")).and_then(Value::as_str).map(str::to_owned).or_else(||meta("og:image"));
    let description=json_ld.as_ref().and_then(|v|v.get("description")).and_then(Value::as_str).map(str::to_owned).or_else(||meta("og:description"));
    Ok(json!({"source_id":source_id,"source_url":source_url,"title":title,"authors":authors,"isbn":isbn,"cover_url":image,"description":description,"subjects":[],"raw":json_ld.unwrap_or_else(||json!({"title":title}))}))
}

pub fn list(db: &Database) -> Result<Vec<BookMetadataRow>, AppError> {
    let c = db.connect()?;
    let mut q = c.prepare(
        "SELECT b.book_id,b.title,coalesce(b.author,''),coalesce(m.cover_url,b.cover,''),
         coalesce(m.isbn13,m.isbn10,''),coalesce(m.publisher,''),coalesce(m.published_date,''),m.page_count,
         coalesce(m.subjects_json,'[]'),coalesce(s.sources,''),s.last_fetched_at
         FROM books b
         LEFT JOIN book_metadata_sources m ON m.book_id=b.book_id AND m.source=(
           SELECT source FROM book_metadata_sources x WHERE x.book_id=b.book_id
           ORDER BY CASE source WHEN 'manual' THEN 0 WHEN 'douban' THEN 1 WHEN 'open_library' THEN 2 WHEN 'google_books' THEN 3 ELSE 9 END LIMIT 1)
         LEFT JOIN (SELECT book_id,group_concat(source) sources,max(fetched_at) last_fetched_at FROM book_metadata_sources GROUP BY book_id) s ON s.book_id=b.book_id
         WHERE b.is_deleted=0 ORDER BY b.read_update_time DESC,b.title"
    )?;
    let rows = q.query_map([], |r| {
        let subjects_raw: String = r.get(8)?;
        let sources_raw: String = r.get(9)?;
        let fetched: Option<i64> = r.get(10)?;
        Ok(BookMetadataRow {
            book_id:r.get(0)?, title:r.get(1)?, author:r.get(2)?, cover:r.get(3)?, isbn:r.get(4)?,
            publisher:r.get(5)?, published_date:r.get(6)?, page_count:r.get(7)?,
            subjects:serde_json::from_str(&subjects_raw).unwrap_or_default(),
            sources:sources_raw.split(',').filter(|v| !v.is_empty()).map(str::to_owned).collect(),
            last_fetched_at:fetched.map(|v| v.to_string()),
            metadata_status:if fetched.is_some() { "ready".into() } else { "missing".into() },
        })
    })?.collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

pub async fn fetch(db: &Database, book_id: &str, source: &str, force: bool) -> Result<MetadataFetchResult, AppError> {
    if !matches!(source, "open_library" | "google_books") {
        return Err(AppError::Message("不支持的数据源".into()));
    }
    let c = db.connect()?;
    let (title, author): (String,String) = c.query_row("SELECT title,coalesce(author,'') FROM books WHERE book_id=?1 AND is_deleted=0", [book_id], |r| Ok((r.get(0)?,r.get(1)?)))?;
    if !force {
        let fresh: Option<i64> = c.query_row("SELECT fetched_at FROM book_metadata_sources WHERE book_id=?1 AND source=?2", params![book_id,source], |r| r.get(0)).optional()?;
        if fresh.is_some_and(|v| now()-v < 7*24*60*60) {
            return Ok(MetadataFetchResult { book_id:book_id.into(), source:source.into(), status:"cached".into(), message:"使用 7 天内的本地缓存".into() });
        }
    }
    drop(c);
    let client=reqwest::Client::builder().timeout(Duration::from_secs(12)).user_agent("ReadFlow/0.1 personal metadata client").build()?;
    let data = if source == "open_library" { fetch_open_library(&client,&title,&author).await? } else { fetch_google_books(&client,&title,&author).await? };
    let Some(data)=data else { return Ok(MetadataFetchResult { book_id:book_id.into(),source:source.into(),status:"not_found".into(),message:"未找到可靠匹配".into() }) };
    save(db,book_id,source,&data)?;
    Ok(MetadataFetchResult { book_id:book_id.into(),source:source.into(),status:"updated".into(),message:"已保存到本地".into() })
}

async fn fetch_open_library(client:&reqwest::Client,title:&str,author:&str)->Result<Option<Value>,AppError>{
    let value:Value=client.get("https://openlibrary.org/search.json").query(&[("title",title),("author",author),("limit","1")]).send().await?.error_for_status()?.json().await?;
    let Some(d)=value.get("docs").and_then(Value::as_array).and_then(|v|v.first()) else{return Ok(None)};
    let key=d.get("key").and_then(Value::as_str).unwrap_or("");
    let cover=d.get("cover_i").and_then(Value::as_i64).map(|id|format!("https://covers.openlibrary.org/b/id/{id}-L.jpg"));
    Ok(Some(json!({"source_id":key,"source_url":format!("https://openlibrary.org{key}"),"title":d.get("title"),"authors":d.get("author_name"),"isbn":d.get("isbn"),"publisher":d.get("publisher").and_then(Value::as_array).and_then(|v|v.first()),"published_date":d.get("first_publish_year"),"page_count":d.get("number_of_pages_median"),"subjects":d.get("subject"),"cover_url":cover,"raw":d})))
}

async fn fetch_google_books(client:&reqwest::Client,title:&str,author:&str)->Result<Option<Value>,AppError>{
    let query=format!("intitle:{title} inauthor:{author}");
    let value:Value=client.get("https://www.googleapis.com/books/v1/volumes").query(&[("q",query.as_str()),("maxResults","1")]).send().await?.error_for_status()?.json().await?;
    let Some(item)=value.get("items").and_then(Value::as_array).and_then(|v|v.first()) else{return Ok(None)};
    let info=&item["volumeInfo"];
    Ok(Some(json!({"source_id":item.get("id"),"source_url":info.get("infoLink"),"title":info.get("title"),"authors":info.get("authors"),"isbn":info.get("industryIdentifiers").and_then(Value::as_array).map(|ids|ids.iter().filter_map(|x|x.get("identifier")).cloned().collect::<Vec<_>>()),"publisher":info.get("publisher"),"published_date":info.get("publishedDate"),"page_count":info.get("pageCount"),"subjects":info.get("categories"),"cover_url":info.get("imageLinks").and_then(|x|x.get("thumbnail")),"description":info.get("description"),"rating":info.get("averageRating"),"rating_count":info.get("ratingsCount"),"raw":item})))
}

fn save(db:&Database,book_id:&str,source:&str,d:&Value)->Result<(),AppError>{
    let strings=|key:&str| d.get(key).and_then(Value::as_array).map(|v|v.iter().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>()).unwrap_or_default();
    let isbns=strings("isbn");
    let isbn10=isbns.iter().find(|v|v.len()==10).cloned(); let isbn13=isbns.iter().find(|v|v.len()==13).cloned();
    let text=|key:&str| d.get(key).and_then(|v| if v.is_string(){v.as_str().map(str::to_owned)}else if v.is_number(){Some(v.to_string())}else{None});
    db.connect()?.execute("INSERT INTO book_metadata_sources(book_id,source,source_id,source_url,isbn10,isbn13,title,authors_json,publisher,published_date,page_count,subjects_json,cover_url,description,rating,rating_count,raw_json,fetched_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18) ON CONFLICT(book_id,source) DO UPDATE SET source_id=excluded.source_id,source_url=excluded.source_url,isbn10=excluded.isbn10,isbn13=excluded.isbn13,title=excluded.title,authors_json=excluded.authors_json,publisher=excluded.publisher,published_date=excluded.published_date,page_count=excluded.page_count,subjects_json=excluded.subjects_json,cover_url=excluded.cover_url,description=excluded.description,rating=excluded.rating,rating_count=excluded.rating_count,raw_json=excluded.raw_json,fetched_at=excluded.fetched_at",
        params![book_id,source,text("source_id").unwrap_or_default(),text("source_url"),isbn10,isbn13,text("title"),serde_json::to_string(&strings("authors"))?,text("publisher"),text("published_date"),d.get("page_count").and_then(Value::as_i64),serde_json::to_string(&strings("subjects"))?,text("cover_url"),text("description"),d.get("rating").and_then(Value::as_f64),d.get("rating_count").and_then(Value::as_i64),serde_json::to_string(&d["raw"])?,now()])?;
    Ok(())
}
