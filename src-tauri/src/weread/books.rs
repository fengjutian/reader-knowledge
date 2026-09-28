use super::{client::WeReadClient, models::ShelfResponse};
use crate::error::AppError;
use serde_json::Map;

impl WeReadClient {
    pub async fn shelf(&self) -> Result<ShelfResponse, AppError> {
        self.call("/shelf/sync", Map::new()).await
    }
}
