use crate::{error::AppError, models::ChatMessage};
use serde::Serialize;
use std::{future::Future, pin::Pin, sync::atomic::{AtomicBool, Ordering}, time::Duration};
pub type ProviderFuture<'a> = Pin<Box<dyn Future<Output = Result<String, AppError>> + Send + 'a>>;
pub trait AiProvider: Send + Sync {
    fn chat<'a>(&'a self, messages: &'a [ChatMessage]) -> ProviderFuture<'a>;
}

/// 单次流式请求的整体时长上限。
pub const MAX_STREAM_DURATION: Duration = Duration::from_secs(300);
/// 单次流式请求允许读取的响应体总量上限。
pub const MAX_STREAM_RESPONSE_BYTES: usize = 16 * 1024 * 1024;

/// 前端订阅的事件名。所有 payload 都带 `requestId`，避免串流污染其他会话。
pub const AI_STREAM_EVENT: &str = "ai-stream";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamStop {
    /// 用户主动取消
    Cancelled,
    /// 超过整体时长上限
    TimedOut,
    /// 超过响应体大小上限
    TooLarge,
}

/// 流式读取过程中每收到一个分片就调用一次，决定是否继续读取。
///
/// 拆成纯函数是为了能直接测超时 / 超限 / 取消三个分支，不必真的建立连接。
pub fn guard_stream(
    cancelled: &AtomicBool,
    elapsed: Duration,
    consumed: usize,
    chunk: usize,
) -> Result<(), StreamStop> {
    if cancelled.load(Ordering::Relaxed) {
        return Err(StreamStop::Cancelled);
    }
    if elapsed >= MAX_STREAM_DURATION {
        return Err(StreamStop::TimedOut);
    }
    if consumed.saturating_add(chunk) > MAX_STREAM_RESPONSE_BYTES {
        return Err(StreamStop::TooLarge);
    }
    Ok(())
}

impl StreamStop {
    pub fn message(self) -> &'static str {
        match self {
            StreamStop::Cancelled => "生成已取消",
            StreamStop::TimedOut => "AI 响应超时，请稍后重试",
            StreamStop::TooLarge => "AI 响应超过安全大小限制",
        }
    }
}

/// 发给前端的流式事件，序列化成 `src/types/domain.ts` 里的 `AiStreamEvent`。
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AiStreamEvent {
    Started {
        request_id: String,
        /// 服务不支持流式、已回退到非流式请求时给出的说明
        notice: Option<String>,
    },
    Delta {
        request_id: String,
        content: String,
    },
    Completed {
        request_id: String,
        answer: crate::models::AiAnswer,
    },
    Failed {
        request_id: String,
        message: String,
    },
    Cancelled {
        request_id: String,
    },
}

impl AiStreamEvent {
    pub fn request_id(&self) -> &str {
        match self {
            AiStreamEvent::Started { request_id, .. }
            | AiStreamEvent::Delta { request_id, .. }
            | AiStreamEvent::Completed { request_id, .. }
            | AiStreamEvent::Failed { request_id, .. }
            | AiStreamEvent::Cancelled { request_id } => request_id,
        }
    }

    pub fn started(request_id: &str) -> Self {
        AiStreamEvent::Started { request_id: request_id.to_owned(), notice: None }
    }

    pub fn delta(request_id: &str, content: impl Into<String>) -> Self {
        AiStreamEvent::Delta { request_id: request_id.to_owned(), content: content.into() }
    }

    pub fn completed(request_id: &str, answer: crate::models::AiAnswer) -> Self {
        AiStreamEvent::Completed { request_id: request_id.to_owned(), answer }
    }

    pub fn failed(request_id: &str, message: impl Into<String>) -> Self {
        AiStreamEvent::Failed { request_id: request_id.to_owned(), message: message.into() }
    }

    pub fn cancelled(request_id: &str) -> Self {
        AiStreamEvent::Cancelled { request_id: request_id.to_owned() }
    }

    /// 服务明确不支持流式时的回退提示
    pub fn with_notice(mut self, notice: impl Into<String>) -> Self {
        if let AiStreamEvent::Started { notice: slot, .. } = &mut self {
            *slot = Some(notice.into());
        }
        self
    }
}

/// 是否说明「这个 endpoint 不支持流式」。
///
/// 只在 400 / 404 / 422 这类请求本身有问题的响应上判定，并且要求错误信息里
/// 明确提到 stream，避免把普通的参数错误误判成不支持流式。
pub fn looks_like_no_stream_support(status: u16, body: &str) -> bool {
    if !matches!(status, 400 | 404 | 422) {
        return false;
    }
    let body = body.to_lowercase();
    body.contains("stream") && (body.contains("not support") || body.contains("unsupported") || body.contains("不支持"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 取消标记会立刻停止读取() {
        let cancelled = AtomicBool::new(true);
        assert_eq!(
            guard_stream(&cancelled, Duration::from_secs(1), 0, 16),
            Err(StreamStop::Cancelled)
        );
    }

    #[test]
    fn 超过时长上限停止读取() {
        let running = AtomicBool::new(false);
        assert_eq!(
            guard_stream(&running, MAX_STREAM_DURATION, 0, 16),
            Err(StreamStop::TimedOut)
        );
        assert!(guard_stream(&running, MAX_STREAM_DURATION - Duration::from_secs(1), 0, 16).is_ok());
    }

    #[test]
    fn 超过响应体上限停止读取() {
        let running = AtomicBool::new(false);
        assert_eq!(
            guard_stream(&running, Duration::from_secs(1), MAX_STREAM_RESPONSE_BYTES, 1),
            Err(StreamStop::TooLarge)
        );
        assert!(guard_stream(&running, Duration::from_secs(1), 1024, 16).is_ok());
    }

    #[test]
    fn 识别不支持流式的错误() {
        assert!(looks_like_no_stream_support(400, "{\"error\":{\"message\":\"stream is not supported\"}}"));
        assert!(looks_like_no_stream_support(422, "Stream 未开启，不支持"));
        // 普通错误不能被误判成不支持流式
        assert!(!looks_like_no_stream_support(400, "{\"error\":{\"message\":\"invalid api key\"}}"));
        assert!(!looks_like_no_stream_support(401, "stream is not supported"));
        assert!(!looks_like_no_stream_support(500, "stream is not supported"));
    }

    #[test]
    fn 事件统一带请求标识并序列化成前端类型() {
        let event = AiStreamEvent::delta("req-1", "你好");
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "delta");
        assert_eq!(json["requestId"], "req-1");
        assert_eq!(json["content"], "你好");

        let started = serde_json::to_value(AiStreamEvent::started("req-2").with_notice("已回退到非流式")).unwrap();
        assert_eq!(started["type"], "started");
        assert_eq!(started["notice"], "已回退到非流式");
        assert_eq!(started["requestId"], "req-2");
    }

    #[test]
    fn 各事件都能取回请求标识() {
        assert_eq!(AiStreamEvent::started("a").request_id(), "a");
        assert_eq!(AiStreamEvent::delta("b", "x").request_id(), "b");
        assert_eq!(AiStreamEvent::failed("c", "boom").request_id(), "c");
        assert_eq!(AiStreamEvent::cancelled("d").request_id(), "d");
    }
}
