use serde::Serialize;
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("database error: {0}")] Database(#[from] rusqlite::Error),
    #[error("credential error: {0}")] Credential(#[from] keyring::Error),
    #[error("network error: {0}")] Network(#[from] reqwest::Error),
    #[error("{0}")] Message(String),
}
impl Serialize for AppError { fn serialize<S>(&self, serializer:S)->Result<S::Ok,S::Error> where S:serde::Serializer { serializer.serialize_str(&self.to_string()) } }
