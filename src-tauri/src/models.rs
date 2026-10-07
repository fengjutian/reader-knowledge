use serde::{Deserialize, Serialize};
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DashboardStats {
    pub books: i64,
    pub highlights: i64,
    pub thoughts: i64,
    pub last_synced_at: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseTableStat { pub name: String, pub label: String, pub rows: i64 }
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseCategoryStat { pub label: String, pub count: i64 }
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataOverview {
    /// 已有微信读书元数据的书籍数
    pub weread: i64,
    /// 已有豆瓣元数据的书籍数
    pub douban: i64,
    /// 仍未补全任何元数据的在架书籍数
    pub missing: i64,
    /// 向量索引条数
    pub vectors: i64,
    /// 关系分析缓存条数
    pub relation_cache: i64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseOverview { pub size_bytes: u64, pub tables: Vec<DatabaseTableStat>, pub categories: Vec<DatabaseCategoryStat>, pub metadata: MetadataOverview, pub last_synced_at: Option<String> }
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseRow { pub id: String, pub primary: String, pub secondary: String, pub detail: String, pub created_at: String }
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatabaseRows { pub total: i64, pub rows: Vec<DatabaseRow> }
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Book {
    pub id: String,
    pub title: String,
    pub author: String,
    pub category: String,
    pub cover: String,
    pub highlight_count: i64,
    pub thought_count: i64,
    pub progress: i64,
    pub updated_at: String,
    pub reading_status: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookPage {
    pub total: i64,
    pub books: Vec<Book>,
    pub categories: Vec<String>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookDetail {
    #[serde(flatten)]
    pub book: Book,
    pub deep_link: Option<String>,
    pub finished: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookMetadataRow {
    pub book_id: String,
    pub title: String,
    pub author: String,
    pub cover: String,
    pub isbn: String,
    pub publisher: String,
    pub published_date: String,
    pub page_count: Option<i64>,
    pub subjects: Vec<String>,
    pub sources: Vec<String>,
    pub last_fetched_at: Option<String>,
    pub metadata_status: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataFetchResult {
    pub book_id: String,
    pub source: String,
    pub status: String,
    pub message: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookMetadataSourceDetail {
    pub source: String, pub source_id: String, pub source_url: Option<String>, pub title: String,
    pub authors: Vec<String>, pub isbn: String, pub publisher: String, pub published_date: String,
    pub page_count: Option<i64>, pub subjects: Vec<String>, pub cover_url: String,
    pub description: String, pub rating: Option<f64>, pub rating_count: Option<i64>, pub fetched_at: String,
    pub author_name: String, pub author_avatar: String, pub author_url: String,
    pub author_bio: String, pub table_of_contents: String,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryTerm { pub id:i64,pub term:String,pub canonical_name:String,pub aliases:Vec<String>,pub definition:String,pub source:String,pub source_title:String,pub source_url:String,pub wikipedia_snapshot:String,pub status:String,pub updated_at:i64 }
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WikipediaCandidate { pub title:String,pub description:String,pub excerpt:String,pub url:String }
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    #[serde(rename = "type")]
    pub note_type: String,
    pub book_id: String,
    pub book_title: String,
    pub chapter: String,
    pub content: String,
    pub created_at: String,
}
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    #[serde(flatten)]
    pub note: Note,
    pub score: f64,
}

pub const SEARCH_ENTITY_TYPES: [&str; 4] = ["book", "highlight", "thought", "source"];

/// 全局搜索结果。可区分实体的联合模型：书籍 / 划线 / 想法共用一个列表返回。
#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSearchResult {
    pub id: String,
    #[serde(rename = "type")]
    pub entity_type: String,
    pub book_id: String,
    pub title: String,
    /// 书籍结果是「作者 · 分类」，笔记结果是章节名。
    pub subtitle: String,
    /// 已经在后端截断的纯文本摘要，前端不直接渲染服务端 HTML。
    pub snippet: String,
    pub score: f64,
    pub updated_at: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSearchRequest {
    #[serde(default)]
    pub query: String,
    /// 空表示全部类型
    #[serde(default)]
    pub types: Vec<String>,
    #[serde(default)]
    pub book_id: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct GlobalSearchPage {
    pub results: Vec<GlobalSearchResult>,
    /// 是否还有下一页，前端据此决定是否显示「加载更多」
    pub has_more: bool,
}
#[derive(Serialize, Clone)]
pub struct SyncProgress {
    pub status: String,
    pub progress: i32,
    pub books: i64,
    pub highlights: i64,
    pub thoughts: i64,
    #[serde(rename = "processedBooks")]
    pub processed_books: usize,
    #[serde(rename = "totalBooks")]
    pub total_books: usize,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AiSettings {
    pub provider: String,
    pub endpoint: String,
    pub model: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct EmbeddingSettings {
    pub provider: String,
    pub endpoint: String,
    pub model: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticRelation {
    pub id: String,
    pub from: String,
    pub to: String,
    pub score: f64,
    pub keywords: Vec<String>,
    pub relation: String,
    pub evidence: Vec<SemanticEvidence>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticEvidence {
    pub book_id: String,
    pub note_id: String,
    pub text: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LocalModelStatus {
    pub installed: bool,
    pub size_bytes: u64,
    pub model: String,
}

#[derive(Serialize, Clone)]
pub struct Citation {
    pub index: usize,
    pub note: Note,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryCitation {
    pub index: usize,
    pub term: String,
    pub definition: String,
    pub source: String,
    pub source_url: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AiAnswer {
    pub content: String,
    pub citations: Vec<Citation>,
    pub glossary_citations: Vec<GlossaryCitation>,
    pub sources_considered: usize,
    /// 非阻断提示：rerank 降级、或服务不支持流式而回退时说明发生了什么。
    /// 回答本身是完整的，前端只需把它显示成一条提示。
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub rerank_note: String,
    /// 来自导入资料的引用。编号接在 `citations` 之后，与 prompt 中给出的编号一致。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub source_citations: Vec<SourceCitation>,
}

/// 指向导入资料某一段的引用。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SourceCitation {
    pub index: usize,
    pub source_id: String,
    pub source_type: String,
    pub title: String,
    pub locator: crate::import::Locator,
    pub quote: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiTurn {
    pub question: String,
    pub answer: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AiRequest {
    pub question: String,
    pub mode: String,
    #[serde(default)]
    pub book_ids: Vec<String>,
    #[serde(default)]
    pub history: Vec<AiTurn>,
}

// ---------- 概念级知识图谱 ----------

pub const ENTITY_KINDS: [&str; 3] = ["concept", "topic", "idea"];
pub const ENTITY_STATUSES: [&str; 3] = ["suggested", "confirmed", "hidden"];
pub const RELATION_KINDS: [&str; 7] = ["broader", "narrower", "related", "supports", "conflicts", "causes", "applies"];

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConceptEvidence {
    pub note_id: String,
    pub book_id: String,
    pub quote: String,
    pub confidence: f64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeEntity {
    pub id: String,
    pub kind: String,
    pub canonical_name: String,
    pub description: String,
    pub aliases: Vec<String>,
    pub status: String,
    pub evidence_count: i64,
    pub updated_at: String,
    pub evidence: Vec<ConceptEvidence>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeRelation {
    pub id: String,
    pub from_entity_id: String,
    pub to_entity_id: String,
    pub relation: String,
    pub summary: String,
    pub confidence: f64,
    pub evidence: Vec<ConceptEvidence>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConceptGraph {
    pub entities: Vec<KnowledgeEntity>,
    pub relations: Vec<KnowledgeRelation>,
    /// 是否因为节点过多而只返回了高置信度子图
    pub truncated: bool,
    pub total_entities: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConceptScanRequest {
    /// 只扫描这本书；为空表示全部书籍
    #[serde(default)]
    pub book_id: Option<String>,
    /// 只处理内容变化过的笔记（复用 hash 缓存）
    #[serde(default)]
    pub changed_only: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConceptScanResult {
    pub books_scanned: i64,
    pub books_failed: i64,
    pub notes_scanned: i64,
    pub notes_skipped: i64,
    pub entities_created: i64,
    pub relations_created: i64,
    pub rejected: i64,
    pub failures: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConceptGraphQuery {
    #[serde(default)]
    pub kinds: Vec<String>,
    #[serde(default)]
    pub book_id: Option<String>,
    #[serde(default)]
    pub min_confidence: Option<f64>,
    /// 只返回该中心实体的邻居
    #[serde(default)]
    pub center_id: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub include_hidden: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityCorrection {
    pub id: String,
    /// 新名称；为空表示只改状态
    #[serde(default)]
    pub canonical_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// 追加别名，已存在的会被忽略
    #[serde(default)]
    pub aliases: Vec<String>,
    /// suggested / confirmed / hidden
    #[serde(default)]
    pub status: Option<String>,
}

impl Default for ConceptGraphQuery {
    fn default() -> Self {
        Self { kinds: Vec::new(), book_id: None, min_confidence: None, center_id: None, limit: None, include_hidden: false }
    }
}

impl Default for EntityCorrection {
    fn default() -> Self {
        Self { id: String::new(), canonical_name: None, description: None, aliases: Vec::new(), status: None }
    }
}

// ---------- 导入资料 ----------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct LibrarySource {
    pub id: String,
    pub source_type: String,
    pub title: String,
    pub author: Option<String>,
    /// 网页是原 URL；PDF / EPUB 是原始文件路径
    pub origin: Option<String>,
    pub page_count: i64,
    /// 封面存在应用数据目录时给出的相对文件名
    pub cover: Option<String>,
    pub document_count: i64,
    pub imported_at: String,
    /// 软删除后仍在库里，只是不再出现在列表里
    pub deleted: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SourceDocumentItem {
    pub id: String,
    pub position: i64,
    pub heading: String,
    pub content: String,
    pub locator: crate::import::Locator,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDetail {
    #[serde(flatten)]
    pub source: LibrarySource,
    pub documents: Vec<SourceDocumentItem>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewWebRequest {
    pub url: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewFileRequest {
    /// 用户选中的本地文件路径
    pub path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPreview {
    pub source_type: String,
    pub title: String,
    pub author: Option<String>,
    pub origin: Option<String>,
    pub page_count: i64,
    /// 预览里的前几块，让用户确认内容抓对了
    pub sample: Vec<SourceDocumentItem>,
    pub document_count: i64,
    /// 抓取过程中的提醒，例如「这一章正文过短已跳过」
    pub warnings: Vec<String>,
    /// 同一内容已导入过时为 true，前端应提示而不是重复入库
    pub duplicate: bool,
    /// 重复时指向已有资料
    pub duplicate_of: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmImportRequest {
    pub source_type: String,
    /// 用户在预览里改过的标题，优先于抓取结果
    pub title: String,
    pub author: Option<String>,
    /// 预览时显示的来源，仅用于回显；真正入库的 origin 由抓取/读取结果决定
    #[serde(default)]
    pub origin: Option<String>,
    /// 预览时显示的页数，仅用于回显；真正入库的页数由解析结果决定
    #[serde(default)]
    #[allow(dead_code)]
    pub page_count: i64,
    /// 网页导入时重新抓取；本地文件导入时读这个路径
    pub url: Option<String>,
    pub path: Option<String>,
}

impl ConfirmImportRequest {
    /// 入库用的来源标识：本地文件优先用 path，网页用 url。
    pub fn origin_hint(&self) -> Option<String> {
        self.path
            .clone()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| self.url.clone().filter(|value| !value.trim().is_empty()))
            .or_else(|| self.origin.clone().filter(|value| !value.trim().is_empty()))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeEntitiesRequest {
    /// 被合并掉的实体（会消失）
    pub source_id: String,
    pub target_id: String,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RerankerSettings {
    pub provider: String,
    pub endpoint: String,
    pub model: String,
    pub top_n: i64,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RelationAnalysisRequest {
    pub left_book_id: String,
    pub right_book_id: String,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RelationEvidence {
    pub book_id: String,
    pub note_id: String,
    pub note_type: String,
    pub text: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RelationClaim {
    pub relation: String,
    pub summary: String,
    pub confidence: f64,
    pub evidence: Vec<RelationEvidence>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct RelationAnalysis {
    pub relation: String,
    pub concepts: Vec<String>,
    pub summary: String,
    pub confidence: f64,
    pub claims: Vec<RelationClaim>,
    pub cached: bool,
}
