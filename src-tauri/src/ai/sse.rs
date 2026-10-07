//! 增量 SSE 解析器。
//!
//! 网络分片与 SSE 事件边界没有任何对应关系：一条事件可能跨多个 chunk，
//! 一个 chunk 也可能包含多条事件，中文等多字节字符还可能被拆在两个 chunk 之间。
//! 这里始终以字节缓冲，只有拿到完整的行（以 `\n` 结尾）才做 UTF-8 解码；
//! 多字节字符不可能包含 `\n`，所以按 `\n` 切分天然不会切坏 UTF-8 序列。

use crate::error::AppError;

/// 单个未完成事件的缓冲上限，防止对端发送永不结束的超长行。
pub const MAX_SSE_EVENT_BYTES: usize = 256 * 1024;
/// 单次流式请求允许读取的响应体总量上限。
pub const MAX_STREAM_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SseEvent {
    /// `choices[0].delta.content` 中的增量正文
    Delta(String),
    /// `data: [DONE]`
    Done,
}

#[derive(Debug)]
pub struct SseParser {
    buffer: Vec<u8>,
    data: String,
    total: usize,
    max_total: usize,
    done: bool,
}

impl SseParser {
    pub fn new() -> Self {
        Self::with_limit(MAX_STREAM_BYTES)
    }

    pub fn with_limit(max_total: usize) -> Self {
        Self { buffer: Vec::new(), data: String::new(), total: 0, max_total, done: false }
    }

    /// 已经消费掉的响应体字节数
    pub fn consumed(&self) -> usize {
        self.total
    }

    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<SseEvent>, AppError> {
        if self.done {
            return Ok(Vec::new());
        }
        self.total = self.total.saturating_add(chunk.len());
        if self.total > self.max_total {
            return Err(AppError::Message("AI 流式响应超过安全大小限制".into()));
        }
        self.buffer.extend_from_slice(chunk);
        let mut events = Vec::new();
        while let Some(position) = self.buffer.iter().position(|byte| *byte == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=position).collect();
            // 去掉换行符本身，再去掉可选的 `\r`（CRLF）。
            let line = &line[..line.len() - 1];
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            self.push_line(line, &mut events)?;
            if self.done {
                self.buffer.clear();
                break;
            }
        }
        if self.buffer.len() > MAX_SSE_EVENT_BYTES {
            return Err(AppError::Message("AI 流式响应事件格式无效".into()));
        }
        Ok(events)
    }

    /// 流结束时调用：处理没有以空行收尾的最后一条事件。
    pub fn finish(&mut self) -> Result<Vec<SseEvent>, AppError> {
        let mut events = Vec::new();
        if !self.buffer.is_empty() {
            let line = std::mem::take(&mut self.buffer);
            let line = line.strip_suffix(b"\r").unwrap_or(&line);
            let text = String::from_utf8(line.to_vec())
                .map_err(|_| AppError::Message("AI 流式响应不是有效 UTF-8".into()))?;
            self.push_text(&text, &mut events)?;
        }
        self.dispatch(&mut events)?;
        self.done = true;
        Ok(events)
    }

    fn push_line(&mut self, line: &[u8], events: &mut Vec<SseEvent>) -> Result<(), AppError> {
        if line.len() > MAX_SSE_EVENT_BYTES {
            return Err(AppError::Message("AI 流式响应事件格式无效".into()));
        }
        let text = String::from_utf8(line.to_vec())
            .map_err(|_| AppError::Message("AI 流式响应不是有效 UTF-8".into()))?;
        self.push_text(&text, events)
    }

    fn push_text(&mut self, line: &str, events: &mut Vec<SseEvent>) -> Result<(), AppError> {
        if line.is_empty() {
            return self.dispatch(events);
        }
        // 以 `:` 开头的行是注释（常见于心跳），直接忽略。
        if line.starts_with(':') {
            return Ok(());
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        match field {
            "data" => {
                if value.trim() == "[DONE]" {
                    self.data.clear();
                    self.done = true;
                    events.push(SseEvent::Done);
                    return Ok(());
                }
                if self.data.len().saturating_add(value.len()) > MAX_SSE_EVENT_BYTES {
                    return Err(AppError::Message("AI 流式响应事件格式无效".into()));
                }
                if !self.data.is_empty() {
                    self.data.push('\n');
                }
                self.data.push_str(value);
                Ok(())
            }
            // event / id / retry 字段对本应用没有意义。
            _ => Ok(()),
        }
    }

    fn dispatch(&mut self, events: &mut Vec<SseEvent>) -> Result<(), AppError> {
        if self.data.is_empty() {
            return Ok(());
        }
        let payload = std::mem::take(&mut self.data);
        let payload = payload.trim();
        if payload.is_empty() {
            return Ok(());
        }
        if let Some(delta) = parse_delta_payload(payload)? {
            events.push(SseEvent::Delta(delta));
        }
        Ok(())
    }
}

impl Default for SseParser {
    fn default() -> Self {
        Self::new()
    }
}

/// 从一条 SSE `data` 载荷里取出增量正文。
///
/// - `data: [DONE]` 由上层处理，这里只处理 JSON。
/// - 缺少 `choices[0].delta.content` 时返回 `Ok(None)`（例如 usage-only 帧）。
/// - JSON 非法或对端在流中返回 error 时返回错误，不能当作正常结束。
pub fn parse_delta_payload(payload: &str) -> Result<Option<String>, AppError> {
    let value: serde_json::Value = serde_json::from_str(payload)
        .map_err(|_| AppError::Message("AI 流式响应格式无效".into()))?;
    if let Some(message) = value.get("error").and_then(|error| {
        error
            .get("message")
            .and_then(serde_json::Value::as_str)
            .or_else(|| error.as_str())
    }) {
        let message: String = message.chars().take(200).collect();
        return Err(AppError::Message(format!("AI 服务返回错误：{message}")));
    }
    let Some(choice) = value.get("choices").and_then(|choices| choices.as_array()).and_then(|choices| choices.first()) else {
        return Ok(None);
    };
    let content = choice
        .get("delta")
        .and_then(|delta| delta.get("content"))
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            choice
                .get("message")
                .and_then(|message| message.get("content"))
                .and_then(serde_json::Value::as_str)
        });
    Ok(content.map(str::to_owned))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn collect(parser: &mut SseParser, chunks: &[&str]) -> Vec<SseEvent> {
        let mut events = Vec::new();
        for chunk in chunks {
            events.extend(parser.push(chunk.as_bytes()).expect("chunk 解析失败"));
        }
        events
    }

    #[test]
    fn 一条事件跨多个分片() {
        let mut parser = SseParser::new();
        let events = collect(
            &mut parser,
            &["data: {\"choices\":[{\"delta\":{\"con", "tent\":\"你好\"}}]}\n", "\n"],
        );
        assert_eq!(events, vec![SseEvent::Delta("你好".into())]);
    }

    #[test]
    fn 一个分片包含多条事件() {
        let mut parser = SseParser::new();
        let body = "data: {\"choices\":[{\"delta\":{\"content\":\"A\"}}]}\n\n\
                    data: {\"choices\":[{\"delta\":{\"content\":\"B\"}}]}\n\n";
        let events = collect(&mut parser, &[body]);
        assert_eq!(events, vec![SseEvent::Delta("A".into()), SseEvent::Delta("B".into())]);
    }

    #[test]
    fn 处理_done_标记() {
        let mut parser = SseParser::new();
        let events = collect(
            &mut parser,
            &["data: {\"choices\":[{\"delta\":{\"content\":\"A\"}}]}\n\ndata: [DO", "NE]\n\n"],
        );
        assert_eq!(events, vec![SseEvent::Delta("A".into()), SseEvent::Done]);
    }

    #[test]
    fn done_之后的分片被忽略() {
        let mut parser = SseParser::new();
        collect(&mut parser, &["data: [DONE]\n\n"]);
        assert!(parser.push(b"data: {\"choices\":[{\"delta\":{\"content\":\"A\"}}]}\n\n").unwrap().is_empty());
    }

    #[test]
    fn 中文字符被拆分在分片之间() {
        let mut parser = SseParser::new();
        let full = "data: {\"choices\":[{\"delta\":{\"content\":\"深度求索\"}}]}\n\n";
        let bytes = full.as_bytes();
        // 逐字节喂入，确保每个多字节字符都跨 chunk。
        let events = {
            let mut events = Vec::new();
            for byte in bytes {
                events.extend(parser.push(&[*byte]).unwrap());
            }
            events
        };
        assert_eq!(events, vec![SseEvent::Delta("深度求索".into())]);
    }

    #[test]
    fn 忽略注释行与空行() {
        let mut parser = SseParser::new();
        let events = collect(
            &mut parser,
            &[
                ": keep-alive\n\n\n",
                ": ping\ndata: {\"choices\":[{\"delta\":{\"content\":\"好\"}}]}\n\n",
            ],
        );
        assert_eq!(events, vec![SseEvent::Delta("好".into())]);
    }

    #[test]
    fn 缺少_delta_content_时不产生事件() {
        let mut parser = SseParser::new();
        let events = collect(
            &mut parser,
            &["data: {\"choices\":[{\"delta\":{\"role\":\"assistant\"}}]}\n\ndata: {\"choices\":[],\"usage\":{\"total_tokens\":9}}\n\n"],
        );
        assert!(events.is_empty());
    }

    #[test]
    fn 非法_json_报错而不是当成正常结束() {
        let mut parser = SseParser::new();
        let error = parser.push(b"data: {not json}\n\n").unwrap_err();
        assert!(error.to_string().contains("格式无效"), "{error}");
    }

    #[test]
    fn 流中的错误载荷变成错误事件() {
        let mut parser = SseParser::new();
        let error = parser
            .push(b"data: {\"error\":{\"message\":\"context length exceeded\"}}\n\n")
            .unwrap_err();
        assert!(error.to_string().contains("context length exceeded"), "{error}");
    }

    #[test]
    fn 响应体超过上限时报错() {
        let mut parser = SseParser::with_limit(32);
        let error = parser.push(&vec![b'a'; 64]).unwrap_err();
        assert!(error.to_string().contains("超过安全大小限制"), "{error}");
    }

    #[test]
    fn 没有空行收尾的事件在流结束时仍然会被解析() {
        let mut parser = SseParser::new();
        let mut events = collect(&mut parser, &["data: {\"choices\":[{\"delta\":{\"content\":\"尾\"}}]}\n"]);
        assert!(events.is_empty());
        events.extend(parser.finish().unwrap());
        assert_eq!(events, vec![SseEvent::Delta("尾".into())]);
    }

    #[test]
    fn 解析_payload_返回增量文本() {
        assert_eq!(parse_delta_payload("{\"choices\":[{\"delta\":{\"content\":\"x\"}}]}").unwrap(), Some("x".into()));
        assert_eq!(parse_delta_payload("{\"choices\":[{\"delta\":{}}]}").unwrap(), None);
        assert_eq!(parse_delta_payload("{\"choices\":[]}").unwrap(), None);
        assert!(parse_delta_payload("{").is_err());
    }
}
