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
pub struct DatabaseOverview { pub size_bytes: u64, pub tables: Vec<DatabaseTableStat>, pub categories: Vec<DatabaseCategoryStat>, pub last_synced_at: Option<String> }
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
}
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
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    #[serde(flatten)]
    pub note: Note,
    pub score: f64,
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

#[derive(Serialize)]
pub struct Citation {
    pub index: usize,
    pub note: Note,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiAnswer {
    pub content: String,
    pub citations: Vec<Citation>,
    pub sources_considered: usize,
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

#[derive(Deserialize)]
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
