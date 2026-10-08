//! dump 的流式解析：bz2 边解压边读，XML 逐事件消费。
//!
//! 安全约束（需求 11.4 / 11.5 / 7.3）：
//! - 不启用任何实体/DTD 展开，外部实体一律丢弃，XXE 无从触发；
//! - 只处理主命名空间 ns=0；
//! - 单页文本有大小上限，解压产物有总量上限与压缩比上限，防压缩炸弹；
//! - 每处理完一页立即释放节点与正文缓冲，内存不随已处理页数增长；
//! - 解析器只吐 WikiPage，清洗、过滤、写库都在上层做。

use crate::error::AppError;
use quick_xml::events::Event;
use quick_xml::reader::Reader;
use std::fs::File;
use std::io::{BufRead, BufReader, Read};

/// 单页正文上限。超过就当作坏数据记问题并跳过这一页。
pub const MAX_PAGE_TEXT_BYTES: usize = 4 * 1024 * 1024;
/// 单次解压产物上限，默认 64 GiB，够放下今天的全量 dump。
pub const MAX_DECOMPRESSED_BYTES: u64 = 64 * 1024 * 1024 * 1024;
/// 压缩比上限。正常 dump 约 10 倍左右，超过 200 倍视为压缩炸弹。
pub const MAX_COMPRESSION_RATIO: u64 = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiPage {
    pub page_id: i64,
    pub namespace: i64,
    pub title: String,
    pub redirect_target: Option<String>,
    pub revision_id: i64,
    pub revision_timestamp: String,
    pub text: String,
}

impl WikiPage {
    pub fn is_redirect(&self) -> bool {
        self.redirect_target.is_some()
    }
}

#[derive(Debug, Clone)]
pub struct ParseIssue {
    pub page_id: Option<i64>,
    pub title: String,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
pub struct ParseStats {
    pub pages_seen: u64,
    pub pages_emitted: u64,
    pub pages_skipped_namespace: u64,
    pub pages_skipped_malformed: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// 继续处理下一页。
    Continue,
    /// 主动停止（例如达到 --limit 或收到取消信号）。
    Stop,
}

/// 从任意已解压的字节流解析。测试直接喂内存字符串，生产喂 bz2 管道。
///
/// 坏数据通过 `on_issue` 逐条上报，不在返回值里累积：全量 dump 的问题条数
/// 没有上限，攒起来会变成内存泄漏。
pub fn parse_dump<R: BufRead, F, G>(
    reader: R,
    sink: &mut F,
    on_issue: &mut G,
) -> Result<ParseStats, AppError>
where
    F: FnMut(WikiPage) -> Result<Flow, AppError>,
    G: FnMut(ParseIssue),
{
    let mut reader = Reader::from_reader(reader);
    {
        let config = reader.config_mut();
        // 实体由我们自己处理（只认 5 个预定义实体和数字实体），
        // 不依赖库的实体解析，从根上杜绝外部实体。
        config.allow_dangling_amp = true;
        config.allow_unmatched_ends = true;
        config.check_comments = false;
        config.check_end_names = false;
        config.expand_empty_elements = true;
        config.trim_text_start = false;
        config.trim_text_end = false;
    }
    let mut stats = ParseStats::default();
    let mut buffer: Vec<u8> = Vec::with_capacity(64 * 1024);

    let mut page: Option<PageBuilder> = None;
    let mut in_revision = false;
    let mut field: Option<Field> = None;
    let mut in_text = false;
    let mut text_overflow = false;
    let mut stop = false;

    loop {
        let event = match reader.read_event_into(&mut buffer) {
            Ok(event) => event,
            Err(error) => {
                // 流级错误：无法定位到某一页，只能整体中止。
                return Err(AppError::Message(format!("dump 解析失败：{error}")));
            }
        };
        match event {
            Event::Eof => break,
            Event::DocType(_) => {
                // 正常 dump 不带 DTD；出现了也绝不展开。
                stats.pages_skipped_malformed += 1;
            }
            // expand_empty_elements 已开启，<redirect /> 会变成 Start+End，
            // 两种事件按同一种语义处理即可。
            Event::Start(element) | Event::Empty(element) => handle_start(
                &element,
                &mut page,
                &mut in_revision,
                &mut field,
                &mut in_text,
                &mut text_overflow,
            ),
            Event::End(element) => handle_end(&element, &mut page, &mut in_revision, &mut field, &mut in_text),
            Event::Text(value) => append_text(
                &mut value.into_inner().into_owned(),
                &mut page,
                &mut field,
                &mut in_text,
                &mut text_overflow,
            ),
            Event::CData(value) => append_text(
                &mut value.into_inner().into_owned(),
                &mut page,
                &mut field,
                &mut in_text,
                &mut text_overflow,
            ),
            Event::GeneralRef(reference) => {
                // 只解析预定义实体和数字实体；DTD 声明的自定义实体直接丢弃。
                let value = reference.into_inner().into_owned();
                let decoded = decode_predefined(&value).unwrap_or_default();
                append_text(&decoded, &mut page, &mut field, &mut in_text, &mut text_overflow);
            }
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) => {}
        }
        buffer.clear();

        let ready = page.as_ref().is_some_and(|builder| builder.finished);
        if ready {
            let builder = page.take().expect("刚判断过 finished");
            stats.pages_seen += 1;
            if builder.namespace != 0 {
                stats.pages_skipped_namespace += 1;
            } else if builder.invalid_reason.is_some() || text_overflow {
                stats.pages_skipped_malformed += 1;
                on_issue(ParseIssue {
                    page_id: Some(builder.page_id),
                    title: builder.title,
                    message: builder
                        .invalid_reason
                        .unwrap_or_else(|| "正文超过单页上限".to_string()),
                });
            } else {
                stats.pages_emitted += 1;
                let page = WikiPage {
                    page_id: builder.page_id,
                    namespace: builder.namespace,
                    title: builder.title,
                    redirect_target: builder.redirect_target,
                    revision_id: builder.revision_id,
                    revision_timestamp: builder.revision_timestamp,
                    text: builder.text,
                };
                match sink(page)? {
                    Flow::Continue => {}
                    Flow::Stop => {
                        stop = true;
                        break;
                    }
                }
            }
        }
    }
    let _ = stop;
    Ok(stats)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Field {
    Title,
    Namespace,
    PageId,
    RevisionId,
    Timestamp,
}

#[derive(Default)]
struct PageBuilder {
    finished: bool,
    page_id: i64,
    namespace: i64,
    title: String,
    redirect_target: Option<String>,
    revision_id: i64,
    revision_timestamp: String,
    text: String,
    invalid_reason: Option<String>,
}

fn handle_start(
    element: &quick_xml::events::BytesStart<'_>,
    page: &mut Option<PageBuilder>,
    in_revision: &mut bool,
    field: &mut Option<Field>,
    in_text: &mut bool,
    text_overflow: &mut bool,
) {
    let name = element.local_name().as_ref().to_ascii_lowercase();
    match name.as_str() {
        "page" => {
            if page.is_none() {
                *page = Some(PageBuilder::default());
                *text_overflow = false;
            }
        }
        "revision" => *in_revision = true,
        "title" if page.is_some() && !*in_revision => *field = Some(Field::Title),
        "ns" if page.is_some() => *field = Some(Field::Namespace),
        "id" if page.is_some() => {
            // page id 与 revision id 同名，靠 in_revision 区分，不能写反。
            *field = Some(if *in_revision { Field::RevisionId } else { Field::PageId });
        }
        "timestamp" if page.is_some() && *in_revision => *field = Some(Field::Timestamp),
        "redirect" if page.is_some() => {
            if let Some(builder) = page.as_mut() {
                let target = attribute(element, "title").unwrap_or_default();
                let target = target.trim().to_string();
                if !target.is_empty() {
                    builder.redirect_target = Some(target);
                }
            }
        }
        "text" if page.is_some() && *in_revision => {
            *in_text = true;
            *field = None;
        }
        _ => {}
    }
}

fn handle_end(
    element: &quick_xml::events::BytesEnd<'_>,
    page: &mut Option<PageBuilder>,
    in_revision: &mut bool,
    field: &mut Option<Field>,
    in_text: &mut bool,
) {
    let name = element.local_name().as_ref().to_ascii_lowercase();
    match name.as_str() {
        "page" => {
            if let Some(builder) = page.as_mut() {
                builder.finished = true;
            }
        }
        "revision" => *in_revision = false,
        "text" => *in_text = false,
        "title" | "ns" | "id" | "timestamp" | "redirect" => *field = None,
        _ => {}
    }
}

fn append_text(
    value: &str,
    page: &mut Option<PageBuilder>,
    field: &mut Option<Field>,
    in_text: &mut bool,
    text_overflow: &mut bool,
) {
    let Some(builder) = page.as_mut() else {
        return;
    };
    if *in_text {
        if builder.text.len() + value.len() > MAX_PAGE_TEXT_BYTES {
            // 不再累积，标记溢出后整页按坏数据处理。
            *text_overflow = true;
            builder.text.clear();
            builder.invalid_reason = Some("正文超过单页上限".to_string());
            return;
        }
        builder.text.push_str(value);
        return;
    }
    match field {
        Some(Field::Title) => builder.title.push_str(value),
        Some(Field::Namespace) => {
            if let Ok(parsed) = value.trim().parse() {
                builder.namespace = parsed;
            }
        }
        Some(Field::PageId) => {
            if let Ok(parsed) = value.trim().parse() {
                builder.page_id = parsed;
            }
        }
        Some(Field::RevisionId) => {
            if let Ok(parsed) = value.trim().parse() {
                builder.revision_id = parsed;
            }
        }
        Some(Field::Timestamp) => builder.revision_timestamp.push_str(value),
        None => {}
    }
}

fn attribute(element: &quick_xml::events::BytesStart<'_>, name: &str) -> Option<String> {
    // 0.42 的 attributes() 迭代项是 Result，单个属性解析失败不影响整页，跳过即可。
    element.attributes().flatten().find_map(|attribute| {
        let local = attribute.key.local_name();
        local
            .as_ref()
            .eq_ignore_ascii_case(name)
            .then(|| decode_attributes(&attribute.value))
    })
}

fn decode_attributes(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let chars: Vec<char> = value.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '&' {
            let rest = &chars[index..];
            let mut cursor = 1;
            while cursor < rest.len() && rest[cursor] != ';' && cursor < 12 {
                cursor += 1;
            }
            if rest.get(cursor) == Some(&';') {
                let name: String = rest[1..cursor].iter().collect();
                if let Some(value) = decode_predefined(&name) {
                    out.push_str(&value);
                    index += cursor + 1;
                    continue;
                }
            }
        }
        out.push(chars[index]);
        index += 1;
    }
    out
}

/// 只认 XML 预定义实体与字符引用。DTD 里声明的实体名不会命中这里，
/// 因此外部实体永远不会被展开（XXE 防护）。
fn decode_predefined(name: &str) -> Option<String> {
    if let Some(digits) = name.strip_prefix('#') {
        let code = match digits
            .strip_prefix('x')
            .or_else(|| digits.strip_prefix('X'))
        {
            Some(hex) => u32::from_str_radix(hex, 16).ok(),
            None => digits.parse::<u32>().ok(),
        }?;
        return char::from_u32(code)
            .filter(|value| *value != '\0')
            .map(|value| value.to_string());
    }
    match name {
        "amp" => Some("&".to_string()),
        "lt" => Some("<".to_string()),
        "gt" => Some(">".to_string()),
        "quot" => Some("\"".to_string()),
        "apos" => Some("'".to_string()),
        _ => None,
    }
}

/// dump 解压后的读取句柄类型：File -> 计数器 -> bz2 -> 护栏 -> BufReader。
pub type DumpReader = BufReader<GuardedDecoder<bzip2::read::MultiBzDecoder<CountingReader<File>>>>;

/// 打开一个 dump 文件：File -> bz2 解压 -> 总量/压缩比护栏 -> BufReader。
pub fn open_dump_reader(path: &std::path::Path) -> Result<DumpReader, AppError> {
    let file = File::open(path)
        .map_err(|error| AppError::Message(format!("打开 dump 文件失败：{error}")))?;
    let compressed = Counter::new();
    let decoder = bzip2::read::MultiBzDecoder::new(CountingReader::with_counter(file, compressed.clone()));
    Ok(BufReader::with_capacity(256 * 1024, GuardedDecoder::new(decoder, compressed)))
}

/// 共享计数器：让压缩侧与解压侧都知道彼此的字节数。
#[derive(Clone)]
pub struct Counter(std::sync::Arc<std::sync::atomic::AtomicU64>);

impl Counter {
    fn new() -> Self {
        Self(std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)))
    }
    fn add(&self, value: u64) {
        self.0.fetch_add(value, std::sync::atomic::Ordering::Relaxed);
    }
    fn get(&self) -> u64 {
        self.0.load(std::sync::atomic::Ordering::Relaxed)
    }
}

pub struct CountingReader<R> {
    inner: R,
    counter: Counter,
}

impl<R> CountingReader<R> {
    pub fn with_counter(inner: R, counter: Counter) -> Self {
        Self { inner, counter }
    }
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.counter.add(read as u64);
        Ok(read)
    }
}

/// 解压产物护栏：总量上限 + 压缩比上限。
pub struct GuardedDecoder<R: Read> {
    inner: R,
    compressed: Counter,
    produced: u64,
    last_check: u64,
}

impl<R: Read> GuardedDecoder<R> {
    fn new(inner: R, compressed: Counter) -> Self {
        Self { inner, compressed, produced: 0, last_check: 0 }
    }

    pub fn produced_bytes(&self) -> u64 {
        self.produced
    }
}

impl<R: Read> Read for GuardedDecoder<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.produced += read as u64;
        if self.produced > MAX_DECOMPRESSED_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "解压数据量超过上限，疑似压缩炸弹",
            ));
        }
        // 每 32 MiB 检查一次压缩比，避免每次读都算。
        if self.produced >= self.last_check + 32 * 1024 * 1024 {
            self.last_check = self.produced;
            let compressed = self.compressed.get();
            if compressed > 0 && self.produced / compressed.max(1) > MAX_COMPRESSION_RATIO {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "解压压缩比异常，疑似压缩炸弹",
                ));
            }
        }
        Ok(read)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const FIXTURE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<mediawiki xmlns="http://www.mediawiki.org/xml/export-0.11/" xmlns:xsi="http://www.w3.org/2001/XMLSchema-instance" xsi:schemaLocation="http://www.mediawiki.org/xml/export-0.11/ http://www.mediawiki.org/xml/export-0.11.xsd">
  <siteinfo>
    <sitename>维基百科</sitename>
    <base>https://zh.wikipedia.org/wiki/Main_Page</base>
  </siteinfo>
  <page>
    <title>人工智能</title>
    <ns>0</ns>
    <id>1001</id>
    <revision>
      <id>2001</id>
      <timestamp>2026-09-01T02:03:04Z</timestamp>
      <contributor><username>Someone</username></contributor>
      <comment>修订说明</comment>
      <model>wikitext</model>
      <format>text/x-wiki</format>
      <text xml:space="preserve" bytes="60">'''人工智能'''（英语：artificial intelligence，缩写：AI）是研究、开发用于模拟、延伸和扩展人的智能的理论、方法、应用及相关学科的总称。<ref name="a">脚注</ref></text>
    </revision>
  </page>
  <page>
    <title>机器学习</title>
    <ns>0</ns>
    <id>1002</id>
    <redirect title="人工智能" />
    <revision>
      <id>2002</id>
      <timestamp>2026-09-02T00:00:00Z</timestamp>
      <text xml:space="preserve" bytes="10">#REDIRECT [[人工智能]]</text>
    </revision>
  </page>
  <page>
    <title>Template:Infobox</title>
    <ns>10</ns>
    <id>1003</id>
    <revision>
      <id>2003</id>
      <timestamp>2026-09-02T00:00:00Z</timestamp>
      <text xml:space="preserve">模板内容</text>
    </revision>
  </page>
  <page>
    <title>不完整页面</title>
    <ns>0</ns>
    <id>1004</id>
    <revision>
      <id>2004</id>
      <timestamp>2026-09-03T00:00:00Z</timestamp>
    </revision>
  </page>
</mediawiki>
"#;

    fn collect(input: &str) -> (Vec<WikiPage>, ParseStats, Vec<ParseIssue>) {
        let mut pages = Vec::new();
        let mut issues = Vec::new();
        let stats = parse_dump(
            input.as_bytes(),
            &mut |page| {
                pages.push(page);
                Ok(Flow::Continue)
            },
            &mut |issue| issues.push(issue),
        )
        .expect("parse");
        (pages, stats, issues)
    }

    fn pages_of(input: &str) -> Vec<WikiPage> {
        collect(input).0
    }

    #[test]
    fn 解析出主命名空间页面() {
        let (pages, stats, _) = collect(FIXTURE);
        assert_eq!(stats.pages_seen, 4);
        assert_eq!(stats.pages_emitted, 3, "ns=10 的模板页必须跳过");
        assert_eq!(stats.pages_skipped_namespace, 1);
        assert_eq!(pages[0].title, "人工智能");
        assert_eq!(pages[0].page_id, 1001);
        assert_eq!(pages[0].revision_id, 2001);
        assert_eq!(pages[0].namespace, 0);
        assert_eq!(pages[0].revision_timestamp, "2026-09-01T02:03:04Z");
    }

    #[test]
    fn page_id_与_revision_id_不混淆() {
        let pages = pages_of(FIXTURE);
        let page = pages.iter().find(|page| page.page_id == 1001).expect("page 1001");
        assert_eq!(page.revision_id, 2001, "revision id 必须是 2001 而不是 1001");
        assert_ne!(page.page_id, page.revision_id);
    }

    #[test]
    fn 重定向目标可识别() {
        let pages = pages_of(FIXTURE);
        let redirect = pages.iter().find(|page| page.page_id == 1002).expect("redirect page");
        assert!(redirect.is_redirect());
        assert_eq!(redirect.redirect_target.as_deref(), Some("人工智能"));
    }

    #[test]
    fn 正文保留中文与粗体标记() {
        let pages = pages_of(FIXTURE);
        let page = pages.iter().find(|page| page.page_id == 1001).expect("page");
        assert!(page.text.contains("人工智能"));
        assert!(page.text.contains("'''人工智能'''"));
    }

    #[test]
    fn 缺少正文的页面仍然产出() {
        let pages = pages_of(FIXTURE);
        let page = pages.iter().find(|page| page.page_id == 1004).expect("page");
        assert!(page.text.is_empty());
        assert_eq!(page.revision_id, 2004);
    }

    #[test]
    fn 实体被解码() {
        let xml = r#"<mediawiki><page><title>A &amp; B</title><ns>0</ns><id>7</id><revision><id>8</id><text>1 &lt; 2 &amp;&amp; 3 &gt; 2 &#65;</text></revision></page></mediawiki>"#;
        let pages = pages_of(xml);
        assert_eq!(pages[0].title, "A & B");
        assert_eq!(pages[0].text, "1 < 2 && 3 > 2 A");
    }

    #[test]
    fn 外部实体不会被展开() {
        // 经典 XXE 载荷：即使文件里带 DOCTYPE，外部实体也只会变成空串。
        let xml = r#"<?xml version="1.0"?>
<!DOCTYPE mediawiki [<!ENTITY xxe SYSTEM "file:///etc/passwd">]>
<mediawiki><page><title>&xxe;</title><ns>0</ns><id>9</id><revision><id>10</id><text>&xxe;</text></revision></page></mediawiki>"#;
        let pages = pages_of(xml);
        assert!(!pages[0].text.contains("root:"), "不能读到本地文件内容");
        assert!(!pages[0].title.contains("root:"));
    }

    #[test]
    fn flow_stop_可以提前结束() {
        let mut seen = 0;
        let stats = parse_dump(
            FIXTURE.as_bytes(),
            &mut |_| {
                seen += 1;
                Ok(if seen >= 1 { Flow::Stop } else { Flow::Continue })
            },
            &mut |_| {},
        )
        .expect("parse");
        assert_eq!(seen, 1);
        assert_eq!(stats.pages_emitted, 1);
    }

    #[test]
    fn 流级格式错误返回错误() {
        // 引号没闭合会在读属性时直接报错，必须冒泡成流级错误。
        let error = parse_dump(
            r#"<mediawiki><page><title attr="未闭合></title></page></mediawiki>"#.as_bytes(),
            &mut |_| Ok(Flow::Continue),
            &mut |_| {},
        )
        .expect_err("should fail");
        assert!(error.to_string().contains("解析失败"));
    }

    #[test]
    fn 截断的_xml_在_eof_处安全结束() {
        // 半个页面没有 </page>，配置允许未配对结束标签，所以应当安静地少收一条而不是崩。
        let (pages, stats, _) = collect("<mediawiki><page><title>未闭合</title><ns>0</ns><id>99</id>");
        assert!(pages.is_empty());
        assert_eq!(stats.pages_emitted, 0);
    }

    #[test]
    fn 超长正文被标记为坏数据() {
        let huge = "字".repeat(MAX_PAGE_TEXT_BYTES + 10);
        let xml = format!(
            "<mediawiki><page><title>大页</title><ns>0</ns><id>11</id><revision><id>12</id><text>{huge}</text></revision></page></mediawiki>"
        );
        let (pages, stats, issues) = collect(&xml);
        assert!(pages.is_empty());
        assert_eq!(stats.pages_skipped_malformed, 1);
        assert_eq!(issues.len(), 1);
        assert!(issues[0].message.contains("上限"));
    }

    #[test]
    fn bz2_管道能解析真实压缩包() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("fixture.xml.bz2");
        let mut encoder = bzip2::write::BzEncoder::new(
            File::create(&path).expect("create bz2"),
            bzip2::Compression::best(),
        );
        encoder.write_all(FIXTURE.as_bytes()).expect("write");
        encoder.finish().expect("finish");

        let reader = open_dump_reader(&path).expect("open dump");
        let mut pages = Vec::new();
        let stats = parse_dump(
            reader,
            &mut |page| {
                pages.push(page);
                Ok(Flow::Continue)
            },
            &mut |_| {},
        )
        .expect("parse bz2");
        assert_eq!(pages.len(), 3);
        assert_eq!(stats.pages_seen, 4);
        assert_eq!(pages[0].title, "人工智能");
    }

    #[test]
    fn 连续解析不会累积状态() {
        for _ in 0..50 {
            let (pages, stats, _) = collect(FIXTURE);
            assert_eq!(pages.len(), 3);
            assert_eq!(stats.pages_seen, 4);
        }
    }
}
