use crate::error::AppError;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

pub const GATEWAY_URL: &str = "https://i.weread.qq.com/api/agent/gateway";
pub const SKILL_VERSION: &str = "1.0.4";

pub struct WeReadClient {
    api_key: String,
    http: reqwest::Client,
}

impl WeReadClient {
    pub fn new(api_key: String) -> Result<Self, AppError> {
        if !api_key.starts_with("wrk-") {
            return Err(AppError::Message(
                "微信读书 API Key 格式无效，应以 wrk- 开头".into(),
            ));
        }
        Ok(Self {
            api_key,
            http: reqwest::Client::new(),
        })
    }

    pub async fn call<T: DeserializeOwned>(
        &self,
        api_name: &str,
        params: Map<String, Value>,
    ) -> Result<T, AppError> {
        let mut body = params;
        body.insert("api_name".into(), Value::String(api_name.into()));
        body.insert("skill_version".into(), Value::String(SKILL_VERSION.into()));
        let response: Value = self
            .http
            .post(GATEWAY_URL)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        if let Some(upgrade) = response.get("upgrade_info") {
            let message = upgrade
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("微信读书技能版本需要升级");
            return Err(AppError::UpgradeRequired(message.into()));
        }
        if response.get("errcode").and_then(Value::as_i64).unwrap_or(0) != 0 {
            let message = response
                .get("errmsg")
                .and_then(Value::as_str)
                .unwrap_or("微信读书接口请求失败");
            return Err(AppError::Message(message.into()));
        }
        serde_json::from_value(response).map_err(AppError::from)
    }

    pub async fn test(&self) -> Result<(), AppError> {
        let _: Value = self.call("/_list", Map::new()).await?;
        Ok(())
    }
}
