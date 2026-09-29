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

impl OpenAiCompatibleProvider {
    pub async fn embed(&self, endpoint: &str, model: &str, input: &[String]) -> Result<Vec<Vec<f32>>, crate::error::AppError> {
        let response = self.http.post(endpoint).bearer_auth(&self.api_key).json(&serde_json::json!({ "model": model, "input": input })).send().await?;
        let status = response.status();
        if !status.is_success() {
            return Err(crate::error::AppError::Message(format!("Embedding 服务请求失败（HTTP {}）", status.as_u16())));
        }
        let value: serde_json::Value = response.json().await?;
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
