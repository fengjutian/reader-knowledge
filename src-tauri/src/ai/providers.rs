use super::provider::{AiProvider, ProviderFuture};
use crate::models::ChatMessage;
pub struct OpenAiCompatibleProvider {
    pub endpoint: String,
    pub model: String,
    pub api_key: String,
    pub http: reqwest::Client,
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
                    value = Some(response.json::<serde_json::Value>().await?);
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
                    let message = if status.as_u16() == 429 || status.is_server_error() {
                        format!("AI 服务暂时繁忙（HTTP {}），已自动重试，请稍后再试", status.as_u16())
                    } else if status.as_u16() == 401 || status.as_u16() == 403 {
                        "AI 服务认证失败，请到设置中检查 API Key".into()
                    } else {
                        format!("AI 服务请求失败（HTTP {}），请检查服务配置", status.as_u16())
                    };
                    return Err(crate::error::AppError::Message(message));
                }
            };
            value["choices"][0]["message"]["content"]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| crate::error::AppError::Message("AI 响应格式无效".into()))
        })
    }
}
