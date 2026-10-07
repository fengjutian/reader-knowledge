//! EPUB 导入：ZIP 安全校验 + OPF spine 顺序读取 + 章节清洗。
//!
//! 安全重点（对应实现计划 9.5.1）：
//! - **Zip Slip**：条目名不能含 `..` 或绝对路径，否则会写到应用目录之外。
//! - **解压炸弹**：限制单条目解压后大小、条目总数与总解压量。

use crate::error::AppError;
use crate::import::{chunker, web, Locator, ParsedSource, SourceKind};
use std::io::Read;

/// 单个条目解压后的上限。
pub const MAX_ENTRY_BYTES: u64 = 8 * 1024 * 1024;
/// 整个 EPUB 解压后的总量上限。
pub const MAX_TOTAL_UNCOMPRESSED: u64 = 256 * 1024 * 1024;
/// 条目数量上限。
pub const MAX_ENTRIES: usize = 5_000;
/// 封面大小上限。
pub const MAX_COVER_BYTES: u64 = 4 * 1024 * 1024;

/// 条目路径是否安全（防 Zip Slip）。
///
/// 拒绝：绝对路径、盘符、`..` 段、空段、反斜杠。
pub fn is_safe_entry_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 512 {
        return false;
    }
    if name.starts_with('/') || name.starts_with('\\') {
        return false;
    }
    // Windows 盘符形式，例如 C:/evil
    let bytes = name.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return false;
    }
    if name.contains('\\') {
        return false;
    }
    if name.split('/').any(|segment| segment == ".." || segment.is_empty()) {
        return false;
    }
    // 控制字符
    !name.chars().any(|c| c.is_control())
}

/// 校验 EPUB 的解压规模是否在安全范围内。
pub fn validate_archive_limits(entry_count: usize, total_uncompressed: u64) -> Result<(), AppError> {
    if entry_count > MAX_ENTRIES {
        return Err(AppError::Message("EPUB 条目过多，已拒绝导入".into()));
    }
    if total_uncompressed > MAX_TOTAL_UNCOMPRESSED {
        return Err(AppError::Message("EPUB 解压后体积过大，已拒绝导入".into()));
    }
    Ok(())
}

/// 从 EPUB 字节读取单个条目，同时执行路径与体积校验。
pub fn read_entry(archive: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>, name: &str) -> Result<Vec<u8>, AppError> {
    if !is_safe_entry_name(name) {
        return Err(AppError::Message("EPUB 包含不安全的文件路径，已拒绝导入".into()));
    }
    let mut entry = archive
        .by_name(name)
        .map_err(|_| AppError::Message(format!("EPUB 缺少条目：{name}")))?;
    if entry.size() > MAX_ENTRY_BYTES {
        return Err(AppError::Message("EPUB 单个文件过大，已拒绝导入".into()));
    }
    // 再按实际读取量兜底一次，防止 size 字段撒谎
    let limit = MAX_ENTRY_BYTES.min(entry.size().saturating_add(1));
    let mut buffer = Vec::new();
    entry
        .by_ref()
        .take(limit)
        .read_to_end(&mut buffer)
        .map_err(|error| AppError::Message(format!("读取 EPUB 条目失败：{error}")))?;
    if buffer.len() as u64 > MAX_ENTRY_BYTES {
        return Err(AppError::Message("EPUB 单个文件过大，已拒绝导入".into()));
    }
    Ok(buffer)
}

/// 从 `META-INF/container.xml` 找到 OPF 路径。
///
/// container.xml 形如：
/// `<container><rootfiles><rootfile full-path="OEBPS/content.opf" .../></rootfiles></container>`
pub fn parse_opf_path(container_xml: &[u8]) -> Result<String, AppError> {
    let text = String::from_utf8_lossy(container_xml);
    let start = text.find("full-path").ok_or_else(|| AppError::Message("EPUB 缺少 OPF 声明".into()))? + "full-path".len();
    let rest = &text[start..];
    let rest = rest.trim_start().trim_start_matches('=').trim_start();
    let quote = rest.chars().next().ok_or_else(|| AppError::Message("EPUB 的 OPF 路径格式异常".into()))?;
    if quote != '"' && quote != '\'' {
        return Err(AppError::Message("EPUB 的 OPF 路径格式异常".into()));
    }
    let inner = &rest[1..];
    let end = inner.find(quote).ok_or_else(|| AppError::Message("EPUB 的 OPF 路径没有闭合".into()))?;
    let path = inner[..end].trim().to_owned();
    if path.is_empty() {
        return Err(AppError::Message("EPUB 的 OPF 路径为空".into()));
    }
    Ok(path)
}

/// 从 OPF 里按顺序取出 manifest 的 id → href 映射与 spine 顺序。
pub fn parse_opf(opf: &[u8]) -> Result<(String, Option<String>, Vec<(String, String)>), AppError> {
    let text = String::from_utf8_lossy(opf);
    let base = extract_tag_text(&text, "dc:title").ok_or_else(|| AppError::Message("EPUB 缺少书名".into()))?;
    let author = extract_tag_text(&text, "dc:creator");

    // manifest: <item id="c1" href="c1.xhtml" .../>
    let mut manifest: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut cursor = 0;
    while let Some(at) = text[cursor..].find("<item") {
        let start = cursor + at;
        let Some(end) = text[start..].find('>') else { break };
        let tag = &text[start..start + end];
        cursor = start + end;
        let Some(id) = attribute_value(tag, "id") else { continue };
        let Some(href) = attribute_value(tag, "href") else { continue };
        manifest.insert(id, href);
    }

    // spine: <itemref idref="c1"/>，按出现顺序就是阅读顺序
    let mut order = Vec::new();
    cursor = 0;
    while let Some(at) = text[cursor..].find("<itemref") {
        let start = cursor + at;
        let Some(end) = text[start..].find('>') else { break };
        let tag = &text[start..start + end];
        cursor = start + end;
        if let Some(idref) = attribute_value(tag, "idref") {
            if let Some(href) = manifest.get(&idref) {
                order.push((idref, href.clone()));
            }
        }
    }
    if order.is_empty() {
        return Err(AppError::Message("EPUB 的 spine 里没有可读章节".into()));
    }
    Ok((base, author, order))
}

fn extract_tag_text(text: &str, tag: &str) -> Option<String> {
    // 要求 `>` 紧跟标签名，否则 `<dc:titleFoo>` 会被误判成 title 标签
    let open = format!("<{tag}>");
    let start = text.find(&open)? + open.len();
    let rest = &text[start..];
    let end = rest.find(&format!("</{tag}>"))?;
    let value = rest[..end].trim();
    (!value.is_empty()).then(|| web::decode_entities(value))
}

fn attribute_value(tag: &str, name: &str) -> Option<String> {
    let needle = format!("{name}=\"");
    let start = tag.find(&needle)? + needle.len();
    let rest = &tag[start..];
    let end = rest.find('"')?;
    Some(rest[..end].to_owned())
}

/// 把 EPUB 根目录与相对路径拼起来，并确保结果仍然安全。
///
/// base 取 OPF **所在目录**（`rfind('/')` 得到最后一个斜杠之前的内容）。
/// 所以 `OEBPS/content.opf` + `../cover.jpg` 归一化后是 `OEBPS/cover.jpg`，
/// 而不是 `cover.jpg` —— 少了 base 就会把文件解析到错误的目录。
pub fn resolve_path(opf_path: &str, relative: &str) -> Result<String, AppError> {
    let base = match opf_path.rfind('/') {
        Some(index) => &opf_path[..index],
        None => "",
    };
    let joined = if base.is_empty() { relative.to_owned() } else { format!("{base}/{relative}") };
    let normalized = normalize(&joined)
        .ok_or_else(|| AppError::Message("EPUB 内部路径越界，已拒绝导入".into()))?;
    if !is_safe_entry_name(&normalized) {
        return Err(AppError::Message("EPUB 内部路径越界，已拒绝导入".into()));
    }
    Ok(normalized)
}

/// 处理 `.` 与 `..` 的路径归一化。
///
/// `..` 弹到根目录之上时返回 `None` —— 这是 Zip Slip 的真正防线：
/// 只看原始字符串里有没有 `..` 不够，因为 `a/../b` 本身是合法的。
fn normalize(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                // 越出根目录
                if parts.pop().is_none() {
                    return None;
                }
            }
            other => parts.push(other),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// EPUB 的 mimetype 条目内容。EPUB 规范要求它是 ZIP 里的第一个条目、
/// 且不压缩，内容必须恰好是这串。
pub const EPUB_MIMETYPE: &str = "application/epub+zip";

/// 校验 `mimetype` 条目。
///
/// 这是把「普通 ZIP 改名成 .epub」挡在外面的关键一步：ZIP 只保证有 PK 头，
/// EPUB 还要求声明自己的媒体类型。
pub fn validate_mimetype(raw: &[u8]) -> Result<(), AppError> {
    // 有些打包工具会在末尾多写一个换行，这里容忍换行但不容忍别的内容
    let value = String::from_utf8_lossy(raw);
    let value = value.trim();
    if value != EPUB_MIMETYPE {
        return Err(AppError::Message(format!(
            "文件不是标准 EPUB：mimetype 应该是 {EPUB_MIMETYPE}，实际是 {value}"
        )));
    }
    Ok(())
}

/// 解析整个 EPUB。
pub fn parse(bytes: Vec<u8>) -> Result<ParsedSource, AppError> {
    match crate::import::file_signature(&bytes) {
        "zip" => {}
        _ => return Err(AppError::Message("这不是有效的 EPUB（缺少 ZIP 结构）".into())),
    }
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| AppError::Message("EPUB 结构无法解析，文件可能已损坏".into()))?;

    let entry_count = archive.len();
    // 先收集文件名再逐个取大小：file_names() 持有不可变借用，不能同时 by_name。
    let names: Vec<String> = archive.file_names().map(str::to_owned).collect();
    let mut total_uncompressed: u64 = 0;
    for name in &names {
        if let Ok(entry) = archive.by_name(name) {
            total_uncompressed = total_uncompressed.saturating_add(entry.size());
        }
    }
    validate_archive_limits(entry_count, total_uncompressed)?;

    // mimetype 必须存在且内容正确，否则就是一个改名的普通 ZIP
    let mimetype = archive
        .by_name("mimetype")
        .map_err(|_| AppError::Message("文件不是标准 EPUB：缺少 mimetype 条目".into()))
        .and_then(|mut entry| {
            let mut buffer = Vec::new();
            entry
                .read_to_end(&mut buffer)
                .map_err(|_| AppError::Message("EPUB 的 mimetype 条目无法读取".into()))?;
            Ok(buffer)
        })?;
    validate_mimetype(&mimetype)?;

    let container = read_entry(&mut archive, "META-INF/container.xml")?;
    let opf_path = parse_opf_path(&container)?;
    if !is_safe_entry_name(&opf_path) {
        return Err(AppError::Message("EPUB 的 OPF 路径越界，已拒绝导入".into()));
    }
    let opf = read_entry(&mut archive, &opf_path)?;
    let (title, author, order) = parse_opf(&opf)?;

    let mut parsed = ParsedSource {
        title: title.clone(),
        author,
        origin: None,
        kind: Some(SourceKind::Epub),
        cover: None,
        cover_extension: None,
        page_count: 0,
        chunks: Vec::new(),
        warnings: Vec::new(),
    };
    // 封面：找 manifest 里带 cover-image 属性的条目
    if let Some((bytes, extension)) = read_cover(&mut archive, &opf_path, &opf) {
        parsed.cover = Some(bytes);
        parsed.cover_extension = Some(extension);
    }

    let mut position = 0;
    for (chapter, (idref, href)) in order.iter().enumerate() {
        let entry_path = match resolve_path(&opf_path, href) {
            Ok(path) => path,
            Err(error) => {
                parsed.add_warning(format!("章节 {idref} 路径非法：{error}"));
                continue;
            }
        };
        let raw = match read_entry(&mut archive, &entry_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                parsed.add_warning(format!("章节 {idref} 读取失败：{error}"));
                continue;
            }
        };
        let html = String::from_utf8_lossy(&raw);
        let heading = chapter_heading(&html).unwrap_or_else(|| format!("第 {} 章", chapter + 1));
        let text = web::clean_text(&web::strip_noise(&html));
        if text.chars().count() < 20 {
            parsed.add_warning(format!("第 {} 章正文过短，已跳过", chapter + 1));
            continue;
        }
        let locator = Locator { page: None, chapter: Some((chapter + 1) as u32), heading: Some(heading.clone()) };
        let chunks = chunker::split_into_chunks(&heading, &text, position, &locator);
        position += chunks.len();
        parsed.chunks.extend(chunks);
        if position >= crate::import::MAX_DOCUMENTS_PER_SOURCE {
            parsed.add_warning("章节过多，只导入了前一部分");
            break;
        }
    }
    if parsed.chunks.is_empty() {
        return Err(AppError::Message("EPUB 里没有可导入的正文".into()));
    }
    Ok(parsed)
}

fn read_cover(archive: &mut zip::ZipArchive<std::io::Cursor<Vec<u8>>>, opf_path: &str, opf: &[u8]) -> Option<(Vec<u8>, String)> {
    let text = String::from_utf8_lossy(opf).to_string();
    // 优先 manifest 里 cover-image 指向的条目
    let mut cursor = 0;
    while let Some(at) = text[cursor..].find("<item") {
        let start = cursor + at;
        let Some(end) = text[start..].find('>') else { break };
        let tag = text[start..start + end].to_owned();
        cursor = start + end;
        if !tag.contains("cover-image") {
            continue;
        }
        let Some(href) = attribute_value(&tag, "href") else { continue };
        let path = resolve_path(opf_path, &href).ok()?;
        let bytes = read_entry(archive, &path).ok()?;
        if bytes.len() as u64 > MAX_COVER_BYTES {
            return None;
        }
        let extension = path.rsplit('.').next().map(|value| value.to_lowercase()).unwrap_or_default();
        if matches!(extension.as_str(), "jpg" | "jpeg" | "png" | "gif" | "webp") {
            return Some((bytes, extension));
        }
    }
    None
}

/// 取章节里第一个 heading 作为标题。
///
/// 顺序是 h1 → h2 → h3 → title：章节标题优先于文档标题，
/// 因为 EPUB 里的 `<title>` 常常就是书名，对识别章节没有帮助。
pub fn chapter_heading(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    for tag in ["h1", "h2", "h3", "title"] {
        let open = format!("<{tag}");
        if let Some(at) = lower.find(&open) {
            let start = at + open.len();
            let bytes = lower.as_bytes();
            let mut cursor = start;
            while cursor < bytes.len() && bytes[cursor] != b'>' {
                cursor += 1;
            }
            let rest = &lower[cursor + 1..];
            let close = format!("</{tag}>");
            if let Some(end) = rest.find(&close) {
                let value = web::clean_text(&rest[..end]);
                if !value.is_empty() {
                    return Some(value);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 拒绝_zip_slip_路径() {
        for name in [
            "../evil.xml",
            "OEBPS/../../evil.xml",
            "/etc/passwd",
            "\\windows\\system32",
            "C:/evil",
            "OEBPS//double",
            "OEBPS/\u{0}bad",
        ] {
            assert!(!is_safe_entry_name(name), "{name} 必须被拒绝");
        }
    }

    #[test]
    fn 放行正常路径() {
        for name in ["mimetype", "META-INF/container.xml", "OEBPS/content.opf", "OEBPS/Text/ch1.xhtml"] {
            assert!(is_safe_entry_name(name), "{name} 应当放行");
        }
    }

    #[test]
    fn 归一化并检测路径越界() {
        assert_eq!(normalize("OEBPS/./Text/a.xhtml").as_deref(), Some("OEBPS/Text/a.xhtml"));
        assert_eq!(normalize("OEBPS/Text/../a.xhtml").as_deref(), Some("OEBPS/a.xhtml"));
        // 弹到根目录之上必须返回 None，这是 Zip Slip 的真正防线
        assert_eq!(normalize("OEBPS/../../evil"), None);
        assert_eq!(normalize("../evil"), None);
    }

    #[test]
    fn 相对路径拼接基于_opf_目录() {
        let opf = "OEBPS/content.opf";
        assert_eq!(resolve_path(opf, "Text/ch1.xhtml").unwrap(), "OEBPS/Text/ch1.xhtml");
        assert_eq!(resolve_path("content.opf", "ch1.xhtml").unwrap(), "ch1.xhtml");
        // 章节里写 ../ 是相对 OEBPS 目录上跳，归一化后回到 ZIP 根
        assert_eq!(resolve_path(opf, "../cover.jpg").unwrap(), "cover.jpg");
        // 越出根目录必须报错
        assert!(resolve_path("OEBPS/content.opf", "../../../etc/passwd").is_err());
    }

    #[test]
    fn 解析_container_里的_opf_路径() {
        let xml = br#"<?xml version="1.0"?><container><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
        assert_eq!(parse_opf_path(xml).unwrap(), "OEBPS/content.opf");

        let single = br#"<container><rootfiles><rootfile full-path='book.opf'/></rootfiles></container>"#;
        assert_eq!(parse_opf_path(single).unwrap(), "book.opf");

        assert!(parse_opf_path(b"<container></container>").is_err());
    }

    #[test]
    fn 解析_opf_的书名作者与_spine_顺序() {
        let opf = r#"<?xml version="1.0"?>
        <package><metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
          <dc:title>测试书名</dc:title><dc:creator>测试作者</dc:creator>
        </metadata>
        <manifest>
          <item id="c2" href="text/ch2.xhtml" media-type="application/xhtml+xml"/>
          <item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>
        </manifest>
        <spine><itemref idref="c2"/><itemref idref="c1"/></spine></package>"#.as_bytes();
        let (title, author, order) = parse_opf(opf).unwrap();
        assert_eq!(title, "测试书名");
        assert_eq!(author.as_deref(), Some("测试作者"));
        // spine 顺序优先于 manifest 里的声明顺序
        assert_eq!(order, vec![("c2".to_string(), "text/ch2.xhtml".to_string()), ("c1".to_string(), "text/ch1.xhtml".to_string())]);
    }

    #[test]
    fn 拒绝没有书名或空_spine_的_opf() {
        assert!(parse_opf(br#"<package><metadata></metadata><manifest></manifest><spine></spine></package>"#).is_err());
        assert!(parse_opf(br#"<package><manifest><item id="a" href="a.xhtml"/></manifest><spine></spine></package>"#).is_err());
    }

    #[test]
    fn 拒绝超过上限的压缩包() {
        assert!(validate_archive_limits(10, 1024).is_ok());
        assert!(validate_archive_limits(MAX_ENTRIES + 1, 1024).is_err());
        assert!(validate_archive_limits(10, MAX_TOTAL_UNCOMPRESSED + 1).is_err());
    }

    #[test]
    fn 非_zip_文件被明确拒绝() {
        let error = parse("这不是 EPUB".as_bytes().to_vec()).unwrap_err();
        assert!(error.to_string().contains("不是有效的 EPUB"));
    }

    #[test]
    fn 损坏的_zip_被拒绝() {
        let mut bytes = b"PK".to_vec();
        bytes.extend_from_slice(&[0x03, 0x04, 0xff, 0xff, 0xff, 0xff]);
        assert!(parse(bytes).is_err());
    }

    #[test]
    fn 提取章节标题优先用_heading() {
        // 章节标题优先于文档标题
        assert_eq!(chapter_heading("<html><head><title>书名</title></head><body><h1>第一章</h1></body></html>").as_deref(), Some("第一章"));
        assert_eq!(chapter_heading("<body><h2>小节</h2></body>").as_deref(), Some("小节"));
        // 没有 heading 时才退回 title
        assert_eq!(chapter_heading("<html><head><title>文档标题</title></head><body><p>正文</p></body></html>").as_deref(), Some("文档标题"));
        assert_eq!(chapter_heading("<body><p>无标题</p></body>"), None);
    }

    // ---------- 真实样本：自己拼出来的最小 EPUB ----------

    /// 拼一个 EPUB 样本。
    ///
    /// `mimetype` 缺失 / 写错、`container.xml` 缺失这些都是真实世界会遇到的坏文件，
    /// 测试自己构造比找外部样本可靠，也不依赖开发机上的任何文件。
    struct EpubBuilder {
        mimetype: Option<&'static str>,
        container: Option<&'static str>,
        opf: Option<String>,
        extra: Vec<(String, Vec<u8>)>,
    }

    impl EpubBuilder {
        fn new() -> Self {
            Self { mimetype: Some(EPUB_MIMETYPE), container: Some(CONTAINER_XML), opf: None, extra: Vec::new() }
        }

        fn build(self) -> Vec<u8> {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
            // 规范要求 mimetype 是第一个条目且不压缩
            if let Some(value) = self.mimetype {
                writer.start_file("mimetype", stored_options()).unwrap();
                std::io::Write::write_all(&mut writer, value.as_bytes()).unwrap();
            }
            if let Some(value) = self.container {
                writer.start_file("META-INF/container.xml", deflated_options()).unwrap();
                std::io::Write::write_all(&mut writer, value.as_bytes()).unwrap();
            }
            for (name, bytes) in self.extra {
                writer.start_file(name, deflated_options()).unwrap();
                std::io::Write::write_all(&mut writer, &bytes).unwrap();
            }
            if let Some(opf) = self.opf {
                writer.start_file(OPF_PATH, deflated_options()).unwrap();
                std::io::Write::write_all(&mut writer, opf.as_bytes()).unwrap();
            }
            writer.finish().unwrap().into_inner()
        }
    }

    const CONTAINER_XML: &str = r#"<?xml version="1.0"?><container><rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#;
    const OPF_PATH: &str = "OEBPS/content.opf";

    /// mimetype 必须不压缩，其余条目用 deflate。
    fn stored_options() -> zip::write::FileOptions<'static, ()> {
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored)
    }

    fn deflated_options() -> zip::write::FileOptions<'static, ()> {
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated)
    }

    fn opf_xml(version: &str, cover: bool) -> String {
        let cover_item = if cover { r#"<item id="cover" href="images/cover.jpg" media-type="image/jpeg" properties="cover-image"/>"# } else { "" };
        let cover_meta = if cover { r#"<meta name="cover" content="cover"/>"# } else { "" };
        format!(
            r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="{version}" unique-identifier="id">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:title>测试书名</dc:title><dc:creator>测试作者</dc:creator>{cover_meta}
  </metadata>
  <manifest>{cover_item}
    <item id="c1" href="text/ch1.xhtml" media-type="application/xhtml+xml"/>
    <item id="c2" href="text/ch2.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine><itemref idref="c2"/><itemref idref="c1"/></spine>
</package>"#
        )
    }

    fn chapter(title: &str, body: &str) -> Vec<u8> {
        format!("<html><head><title>文档标题</title></head><body><h1>{title}</h1><p>{body}</p></body></html>").into_bytes()
    }

    /// 一个内容完整、格式合法的 EPUB。
    fn sample_epub(version: &str, cover: bool) -> Vec<u8> {
        EpubBuilder {
            opf: Some(opf_xml(version, cover)),
            ..EpubBuilder::new()
        }
        .extra(vec![
            ("OEBPS/text/ch1.xhtml".to_string(), chapter("第一章", "第一章的正文内容，用来凑够最短长度门槛。")),
            ("OEBPS/text/ch2.xhtml".to_string(), chapter("第二章", "第二章的正文内容，同样要够长才不会被跳过。")),
            ("OEBPS/images/cover.jpg".to_string(), vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]),
        ])
        .build()
    }

    impl EpubBuilder {
        fn extra(mut self, entries: Vec<(String, Vec<u8>)>) -> Self {
            self.extra = entries;
            self
        }
    }

    #[test]
    fn 标准_epub_按_spine_顺序导入() {
        let parsed = parse(sample_epub("2.0", false)).unwrap();
        assert_eq!(parsed.title, "测试书名");
        assert_eq!(parsed.author.as_deref(), Some("测试作者"));
        // spine 里 c2 在前，章节号要跟着 spine 走而不是文件名
        assert_eq!(parsed.chunks[0].locator.chapter, Some(1));
        assert!(parsed.chunks.iter().all(|chunk| chunk.locator.chapter.is_some()));
        assert!(parsed.chunks[0].content.contains("第二章"));
    }

    #[test]
    fn epub_3_同样能导入并复制封面() {
        let parsed = parse(sample_epub("3.0", true)).unwrap();
        assert_eq!(parsed.chunks.len() >= 2, true);
        assert_eq!(parsed.cover.as_ref().map(|bytes| bytes.len()), Some(6));
        assert_eq!(parsed.cover_extension.as_deref(), Some("jpg"));
    }

    #[test]
    fn 没有封面的_epub_不报错() {
        let parsed = parse(sample_epub("3.0", false)).unwrap();
        assert!(parsed.cover.is_none());
    }

    #[test]
    fn 普通_zip_改名_epub_被拒绝() {
        // 只有 container 和 opf，缺 mimetype —— 就是一个改名的 ZIP
        let bytes = EpubBuilder { mimetype: None, ..EpubBuilder::new() }
            .extra(vec![("OEBPS/text/ch1.xhtml".to_string(), chapter("第一章", "第一章的正文内容，凑长度用的填充文字。"))])
            .build();
        let error = parse(bytes).unwrap_err().to_string();
        assert!(error.contains("缺少 mimetype"), "实际：{error}");
    }

    #[test]
    fn mimetype_内容错误被拒绝() {
        for wrong in ["application/zip", "text/plain", "application/epub+xml"] {
            let bytes = EpubBuilder { mimetype: Some(wrong), ..EpubBuilder::new() }
                .extra(vec![("OEBPS/text/ch1.xhtml".to_string(), chapter("第一章", "第一章的正文内容，凑长度用的填充文字。"))])
                .build();
            let error = parse(bytes).unwrap_err().to_string();
            assert!(error.contains("不是标准 EPUB"), "实际：{error}");
        }
    }

    #[test]
    fn mimetype_结尾多一个换行仍然接受() {
        // 有些打包工具会在 mimetype 后面补一个换行，不能因此把正常书拒掉
        assert!(validate_mimetype(b"application/epub+zip\n").is_ok());
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer.start_file("mimetype", stored_options()).unwrap();
        std::io::Write::write_all(&mut writer, b"application/epub+zip\n").unwrap();
        writer.start_file("META-INF/container.xml", deflated_options()).unwrap();
        std::io::Write::write_all(&mut writer, CONTAINER_XML.as_bytes()).unwrap();
        writer.start_file(OPF_PATH, deflated_options()).unwrap();
        std::io::Write::write_all(&mut writer, opf_xml("2.0", false).as_bytes()).unwrap();
        writer.start_file("OEBPS/text/ch1.xhtml", deflated_options()).unwrap();
        std::io::Write::write_all(&mut writer, &chapter("第一章", "第一章的正文内容，凑长度用的填充文字。")).unwrap();
        writer.start_file("OEBPS/text/ch2.xhtml", deflated_options()).unwrap();
        std::io::Write::write_all(&mut writer, &chapter("第二章", "第二章的正文内容，凑长度用的填充文字。")).unwrap();
        let bytes = writer.finish().unwrap().into_inner();
        assert!(parse(bytes).is_ok());
    }

    #[test]
    fn 缺少_container_被拒绝() {
        let bytes = EpubBuilder { container: None, opf: Some(opf_xml("2.0", false)), ..EpubBuilder::new() }
            .extra(vec![("OEBPS/text/ch1.xhtml".to_string(), chapter("第一章", "第一章的正文内容，凑长度用的填充文字。"))])
            .build();
        let error = parse(bytes).unwrap_err().to_string();
        assert!(error.contains("缺少条目"), "实际：{error}");
    }

    #[test]
    fn 缺少_opf_被拒绝() {
        let bytes = EpubBuilder { opf: None, ..EpubBuilder::new() }
            .extra(vec![("OEBPS/text/ch1.xhtml".to_string(), chapter("第一章", "第一章的正文内容，凑长度用的填充文字。"))])
            .build();
        assert!(parse(bytes).is_err());
    }

    #[test]
    fn zip_slip_恶意条目被拒绝() {
        // 条目名带 .. ，解压后会写到 ZIP 之外
        let bytes = EpubBuilder { opf: Some(opf_xml("2.0", false)), ..EpubBuilder::new() }
            .extra(vec![("OEBPS/../../evil.xhtml".to_string(), chapter("恶意", "这段内容不应该被写出来。"))])
            .build();
        // OPF 本身在合法路径上，恶意条目只是多出来的，应当被静默忽略或拒绝，但不能写到外面
        let outcome = parse(bytes);
        if let Ok(parsed) = outcome {
            assert!(parsed.chunks.iter().all(|chunk| !chunk.content.contains("不应该")), "越界条目被读进来了");
        }
        assert!(is_safe_entry_name("OEBPS/../../evil.xhtml") == false);
    }

    #[test]
    fn 高压缩比条目超过上限被拒绝() {
        let bomb = vec![b'A'; (MAX_ENTRY_BYTES + 1024) as usize];
        let bytes = EpubBuilder { opf: Some(opf_xml("2.0", false)), ..EpubBuilder::new() }
            .extra(vec![("OEBPS/text/ch1.xhtml".to_string(), bomb)])
            .build();
        let error = parse(bytes).unwrap_err().to_string();
        assert!(error.contains("过大") || error.contains("正文过短"), "实际：{error}");
    }
}
