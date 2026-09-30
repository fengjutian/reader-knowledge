use crate::error::AppError;
use serde::de::DeserializeOwned;

pub const MAX_API_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_HTML_RESPONSE_BYTES: usize = 4 * 1024 * 1024;

async fn limited_bytes(
    mut response: reqwest::Response,
    limit: usize,
    label: &str,
) -> Result<Vec<u8>, AppError> {
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(AppError::Message(format!("{label}响应超过安全大小限制")));
    }

    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(AppError::Message(format!("{label}响应超过安全大小限制")));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

pub async fn limited_json<T: DeserializeOwned>(
    response: reqwest::Response,
    limit: usize,
    label: &str,
) -> Result<T, AppError> {
    let body = limited_bytes(response, limit, label).await?;
    serde_json::from_slice(&body).map_err(AppError::from)
}

pub async fn limited_text(
    response: reqwest::Response,
    limit: usize,
    label: &str,
) -> Result<String, AppError> {
    let body = limited_bytes(response, limit, label).await?;
    String::from_utf8(body).map_err(|_| AppError::Message(format!("{label}响应不是有效 UTF-8")))
}
