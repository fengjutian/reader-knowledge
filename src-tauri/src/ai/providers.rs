use super::provider::{guard_stream, looks_like_no_stream_support, AiProvider, ProviderFuture, StreamStop};
use crate::ai::sse::{SseParser, MAX_STREAM_BYTES};
use crate::http::{limited_json, MAX_API_RESPONSE_BYTES};
use crate::models::ChatMessage;
use std::sync::atomic::AtomicBool;
use std::time::Instant;
pub struct OpenAiCompatibleProvider {
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
    pub http: reqwest::Client,
}

/// 一次流式调用的结果。取消不是错误，单独用 `Cancelled` 表达，
/// 这样调用方不会把用户主动停止误报成失败。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamOutcome {
    Completed(String),
    Cancelled,
}

impl AiProvider for OpenAiCompatibleProvider {
    fn chat<'a>(&'a self, messages: &'a [ChatMessage]) -> ProviderFuture<'a> {
        Box::pin(async move {
            let body = serde_json::json!({"model":self.model,"messages":messages});
            let mut last_status = None;
            let mut value = None;
            for attempt in 0..3 {
                let response = self.http.post(&self.endpoint).bearer_auth(&self.api_key).json(&body).send().await?;
                let status = response.status();
                if status.is_success() {
                    value = Some(limited_json::<serde_json::Value>(response, MAX_API_RESPONSE_BYTES, "AI").await?);
                    break;
                }
                last_status = Some(status);
                if !(status.as_u16() == 429 || status.is_server_error()) || attempt == 2 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(600 * (attempt + 1) as u64)).await;
            }
            let value = match value {
                Some(value) => value,
                None => {
                    let status = last_status.expect("AI request must have a response status");
                    return Err(status_error(status.as_u16()));
                }
            };
            value["choices"][0]["message"]["content"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| crate::error::AppError::Message("AI 响应格式无效".into()))
        })
    }
}

impl OpenAiCompatibleProvider {
    /// 流式对话。`on_delta` 每收到一段增量正文调用一次。
    ///
    /// - 429 / 5xx 会在**还没有吐出任何增量**时有限重试；一旦开始输出就不再重试，
    ///   避免把重复内容拼接到同一条回答里。
    /// - 取消标记在每个分片之间检查，用户点「停止生成」后最多再读一个分片。
    pub async fn chat_stream<F>(
        &self,
        messages: &[ChatMessage],
        cancelled: &AtomicBool,
        mut on_delta: F,
    ) -> Result<(StreamOutcome, Option<String>), crate::error::AppError>
    where
        F: FnMut(&str),
    {
        let body = serde_json::json!({"model": self.model, "messages": messages, "stream": true});
        let mut response = None;
        let mut last_status = None;
        let mut fallback_reason = None;
        for attempt in 0..3 {
            let sent = self.http.post(&self.endpoint).bearer_auth(&self.api_key).json(&body).send().await?;
            let status = sent.status();
            if status.is_success() {
                response = Some(sent);
                break;
            }
            last_status = Some(status.as_u16());
            // 只有明确是「不支持流式」才回退，其余状态沿用既有错误文案。
            let text = crate::http::limited_text(sent, MAX_API_RESPONSE_BYTES, "AI").await.unwrap_or_default();
            if looks_like_no_stream_support(status.as_u16(), &text) {
                fallback_reason = Some("当前服务不支持流式输出，已自动改用一次性请求".to_owned());
                break;
            }
            if !(status.as_u16() == 429 || status.is_server_error()) || attempt == 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(600 * (attempt + 1) as u64)).await;
        }

        let Some(mut response) = response else {
            if let Some(reason) = fallback_reason {
                // 服务不支持流式时安全回退：整体作为一段增量交给前端。
                let content = AiProvider::chat(self, messages).await?;
                on_delta(&content);
                return Ok((StreamOutcome::Completed(content), Some(reason)));
            }
            let status = last_status.expect("AI stream request must have a response status");
            return Err(status_error(status));
        };

        let mut parser = SseParser::with_limit(MAX_STREAM_BYTES);
        let mut content = String::new();
        let started = Instant::now();
        loop {
            let chunk = match response.chunk().await? {
                Some(chunk) => chunk,
                None => break,
            };
            if let Err(stop) = guard_stream(cancelled, started.elapsed(), parser.consumed(), chunk.len()) {
                return match stop {
                    StreamStop::Cancelled => Ok((StreamOutcome::Cancelled, None)),
                    other => Err(crate::error::AppError::Message(other.message().into())),
                };
            }
            for event in parser.push(&chunk)? {
                match event {
                    super::sse::SseEvent::Done => return Ok((StreamOutcome::Completed(content), None)),
                    super::sse::SseEvent::Delta(delta) => {
                        if delta.is_empty() {
                            continue;
                        }
                        content.push_str(&delta);
                        on_delta(&delta);
                    }
                }
            }
        }
        for event in parser.finish()? {
            if let super::sse::SseEvent::Delta(delta) = event {
                if !delta.is_empty() {
                    content.push_str(&delta);
                    on_delta(&delta);
                }
            }
        }
        if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok((StreamOutcome::Cancelled, None));
        }
        Ok((StreamOutcome::Completed(content), None))
    }

    pub async fn embed(&self, endpoint: &str, model: &str, input: &[String]) -> Result<Vec<Vec<f32>>, crate::error::AppError> {
        let response = self.http.post(endpoint).bearer_auth(&self.api_key).json(&serde_json::json!({ "model": model, "input": input })).send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(crate::error::AppError::Message(format!("Embedding 服务请求失败（HTTP {}）", status.as_u16())));
        }
        let value: serde_json::Value = limited_json(response, MAX_API_RESPONSE_BYTES, "Embedding").await?;
        let data = value["data"].as_array().ok_or_else(|| crate::error::AppError::Message("Embedding 响应格式无效".into()))?;
        let mut ordered = data.iter().map(|item| {
            let index = item["index"].as_u64().unwrap_or(0) as usize;
            let vector = item["embedding"].as_array().ok_or_else(|| crate::error::AppError::Message("Embedding 向量格式无效".into()))?.iter().map(|value| value.as_f64().unwrap_or(0.0) as f32).collect::<Vec<_>>();
            Ok((index, vector))
        }).collect::<Result<Vec<_>, crate::error::AppError>>()?;
        ordered.sort_by_key(|item| item.0);
        if ordered.len() != input.len() { return Err(crate::error::AppError::Message("Embedding 返回数量与输入不一致".into())); }
        Ok(ordered.into_iter().map(|item| item.1).collect())
    }
}

fn status_error(status: u16) -> crate::error::AppError {
    let message = if status == 429 || (500..600).contains(&status) {
        format!("AI 服务暂时繁忙（HTTP {status}），已自动重试，请稍后再试")
    } else if status == 401 || status == 403 {
        "AI 服务认证失败，请到设置中检查 API Key".to_owned()
    } else {
        format!("AI 服务请求失败（HTTP {status}），请检查服务配置")
    };
    crate::error::AppError::Message(message)
}
