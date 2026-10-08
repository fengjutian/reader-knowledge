//! 导入配置。优先读环境变量，缺失项回落到应用数据目录附近的默认值，
//! 启动时校验（需求 20：环境变量必须在启动时校验）。

use crate::error::AppError;
use std::path::{Path, PathBuf};

/// 维基文本许可证标识，显示端要原样展示。
pub const WIKIPEDIA_LICENSE: &str = "CC BY-SA 4.0";
pub const WIKIPEDIA_LICENSE_URL: &str = "https://creativecommons.org/licenses/by-sa/4.0/";
pub const DEFAULT_DUMP_BASE_URL: &str = "https://dumps.wikimedia.org/zhwiki/latest/";
/// 生产导入用的正文 dump（multistream 支持流式解压与索引定位）。
pub const DUMP_ARTICLES_FILENAME: &str = "zhwiki-latest-pages-articles-multistream.xml.bz2";
pub const DUMP_MD5SUMS_FILENAME: &str = "zhwiki-latest-md5sums.txt";

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportMode {
    /// 一期只开放摘要模式；其余模式保留枚举，界面不暴露。
    TitlesOnly,
    Summary,
    FullText,
}

impl ImportMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ImportMode::TitlesOnly => "titles_only",
            ImportMode::Summary => "summary",
            ImportMode::FullText => "full_text",
        }
    }

    pub fn parse(value: &str) -> Result<Self, AppError> {
        match value.trim().to_lowercase().as_str() {
            "titles_only" | "titles" => Ok(ImportMode::TitlesOnly),
            "summary" => Ok(ImportMode::Summary),
            "full_text" | "full" => Ok(ImportMode::FullText),
            other => Err(AppError::Message(format!("未知导入模式：{other}"))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct WikipediaConfig {
    pub base_url: String,
    pub temp_dir: PathBuf,
    pub batch_size: usize,
    pub max_concurrency: usize,
    pub summary_min_chars: usize,
    pub summary_max_chars: usize,
    pub allowed_hosts: Vec<String>,
    pub user_agent: String,
    pub staging_retention_days: i64,
    pub max_retries: u32,
    /// staging 每多少条落一次检查点（需求 7.3.7 建议 500~1000）。
    pub checkpoint_every: u64,
    /// 进度写库的节流间隔，避免每条都写库。
    pub progress_interval_ms: u64,
}

impl Default for WikipediaConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_DUMP_BASE_URL.to_string(),
            temp_dir: PathBuf::from("."),
            batch_size: 500,
            max_concurrency: 1,
            summary_min_chars: 300,
            summary_max_chars: 600,
            allowed_hosts: vec!["dumps.wikimedia.org".to_string()],
            user_agent: default_user_agent(),
            staging_retention_days: 14,
            max_retries: 3,
            checkpoint_every: 500,
            progress_interval_ms: 1000,
        }
    }
}

/// Wikimedia 要求 User-Agent 里带上联系方式，这里默认用项目名 + 仓库地址。
fn default_user_agent() -> String {
    "wxreader-knowledge/0.3 (https://github.com/wxreader-knowledge; local personal knowledge app)".to_string()
}

impl WikipediaConfig {
    /// 从环境变量装配。`app_data_dir` 只在变量缺失时用于推导临时目录。
    pub fn from_env(app_data_dir: &Path) -> Result<Self, AppError> {
        let mut config = Self::default();
        if let Ok(value) = std::env::var("WIKIPEDIA_DUMP_BASE_URL") {
            if !value.trim().is_empty() {
                config.base_url = value;
            }
        }
        if let Ok(value) = std::env::var("WIKIPEDIA_IMPORT_TEMP_DIR") {
            if !value.trim().is_empty() {
                config.temp_dir = PathBuf::from(value);
            }
        } else {
            config.temp_dir = app_data_dir.join("wikipedia-import");
        }
        if let Ok(value) = std::env::var("WIKIPEDIA_USER_AGENT") {
            if !value.trim().is_empty() {
                config.user_agent = value;
            }
        }
        if let Ok(value) = std::env::var("WIKIPEDIA_IMPORT_ALLOWED_HOSTS") {
            let hosts: Vec<String> = value
                .split(',')
                .map(|host| host.trim().to_lowercase())
                .filter(|host| !host.is_empty())
                .collect();
            if !hosts.is_empty() {
                config.allowed_hosts = hosts;
            }
        }
        config.batch_size = env_usize("WIKIPEDIA_IMPORT_BATCH_SIZE", config.batch_size)?.max(1);
        config.max_concurrency = env_usize("WIKIPEDIA_IMPORT_MAX_CONCURRENCY", config.max_concurrency)?.max(1);
        config.summary_max_chars =
            env_usize("WIKIPEDIA_IMPORT_SUMMARY_MAX_CHARS", config.summary_max_chars)?.max(80);
        config.staging_retention_days =
            env_i64("WIKIPEDIA_STAGING_RETENTION_DAYS", config.staging_retention_days)?;
        config.max_retries = env_usize("WIKIPEDIA_IMPORT_MAX_RETRIES", config.max_retries as usize)? as u32;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if !self.base_url.starts_with("https://") {
            return Err(AppError::Message(
                "dump 地址必须是 HTTPS，拒绝明文下载".to_string(),
            ));
        }
        if self.allowed_hosts.is_empty() {
            return Err(AppError::Message("必须配置至少一个允许的 dump 域名".to_string()));
        }
        if self.user_agent.trim().len() < 10 {
            return Err(AppError::Message(
                "User-Agent 过短，请设置 WIKIPEDIA_USER_AGENT 包含应用名与联系方式".to_string(),
            ));
        }
        if self.summary_min_chars >= self.summary_max_chars {
            return Err(AppError::Message("摘要下限必须小于上限".to_string()));
        }
        Ok(())
    }

    /// 临时目录必须可创建、可写，否则导入一定失败，不如启动就报错。
    pub fn ensure_temp_dir(&self) -> Result<PathBuf, AppError> {
        std::fs::create_dir_all(&self.temp_dir).map_err(|error| {
            AppError::Message(format!(
                "无法创建维基导入临时目录 {}：{error}",
                self.temp_dir.display()
            ))
        })?;
        let probe = self.temp_dir.join(".write-probe");
        std::fs::write(&probe, b"ok")
            .map_err(|error| AppError::Message(format!("维基导入临时目录不可写：{error}")))?;
        let _ = std::fs::remove_file(&probe);
        Ok(self.temp_dir.clone())
    }

    pub fn summary_options(&self) -> crate::wikipedia::wikitext::SummaryOptions {
        crate::wikipedia::wikitext::SummaryOptions {
            min_chars: self.summary_min_chars,
            max_chars: self.summary_max_chars,
        }
    }
}

fn env_usize(key: &str, fallback: usize) -> Result<usize, AppError> {
    match std::env::var(key) {
        Ok(value) if !value.trim().is_empty() => value
            .trim()
            .parse()
            .map_err(|_| AppError::Message(format!("环境变量 {key} 不是合法数字：{value}"))),
        _ => Ok(fallback),
    }
}

fn env_i64(key: &str, fallback: i64) -> Result<i64, AppError> {
    match std::env::var(key) {
        Ok(value) if !value.trim().is_empty() => value
            .trim()
            .parse()
            .map_err(|_| AppError::Message(format!("环境变量 {key} 不是合法数字：{value}"))),
        _ => Ok(fallback),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 导入模式可解析() {
        assert_eq!(ImportMode::parse("summary").unwrap(), ImportMode::Summary);
        assert_eq!(ImportMode::parse(" FULL_TEXT ").unwrap(), ImportMode::FullText);
        assert_eq!(ImportMode::parse("titles").unwrap(), ImportMode::TitlesOnly);
        assert!(ImportMode::parse("bogus").is_err());
    }

    #[test]
    fn 默认配置通过校验() {
        assert!(WikipediaConfig::default().validate().is_ok());
    }

    #[test]
    fn 非_https_地址被拒绝() {
        let mut config = WikipediaConfig::default();
        config.base_url = "http://dumps.wikimedia.org/zhwiki/latest/".to_string();
        let error = config.validate().expect_err("http 应被拒绝");
        assert!(error.to_string().contains("HTTPS"));
    }

    #[test]
    fn 过短_user_agent_被拒绝() {
        let mut config = WikipediaConfig::default();
        config.user_agent = "x".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn 摘要上下限颠倒被拒绝() {
        let mut config = WikipediaConfig::default();
        config.summary_min_chars = 900;
        config.summary_max_chars = 300;
        assert!(config.validate().is_err());
    }

    #[test]
    fn 临时目录会被创建并可写() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut config = WikipediaConfig::default();
        config.temp_dir = dir.path().join("nested");
        let resolved = config.ensure_temp_dir().expect("ensure");
        assert!(resolved.exists());
        assert!(!resolved.join(".write-probe").exists(), "探测文件应被清理");
    }

    #[test]
    fn 环境变量可覆盖配置() {
        let dir = tempfile::tempdir().expect("temp dir");
        std::env::set_var("WIKIPEDIA_IMPORT_BATCH_SIZE", "42");
        std::env::set_var("WIKIPEDIA_IMPORT_SUMMARY_MAX_CHARS", "700");
        let config = WikipediaConfig::from_env(dir.path()).expect("from env");
        assert_eq!(config.batch_size, 42);
        assert_eq!(config.summary_max_chars, 700);
        assert_eq!(config.summary_min_chars, 300);
        std::env::remove_var("WIKIPEDIA_IMPORT_BATCH_SIZE");
        std::env::remove_var("WIKIPEDIA_IMPORT_SUMMARY_MAX_CHARS");
    }
}
