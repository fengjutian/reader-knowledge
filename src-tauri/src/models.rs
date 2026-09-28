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
pub struct Book {
    pub id: String,
    pub title: String,
    pub author: String,
    pub cover: String,
    pub highlight_count: i64,
    pub thought_count: i64,
    pub progress: i64,
    pub updated_at: String,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookDetail {
    #[serde(flatten)]
    pub book: Book,
    pub category: String,
    pub deep_link: Option<String>,
    pub finished: bool,
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
pub struct AiRequest {
    pub question: String,
    pub mode: String,
    #[serde(default)]
    pub book_ids: Vec<String>,
}
