use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfResponse {
    #[serde(default)]
    pub books: Vec<ShelfBook>,
    #[serde(default)]
    pub albums: Vec<serde_json::Value>,
    pub mp: Option<serde_json::Value>,
}
impl ShelfResponse {
    pub fn visible_entry_count(&self) -> usize {
        self.books.len() + self.albums.len() + usize::from(self.mp.is_some())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfBook {
    pub book_id: String,
    pub title: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub cover: String,
    pub category: Option<String>,
    pub deep_link: Option<String>,
    pub read_update_time: Option<i64>,
    #[serde(default)]
    pub finish_reading: i64,
    pub update_time: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebooksResponse {
    #[serde(default)]
    pub books: Vec<NotebookBook>,
    #[serde(default)]
    pub has_more: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookBook {
    pub book_id: String,
    #[serde(default)]
    pub review_count: i64,
    #[serde(default)]
    pub note_count: i64,
    #[serde(default)]
    pub bookmark_count: i64,
    pub sort: i64,
}
impl NotebookBook {
    pub fn total_note_count(&self) -> i64 {
        self.review_count + self.note_count + self.bookmark_count
    }
}

#[derive(Debug, Deserialize)]
pub struct BookmarkListResponse {
    #[serde(default)]
    pub updated: Vec<serde_json::Value>,
    #[serde(default)]
    pub chapters: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewsResponse {
    #[serde(default)]
    pub reviews: Vec<serde_json::Value>,
    #[serde(default)]
    pub has_more: i64,
    #[serde(default)]
    pub synckey: i64,
}
