use crate::{database::Database, error::AppError, models::{BookMetadataRow, BookMetadataSourceDetail, MetadataFetchResult}};
use crate::http::{limited_json, limited_text, MAX_API_RESPONSE_BYTES, MAX_HTML_RESPONSE_BYTES};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Value};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as i64
}

pub async fn fetch_douban_url(db: &Database, book_id: &str, input_url: &str) -> Result<MetadataFetchResult, AppError> {
    let input=input_url.trim();
    let url = if input.starts_with("https://book.douban.com/subject/") {
        reqwest::Url::parse(input).map_err(|_| AppError::Message("请输入有效的豆瓣图书链接".into()))?
    } else {
        find_douban_book(db,book_id,input).await?
    };
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
    let html = limited_text(response, MAX_HTML_RESPONSE_BYTES, "豆瓣图书页").await?;
    if html.contains("sec.douban.com") || html.contains("captcha") || html.contains("安全验证") {
        return Err(AppError::Message("豆瓣要求安全验证，本次拉取已停止".into()));
    }
    let data = parse_douban_html(&html, &source_id, url.as_str())?;
    save(db, book_id, "douban", &data)?;
    Ok(MetadataFetchResult { book_id:book_id.into(), source:"douban".into(), status:"updated".into(), message:"豆瓣公开条目信息已保存到本地".into() })
}

async fn find_douban_book(db:&Database,book_id:&str,query:&str)->Result<reqwest::Url,AppError>{
    let (title,author):(String,String)=db.connect()?.query_row("SELECT title,coalesce(author,'') FROM books WHERE book_id=?1 AND is_deleted=0",[book_id],|r|Ok((r.get(0)?,r.get(1)?)))?;
    let search=if query.is_empty(){core_title(&title)}else{core_title(query)};
    let client=reqwest::Client::builder().timeout(Duration::from_secs(12)).user_agent("Mozilla/5.0 (compatible; ReadFlow/0.1; personal metadata lookup)").build()?;
    let response=client.get("https://search.douban.com/book/subject_search").query(&[("search_text",search.as_str()),("cat","1001")]).send().await?.error_for_status()?;
    let html=limited_text(response,MAX_HTML_RESPONSE_BYTES,"豆瓣搜索页").await?;
    let marker="window.__DATA__ = ";
    let start=html.find(marker).ok_or_else(||AppError::Message("豆瓣搜索页未返回可识别结果".into()))?+marker.len();
    let end=html[start..].find(";</script>").or_else(||html[start..].find(";\n")).ok_or_else(||AppError::Message("豆瓣搜索结果格式已变化".into()))?+start;
    let value:Value=serde_json::from_str(html[start..end].trim())?;
    let items=value.get("items").and_then(Value::as_array).ok_or_else(||AppError::Message("豆瓣没有找到这本书".into()))?;
    let target=normalized(&core_title(&title)); let author_key=normalized(&author);
    let best=items.iter().max_by_key(|item|{
        let candidate=normalized(&core_title(item.get("title").and_then(Value::as_str).unwrap_or("")));
        let mut score=if candidate==target{100}else if candidate.contains(&target)||target.contains(&candidate){60}else{0};
        let abstract_text=normalized(item.get("abstract").and_then(Value::as_str).unwrap_or(""));
        if !author_key.is_empty()&&abstract_text.contains(&author_key){score+=30;} score
    }).filter(|item|{
        let candidate=normalized(&core_title(item.get("title").and_then(Value::as_str).unwrap_or("")));
        candidate==target||candidate.contains(&target)||target.contains(&candidate)
    }).ok_or_else(||AppError::Message("豆瓣没有找到可靠匹配".into()))?;
    let url=best.get("url").and_then(Value::as_str).ok_or_else(||AppError::Message("豆瓣搜索结果缺少条目链接".into()))?;
    reqwest::Url::parse(url).map_err(|_|AppError::Message("豆瓣返回了无效条目链接".into()))
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
    let info=html_section(html,"<div id=\"info\">","</div>").unwrap_or_default();
    let publisher=info_field(info,"出版社:");
    let published_date=info_field(info,"出版年:");
    let page_count=info_field(info,"页数:").and_then(|v|v.chars().filter(|c|c.is_ascii_digit()).collect::<String>().parse::<i64>().ok());
    let page_isbn=info_field(info,"ISBN:");
    let isbn=if isbn.is_empty(){page_isbn.into_iter().collect()}else{isbn};
    let rating=property_text(html,"v:average").and_then(|v|v.parse::<f64>().ok());
    let rating_count=property_text(html,"v:votes").and_then(|v|v.parse::<i64>().ok());
    let subjects=douban_tags(html);
    let author_block=html_section(html,"<div id=\"authors\"","</ul>").unwrap_or_default();
    let author_url=attribute_after(author_block,"<a href=\"","\"");
    let author_avatar=attribute_after(author_block,"<img src=\"","\"");
    let author_name=attribute_after(author_block,"title=\"","\"").or_else(||authors.first().cloned());
    let author_bio=section_after_heading(html,"作者简介").map(strip_html_with_breaks);
    let toc_marker=format!("id=\"dir_{source_id}_full\"");
    let table_of_contents=html.find(&toc_marker).and_then(|start|{let rest=&html[start..];let body=rest.find('>')?+1;let end=rest[body..].find("</div>")?+body;Some(strip_html_with_breaks(&rest[body..end]))});
    Ok(json!({"source_id":source_id,"source_url":source_url,"title":title,"authors":authors,"isbn":isbn,"publisher":publisher,"published_date":published_date,"page_count":page_count,"cover_url":image,"description":description,"rating":rating,"rating_count":rating_count,"subjects":subjects,"author_name":author_name,"author_avatar":author_avatar,"author_url":author_url,"author_bio":author_bio,"table_of_contents":table_of_contents,"raw":json_ld.unwrap_or_else(||json!({"title":title}))}))
}

fn html_section<'a>(html:&'a str,start_marker:&str,end_marker:&str)->Option<&'a str>{let start=html.find(start_marker)?+start_marker.len();let end=html[start..].find(end_marker)?+start;Some(&html[start..end])}
fn strip_html(value:&str)->String{let mut result=String::new();let mut inside=false;for ch in value.chars(){match ch{'<'=>inside=true,'>'=>inside=false,_ if !inside=>result.push(ch),_=>{}}}result.replace("&nbsp;"," ").replace("&amp;","&").split_whitespace().collect::<Vec<_>>().join(" ")}
fn info_field(info:&str,label:&str)->Option<String>{let marker=format!(">{label}</span>");let start=info.find(&marker)?+marker.len();let end=info[start..].find("<br").unwrap_or(info.len()-start)+start;let value=strip_html(&info[start..end]);(!value.is_empty()).then_some(value)}
fn property_text(html:&str,property:&str)->Option<String>{let marker=format!("property=\"{property}\"");let start=html.find(&marker)?+marker.len();let content=&html[start..];let gt=content.find('>')?+1;let end=content[gt..].find('<')?+gt;Some(strip_html(&content[gt..end]))}
fn douban_tags(html:&str)->Vec<String>{let mut tags=Vec::new();let mut rest=html;while let Some(pos)=rest.find("https://book.douban.com/tag/"){rest=&rest[pos..];let Some(gt)=rest.find('>')else{break};let body=&rest[gt+1..];let Some(end)=body.find("</a>")else{break};let tag=strip_html(&body[..end]);if !tag.is_empty()&&!tags.contains(&tag){tags.push(tag)}rest=&body[end+4..];if tags.len()>=12{break}}tags}
fn attribute_after(html:&str,marker:&str,end_marker:&str)->Option<String>{let start=html.find(marker)?+marker.len();let end=html[start..].find(end_marker)?+start;Some(html[start..end].to_owned())}
fn section_after_heading<'a>(html:&'a str,heading:&str)->Option<&'a str>{let start=html.find(&format!("<span>{heading}</span>"))?;let rest=&html[start..];let intro=rest.find("<div class=\"intro\">")?+"<div class=\"intro\">".len();let end=rest[intro..].find("</div>")?+intro;Some(&rest[intro..end])}
fn strip_html_with_breaks(value:&str)->String{strip_html(&value.replace("<br/>","\n").replace("<br>","\n").replace("</p>","\n"))}

pub fn list(db: &Database, source: &str) -> Result<Vec<BookMetadataRow>, AppError> {
    if !matches!(source, "weread" | "douban" | "open_library" | "google_books" | "manual") {
        return Err(AppError::Message("不支持的数据源".into()));
    }
    let c = db.connect()?;
    let mut q = c.prepare(
        "SELECT b.book_id,b.title,coalesce(b.author,''),coalesce(m.cover_url,b.cover,''),
         coalesce(m.isbn13,m.isbn10,''),coalesce(m.publisher,''),coalesce(m.published_date,''),m.page_count,
         coalesce(m.subjects_json,'[]'),coalesce(s.sources,''),s.last_fetched_at
         FROM books b
         LEFT JOIN book_metadata_sources m ON m.book_id=b.book_id AND m.source=?1
         LEFT JOIN (SELECT book_id,group_concat(source) sources,max(fetched_at) last_fetched_at FROM book_metadata_sources GROUP BY book_id) s ON s.book_id=b.book_id
         WHERE b.is_deleted=0 ORDER BY b.read_update_time DESC,b.title"
    )?;
    let rows = q.query_map([source], |r| {
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

pub fn details(db:&Database,book_id:&str)->Result<Vec<BookMetadataSourceDetail>,AppError>{
    let c=db.connect()?;
    let mut q=c.prepare("SELECT s.source,s.source_id,s.source_url,coalesce(s.title,''),coalesce(s.authors_json,'[]'),coalesce(s.isbn13,s.isbn10,''),coalesce(s.publisher,''),coalesce(s.published_date,''),s.page_count,coalesce(s.subjects_json,'[]'),coalesce(s.cover_url,''),coalesce(s.description,''),s.rating,s.rating_count,datetime(s.fetched_at,'unixepoch','localtime'),coalesce(e.author_name,''),coalesce(e.author_avatar,''),coalesce(e.author_url,''),coalesce(e.author_bio,''),coalesce(e.table_of_contents,'') FROM book_metadata_sources s LEFT JOIN book_metadata_extras e ON e.book_id=s.book_id AND e.source=s.source WHERE s.book_id=?1 ORDER BY CASE s.source WHEN 'weread' THEN 0 WHEN 'douban' THEN 1 WHEN 'manual' THEN 2 WHEN 'open_library' THEN 3 WHEN 'google_books' THEN 4 ELSE 9 END")?;
    let rows=q.query_map([book_id],|r|Ok(BookMetadataSourceDetail{source:r.get(0)?,source_id:r.get(1)?,source_url:r.get(2)?,title:r.get(3)?,authors:serde_json::from_str(&r.get::<_,String>(4)?).unwrap_or_default(),isbn:r.get(5)?,publisher:r.get(6)?,published_date:r.get(7)?,page_count:r.get(8)?,subjects:serde_json::from_str(&r.get::<_,String>(9)?).unwrap_or_default(),cover_url:r.get(10)?,description:r.get(11)?,rating:r.get(12)?,rating_count:r.get(13)?,fetched_at:r.get(14)?,author_name:r.get(15)?,author_avatar:r.get(16)?,author_url:r.get(17)?,author_bio:r.get(18)?,table_of_contents:r.get(19)?}))?.collect::<Result<Vec<_>,_>>()?;
    Ok(rows)
}

pub async fn fetch(db: &Database, book_id: &str, source: &str, force: bool) -> Result<MetadataFetchResult, AppError> {
    if !matches!(source, "smart" | "weread" | "open_library" | "google_books") {
        return Err(AppError::Message("不支持的数据源".into()));
    }
    let c = db.connect()?;
    let (title, author): (String,String) = c.query_row("SELECT title,coalesce(author,'') FROM books WHERE book_id=?1 AND is_deleted=0", [book_id], |r| Ok((r.get(0)?,r.get(1)?)))?;
    if !force && source != "smart" {
        let fresh: Option<i64> = c.query_row("SELECT fetched_at FROM book_metadata_sources WHERE book_id=?1 AND source=?2", params![book_id,source], |r| r.get(0)).optional()?;
        if fresh.is_some_and(|v| now()-v < 7*24*60*60) {
            return Ok(MetadataFetchResult { book_id:book_id.into(), source:source.into(), status:"cached".into(), message:"使用 7 天内的本地缓存".into() });
        }
    }
    drop(c);
    if source == "weread" {
        let secret = keyring::Entry::new("ReadFlow", "weread")?
            .get_password()
            .map_err(|_| AppError::Message("请先在设置中配置微信读书 Agent Gateway API Key".into()))?;
        let client = crate::weread::client::WeReadClient::new(secret)?;
        let mut params = serde_json::Map::new();
        params.insert("bookId".into(), Value::String(book_id.into()));
        let value: Value = client.call("/book/info", params).await?;
        let authors = value.get("author").and_then(Value::as_str).filter(|v| !v.is_empty()).map(|v| vec![v]).unwrap_or_default();
        let isbns = value.get("isbn").and_then(Value::as_str).filter(|v| !v.is_empty()).map(|v| vec![v]).unwrap_or_default();
        let subjects = value.get("category").and_then(Value::as_str).filter(|v| !v.is_empty()).map(|v| vec![v]).unwrap_or_default();
        let data = json!({
            "source_id": value.get("bookId").and_then(Value::as_str).unwrap_or(book_id),
            "source_url": value.get("deepLink"),
            "title": value.get("title"), "authors": authors, "isbn": isbns,
            "publisher": value.get("publisher"), "published_date": value.get("publishTime"),
            "subjects": subjects, "cover_url": value.get("cover"), "description": value.get("intro"),
            "rating": value.get("newRating"), "rating_count": value.get("newRatingCount"), "raw": value
        });
        save(db, book_id, "weread", &data)?;
        return Ok(MetadataFetchResult { book_id: book_id.into(), source: "weread".into(), status: "updated".into(), message: "微信读书信息已独立保存到本地".into() });
    }
    let client=reqwest::Client::builder().timeout(Duration::from_secs(12)).user_agent("ReadFlow/0.1 personal metadata client").build()?;
    let google_key=keyring::Entry::new("ReadFlow","google_books").ok().and_then(|entry|entry.get_password().ok()).filter(|value|!value.trim().is_empty());
    let (actual_source,data) = match source {
        "open_library" => match fetch_open_library(&client,&title,&author).await? {
            Some(value) => ("open_library",Some(value)),
            None => ("google_books",fetch_google_books(&client,&title,&author,google_key.as_deref()).await?),
        },
        "google_books" => ("google_books",fetch_google_books(&client,&title,&author,google_key.as_deref()).await?),
        _ => match fetch_open_library(&client,&title,&author).await? {
            Some(value) => ("open_library",Some(value)),
            None => ("google_books",fetch_google_books(&client,&title,&author,google_key.as_deref()).await?),
        },
    };
    let Some(data)=data else { return Ok(MetadataFetchResult { book_id:book_id.into(),source:source.into(),status:"not_found".into(),message:"未找到可靠匹配".into() }) };
    save(db,book_id,actual_source,&data)?;
    Ok(MetadataFetchResult { book_id:book_id.into(),source:actual_source.into(),status:"updated".into(),message:format!("已从 {actual_source} 保存到本地") })
}

async fn fetch_open_library(client:&reqwest::Client,title:&str,author:&str)->Result<Option<Value>,AppError>{
    let core=core_title(title);
    let mut searches=vec![("title",title.to_owned(),Some(author.to_owned()))];
    if core!=title { searches.push(("title",core.clone(),Some(author.to_owned()))); }
    searches.push(("q",core,None));
    let mut found=None;
    for (key,term,by_author) in searches {
        let mut request=client.get("https://openlibrary.org/search.json").query(&[(key,term.as_str()),("limit","5")]);
        if let Some(ref author)=by_author { if !author.trim().is_empty() { request=request.query(&[("author",author)]); } }
        let response=request.send().await?.error_for_status()?;
        let value:Value=limited_json(response,MAX_API_RESPONSE_BYTES,"Open Library").await?;
        if let Some(d)=best_match(value.get("docs").and_then(Value::as_array),title,author,"title","author_name") { found=Some(d.clone()); break; }
    }
    let Some(d)=found else{return Ok(None)};
    let key=d.get("key").and_then(Value::as_str).unwrap_or("");
    let cover=d.get("cover_i").and_then(Value::as_i64).map(|id|format!("https://covers.openlibrary.org/b/id/{id}-L.jpg"));
    Ok(Some(json!({"source_id":key,"source_url":format!("https://openlibrary.org{key}"),"title":d.get("title"),"authors":d.get("author_name"),"isbn":d.get("isbn"),"publisher":d.get("publisher").and_then(Value::as_array).and_then(|v|v.first()),"published_date":d.get("first_publish_year"),"page_count":d.get("number_of_pages_median"),"subjects":d.get("subject"),"cover_url":cover,"raw":d})))
}

async fn fetch_google_books(client:&reqwest::Client,title:&str,author:&str,api_key:Option<&str>)->Result<Option<Value>,AppError>{
    let core=core_title(title);
    let queries=[format!("intitle:{title} inauthor:{author}"),format!("intitle:{core}"),core];
    let mut found=None;
    for query in queries {
        let mut request=client.get("https://books.googleapis.com/books/v1/volumes").query(&[("q",query.as_str()),("maxResults","5"),("printType","books")]);
        if let Some(key)=api_key { request=request.query(&[("key",key)]); }
        let response=request.send().await?;
        if response.status()==reqwest::StatusCode::TOO_MANY_REQUESTS { return Err(AppError::Message("Google Books 请求已限流，请在设置中配置 API Key 或稍后重试".into())); }
        let value:Value=limited_json(response.error_for_status()?,MAX_API_RESPONSE_BYTES,"Google Books").await?;
        if let Some(items)=value.get("items").and_then(Value::as_array) {
            let best=items.iter().max_by_key(|item| match_score(item.get("volumeInfo").unwrap_or(&Value::Null),title,author,"title","authors"));
            if let Some(item)=best.filter(|item|match_score(item.get("volumeInfo").unwrap_or(&Value::Null),title,author,"title","authors")>0) { found=Some(item.clone()); break; }
        }
    }
    let Some(item)=found else{return Ok(None)};
    let info=&item["volumeInfo"];
    Ok(Some(json!({"source_id":item.get("id"),"source_url":info.get("infoLink"),"title":info.get("title"),"authors":info.get("authors"),"isbn":info.get("industryIdentifiers").and_then(Value::as_array).map(|ids|ids.iter().filter_map(|x|x.get("identifier")).cloned().collect::<Vec<_>>()),"publisher":info.get("publisher"),"published_date":info.get("publishedDate"),"page_count":info.get("pageCount"),"subjects":info.get("categories"),"cover_url":info.get("imageLinks").and_then(|x|x.get("thumbnail")),"description":info.get("description"),"rating":info.get("averageRating"),"rating_count":info.get("ratingsCount"),"raw":item})))
}

fn core_title(title:&str)->String {
    title.split(['：',':','（','(','【','[']).next().unwrap_or(title).trim().to_owned()
}
fn normalized(value:&str)->String { value.chars().filter(|c|c.is_alphanumeric()).flat_map(char::to_lowercase).collect() }
fn match_score(item:&Value,title:&str,author:&str,title_key:&str,authors_key:&str)->i32 {
    let candidate=item.get(title_key).and_then(Value::as_str).unwrap_or("");
    let target=normalized(&core_title(title)); let candidate=normalized(&core_title(candidate));
    let mut score=if candidate==target { 100 } else if !target.is_empty()&&(candidate.contains(&target)||target.contains(&candidate)) { 60 } else { 0 };
    let authors=item.get(authors_key).and_then(Value::as_array).map(|v|v.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")).unwrap_or_default();
    let author=normalized(author); let authors=normalized(&authors); if !author.is_empty()&&(authors.contains(&author)||author.contains(&authors)) { score+=20; }
    score
}
fn best_match<'a>(items:Option<&'a Vec<Value>>,title:&str,author:&str,title_key:&str,authors_key:&str)->Option<&'a Value>{
    items?.iter().max_by_key(|item|match_score(item,title,author,title_key,authors_key)).filter(|item|match_score(item,title,author,title_key,authors_key)>0)
}

fn save(db:&Database,book_id:&str,source:&str,d:&Value)->Result<(),AppError>{
    let strings=|key:&str| d.get(key).and_then(Value::as_array).map(|v|v.iter().filter_map(Value::as_str).map(str::to_owned).collect::<Vec<_>>()).unwrap_or_default();
    // ISBN 常见带连字符/空格（Open Library 返回 "978-0-14-103614-4"），先归一化再判长度。
    let normalize_isbn=|v:&str| v.chars().filter(|c|c.is_alphanumeric()).collect::<String>();
    let isbns=strings("isbn").iter().map(|v|normalize_isbn(v)).collect::<Vec<_>>();
    let isbn10=isbns.iter().find(|v|v.len()==10).cloned(); let isbn13=isbns.iter().find(|v|v.len()==13).cloned();
    let text=|key:&str| d.get(key).and_then(|v| if v.is_string(){v.as_str().map(str::to_owned)}else if v.is_number(){Some(v.to_string())}else{None});
    let c=db.connect()?;
    // 可空列一律用 coalesce(excluded.x, 旧值)：不同来源的字段集不一样，
    // 无条件覆盖会把同步写入的字段（例如 weread 给的 page_count）抹成 NULL。
    c.execute("INSERT INTO book_metadata_sources(book_id,source,source_id,source_url,isbn10,isbn13,title,authors_json,publisher,published_date,page_count,subjects_json,cover_url,description,rating,rating_count,raw_json,fetched_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18) ON CONFLICT(book_id,source) DO UPDATE SET source_id=coalesce(excluded.source_id,source_id),source_url=coalesce(excluded.source_url,source_url),isbn10=coalesce(excluded.isbn10,isbn10),isbn13=coalesce(excluded.isbn13,isbn13),title=coalesce(excluded.title,title),authors_json=CASE WHEN excluded.authors_json='[]' THEN book_metadata_sources.authors_json ELSE excluded.authors_json END,publisher=coalesce(excluded.publisher,publisher),published_date=coalesce(excluded.published_date,published_date),page_count=coalesce(excluded.page_count,page_count),subjects_json=CASE WHEN excluded.subjects_json='[]' THEN book_metadata_sources.subjects_json ELSE excluded.subjects_json END,cover_url=coalesce(excluded.cover_url,cover_url),description=coalesce(excluded.description,description),rating=coalesce(excluded.rating,rating),rating_count=coalesce(excluded.rating_count,rating_count),raw_json=excluded.raw_json,fetched_at=excluded.fetched_at",
        params![book_id,source,text("source_id").unwrap_or_default(),text("source_url"),isbn10,isbn13,text("title"),serde_json::to_string(&strings("authors"))?,text("publisher"),text("published_date"),d.get("page_count").and_then(Value::as_i64),serde_json::to_string(&strings("subjects"))?,text("cover_url"),text("description"),d.get("rating").and_then(Value::as_f64),d.get("rating_count").and_then(Value::as_i64),serde_json::to_string(&d["raw"])?,now()])?;
    c.execute("INSERT INTO book_metadata_extras(book_id,source,author_name,author_avatar,author_url,author_bio,table_of_contents) VALUES(?1,?2,?3,?4,?5,?6,?7) ON CONFLICT(book_id,source) DO UPDATE SET author_name=excluded.author_name,author_avatar=excluded.author_avatar,author_url=excluded.author_url,author_bio=excluded.author_bio,table_of_contents=excluded.table_of_contents",params![book_id,source,text("author_name"),text("author_avatar"),text("author_url"),text("author_bio"),text("table_of_contents")])?;
    Ok(())
}
