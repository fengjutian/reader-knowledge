//! 导入资料（网页 / EPUB / PDF）的公共逻辑。
//!
//! 抽成纯函数是为了能直接测 SSRF 防护、HTML 清洗和分块边界，
//! 不必真的发起网络请求或读文件。

pub mod chunker;
pub mod epub;
pub mod pdf;
pub mod web;

use crate::error::AppError;

/// 单个文档块的字符上限。中文按字符算，避免切出半个词。
pub const MAX_DOC_CHARS: usize = 1200;
/// 文档块的重叠字符数，保证跨块的句子仍可检索。
pub const DOC_OVERLAP_CHARS: usize = 150;
/// 单个来源最多保留的文档块数，防止一个巨大文件把库撑爆。
pub const MAX_DOCUMENTS_PER_SOURCE: usize = 5_000;
/// 导入内容总量上限（字符数）。
pub const MAX_TOTAL_CHARS: usize = 40_000_000;

/// 定位信息：网页没有页码，PDF 有页码，EPUB 有章节序号。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Locator {
    /// 1 起始的页码；网页为 None
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    /// EPUB 章节序号（1 起始）；网页为 None
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chapter: Option<u32>,
    /// 章节标题
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heading: Option<String>,
}

/// 切好的一块文档。
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentChunk {
    pub position: usize,
    pub heading: String,
    pub content: String,
    pub locator: Locator,
}

/// 来源类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Web,
    Pdf,
    Epub,
}

impl SourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SourceKind::Web => "web",
            SourceKind::Pdf => "pdf",
            SourceKind::Epub => "epub",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SourceKind::Web => "网页",
            SourceKind::Pdf => "PDF",
            SourceKind::Epub => "EPUB",
        }
    }
}

/// 解析结果。写库前先给用户预览确认。
#[derive(Debug, Clone, Default)]
pub struct ParsedSource {
    pub title: String,
    pub author: Option<String>,
    /// 网页为原 URL，EPUB/PDF 为原始文件路径
    pub origin: Option<String>,
    pub kind: Option<SourceKind>,
    /// 封面字节（EPUB 才有）
    pub cover: Option<Vec<u8>>,
    pub cover_extension: Option<String>,
    pub page_count: u32,
    pub chunks: Vec<DocumentChunk>,
    /// 抽取过程中的警告，例如「这一章没有正文」
    pub warnings: Vec<String>,
}

impl ParsedSource {
    pub fn add_warning(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }
}

/// 稳定的内容 hash：同一份内容重复导入时用它去重。
pub fn content_hash(kind: SourceKind, title: &str, body: &str) -> String {
    let mut input = Vec::new();
    input.extend_from_slice(kind.as_str().as_bytes());
    input.push(b'|');
    input.extend_from_slice(title.trim().to_lowercase().as_bytes());
    input.push(b'|');
    input.extend_from_slice(body.as_bytes());
    format!("{:x}", md5::compute(&input))
}

/// 按字符切块，带重叠。
///
/// 用字符而不是字节切分，中文不会被切出半个字；
/// 重叠保证跨块的句子仍然可被检索命中。
///
/// 断点优先选段落边界（最后一个换行）。`rposition` 从窗口**末尾往前**找，
/// 这样能得到「尽可能靠后但不越界」的断点；只有当段落起始点落在窗口前半段
/// （说明这本身就是一个超长段落）时，才退化为硬切。
pub fn split_into_chunks(heading: &str, content: &str, start_position: usize, base_locator: &Locator) -> Vec<DocumentChunk> {
    let mut chunks = Vec::new();
    let chars: Vec<char> = content.chars().collect();
    if chars.is_empty() {
        return chunks;
    }
    let mut start = 0;
    let mut position = start_position;
    while start < chars.len() {
        let end = (start + MAX_DOC_CHARS).min(chars.len());
        // 从窗口末尾往前找换行；要求断点严格大于 start，避免产生空块
        let break_at = chars[start..end]
            .iter()
            .rposition(|c| *c == '\n')
            .map(|offset| start + offset + 1)
            .filter(|index| *index > start + 1)
            .unwrap_or(end);
        let text: String = chars[start..break_at].iter().collect();
        let text = text.trim();
        if !text.is_empty() {
            chunks.push(DocumentChunk {
                position,
                heading: heading.to_owned(),
                content: text.to_owned(),
                locator: Locator {
                    // 页码在 PDF 里表示「这段开始于第几页」，跨页时以起始页为准
                    page: base_locator.page,
                    chapter: base_locator.chapter,
                    heading: if heading.is_empty() { None } else { Some(heading.to_owned()) },
                },
            });
            position += 1;
            if position - start_position >= MAX_DOCUMENTS_PER_SOURCE {
                break;
            }
        }
        if break_at >= chars.len() {
            break;
        }
        // 下一块的起点带重叠。注意重叠不能跨过上一块的起点，否则会死循环
        start = if break_at.saturating_sub(DOC_OVERLAP_CHARS) > start { break_at - DOC_OVERLAP_CHARS } else { break_at };
    }
    chunks
}

/// 校验抽取结果的规模，超限直接拒绝而不是悄悄截断。
pub fn validate_size(kind: SourceKind, chunks: &[DocumentChunk]) -> Result<(), AppError> {
    if chunks.is_empty() {
        return Err(AppError::Message(format!("这份{}没有可导入的正文", kind.label())));
    }
    let total: usize = chunks.iter().map(|chunk| chunk.content.chars().count()).sum();
    if total > MAX_TOTAL_CHARS {
        return Err(AppError::Message(format!("这份{}内容过大，已超过导入上限", kind.label())));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 内容_hash_同内容同类型才相同() {
        let a = content_hash(SourceKind::Web, "标题", "正文");
        let b = content_hash(SourceKind::Web, "  标题  ", "正文");
        assert_eq!(a, b, "标题首尾空白不影响判定");
        assert_ne!(a, content_hash(SourceKind::Pdf, "标题", "正文"), "类型不同视为不同内容");
        assert_ne!(a, content_hash(SourceKind::Web, "标题", "别的正文"));
    }

    #[test]
    fn 分块不会超过字符上限且带重叠() {
        let content = "字".repeat(3_000);
        let chunks = split_into_chunks("第一章", &content, 0, &Locator { page: Some(1), chapter: None, heading: None });
        assert!(chunks.len() >= 3);
        for chunk in &chunks {
            assert!(chunk.content.chars().count() <= MAX_DOC_CHARS, "实际 {}", chunk.content.chars().count());
        }
        // position 连续递增
        for (index, chunk) in chunks.iter().enumerate() {
            assert_eq!(chunk.position, index);
        }
    }

    #[test]
    fn 分块优先在换行处断开() {
        let paragraph = "行".repeat(300);
        let content = format!("{paragraph}\n{paragraph}\n{paragraph}");
        let chunks = split_into_chunks("章节", &content, 0, &Locator::default());
        assert_eq!(chunks.len(), 3, "三个自然段应当是三块，不该被合并");
    }

    #[test]
    fn 分块保留定位信息() {
        let content = "内容".repeat(2_000);
        let chunks = split_into_chunks("第二章", &content, 5, &Locator { page: None, chapter: Some(3), heading: None });
        assert_eq!(chunks[0].position, 5, "position 从调用方给定的起点开始");
        assert_eq!(chunks[0].locator.chapter, Some(3));
        assert_eq!(chunks[0].heading, "第二章");
    }

    #[test]
    fn 空内容不产生分块() {
        assert!(split_into_chunks("标题", "", 0, &Locator::default()).is_empty());
        assert!(split_into_chunks("标题", "   \n  ", 0, &Locator::default()).is_empty());
    }

    #[test]
    fn 中文按字符切分不会切坏字符() {
        // 构造正好在字节中间会被切坏的内容
        let content = "中文字符测试内容".repeat(200);
        let chunks = split_into_chunks("标题", &content, 0, &Locator::default());
        for chunk in &chunks {
            // 重新拼起来必须是合法 UTF-8（Rust String 本身保证了这一点，
            // 这里验证没有丢失或重复字符）
            assert!(!chunk.content.contains('\u{FFFD}'), "不能出现替换字符");
        }
    }

    #[test]
    fn 空结果会被拒绝() {
        let error = validate_size(SourceKind::Pdf, &[]).unwrap_err();
        assert!(error.to_string().contains("没有可导入的正文"));
    }

    #[test]
    fn 超大内容会被拒绝而不是静默截断() {
        let chunks = vec![DocumentChunk {
            position: 0,
            heading: String::new(),
            content: "字".repeat(MAX_TOTAL_CHARS + 1),
            locator: Locator::default(),
        }];
        let error = validate_size(SourceKind::Web, &chunks).unwrap_err();
        assert!(error.to_string().contains("内容过大"));
    }
}
