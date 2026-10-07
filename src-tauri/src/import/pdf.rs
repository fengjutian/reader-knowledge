//! PDF 导入：按页提取文本。
//!
//! 两条路径，主次分明：
//!
//! 1. **主路径 `parse_with_primary_engine`**：用 `lopdf` 打开文档并逐页取文本。
//!    它能正确处理 xref 表与 xref 流（PDF 1.5+ 的对象压缩）、FlateDecode、
//!    以及嵌入字体的 `ToUnicode` 映射 —— 这些正是手写解析器搞不定、
//!    导致真实中文 PDF 变成乱码的部分。
//! 2. **兜底 `parse_with_legacy_fallback`**：本文件原有的手写顺序扫描器。
//!    它不做 xref 解析、不认识对象流，只在主路径整个失败时才启用，
//!    命中时会在 warnings 里如实说明用的是兼容路径。
//!
//! 明确不支持（会明确提示，而不是静默返回空）：
//! - 扫描版 PDF（只有图片、没有文本层）→ 提示「暂不支持 OCR」
//! - 加密 / 密码保护的 PDF → 明确说「已加密」，不能当成扫描版
//! - 字体编码无法识别的页面 → 跳过该页并给出 warning，整本都乱码则拒绝导入
//!
//! 页码定位由两条路径统一保证：页码从 1 开始，一页拆成多个块时都带同一个页码，
//! 绝不把相邻页合并成一个无法定位的块。

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

/// 加密 / 密码保护的 PDF 给用户的固定提示。
///
/// 不能把它当成扫描版：两者要走的建议完全不同，用户需要知道是哪种问题。
pub const ENCRYPTED_MESSAGE: &str = "这份 PDF 已加密或受密码保护，暂时无法导入。";

/// 扫描版（整本没有文字层）的固定提示。
pub const SCANNED_MESSAGE: &str = "这份 PDF 没有可提取的文字，可能是扫描版；暂不支持 OCR，已跳过导入。";

/// 文本质量：用来识别「提取出来了但其实是乱码」的情况。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextQuality {
    /// 可打印字符占比
    pub printable_ratio: f64,
    /// Unicode 替换字符 U+FFFD 占比
    pub replacement_ratio: f64,
    /// 控制字符占比
    pub control_ratio: f64,
}

impl TextQuality {
    /// 是否明显是乱码。
    ///
    /// 阈值故意放得比较宽：真正能读的正文即使排版再差，可打印占比也很高；
    /// 而 CID 字体映射失败的典型产物是大量 `�`、空字符或私用区码位。
    pub fn is_garbled(&self) -> bool {
        self.printable_ratio < 0.80 || self.replacement_ratio > 0.05 || self.control_ratio > 0.10
    }
}

/// 统计一段提取文本的可读性。
pub fn assess_text_quality(text: &str) -> TextQuality {
    let chars: Vec<char> = text.chars().filter(|c| !c.is_whitespace()).collect();
    if chars.is_empty() {
        return TextQuality { printable_ratio: 0.0, replacement_ratio: 0.0, control_ratio: 0.0 };
    }
    let total = chars.len() as f64;
    let printable = chars.iter().filter(|c| !c.is_control() && **c != '\u{FFFD}').count() as f64;
    let replacement = chars.iter().filter(|c| **c == '\u{FFFD}').count() as f64;
    let control = chars.iter().filter(|c| c.is_control()).count() as f64;
    TextQuality { printable_ratio: printable / total, replacement_ratio: replacement / total, control_ratio: control / total }
}

/// 清理主解析器给出的整页文本。
///
/// lopdf 会把每个文本算子的结果直接拼起来，行内常有用于对齐的尾随空格，
/// 中文 PDF 里还常出现「汉 字」这种被拆开的空格，这些都会影响检索质量。
pub fn clean_extracted_text(input: &str) -> String {
    let normalized = normalize_page_text(input);
    let chars: Vec<char> = normalized.chars().collect();
    let mut output = String::with_capacity(chars.len());
    let mut index = 0;
    while index < chars.len() {
        let current = chars[index];
        if current == ' ' && index + 1 < chars.len() {
            let next = chars[index + 1];
            let previous = output.chars().last();
            // 中文之间的对齐空格直接去掉：不影响语义，还原本来就没有的排版
            let between_cjk = previous.map(is_cjk).unwrap_or(false) && is_cjk(next);
            // 中文标点前后的空格同理
            let before_punctuation = is_cjk_punctuation(next) || (previous.map(is_cjk_punctuation).unwrap_or(false) && is_cjk(next));
            if between_cjk || before_punctuation {
                index += 1;
                continue;
            }
        }
        output.push(current);
        index += 1;
    }
    output
}

fn is_cjk(value: char) -> bool {
    matches!(value as u32, 0x3000..=0x303F | 0x3400..=0x4DBF | 0x4E00..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FA1F)
}

fn is_cjk_punctuation(value: char) -> bool {
    matches!(value, '。' | '，' | '、' | '；' | '：' | '？' | '！' | '“' | '”' | '‘' | '’' | '（' | '）' | '《' | '》' | '【' | '】' | '…' | '—' | '·')
}

/// 从 lopdf 的文档信息字典里取标题。
fn lopdf_title(doc: &lopdf::Document) -> Option<String> {
    let Ok(lopdf::Object::Reference(info_id)) = doc.trailer.get(b"Info") else { return None };
    let Ok(lopdf::Object::Dictionary(info)) = doc.get_object(*info_id) else { return None };
    let Ok(lopdf::Object::String(raw, _)) = info.get(b"Title") else { return None };
    let text = decode_pdf_string(raw);
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// 主解析路径：用成熟库按页取文本。
///
/// lopdf 会正确处理 xref / xref 流（PDF 1.5+ 对象流）、FlateDecode、
/// 以及嵌入字体的 `ToUnicode` 映射 —— 这些正是手写解析器搞不定、
/// 导致真实中文 PDF 变成乱码的部分。
fn parse_with_primary_engine(bytes: &[u8]) -> Result<ParsedSource, AppError> {
    let doc = lopdf::Document::load_mem(bytes)
        .map_err(|error| AppError::Message(format!("PDF 结构无法解析，可能已损坏：{error}")))?;
    if doc.is_encrypted() {
        return Err(AppError::Message(ENCRYPTED_MESSAGE.into()));
    }
    let pages = doc.get_pages();
    let page_count = pages.len();
    if pages.is_empty() {
        return Err(AppError::Message("PDF 里没有找到任何页面，文件可能已损坏".into()));
    }

    let mut parsed = ParsedSource {
        title: lopdf_title(&doc).unwrap_or_default(),
        author: None,
        origin: None,
        kind: Some(SourceKind::Pdf),
        cover: None,
        cover_extension: None,
        page_count: page_count as u32,
        chunks: Vec::new(),
        warnings: Vec::new(),
    };

    let mut position = 0usize;
    let mut pages_with_text = 0usize;
    let mut garbled_pages = 0usize;
    let mut failed_pages = 0usize;

    for page_number in pages.keys() {
        // 带上限的版本：内容流解压后超过 32 MB 会被拒绝，防解压炸弹
        let extracted = doc.extract_text_with_limit(&[*page_number], MAX_STREAM_BYTES);
        let text = match extracted {
            Ok(text) => clean_extracted_text(&text),
            Err(_) => {
                failed_pages += 1;
                continue;
            }
        };
        let text: String = text.chars().take(MAX_PAGE_CHARS).collect();
        if text.trim().is_empty() {
            continue;
        }
        // 提取到了字符但读不出来 —— 不要静默入库
        if assess_text_quality(&text).is_garbled() {
            garbled_pages += 1;
            continue;
        }
        pages_with_text += 1;
        let heading = format!("第 {} 页", page_number);
        let locator = Locator { page: Some(*page_number), chapter: None, heading: Some(heading.clone()) };
        let chunks = chunker::split_into_chunks(&heading, &text, position, &locator);
        position += chunks.len();
        parsed.chunks.extend(chunks);
        if position >= crate::import::MAX_DOCUMENTS_PER_SOURCE {
            parsed.add_warning("页数过多，只导入了前一部分");
            break;
        }
    }

    if parsed.chunks.is_empty() {
        if garbled_pages > 0 {
            return Err(AppError::Message(
                "这份 PDF 的字体编码无法识别，提取出的文字全是乱码；换一份带文字层的 PDF 试试。".into(),
            ));
        }
        if failed_pages == pages.len() {
            return Err(AppError::Message("PDF 的正文流无法读取，可能已损坏".into()));
        }
        return Err(AppError::Message(SCANNED_MESSAGE.into()));
    }
    if parsed.title.is_empty() {
        parsed.title = first_line_title(&parsed.chunks);
    }

    let without_text = pages.len() - pages_with_text;
    if without_text > 0 {
        parsed.add_warning(format!("共 {} 页，其中 {without_text} 页没有可用文字层（可能是图片页）", pages.len()));
    }
    if garbled_pages > 0 {
        parsed.add_warning(format!("有 {garbled_pages} 页的字体编码无法完整识别，这些页已跳过"));
    }
    if failed_pages > 0 {
        parsed.add_warning(format!("有 {failed_pages} 页的正文流无法解压，已跳过"));
    }
    Ok(parsed)
}

/// 兜底路径：原有���手写顺序扫描解析器。
///
/// 只在主解析器整个失败时才用：它不认识对象流，命中率低，
/// 但对极简单、结构规整的文件仍然有效，留着比删掉强。
fn parse_with_legacy_fallback(bytes: &[u8]) -> Result<ParsedSource, AppError> {
    let objects = scan_objects(bytes);
    if objects.is_empty() {
        return Err(AppError::Message("PDF 结构无法解析，可能已损坏".into()));
    }
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
    let mut position = 0usize;
    let mut pages_with_text = 0usize;
    for number in &page_numbers {
        let Some(body) = objects.get(number) else { continue };
        let Some(content) = collect_page_content(&objects, body) else { continue };
        let text: String = extract_text_from_content(&content).chars().take(MAX_PAGE_CHARS).collect();
        if text.trim().is_empty() {
            continue;
        }
        pages_with_text += 1;
        let page_number = page_index_of(&page_numbers, *number);
        let heading = format!("第 {page_number} 页");
        let locator = Locator { page: Some(page_number), chapter: None, heading: Some(heading.clone()) };
        let chunks = chunker::split_into_chunks(&heading, &text, position, &locator);
        position += chunks.len();
        parsed.chunks.extend(chunks);
        if position >= crate::import::MAX_DOCUMENTS_PER_SOURCE {
            break;
        }
    }
    if parsed.chunks.is_empty() {
        return Err(AppError::Message(SCANNED_MESSAGE.into()));
    }
    if parsed.title.is_empty() {
        parsed.title = first_line_title(&parsed.chunks);
    }
    if pages_with_text < page_numbers.len() {
        parsed.add_warning(format!("共 {} 页，其中 {} 页没有文字层（可能是图片页）", page_numbers.len(), page_numbers.len() - pages_with_text));
    }
    parsed.add_warning("这份 PDF 用的是兼容解析路径，部分页面可能不完整");
    Ok(parsed)
}

fn page_index_of(page_numbers: &[u32], number: u32) -> u32 {
    page_numbers.iter().position(|value| *value == number).map(|index| index as u32 + 1).unwrap_or(1)
}

/// 体积闸门。抽成函数是为了能直接测「超限被拒」，而不必真的造一个 200 MB 文件。
pub fn check_size(len: u64) -> Result<(), AppError> {
    if len > MAX_PDF_BYTES {
        return Err(AppError::Message(format!(
            "PDF 体积超过 {} MB 的导入上限",
            MAX_PDF_BYTES / 1024 / 1024
        )));
    }
    Ok(())
}

/// 解析 PDF 并按页生成文档块。
pub fn parse(bytes: Vec<u8>) -> Result<ParsedSource, AppError> {
    check_size(bytes.len() as u64)?;
    crate::import::check_file_signature(SourceKind::Pdf, &bytes)?;

    // 主路径优先；只有主路径失败才退回手写解析器。
    // 两条路都失败时以主路径的错误为准：它更能说明真实原因（加密 / 扫描版 / 损坏）。
    match parse_with_primary_engine(&bytes) {
        Ok(parsed) => Ok(parsed),
        Err(primary) => match parse_with_legacy_fallback(&bytes) {
            Ok(parsed) => Ok(parsed),
            Err(_) => Err(primary),
        },
    }
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
///
/// 页眉页脚会被过滤掉：纯页码、「第 N 页」这类行当书名毫无意义，
/// 而且不同文件抽到的行数还不一样，会让去重 hash 抖动。
fn first_line_title(chunks: &[crate::import::DocumentChunk]) -> String {
    let is_noise = |line: &str| {
        let compact: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        if compact.is_empty() || compact.chars().all(|c| c.is_ascii_digit()) {
            return true;
        }
        // 「第 12 页」「第 3 页 - 书名」这类页眉页脚不能当书名
        compact.starts_with('第') && compact.ends_with('页') && compact.chars().skip(1).all(|c| c.is_ascii_digit() || c == '页')
    };
    chunks
        .first()
        .and_then(|chunk| chunk.content.lines().map(str::trim).find(|line| !is_noise(line)))
        .map(|line| line.chars().take(60).collect::<String>())
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
        let pdf = build_pdf("1.4", &["q Q Q Q"], None, false);
        let error = parse(pdf).unwrap_err().to_string();
        assert!(error.contains("暂不支持 OCR"), "实际：{error}");
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

    #[test]
    fn 页眉页脚不当书名() {
        let chunks = vec![crate::import::DocumentChunk {
            position: 0,
            heading: "第 3 页".into(),
            // 页码行先出现，真正的书名在后面
            content: "12\n第 3 页\n深入理解计算机系统\n正文内容".into(),
            locator: Locator::default(),
        }];
        assert_eq!(first_line_title(&chunks), "深入理解计算机系统");
    }

    #[test]
    fn 只有页码也没有标题时给出兜底名() {
        let chunks = vec![crate::import::DocumentChunk {
            position: 0,
            heading: "第 1 页".into(),
            content: "1\n2\n3".into(),
            locator: Locator::default(),
        }];
        assert_eq!(first_line_title(&chunks), "未命名 PDF");
    }

    // ---------- 文本质量 ----------

    #[test]
    fn 正常文本不算乱码() {
        let text = "The quick brown fox jumps over the lazy dog. 中文段落也应该是可读的。";
        assert!(!assess_text_quality(text).is_garbled());
    }

    #[test]
    fn 替换字符过多判为乱码() {
        let text = "\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}正常字";
        assert!(assess_text_quality(text).is_garbled());
    }

    #[test]
    fn 控制字符过多判为乱码() {
        let text = "\u{0001}\u{0002}\u{0003}\u{0004}\u{0005}\u{0006}\u{0007}文字";
        assert!(assess_text_quality(text).is_garbled());
    }

    #[test]
    fn 清理中文之间的排版空格() {
        // lopdf 逐个算子拼接，中文之间常留下对齐用的空格
        assert_eq!(clean_extracted_text("读 书 的 艺 术"), "读书的艺术");
        assert_eq!(clean_extracted_text("他说 ： 你 好"), "他说：你好");
        // 英文单词之间的空格必须保留
        assert_eq!(clean_extracted_text("hello   world"), "hello world");
    }

    #[test]
    fn 清理不会删掉中文标点() {
        assert_eq!(clean_extracted_text("第一句。第二句！第三句？"), "第一句。第二句！第三句？");
    }

    // ---------- 真实样本 ----------

    /// 用 lopdf 自己写出 PDF 样本。
    ///
    /// 手写 xref 偏移量很容易错一位，而且那样造出来的文件根本不像真实 PDF。
    /// 直接用成熟的 writer，才算真的在测「导入器面对正常 PDF 的表现」。
    fn build_pdf(version: &str, pages: &[&str], title: Option<&str>, modern: bool) -> Vec<u8> {
        use lopdf::{dictionary, Document, Object, Stream};

        let mut doc = Document::with_version(version);
        let font = doc.add_object(dictionary! {
            "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica",
        });
        let pages_id = doc.new_object_id();
        let mut kids: Vec<Object> = Vec::new();
        for content in pages {
            let stream = doc.add_object(Stream::new(dictionary! {}, content.as_bytes().to_vec()));
            let page = doc.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => stream,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
                "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
            });
            kids.push(page.into());
        }
        let count = kids.len() as i64;
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Count" => count, "Kids" => kids }));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        if let Some(value) = title {
            let info = doc.add_object(dictionary! { "Title" => Object::string_literal(value), "Producer" => Object::string_literal("test") });
            doc.trailer.set("Info", info);
        }
        let mut bytes = Vec::new();
        if modern {
            // 对象流 + 交叉引用流：PDF 1.5+ 的真实形态
            doc.save_modern(&mut bytes).expect("写出样本 PDF 失败");
        } else {
            doc.save_to(&mut bytes).expect("写出样本 PDF 失败");
        }
        bytes
    }

    /// 带 ToUnicode 映射的中文样本。
    ///
    /// 真实中文 PDF 的字符码是 CID，只有 ToUnicode CMap 能翻回汉字 ——
    /// 这正是手写解析器会出乱码、而成熟解析器能读对的地方。
    fn build_cjk_pdf(text: &str) -> Vec<u8> {
        use lopdf::{dictionary, Document, Object, Stream};

        let mut doc = Document::with_version("1.7");
        let chars: Vec<char> = text.chars().collect();
        let mut bfchar = String::new();
        for (index, character) in chars.iter().enumerate() {
            let code = index as u32 + 1;
            // ToUnicode 的目标值是 UTF-16BE 的码点序列
            let target: Vec<u16> = character.to_string().encode_utf16().collect();
            bfchar.push_str(&format!("<{code:04X}> <{}>\n", target.iter().map(|unit| format!("{unit:04X}")).collect::<String>()));
        }
        let cmap = format!(
            "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
             /CMapName /Custom def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n\
             {}beginbfchar\n{}endbfchar\nendcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n",
            chars.len(),
            bfchar
        );
        let to_unicode = doc.add_object(Stream::new(dictionary! {}, cmap.into_bytes()));
        let font = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type0",
            "BaseFont" => "NotoSerifCJK",
            "Encoding" => "Identity-H",
            "DescendantFonts" => vec![dictionary! {
                "Type" => "Font", "Subtype" => "CIDFontType2", "BaseFont" => "NotoSerifCJK",
                "CIDSystemInfo" => dictionary! { "Registry" => "Adobe", "Ordering" => "Identity", "Supplement" => 0 },
                "DW" => 1000,
            }.into()],
            "ToUnicode" => to_unicode,
        });
        let pages_id = doc.new_object_id();
        // 每个汉字一个两字节 CID，十六进制串交给 Tj
        let codes: String = (0..chars.len()).map(|index| format!("{:04X}", index + 1)).collect();
        let content = format!("BT /F1 12 Tf 72 720 Td <{codes}> Tj ET");
        let stream = doc.add_object(Stream::new(dictionary! {}, content.into_bytes()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => stream,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        });
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Count" => 1, "Kids" => vec![page.into()] }));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        let info = doc.add_object(dictionary! { "Title" => Object::string_literal("中文样本"), "Producer" => Object::string_literal("test") });
        doc.trailer.set("Info", info);
        let mut bytes = Vec::new();
        doc.save_modern(&mut bytes).expect("写出中文样本 PDF 失败");
        bytes
    }

    fn simple_page(text: &str) -> String {
        format!("BT /F1 12 Tf 72 720 Td ({text}) Tj ET")
    }

    /// 同一页里的两行文字，用 `Td` 真正换行。
    fn two_line_page(first: &str, second: &str) -> String {
        format!("BT /F1 12 Tf 72 720 Td ({first}) Tj 0 -14 Td ({second}) Tj ET")
    }

    /// 一份被加密字典标记过的 PDF。
    ///
    /// 只写入 `/Encrypt` 引用就足以让读取方判定「这是加密文档」，
    /// 样本不需要真的做 RC4/AES 加密 —— 导入器根本不该尝试解密。
    fn build_encrypted_pdf() -> Vec<u8> {
        use lopdf::{dictionary, Document, Object, Stream, StringFormat};

        let mut doc = Document::with_version("1.7");
        let encrypt = doc.add_object(dictionary! {
            "Filter" => "Standard", "V" => 2, "R" => 3, "Length" => 128,
            "O" => Object::String(vec![0u8; 32], StringFormat::Hexadecimal),
            "U" => Object::String(vec![0u8; 32], StringFormat::Hexadecimal),
            "P" => -44,
        });
        let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica" });
        let pages_id = doc.new_object_id();
        let stream = doc.add_object(Stream::new(dictionary! {}, b"BT /F1 12 Tf 72 720 Td (Secret body text) Tj ET".to_vec()));
        let page = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => stream,
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } },
        });
        doc.objects.insert(pages_id, Object::Dictionary(dictionary! { "Type" => "Pages", "Count" => 1, "Kids" => vec![page.into()] }));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        doc.trailer.set("Encrypt", encrypt);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).expect("写出加密样本失败");
        bytes
    }

    #[test]
    fn __debug_dump() {
        let quoted = "BT /F1 12 Tf 72 720 Td (Quoted line) ' 0 -14 Td 1 2 (Second line) \" ET";
        let pdf = build_pdf("1.4", &[quoted], Some("Quoted"), false);
        let parsed = parse(pdf).unwrap();
        println!("QUOTE_DUMP>>>{:?}<<<", parsed.chunks[0].content);

        let kerned = "BT /F1 12 Tf 72 720 Td [(Kerned) -500 (Text)] TJ ET";
        let parsed = parse(build_pdf("1.4", &[kerned], Some("K"), false)).unwrap();
        println!("KERN_DUMP>>>{:?}<<<", parsed.chunks[0].content);

        let page = two_line_page("Readable Title Line", "rest of the body");
        let parsed = parse(build_pdf("1.4", &[&page], None, false)).unwrap();
        println!("TITLE_DUMP>>>{:?}<<<", parsed.chunks[0].content);

        let parsed = parse(build_cjk_pdf("深入理解计算机系统")).unwrap();
        let body: String = parsed.chunks.iter().map(|c| c.content.as_str()).collect();
        println!("CJK_DUMP>>>{body}<<<");
        panic!("debug");
    }

    #[test]
    fn 简单英文_pdf_按页导入且页码从一开始() {
        let first = simple_page("Page one body");
        let second = simple_page("Page two body");
        let pdf = build_pdf("1.4", &[&first, &second], Some("Sample Book"), false);
        let parsed = parse(pdf).unwrap();
        assert_eq!(parsed.title, "Sample Book");
        assert_eq!(parsed.page_count, 2);
        assert_eq!(parsed.chunks[0].locator.page, Some(1));
        assert!(parsed.chunks[0].content.contains("Page one body"));
        // 第二页的块必须带第 2 页，不能沿用第 1 页
        assert!(parsed.chunks.iter().any(|chunk| chunk.locator.page == Some(2)));
    }

    #[test]
    fn 多页_每页都带正确页码_locator() {
        let pages: Vec<String> = (1..=5).map(|index| simple_page(&format!("Body of page {index}"))).collect();
        let refs: Vec<&str> = pages.iter().map(String::as_str).collect();
        let parsed = parse(build_pdf("1.4", &refs, Some("Multi"), false)).unwrap();
        let mut seen: Vec<u32> = parsed.chunks.iter().filter_map(|chunk| chunk.locator.page).collect();
        seen.sort_unstable();
        seen.dedup();
        assert_eq!(seen, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn 引号与数组文本算子都能抽出来() {
        // `'` 与 `"` 都要配一个显式的换行/字距操作符，才是合法的写法
        let quoted = "BT /F1 12 Tf 72 720 Td (Quoted line) ' 0 -14 Td 1 2 (Second line) \" ET";
        let pdf = build_pdf("1.4", &[quoted], Some("Quoted"), false);
        let parsed = parse(pdf).unwrap();
        assert!(parsed.chunks[0].content.contains("Quoted line"), "实际：{}", parsed.chunks[0].content);

        let kerned = "BT [(Kerned) -500 (Text)] TJ ET";
        let pdf = build_pdf("1.4", &[kerned], Some("Kerned"), false);
        assert!(parse(pdf).unwrap().chunks[0].content.contains("Kerned"));
    }

    #[test]
    fn 部分页没有文字时给出警告且导入其余页() {
        let first = simple_page("Text page one");
        let third = simple_page("Text page three");
        let pdf = build_pdf("1.4", &[&first, "q Q 100 0 0 100 20 20 cm /Im0 Do Q", &third], Some("Partial"), false);
        let parsed = parse(pdf).unwrap();
        assert_eq!(parsed.page_count, 3);
        assert!(parsed.warnings.iter().any(|w| w.contains("没有可用文字层")), "实际：{:?}", parsed.warnings);
        assert!(parsed.chunks.iter().any(|chunk| chunk.locator.page == Some(3)));
    }

    #[test]
    fn 全扫描版_明确提示不支持_ocr() {
        let pdf = build_pdf("1.4", &["q Q 100 0 0 100 20 20 cm /Im0 Do Q", "q W 0 0 h Q"], Some("Scanned"), false);
        let error = parse(pdf).unwrap_err().to_string();
        assert!(error.contains("暂不支持 OCR"), "实际：{error}");
    }

    #[test]
    fn 加密_pdf_给出加密提示而不是当成扫描版() {
        let pdf = build_encrypted_pdf();
        let error = parse(pdf).unwrap_err().to_string();
        assert!(error.contains("加密"), "实际：{error}");
        assert!(!error.contains("扫描版"), "加密 PDF 不能被当成扫描版：{error}");
    }

    #[test]
    fn 损坏的_pdf_被拒绝() {
        let page = simple_page("Body");
        let mut pdf = build_pdf("1.4", &[&page], Some("Broken"), false);
        pdf.truncate(pdf.len() / 2);
        assert!(parse(pdf).is_err());
    }

    #[test]
    fn 没有元数据标题时用正文首行() {
        let page = two_line_page("Readable Title Line", "rest of the body");
        let pdf = build_pdf("1.4", &[&page], None, false);
        let parsed = parse(pdf).unwrap();
        assert_eq!(parsed.title, "Readable Title Line");
    }

    #[test]
    fn 带元数据标题时优先用元数据() {
        let page = simple_page("First line of body");
        let pdf = build_pdf("1.4", &[&page], Some("Metadata Title"), false);
        assert_eq!(parse(pdf).unwrap().title, "Metadata Title");
    }

    #[test]
    fn 非_pdf_内容不会因为扩展名被放行() {
        let error = parse(b"PK\x03\x04not-a-pdf".to_vec()).unwrap_err().to_string();
        assert!(error.contains("不是有效的 PDF"), "实际：{error}");
    }

    #[test]
    fn 超出体积上限在解析前就拒绝() {
        let error = check_size(MAX_PDF_BYTES + 1).unwrap_err().to_string();
        assert!(error.contains("体积超过"), "实际：{error}");
        assert!(check_size(MAX_PDF_BYTES).is_ok());
    }

    #[test]
    fn pdf_1_7_对象流与交叉引用流能解析() {
        // 对象流 + xref 流是 PDF 1.5+ 的默认形态：
        // 很多真实 PDF（包括不少中文扫描/排版工具的导出）都是这种结构。
        let first = simple_page("Object stream page one");
        let second = simple_page("Object stream page two");
        let pdf = build_pdf("1.7", &[&first, &second], Some("Modern Book"), true);
        let parsed = parse(pdf).unwrap();
        assert_eq!(parsed.title, "Modern Book");
        assert_eq!(parsed.page_count, 2);
        assert!(parsed.chunks[0].content.contains("Object stream page one"));
        assert!(parsed.chunks.iter().any(|chunk| chunk.locator.page == Some(2)));
    }

    #[test]
    fn 对象流_pdf_不走兼容解析路径() {
        let page = simple_page("Modern only body");
        let pdf = build_pdf("1.7", &[&page], Some("Modern"), true);
        let parsed = parse(pdf).unwrap();
        assert!(
            !parsed.warnings.iter().any(|w| w.contains("兼容解析路径")),
            "主解析器应当直接处理对象流，实际 warnings：{:?}",
            parsed.warnings
        );
    }

    #[test]
    fn 中文_tounicode_字体能正确提取() {
        // CID 码 + ToUnicode CMap：手写解析器会吐乱码，这正是换成熟库的意义
        let parsed = parse(build_cjk_pdf("深入理解计算机系统")).unwrap();
        let body: String = parsed.chunks.iter().map(|chunk| chunk.content.as_str()).collect();
        assert!(body.contains("深入理解计算机系统"), "实际：{body}");
        assert!(!body.contains('\u{FFFD}'), "不能出现替换字符");
    }
}
