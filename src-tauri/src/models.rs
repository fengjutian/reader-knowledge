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
#[derive(Serialize)]
pub struct SyncProgress {
    pub status: String,
    pub progress: i32,
    pub books: i64,
    pub highlights: i64,
    pub thoughts: i64,
}
#[derive(Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}
