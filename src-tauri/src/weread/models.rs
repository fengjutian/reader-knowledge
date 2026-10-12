use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfResponse {
    #[serde(default)]
    pub books: Vec<ShelfBook>,
    #[serde(default)]
    pub albums: Vec<serde_json::Value>,
    pub mp: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShelfBook {
    pub book_id: String,
    pub title: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub cover: String,
    pub category: Option<String>,
    pub reader_link: Option<String>,
    pub deep_link: Option<String>,
    pub read_update_time: Option<i64>,
    #[serde(default)]
    pub finish_reading: i64,
    pub update_time: Option<i64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebooksResponse {
    #[serde(default)]
    pub books: Vec<NotebookBook>,
    #[serde(default)]
    pub has_more: i64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotebookBook {
    pub book_id: String,
    pub book: Option<serde_json::Value>,
    #[serde(default)]
    pub review_count: i64,
    #[serde(default)]
    pub note_count: i64,
    #[serde(default)]
    pub bookmark_count: i64,
    pub sort: i64,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct BookmarkListResponse {
    #[serde(default)]
    pub updated: Vec<serde_json::Value>,
    #[serde(default)]
    pub chapters: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReviewsResponse {
    #[serde(default)]
    pub reviews: Vec<serde_json::Value>,
    #[serde(default)]
    pub has_more: i64,
    #[serde(default)]
    pub synckey: i64,
}

#[cfg(test)]
mod tests {
    use super::ShelfResponse;

    #[test]
    fn shelf_book_deserializes_reader_link() {
        let response: ShelfResponse = serde_json::from_value(serde_json::json!({
            "books": [{
                "bookId": "book-1",
                "title": "测试书籍",
                "readerLink": "https://weread.qq.com/web/reader/book-1",
                "deepLink": "weread://bookDetail/book-1"
            }]
        }))
        .expect("shelf response should deserialize");

        let book = &response.books[0];
        assert_eq!(
            book.reader_link.as_deref(),
            Some("https://weread.qq.com/web/reader/book-1")
        );
        assert_eq!(book.deep_link.as_deref(), Some("weread://bookDetail/book-1"));
    }
}
