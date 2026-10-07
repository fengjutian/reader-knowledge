//! PDF 导入：按页提取文本。
//!
//! 依赖里没有可用的 PDF 库（离线环境拉不到 lopdf / pdf-extract），
//! 所以这里实现一个**最小可用子集**：
//! 1. 顺序扫描 `N M obj … endobj`，建一张对象表（不做 xref 解析）。
//! 2. 找 `/Type /Page` 的页面对象，按对象号排序当作页序。
//! 3. 读取每页 `/Contents` 引用的流，FlateDecode 解压后抽取文本算子。
//! 4. 识别 `BT/ET` 之间的 `Tj` / `TJ` / `'` / `"` 文本算子。
//!
//! 明确不支持（会在结果里如实说明，而不是静默返回空）：
//! - 扫描版 PDF（只有图片、没有文本算子）→ 提示「暂不支持 OCR」
//! - 对象流 / 交叉引用流（PDF 1.5+ 的对象压缩）→ 尝试顺序扫描兜底
//! - 嵌入字体的自定义 CID 编码 → 能取出字符但可能不是可读文字

use crate::error::AppError;
use crate::import::{chunker, Locator, ParsedSource, SourceKind};
use std::collections::BTreeMap;
use std::io::Read;

/// 文件大小上限。
pub const MAX_PDF_BYTES: u64 = 200 * 1024 * 1024;
/// 单个内容流解压后的上限。
pub const MAX_STREAM_BYTES: usize = 32 * 1024 * 1024;
/// 一页里抽取文本的长度上限，避免异常文件产出天文数字的段落。
pub const MAX_PAGE_CHARS: usize = 40_000;

/// 一个 PDF 对象：`对象号 -> 内容`。
type ObjectTable = BTreeMap<u32, Vec<u8>>;

/// 顺序扫描建立对象表。
///
/// PDF 的 xref 可能有各种历史包袱，但 `N G obj` 这个模式在整个文件里
/// 出现的位置是确定的，顺序扫描足以覆盖绝大多数文件。
pub fn scan_objects(data: &[u8]) -> ObjectTable {
    let mut objects = BTreeMap::new();
    let mut index = 0;
    while index + 3 < data.len() {
        // 找到 "<num> <gen> obj"
        let Some((number, next)) = parse_object_header(data, index) else {
            index += 1;
            continue;
        };
        // 对象体到对应的 endobj
        let end = find_keyword(data, next, b"endobj").map(|at| at).unwrap_or(data.len());
        objects.insert(number, data[next..end.min(data.len())].to_vec());
        index = end.saturating_add(6).max(next + 1);
    }
    objects
}

/// 从 `start` 开始尝试解析 `N G obj`，返回对象号与 obj 关键字之后的位置。
fn parse_object_header(data: &[u8], start: usize) -> Option<(u32, usize)> {
    let mut cursor = start;
    while cursor < data.len() && (data[cursor] as char).is_ascii_digit() {
        cursor += 1;
    }
    if cursor == start || cursor >= data.len() {
        return None;
    }
    let number: u32 = std::str::from_utf8(&data[start..cursor]).ok()?.parse().ok()?;
    while cursor < data.len() && (data[cursor] as char).is_ascii_whitespace() {
        cursor += 1;
    }
    // 第二个数字（generation）
    let gen_start = cursor;
    while cursor < data.len() && (data[cursor] as char).is_ascii_digit() {
        cursor += 1;
    }
    if cursor == gen_start {
        return None;
    }
    while cursor < data.len() && (data[cursor] as char).is_ascii_whitespace() {
        cursor += 1;
    }
    if data[cursor..].starts_with(b"obj") {
        Some((number, cursor + 3))
    } else {
        None
    }
}

fn find_keyword(data: &[u8], from: usize, keyword: &[u8]) -> Option<usize> {
    if from >= data.len() || keyword.is_empty() {
        return None;
    }
    (from..=data.len().saturating_sub(keyword.len()))
        .find(|index| data[*index..].starts_with(keyword))
}

/// 在对象体里找 `/Key` 的值，跳过前一个元素的尾部。
///
/// 直接 `find("/Title")` 会误命中 `/Subtitle` 之类的名字，
/// 所以要求匹配位置前面是分隔符（空白或 `<<`/`/`）。
fn dict_value<'a>(body: &'a [u8], key: &str) -> Option<&'a [u8]> {
    let needle = format!("/{key}");
    let bytes = needle.as_bytes();
    let mut index = 0;
    let at = loop {
        let offset = body[index..].windows(bytes.len()).position(|window| window == bytes)?;
        let candidate = index + offset;
        index = candidate + 1;
        let boundary = match body.get(candidate + bytes.len()).copied() {
            None => true,
            Some(byte) => (byte as char).is_ascii_whitespace() || byte == b'/' || byte == b'<' || byte == b'>' || byte == b'[' || byte == b']',
        };
        if boundary {
            break candidate;
        }
    };
    let rest = &body[at + bytes.len()..];
    let mut cursor = 0;
    while cursor < rest.len() && (rest[cursor] as char).is_ascii_whitespace() {
        cursor += 1;
    }
    if cursor >= rest.len() {
        return None;
    }
    // 字面量字符串 `(…)`
    if rest[cursor] == b'(' {
        let mut depth = 0;
        let mut end = cursor;
        while end < rest.len() {
            match rest[end] {
                b'\\' => end += 1,
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some(&rest[cursor + 1..end]);
                    }
                }
                _ => {}
            }
            end += 1;
        }
        return None;
    }
    // 引用形式 `12 0 R`
    if rest[cursor].is_ascii_digit() {
        let start = cursor;
        while cursor < rest.len() && rest[cursor].is_ascii_digit() {
            cursor += 1;
        }
        if cursor < rest.len() && rest[cursor] == b'R' {
            return Some(&rest[start..cursor]);
        }
        return None;
    }
    // 名称形式 `/Name`
    if rest[cursor] == b'/' {
        let start = cursor + 1;
        let mut end = start;
        while end < rest.len() && !(rest[end] as char).is_ascii_whitespace() && rest[end] != b'/' {
            end += 1;
        }
        return Some(&rest[start..end]);
    }
    // 字符串形式
    if rest[cursor] == b'<' && rest.get(cursor + 1) != Some(&b'<') {
        let end = rest[cursor + 1..].iter().position(|byte| *byte == b'>')?;
        return Some(&rest[cursor + 1..cursor + 1 + end]);
    }
    None
}

/// 判断对象体里是否含有指定 name。
///
/// 与 `dict_value` 一样要避免 `/Pages` 命中 `/Type /Page` 这类前缀误配，
/// 所以要求 name 后面紧跟字典分隔符。
fn dict_has_name(body: &[u8], name: &str) -> bool {
    let needle = format!("/{name}");
    let bytes = needle.as_bytes();
    let mut index = 0;
    while let Some(offset) = body[index..].windows(bytes.len()).position(|window| window == bytes) {
        let at = index + offset;
        index = at + 1;
        let after = body.get(at + bytes.len()).copied();
        let boundary = match after {
            None => true,
            Some(byte) => (byte as char).is_ascii_whitespace() || byte == b'/' || byte == b'<' || byte == b'>' || byte == b'[' || byte == b']',
        };
        if boundary {
            return true;
        }
    }
    false
}

/// 从对象体里取出流数据（`stream … endstream`）。
fn stream_data(body: &[u8]) -> Option<&[u8]> {
    let at = find_keyword(body, 0, b"stream")? + 6;
    // stream 关键字后跟一个换行
    let mut start = at;
    while start < body.len() && (body[start] == b'\r' || body[start] == b'\n') {
        start += 1;
    }
    let end = find_keyword(body, start, b"endstream")?;
    Some(&body[start..end])
}

/// 解压内容流。只有 FlateDecode 支持。
pub fn decode_stream(body: &[u8]) -> Result<Vec<u8>, String> {
    let raw = stream_data(body).ok_or_else(|| "对象不是流".to_string())?;
    let raw = raw.strip_suffix(b"\r\n").or_else(|| raw.strip_suffix(b"\n")).unwrap_or(raw);
    if !dict_has_name(body, "FlateDecode") {
        return Ok(raw.to_vec());
    }
    let mut decoder = flate2::read::ZlibDecoder::new(raw);
    let mut output = Vec::new();
    // 限制解压量，防止压缩炸弹
    decoder
        .by_ref()
        .take(MAX_STREAM_BYTES as u64)
        .read_to_end(&mut output)
        .map_err(|error| format!("解压失败：{error}"))?;
    if output.len() >= MAX_STREAM_BYTES {
        return Err("内容流解压后过大".to_string());
    }
    Ok(output)
}

/// 从内容流里抽取文本。
///
/// 逐字节扫描内容流：遇到 `BT` 开始一段文本，遇到 `ET` 结束；
/// 段内处理 `(…)Tj`、`[(…)…]TJ`、`(…)'`、`(…)"` 四种文本算子。
pub fn extract_text_from_content(content: &[u8]) -> String {
    let mut output = String::new();
    let mut index = 0;
    let mut in_text = false;
    let mut pending: Vec<String> = Vec::new();
    while index < content.len() {
        // 段内：遇到 Tj / TJ / ' / " 就把 pending 的字符串落下来
        if in_text {
            if content[index] == b'(' {
                match read_literal_string(content, index) {
                    Some((text, next)) => {
                        pending.push(text);
                        index = next;
                        continue;
                    }
                    None => {
                        index += 1;
                        continue;
                    }
                }
            }
            if content[index] == b'<' && content.get(index + 1) != Some(&b'<') {
                if let Some((text, next)) = read_hex_string(content, index) {
                    pending.push(text);
                    index = next;
                    continue;
                }
            }
            if content[index].is_ascii_alphabetic() {
                let start = index;
                while index < content.len() && content[index].is_ascii_alphabetic() {
                    index += 1;
                }
                let word = &content[start..index];
                match word {
                    b"BT" => in_text = true,
                    b"ET" => in_text = false,
                    b"Tj" | b"TJ" | b"'" | b"\"" => {
                        if !pending.is_empty() {
                            output.push_str(&pending.concat());
                            pending.clear();
                            // 换行算子同样代表一次绘制换行
                            output.push('\n');
                        }
                    }
                    // 其他算子：只是清空待落下的字符串
                    b"Td" | b"TD" | b"T*" | b"TL" => pending.clear(),
                    _ => {}
                }
                continue;
            }
            index += 1;
            continue;
        }
        if content[index] == b'B' && content[index..].starts_with(b"BT") {
            in_text = true;
            index += 2;
            continue;
        }
        index += 1;
    }
    normalize_page_text(&output)
}

/// 读取 `(…)` 字面量字符串，处理转义与嵌套括号。
fn read_literal_string(content: &[u8], start: usize) -> Option<(String, usize)> {
    let mut depth = 0;
    let mut index = start;
    let mut buffer: Vec<u8> = Vec::new();
    while index < content.len() {
        match content[index] {
            b'\\' if index + 1 < content.len() => {
                match content[index + 1] {
                    b'n' => buffer.push(b'\n'),
                    b'r' => buffer.push(b'\r'),
                    b't' => buffer.push(b'\t'),
                    b'b' => buffer.push(0x08),
                    b'f' => buffer.push(0x0c),
                    b'\n' => {}
                    other => buffer.push(other),
                }
                index += 2;
                continue;
            }
            b'(' => {
                depth += 1;
                if depth > 1 {
                    buffer.push(b'(');
                }
                index += 1;
                continue;
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some((decode_pdf_string(&buffer), index + 1));
                }
                buffer.push(b')');
                index += 1;
                continue;
            }
            other => {
                buffer.push(other);
                index += 1;
            }
        }
    }
    None
}

/// 读取 `<…>` 十六进制字符串。
fn read_hex_string(content: &[u8], start: usize) -> Option<(String, usize)> {
    let mut index = start + 1;
    let mut digits: Vec<u8> = Vec::new();
    while index < content.len() && content[index] != b'>' {
        if (content[index] as char).is_ascii_hexdigit() {
            digits.push(content[index]);
        }
        index += 1;
    }
    if index >= content.len() {
        return None;
    }
    let mut buffer = Vec::new();
    for pair in digits.chunks(2) {
        let high = (pair[0] as char).to_digit(16).unwrap_or(0) as u8;
        let low = pair.get(1).and_then(|byte| (*byte as char).to_digit(16)).unwrap_or(0) as u8;
        buffer.push(high * 16 + low);
    }
    Some((decode_pdf_string(&buffer), index + 1))
}

/// 极简 PDFDocEncoding/Latin-1 解码。
///
/// 真正的 PDF 字体编码要查字体对象的 /Differences 或 ToUnicode，
/// 这里只处理常见的 Latin-1 与 UTF-16BE BOM，识别不了时按字节直通，
/// 宁可给出可辨认的片段也不要丢掉整页。
fn decode_pdf_string(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[0] == 0xFE && bytes[1] == 0xFF {
        // UTF-16BE
        let mut units = Vec::new();
        for pair in bytes[2..].chunks(2) {
            if pair.len() == 2 {
                units.push(u16::from_be_bytes([pair[0], pair[1]]));
            }
        }
        return String::from_utf16_lossy(&units);
    }
    bytes.iter().map(|byte| *byte as char).collect()
}

/// 页内换行：连续空行折叠，行内空白压缩。
fn normalize_page_text(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut newlines = 0;
    for character in input.chars() {
        if character == '\n' || character == '\r' {
            newlines += 1;
            if newlines <= 2 {
                result.push('\n');
            }
            continue;
        }
        if newlines > 2 {
            newlines = 3;
        }
        if character.is_whitespace() {
            if !result.ends_with(' ') && !result.is_empty() {
                result.push(' ');
            }
            continue;
        }
        result.push(character);
    }
    result.trim().to_owned()
}

/// 提取 PDF 里的元数据标题。
///
/// PDF 的文本字符串可能是 UTF-16BE（带 `FE FF` BOM）也可能是
/// PDFDocEncoding（单字节）。`decode_pdf_string` 只处理了 UTF-16BE，
/// 这里再兜一层：按字符而非字节读取单字节串，避免多字节被截断。
pub fn extract_title(objects: &ObjectTable) -> Option<String> {
    for body in objects.values() {
        // Info 字典的特征字段
        if !(dict_has_name(body, "Producer") || dict_has_name(body, "Creator") || dict_has_name(body, "CreationDate")) {
            continue;
        }
        let Some(raw) = dict_value(body, "Title") else { continue };
        let text = if raw.len() >= 2 && raw[0] == 0xFE && raw[1] == 0xFF {
            let units: Vec<u16> = raw[2..]
                .chunks(2)
                .filter(|pair| pair.len() == 2)
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            String::from_utf16_lossy(&units)
        } else {
            normalize_page_text(&raw.iter().map(|byte| *byte as char).collect::<String>())
        };
        if !text.trim().is_empty() {
            return Some(text.trim().to_owned());
        }
    }
    None
}

/// 解析 PDF 并按页生成文档块。
pub fn parse(bytes: Vec<u8>) -> Result<ParsedSource, AppError> {
    if bytes.len() as u64 > MAX_PDF_BYTES {
        return Err(AppError::Message("PDF 体积超过导入上限".into()));
    }
    let header = &bytes[..bytes.len().min(5)];
    if !header.starts_with(b"%PDF-") {
        return Err(AppError::Message("这不是有效的 PDF（缺少 %PDF- 头）".into()));
    }
    let objects = scan_objects(&bytes);
    if objects.is_empty() {
        return Err(AppError::Message("PDF 结构无法解析，可能已损坏".into()));
    }

    // 页面对象：含 /Type /Page 且带 /Contents
    let mut page_numbers: Vec<u32> = objects
        .iter()
        .filter(|(_, body)| dict_has_name(body, "Page") && !dict_has_name(body, "Pages") && dict_value(body, "Contents").is_some())
        .map(|(number, _)| *number)
        .collect();
    page_numbers.sort_unstable();

    let mut parsed = ParsedSource {
        title: extract_title(&objects).unwrap_or_default(),
        author: None,
        origin: None,
        kind: Some(SourceKind::Pdf),
        cover: None,
        cover_extension: None,
        page_count: page_numbers.len() as u32,
        chunks: Vec::new(),
        warnings: Vec::new(),
    };

    let mut position = 0;
    let mut pages_with_text = 0;
    for (index, number) in page_numbers.iter().enumerate() {
        let Some(body) = objects.get(number) else { continue };
        let content = match collect_page_content(&objects, body) {
            Some(bytes) => bytes,
            None => continue,
        };
        let text = extract_text_from_content(&content);
        let text: String = text.chars().take(MAX_PAGE_CHARS).collect();
        if text.trim().is_empty() {
            continue;
        }
        pages_with_text += 1;
        let heading = format!("第 {} 页", index + 1);
        let locator = Locator { page: Some((index + 1) as u32), chapter: None, heading: Some(heading.clone()) };
        let chunks = chunker::split_into_chunks(&heading, &text, position, &locator);
        position += chunks.len();
        parsed.chunks.extend(chunks);
        if position >= crate::import::MAX_DOCUMENTS_PER_SOURCE {
            parsed.add_warning("页数过多，只导入了前一部分");
            break;
        }
    }

    if parsed.title.is_empty() {
        parsed.title = first_line_title(&parsed.chunks);
    }
    if parsed.chunks.is_empty() {
        // 计划 9.4.4：扫描版 PDF 明确提示，不导入空资料
        return Err(AppError::Message(
            "这份 PDF 没有可提取的文字，可能是扫描版；暂不支持 OCR，已跳过导入".into(),
        ));
    }
    if pages_with_text < page_numbers.len() {
        parsed.add_warning(format!("共 {} 页，其中 {} 页没有文字层（可能是图片页）", page_numbers.len(), page_numbers.len() - pages_with_text));
    }
    Ok(parsed)
}

/// 收集一页的内容流。`/Contents` 可能是单个引用，也可能是引用数组。
fn collect_page_content(objects: &ObjectTable, page_body: &[u8]) -> Option<Vec<u8>> {
    let raw = dict_value(page_body, "Contents")?;
    let text = String::from_utf8_lossy(raw).to_string();
    let mut streams: Vec<u8> = Vec::new();
    // 单个引用 "12 0 R"
    let mut numbers: Vec<u32> = Vec::new();
    for part in text.split_whitespace() {
        if let Ok(value) = part.trim_end_matches('R').trim().parse::<u32>() {
            numbers.push(value);
        }
    }
    if numbers.is_empty() {
        return None;
    }
    for number in numbers {
        let Some(body) = objects.get(&number) else { continue };
        if let Ok(decoded) = decode_stream(body) {
            streams.extend_from_slice(&decoded);
            streams.push(b'\n');
        }
    }
    (!streams.is_empty()).then_some(streams)
}

/// 没有元数据标题时，用正文第一行当标题。
fn first_line_title(chunks: &[crate::import::DocumentChunk]) -> String {
    chunks
        .first()
        .and_then(|chunk| chunk.content.lines().find(|line| !line.trim().is_empty()))
        .map(|line| line.trim().chars().take(60).collect::<String>())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| "未命名 PDF".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一个最小的合法 PDF，方便测试解析路径。
    fn tiny_pdf(content: &str) -> Vec<u8> {
        let mut bytes = b"%PDF-1.4\n".to_vec();
        let stream = format!("BT /F1 12 Tf 72 720 Td ({content}) Tj ET");
        let object = format!("4 0 obj\n<< /Length {} >>\nstream\n{stream}\nendstream\nendobj\n", stream.len());
        bytes.extend_from_slice(object.as_bytes());
        let page = b"3 0 obj\n<< /Type /Page /Contents 4 0 R /Parent 2 0 R /MediaBox [0 0 612 792] >>\nendobj\n";
        bytes.extend_from_slice(page);
        bytes.extend_from_slice(b"1 0 obj\n<< /Title (My Doc) /Producer (test) >>\nendobj\n");
        bytes.extend_from_slice(b"trailer\n<< /Root 2 0 R >>\n%%EOF\n");
        bytes
    }

    #[test]
    fn 顺序扫描能建立对象表() {
        let objects = scan_objects(&tiny_pdf("hello"));
        assert!(objects.contains_key(&3), "页面对象应当被找到");
        assert!(objects.contains_key(&4), "内容流对象应当被找到");
        assert!(dict_has_name(&objects[&3], "Page"));
    }

    #[test]
    fn 抽取_tj_文本() {
        let content = b"BT /F1 12 Tf 72 720 Td (Hello PDF) Tj ET";
        assert_eq!(extract_text_from_content(content), "Hello PDF");
    }

    #[test]
    fn 抽取_tj_数组文本() {
        let content = b"BT [(Kerned) -500 (Text)] TJ ET";
        assert_eq!(extract_text_from_content(content), "KernedText");
    }

    #[test]
    fn 处理字面量字符串的转义() {
        let content = b"BT (Line1\\nLine2) Tj ET";
        let text = extract_text_from_content(content);
        assert!(text.contains("Line1"));
        assert!(text.contains("Line2"));
    }

    #[test]
    fn 处理十六进制字符串() {
        let content = b"BT <48656C6C6F> Tj ET";
        assert_eq!(extract_text_from_content(content), "Hello");
    }

    #[test]
    fn 解码_utf16_be_字符串() {
        let mut bytes = vec![0xFE, 0xFF];
        for unit in "中文字".encode_utf16() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        assert_eq!(decode_pdf_string(&bytes), "中文字");
    }

    #[test]
    fn 忽略非文本算子() {
        // 图形算子不该被当成文本
        let content = b"1 0 0 RG 5 w 100 100 m 200 200 l S BT (Real Text) Tj ET";
        assert_eq!(extract_text_from_content(content), "Real Text");
    }

    #[test]
    fn 非_pdf_文件被拒绝() {
        let error = parse("这不是 PDF".as_bytes().to_vec()).unwrap_err();
        assert!(error.to_string().contains("不是有效的 PDF"));
    }

    #[test]
    fn 空文件被拒绝() {
        assert!(parse(Vec::new()).is_err());
    }

    #[test]
    fn 扫描版_pdf_明确提示不支持_ocr() {
        // 结构合法但没有任何文本算子
        let mut bytes = b"%PDF-1.4\n".to_vec();
        bytes.extend_from_slice(b"4 0 obj\n<< /Length 10 >>\nstream\nq Q Q Q \nendstream\nendobj\n");
        bytes.extend_from_slice(b"3 0 obj\n<< /Type /Page /Contents 4 0 R >>\nendobj\n");
        let error = parse(bytes).unwrap_err();
        assert!(error.to_string().contains("暂不支持 OCR"), "{error}");
    }

    #[test]
    fn 读取标题元数据() {
        let objects = scan_objects(&tiny_pdf("body"));
        assert_eq!(extract_title(&objects).as_deref(), Some("My Doc"));
    }

    #[test]
    fn 没有标题时用正文首行() {
        let chunks = vec![crate::import::DocumentChunk {
            position: 0,
            heading: "第 1 页".into(),
            content: "第一行标题\n第二行正文".into(),
            locator: Locator::default(),
        }];
        assert_eq!(first_line_title(&chunks), "第一行标题");
    }
}
