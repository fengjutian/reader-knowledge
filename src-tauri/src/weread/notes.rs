use super::{
    client::WeReadClient,
    models::{BookmarkListResponse, NotebookBook, NotebooksResponse, ReviewsResponse},
};
use crate::error::AppError;
use serde_json::{Map, Value};

impl WeReadClient {
    pub async fn all_notebooks(&self) -> Result<Vec<NotebookBook>, AppError> {
        let mut all = Vec::new();
        let mut last_sort: Option<i64> = None;
        loop {
            let mut params = Map::new();
            params.insert("count".into(), Value::from(100));
            if let Some(cursor) = last_sort {
                params.insert("lastSort".into(), Value::from(cursor));
            }
            let page: NotebooksResponse = self.call("/user/notebooks", params).await?;
            if page.has_more == 1 {
                last_sort = page.books.last().map(|book| book.sort);
                if last_sort.is_none() {
                    return Err(AppError::Message(
                        "微信读书笔记分页返回 hasMore=1，但缺少下一页游标".into(),
                    ));
                }
            }
            all.extend(page.books);
            if page.has_more != 1 {
                break;
            }
        }
        Ok(all)
    }

    pub async fn highlights(&self, book_id: &str) -> Result<BookmarkListResponse, AppError> {
        let mut params = Map::new();
        params.insert("bookId".into(), Value::String(book_id.into()));
        self.call("/book/bookmarklist", params).await
    }

    pub async fn all_thoughts(&self, book_id: &str) -> Result<Vec<Value>, AppError> {
        let mut all = Vec::new();
        let mut synckey = 0;
        loop {
            let mut params = Map::new();
            params.insert("bookid".into(), Value::String(book_id.into()));
            params.insert("synckey".into(), Value::from(synckey));
            params.insert("count".into(), Value::from(100));
            let page: ReviewsResponse = self.call("/review/list/mine", params).await?;
            all.extend(page.reviews);
            if page.has_more != 1 {
                break;
            }
            if page.synckey == synckey {
                return Err(AppError::Message(
                    "微信读书想法分页游标未推进，已安全中止同步".into(),
                ));
            }
            synckey = page.synckey;
        }
        Ok(all)
    }
}
