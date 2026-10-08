//! 导入任务的持久化：状态机、staging 写入、校验、发布、冲突保护、审计。
//!
//! 状态迁移只走 `transition`，不允许任意 UPDATE 状态（需求 7.11）。
//! 暂停/取消不靠内存信号：调用方直接改数据库状态，运行中的 runner 在批次
//! 边界读状态决定停在哪，这样 CLI 与界面同时存在也不会互相打架。

use crate::database::Database;
use crate::error::AppError;
use crate::wikipedia::config::{ImportMode, WIKIPEDIA_LICENSE};
use crate::wikipedia::redirect::RedirectOutcome;
use crate::wikipedia::title::normalize_search_key;
use rusqlite::{params, OptionalExtension, Transaction};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Pending,
    Downloading,
    Verifying,
    Parsing,
    ResolvingRedirects,
    Validating,
    ReadyToPublish,
    Publishing,
    Completed,
    Paused,
    Cancelled,
    Failed,
}

impl JobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            JobStatus::Pending => "pending",
            JobStatus::Downloading => "downloading",
            JobStatus::Verifying => "verifying",
            JobStatus::Parsing => "parsing",
            JobStatus::ResolvingRedirects => "resolving_redirects",
            JobStatus::Validating => "validating",
            JobStatus::ReadyToPublish => "ready_to_publish",
            JobStatus::Publishing => "publishing",
            JobStatus::Completed => "completed",
            JobStatus::Paused => "paused",
            JobStatus::Cancelled => "cancelled",
            JobStatus::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Result<Self, AppError> {
        match value {
            "pending" => Ok(JobStatus::Pending),
            "downloading" => Ok(JobStatus::Downloading),
            "verifying" => Ok(JobStatus::Verifying),
            "parsing" => Ok(JobStatus::Parsing),
            "resolving_redirects" => Ok(JobStatus::ResolvingRedirects),
            "validating" => Ok(JobStatus::Validating),
            "ready_to_publish" => Ok(JobStatus::ReadyToPublish),
            "publishing" => Ok(JobStatus::Publishing),
            "completed" => Ok(JobStatus::Completed),
            "paused" => Ok(JobStatus::Paused),
            "cancelled" => Ok(JobStatus::Cancelled),
            "failed" => Ok(JobStatus::Failed),
            other => Err(AppError::Message(format!("未知任务状态：{other}"))),
        }
    }

    /// 是否终态。终态不再接受任何迁移。
    pub fn is_terminal(&self) -> bool {
        matches!(self, JobStatus::Completed | JobStatus::Cancelled | JobStatus::Failed)
    }

    /// 合法迁移表。paused 可以从任意进行中状态进入，
    /// 并通过 checkpoint 里的 paused_from 回到原阶段。
    pub fn can_transition_to(&self, next: JobStatus) -> bool {
        use JobStatus::*;
        if self.is_terminal() {
            return false;
        }
        match self {
            Pending => matches!(next, Downloading | Verifying | Parsing | Failed | Cancelled | Paused),
            Downloading => matches!(next, Verifying | Failed | Cancelled | Paused),
            Verifying => matches!(next, Parsing | Failed | Cancelled | Paused),
            Parsing => matches!(next, ResolvingRedirects | Failed | Cancelled | Paused),
            ResolvingRedirects => matches!(next, Validating | Failed | Cancelled | Paused),
            Validating => matches!(next, ReadyToPublish | Failed | Cancelled | Paused),
            ReadyToPublish => matches!(next, Publishing | Completed | Failed | Cancelled | Paused),
            Publishing => matches!(next, Completed | Failed | Cancelled | Paused),
            // 从 paused 恢复只能回到记录下来的阶段，由上层显式给出目标状态。
            Paused => matches!(next, Downloading | Verifying | Parsing | ResolvingRedirects | Validating | ReadyToPublish | Publishing | Failed | Cancelled),
            // 终态：is_terminal 已经拦掉了，这里只是让 match 穷尽。
            Completed | Cancelled | Failed => false,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ImportRequest {
    /// 留空则用配置里的官方 latest 地址。
    pub source_url: Option<String>,
    /// 留空则用配置里的 base_url。
    pub dump_version: Option<String>,
    pub mode: Option<ImportMode>,
    pub handle_redirects: Option<bool>,
    pub filter_disambiguation: Option<bool>,
    pub filter_list_pages: Option<bool>,
    pub auto_publish: Option<bool>,
    pub max_items: Option<u64>,
    pub batch_size: Option<usize>,
    pub concurrency: Option<usize>,
    pub max_retries: Option<u32>,
    pub temp_dir: Option<PathBuf>,
    /// 本地 dump 文件：离线与测试用，跳过下载与 MD5。
    pub local_file: Option<PathBuf>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportJob {
    pub id: String,
    pub source_type: String,
    pub dump_version: String,
    pub source_url: String,
    pub mode: String,
    pub status: String,
    pub total_bytes: i64,
    pub downloaded_bytes: i64,
    pub scanned_count: i64,
    pub accepted_count: i64,
    pub redirect_count: i64,
    pub filtered_count: i64,
    pub inserted_count: i64,
    pub updated_count: i64,
    pub skipped_count: i64,
    pub conflict_count: i64,
    pub error_count: i64,
    pub current_file: String,
    pub bytes_per_second: f64,
    pub error_message: String,
    pub auto_publish: bool,
    pub started_at: i64,
    pub finished_at: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportIssue {
    pub id: i64,
    pub page_id: Option<i64>,
    pub title: String,
    pub record_type: String,
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub created_at: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishSummary {
    pub inserted: u64,
    pub updated: u64,
    pub skipped: u64,
    pub conflicts: u64,
    pub aliases_inserted: u64,
    pub source_missing: u64,
}

pub fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_secs() as i64)
        .unwrap_or_default()
}

/// staging 里 filter_status 的取值。
pub const FILTER_ACCEPTED: &str = "accepted";
pub const FILTER_REDIRECT: &str = "redirect";
pub const FILTER_FILTERED: &str = "filtered";

pub const PUBLISH_PENDING: &str = "pending";
pub const PUBLISH_DONE: &str = "done";
pub const PUBLISH_CONFLICT: &str = "conflict";
pub const PUBLISH_SKIPPED: &str = "skipped";

#[derive(Debug, Clone)]
pub struct StagingRow {
    pub page_id: i64,
    pub revision_id: i64,
    pub raw_title: String,
    pub normalized_title: String,
    pub redirect_title: Option<String>,
    pub summary: String,
    pub revision_timestamp: String,
    pub content_hash: String,
    pub filter_status: String,
    pub filter_reason: Option<String>,
    pub quality_flag: Option<String>,
}

pub fn create_job(
    db: &Database,
    request: &ImportRequest,
    config: &crate::wikipedia::config::WikipediaConfig,
) -> Result<ImportJob, AppError> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_seconds();
    let source_url = request
        .local_file
        .as_ref()
        .map(|path| format!("file://{}", path.display()))
        .or_else(|| {
            request
                .source_url
                .clone()
                .map(|base| format!("{}{}", base.trim_end_matches('/'), "/zhwiki-latest-pages-articles-multistream.xml.bz2"))
                .or_else(|| Some(format!("{}{}", config.base_url.trim_end_matches('/'), "/zhwiki-latest-pages-articles-multistream.xml.bz2")))
        })
        .unwrap_or_default();
    let dump_version = request
        .dump_version
        .clone()
        .unwrap_or_else(|| config.base_url.trim_end_matches('/').to_string());
    let mode = request.mode.unwrap_or(ImportMode::Summary);
    let config_json = serde_json::json!({
        "mode": mode.as_str(),
        "handleRedirects": request.handle_redirects.unwrap_or(true),
        "filterDisambiguation": request.filter_disambiguation.unwrap_or(true),
        "filterListPages": request.filter_list_pages.unwrap_or(true),
        "maxItems": request.max_items,
        "batchSize": request.batch_size.unwrap_or(config.batch_size),
        "concurrency": request.concurrency.unwrap_or(config.max_concurrency),
        "maxRetries": request.max_retries.unwrap_or(config.max_retries),
        "tempDir": request.temp_dir.clone().unwrap_or_else(|| config.temp_dir.clone()).display().to_string(),
        "localFile": request.local_file.as_ref().map(|path| path.display().to_string()),
        "summaryMinChars": config.summary_min_chars,
        "summaryMaxChars": config.summary_max_chars,
    })
    .to_string();
    let connection = db.connect()?;
    connection.execute(
        "INSERT INTO glossary_import_jobs(
            id, source_type, dump_version, source_url, mode, status, config_json, auto_publish,
            created_at, updated_at)
         VALUES (?1, 'wikipedia', ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)",
        params![
            id,
            dump_version,
            source_url,
            mode.as_str(),
            JobStatus::Pending.as_str(),
            config_json,
            request.auto_publish.unwrap_or(false) as i64,
            now
        ],
    )?;
    audit(&connection, "job_created", Some(&id), &format!("mode={}", mode.as_str()))?;
    get_job(db, &id)?.ok_or_else(|| AppError::Message("任务创建后读不到".to_string()))
}

const JOB_COLUMNS: &str = "id, source_type, dump_version, source_url, mode, status, total_bytes,
    downloaded_bytes, scanned_count, accepted_count, redirect_count, filtered_count,
    inserted_count, updated_count, skipped_count, conflict_count, error_count,
    coalesce(current_file,''), bytes_per_second, coalesce(error_message,''), auto_publish,
    coalesce(started_at,0), coalesce(finished_at,0), created_at, updated_at";

fn map_job(row: &rusqlite::Row<'_>) -> rusqlite::Result<ImportJob> {
    Ok(ImportJob {
        id: row.get(0)?,
        source_type: row.get(1)?,
        dump_version: row.get(2)?,
        source_url: row.get(3)?,
        mode: row.get(4)?,
        status: row.get(5)?,
        total_bytes: row.get(6)?,
        downloaded_bytes: row.get(7)?,
        scanned_count: row.get(8)?,
        accepted_count: row.get(9)?,
        redirect_count: row.get(10)?,
        filtered_count: row.get(11)?,
        inserted_count: row.get(12)?,
        updated_count: row.get(13)?,
        skipped_count: row.get(14)?,
        conflict_count: row.get(15)?,
        error_count: row.get(16)?,
        current_file: row.get(17)?,
        bytes_per_second: row.get(18)?,
        error_message: row.get(19)?,
        auto_publish: row.get::<_, i64>(20)? != 0,
        started_at: row.get(21)?,
        finished_at: row.get(22)?,
        created_at: row.get(23)?,
        updated_at: row.get(24)?,
    })
}

pub fn get_job(db: &Database, id: &str) -> Result<Option<ImportJob>, AppError> {
    let connection = db.connect()?;
    let mut statement = connection.prepare(&format!("SELECT {JOB_COLUMNS} FROM glossary_import_jobs WHERE id = ?1"))?;
    let job = statement
        .query_row([id], map_job)
        .optional()
        .map_err(AppError::from)?;
    Ok(job)
}

pub fn list_jobs(db: &Database, limit: usize) -> Result<Vec<ImportJob>, AppError> {
    let connection = db.connect()?;
    let mut statement = connection.prepare(&format!(
        "SELECT {JOB_COLUMNS} FROM glossary_import_jobs ORDER BY created_at DESC LIMIT ?1"
    ))?;
    let rows = statement
        .query_map([limit.min(200) as i64], map_job)
        .map_err(AppError::from)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(AppError::from)?;
    Ok(rows)
}

pub fn job_config(db: &Database, id: &str) -> Result<serde_json::Value, AppError> {
    let connection = db.connect()?;
    let raw: String = connection
        .query_row("SELECT config_json FROM glossary_import_jobs WHERE id = ?1", [id], |row| row.get(0))
        .optional()?
        .unwrap_or_else(|| "{}".to_string());
    Ok(serde_json::from_str(&raw).unwrap_or(serde_json::Value::Object(Default::default())))
}

/// 唯一的状态写入口。非法迁移直接报错。
pub fn transition(db: &Database, id: &str, next: JobStatus) -> Result<(), AppError> {
    let connection = db.connect()?;
    transition_on(&connection, id, next)
}

pub fn transition_on(connection: &rusqlite::Connection, id: &str, next: JobStatus) -> Result<(), AppError> {
    let current: String = connection
        .query_row("SELECT status FROM glossary_import_jobs WHERE id = ?1", [id], |row| row.get(0))
        .optional()?
        .ok_or_else(|| AppError::Message(format!("任务不存在：{id}")))?;
    let current = JobStatus::parse(&current)?;
    if current == next {
        return Ok(());
    }
    if !current.can_transition_to(next) {
        return Err(AppError::Message(format!(
            "非法状态迁移：{} -> {}",
            current.as_str(),
            next.as_str()
        )));
    }
    let now = now_seconds();
    connection.execute(
        "UPDATE glossary_import_jobs
            SET status = ?2,
                updated_at = ?3,
                started_at = coalesce(started_at, ?3)
          WHERE id = ?1",
        params![id, next.as_str(), now],
    )?;
    Ok(())
}

pub fn mark_failed(db: &Database, id: &str, message: &str) -> Result<(), AppError> {
    let connection = db.connect()?;
    let now = now_seconds();
    let status: Option<String> = connection
        .query_row("SELECT status FROM glossary_import_jobs WHERE id = ?1", [id], |row| row.get(0))
        .optional()?;
    let Some(status) = status else {
        return Ok(());
    };
    if JobStatus::parse(&status)?.is_terminal() {
        return Ok(());
    }
    connection.execute(
        "UPDATE glossary_import_jobs
            SET status = ?2, error_message = ?3, finished_at = ?4, updated_at = ?4
          WHERE id = ?1",
        params![id, JobStatus::Failed.as_str(), truncate(message, 500), now],
    )?;
    audit(&connection, "job_failed", Some(id), &truncate(message, 200))?;
    Ok(())
}

/// 进度写库。调用方负责节流，这里只做覆盖写。
#[allow(clippy::too_many_arguments)]
pub fn update_progress(
    db: &Database,
    id: &str,
    total_bytes: Option<i64>,
    downloaded_bytes: Option<i64>,
    scanned: Option<i64>,
    accepted: Option<i64>,
    redirects: Option<i64>,
    filtered: Option<i64>,
    current_file: Option<&str>,
    bytes_per_second: Option<f64>,
) -> Result<(), AppError> {
    let connection = db.connect()?;
    connection.execute(
        "UPDATE glossary_import_jobs SET
            total_bytes = coalesce(?2, total_bytes),
            downloaded_bytes = coalesce(?3, downloaded_bytes),
            scanned_count = coalesce(?4, scanned_count),
            accepted_count = coalesce(?5, accepted_count),
            redirect_count = coalesce(?6, redirect_count),
            filtered_count = coalesce(?7, filtered_count),
            current_file = coalesce(?8, current_file),
            bytes_per_second = coalesce(?9, bytes_per_second),
            updated_at = ?10
          WHERE id = ?1",
        params![
            id,
            total_bytes,
            downloaded_bytes,
            scanned,
            accepted,
            redirects,
            filtered,
            current_file,
            bytes_per_second,
            now_seconds()
        ],
    )?;
    Ok(())
}

pub fn set_publish_counts(
    db: &Database,
    id: &str,
    inserted: i64,
    updated: i64,
    skipped: i64,
    conflicts: i64,
    errors: i64,
) -> Result<(), AppError> {
    let connection = db.connect()?;
    connection.execute(
        "UPDATE glossary_import_jobs SET
            inserted_count = ?2, updated_count = ?3, skipped_count = ?4,
            conflict_count = ?5, error_count = ?6, updated_at = ?7
          WHERE id = ?1",
        params![id, inserted, updated, skipped, conflicts, errors, now_seconds()],
    )?;
    Ok(())
}

pub fn save_checkpoint(db: &Database, id: &str, checkpoint: &serde_json::Value) -> Result<(), AppError> {
    let connection = db.connect()?;
    connection.execute(
        "UPDATE glossary_import_jobs SET checkpoint_json = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, checkpoint.to_string(), now_seconds()],
    )?;
    Ok(())
}

pub fn load_checkpoint(db: &Database, id: &str) -> Result<serde_json::Value, AppError> {
    let connection = db.connect()?;
    let raw: Option<String> = connection
        .query_row("SELECT checkpoint_json FROM glossary_import_jobs WHERE id = ?1", [id], |row| row.get(0))
        .optional()?;
    Ok(raw
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or(serde_json::Value::Object(Default::default())))
}

/// 暂停：记录来源阶段，方便恢复时回到原地。
pub fn pause_job(db: &Database, id: &str) -> Result<(), AppError> {
    let connection = db.connect()?;
    let current: String = connection
        .query_row("SELECT status FROM glossary_import_jobs WHERE id = ?1", [id], |row| row.get(0))
        .optional()?
        .ok_or_else(|| AppError::Message(format!("任务不存在：{id}")))?;
    let status = JobStatus::parse(&current)?;
    if !status.can_transition_to(JobStatus::Paused) {
        return Err(AppError::Message(format!(
            "{} 状态的任务不能暂停",
            status.as_str()
        )));
    }
    let checkpoint = load_checkpoint(db, id)?;
    let mut object = checkpoint.as_object().cloned().unwrap_or_default();
    object.insert("pausedFrom".to_string(), serde_json::json!(status.as_str()));
    save_checkpoint(db, id, &serde_json::Value::Object(object))?;
    transition_on(&connection, id, JobStatus::Paused)?;
    audit(&connection, "job_paused", Some(id), &status.as_str())?;
    Ok(())
}

/// 恢复：回到 checkpoint 里记录的阶段。
pub fn resume_job(db: &Database, id: &str) -> Result<JobStatus, AppError> {
    let checkpoint = load_checkpoint(db, id)?;
    let resume_to = checkpoint
        .get("pausedFrom")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| JobStatus::parse(value).ok())
        .unwrap_or(JobStatus::Parsing);
    transition(db, id, resume_to)?;
    audit(&db.connect()?, "job_resumed", Some(id), resume_to.as_str())?;
    Ok(resume_to)
}

pub fn cancel_job(db: &Database, id: &str) -> Result<(), AppError> {
    let connection = db.connect()?;
    transition_on(&connection, id, JobStatus::Cancelled)?;
    connection.execute(
        "UPDATE glossary_import_jobs SET finished_at = ?2, updated_at = ?2 WHERE id = ?1",
        params![id, now_seconds()],
    )?;
    audit(&connection, "job_cancelled", Some(id), "")?;
    Ok(())
}

pub fn set_total_bytes(db: &Database, id: &str, total: i64) -> Result<(), AppError> {
    let connection = db.connect()?;
    connection.execute(
        "UPDATE glossary_import_jobs SET total_bytes = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, total, now_seconds()],
    )?;
    Ok(())
}

pub fn set_error_message(db: &Database, id: &str, message: &str) -> Result<(), AppError> {
    let connection = db.connect()?;
    connection.execute(
        "UPDATE glossary_import_jobs SET error_message = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, truncate(message, 500), now_seconds()],
    )?;
    Ok(())
}

pub fn save_report(db: &Database, id: &str, report: &serde_json::Value) -> Result<(), AppError> {
    let connection = db.connect()?;
    connection.execute(
        "UPDATE glossary_import_jobs SET report_json = ?2, updated_at = ?3 WHERE id = ?1",
        params![id, report.to_string(), now_seconds()],
    )?;
    Ok(())
}

pub fn get_report(db: &Database, id: &str) -> Result<Option<serde_json::Value>, AppError> {
    let connection = db.connect()?;
    let raw: Option<String> = connection
        .query_row("SELECT report_json FROM glossary_import_jobs WHERE id = ?1", [id], |row| row.get(0))
        .optional()?;
    Ok(raw.and_then(|value| serde_json::from_str(&value).ok()))
}

pub fn list_issues(
    db: &Database,
    id: &str,
    record_type: Option<&str>,
    limit: usize,
) -> Result<Vec<ImportIssue>, AppError> {
    let connection = db.connect()?;
    let sql = match record_type {
        Some(_) => {
            "SELECT id, page_id, coalesce(title,''), record_type, code, message, retryable, created_at
               FROM glossary_import_issues WHERE job_id = ?1 AND record_type = ?2
              ORDER BY id DESC LIMIT ?3"
        }
        None => {
            "SELECT id, page_id, coalesce(title,''), record_type, code, message, retryable, created_at
               FROM glossary_import_issues WHERE job_id = ?1
              ORDER BY id DESC LIMIT ?2"
        }
    };
    let mut statement = connection.prepare(sql)?;
    let mapper = |row: &rusqlite::Row<'_>| -> rusqlite::Result<ImportIssue> {
        Ok(ImportIssue {
            id: row.get(0)?,
            page_id: row.get(1)?,
            title: row.get(2)?,
            record_type: row.get(3)?,
            code: row.get(4)?,
            message: row.get(5)?,
            retryable: row.get::<_, i64>(6)? != 0,
            created_at: row.get(7)?,
        })
    };
    let rows = match record_type {
        Some(value) => statement
            .query_map(params![id, value, limit.min(500) as i64], mapper)
            .map_err(AppError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)?,
        None => statement
            .query_map(params![id, limit.min(500) as i64], mapper)
            .map_err(AppError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)?,
    };
    Ok(rows)
}

pub fn log_issue(
    db: &Database,
    job_id: &str,
    record_type: &str,
    code: &str,
    message: &str,
    page_id: Option<i64>,
    title: &str,
    retryable: bool,
) -> Result<(), AppError> {
    let connection = db.connect()?;
    log_issue_on(&connection, job_id, record_type, code, message, page_id, title, retryable)
}

#[allow(clippy::too_many_arguments)]
pub fn log_issue_on(
    connection: &rusqlite::Connection,
    job_id: &str,
    record_type: &str,
    code: &str,
    message: &str,
    page_id: Option<i64>,
    title: &str,
    retryable: bool,
) -> Result<(), AppError> {
    connection.execute(
        "INSERT INTO glossary_import_issues(job_id, page_id, title, record_type, code, message, retryable, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            job_id,
            page_id,
            truncate(title, 300),
            record_type,
            code,
            truncate(message, 800),
            retryable as i64,
            now_seconds()
        ],
    )?;
    Ok(())
}

pub fn audit(
    connection: &rusqlite::Connection,
    action: &str,
    job_id: Option<&str>,
    detail: &str,
) -> Result<(), AppError> {
    connection.execute(
        "INSERT INTO glossary_import_audit(action, job_id, detail, created_at)
         VALUES (?1, ?2, ?3, ?4)",
        params![action, job_id, truncate(detail, 500), now_seconds()],
    )?;
    Ok(())
}

/// 批量写 staging。`(job_id, page_id)` 唯一，重复执行只更新未发布的行。
pub fn insert_staging_batch(
    db: &Database,
    job_id: &str,
    rows: &[StagingRow],
) -> Result<usize, AppError> {
    if rows.is_empty() {
        return Ok(0);
    }
    let mut connection = db.connect()?;
    let transaction = connection.transaction()?;
    let now = now_seconds();
    {
        let mut statement = transaction.prepare(
            "INSERT INTO glossary_import_staging(
                job_id, page_id, revision_id, raw_title, normalized_title, redirect_title,
                summary, revision_timestamp, content_hash, filter_status, filter_reason,
                parse_status, quality_flag, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 'ok', ?12, ?13)
             ON CONFLICT(job_id, page_id) DO UPDATE SET
                revision_id = excluded.revision_id,
                raw_title = excluded.raw_title,
                normalized_title = excluded.normalized_title,
                redirect_title = excluded.redirect_title,
                summary = excluded.summary,
                revision_timestamp = excluded.revision_timestamp,
                content_hash = excluded.content_hash,
                filter_status = excluded.filter_status,
                filter_reason = excluded.filter_reason,
                quality_flag = excluded.quality_flag
             WHERE glossary_import_staging.publish_status = 'pending'",
        )?;
        for row in rows {
            statement.execute(params![
                job_id,
                row.page_id,
                row.revision_id,
                row.raw_title,
                row.normalized_title,
                row.redirect_title,
                row.summary,
                row.revision_timestamp,
                row.content_hash,
                row.filter_status,
                row.filter_reason,
                row.quality_flag,
                now
            ])?;
        }
    }
    transaction.commit()?;
    Ok(rows.len())
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchValidation {
    pub accepted: u64,
    pub redirects: u64,
    pub filtered: u64,
    pub title_conflicts: u64,
    pub alias_conflicts: u64,
    pub publishable: u64,
}

/// 批次校验：过滤分布、过滤原因分布、同名冲突与别名冲突。
pub fn validate_batch(db: &Database, job_id: &str) -> Result<BatchValidation, AppError> {
    let connection = db.connect()?;
    let mut validation = BatchValidation::default();
    {
        let mut statement = connection.prepare(
            "SELECT filter_status, count(*) FROM glossary_import_staging
              WHERE job_id = ?1 GROUP BY filter_status",
        )?;
        let rows = statement
            .query_map([job_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
            .map_err(AppError::from)?;
        for row in rows {
            let (status, count) = row.map_err(AppError::from)?;
            match status.as_str() {
                FILTER_ACCEPTED => validation.accepted = count as u64,
                FILTER_REDIRECT => validation.redirects = count as u64,
                _ => validation.filtered = count as u64,
            }
        }
    }
    // 同名但不是同一个维基 page id：不得自动合并。
    validation.title_conflicts = connection
        .query_row(
            "SELECT count(*) FROM glossary_import_staging s
               JOIN glossary_terms t ON t.term = s.normalized_title
              WHERE s.job_id = ?1 AND s.filter_status = 'accepted'
                AND (t.source <> 'wikipedia' OR t.external_page_id IS NULL OR t.external_page_id <> s.page_id)",
            params![job_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .unwrap_or_default() as u64;
    validation.alias_conflicts = connection
        .query_row(
            "SELECT count(*) FROM glossary_import_staging s
               JOIN glossary_terms t
                 ON t.term = (SELECT normalized_title FROM glossary_import_staging x
                               WHERE x.job_id = s.job_id AND x.page_id = s.page_id)
              WHERE s.job_id = ?1 AND s.filter_status = 'redirect'
                AND t.source = 'wikipedia' AND t.external_page_id IS NOT NULL
                AND t.external_page_id <> s.page_id",
            params![job_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()?
        .unwrap_or_default() as u64;
    validation.publishable = validation.accepted.saturating_sub(validation.title_conflicts);
    Ok(validation)
}

/// 过滤原因与错误原因的分布，写进导入报告。
pub fn reason_distribution(
    db: &Database,
    job_id: &str,
) -> Result<(HashMap<String, u64>, HashMap<String, u64>, HashMap<String, u64>), AppError> {
    let connection = db.connect()?;
    let mut filters: HashMap<String, u64> = HashMap::new();
    {
        let mut statement = connection.prepare(
            "SELECT coalesce(filter_reason,'unknown'), count(*)
               FROM glossary_import_staging
              WHERE job_id = ?1 AND filter_status = 'filtered'
              GROUP BY filter_reason",
        )?;
        let rows = statement
            .query_map([job_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
            .map_err(AppError::from)?;
        for row in rows {
            let (reason, count) = row.map_err(AppError::from)?;
            filters.insert(reason, count as u64);
        }
    }
    let mut issues: HashMap<String, u64> = HashMap::new();
    {
        let mut statement = connection.prepare(
            "SELECT record_type, count(*) FROM glossary_import_issues WHERE job_id = ?1 GROUP BY record_type",
        )?;
        let rows = statement
            .query_map([job_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
            .map_err(AppError::from)?;
        for row in rows {
            let (kind, count) = row.map_err(AppError::from)?;
            issues.insert(kind, count as u64);
        }
    }
    let mut codes: HashMap<String, u64> = HashMap::new();
    {
        let mut statement = connection.prepare(
            "SELECT code, count(*) FROM glossary_import_issues WHERE job_id = ?1 GROUP BY code ORDER BY count(*) DESC",
        )?;
        let rows = statement
            .query_map([job_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
            .map_err(AppError::from)?;
        for row in rows {
            let (code, count) = row.map_err(AppError::from)?;
            codes.insert(code, count as u64);
        }
    }
    Ok((filters, issues, codes))
}

/// 解析完一轮之后统一解析重定向：把 redirect_title 改写成最终目标，
/// 断链/环/超深落到 filtered 并写 issue。
pub fn resolve_batch_redirects(
    db: &Database,
    job_id: &str,
    max_depth: usize,
) -> Result<(u64, u64), AppError> {
    let connection = db.connect()?;
    let redirect_map: HashMap<String, String> = {
        let mut statement = connection.prepare(
            "SELECT normalized_title, redirect_title FROM glossary_import_staging
              WHERE job_id = ?1 AND filter_status = 'redirect' AND redirect_title IS NOT NULL",
        )?;
        let rows = statement
            .query_map([job_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(AppError::from)?;
        let mut map = HashMap::new();
        for row in rows {
            let (from, to) = row.map_err(AppError::from)?;
            map.insert(normalize_search_key(&from), normalize_search_key(&to));
        }
        map
    };
    // 本批次真实存在的词条：搜索键 -> 展示标题。
    let known: HashMap<String, String> = {
        let mut statement = connection.prepare(
            "SELECT normalized_title FROM glossary_import_staging
              WHERE job_id = ?1 AND filter_status = 'accepted'",
        )?;
        let rows = statement
            .query_map([job_id], |row| row.get::<_, String>(0))
            .map_err(AppError::from)?;
        let mut map = HashMap::new();
        for row in rows {
            let display = row.map_err(AppError::from)?;
            map.insert(normalize_search_key(&display), display);
        }
        map
    };

    let rows: Vec<(i64, String, String)> = {
        let mut statement = connection.prepare(
            "SELECT page_id, normalized_title, redirect_title FROM glossary_import_staging
              WHERE job_id = ?1 AND filter_status = 'redirect' AND redirect_title IS NOT NULL",
        )?;
        let mapped = statement
            .query_map([job_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(AppError::from)?;
        mapped.collect::<Result<Vec<_>, _>>().map_err(AppError::from)?
    };

    let mut resolved = 0_u64;
    let mut broken = 0_u64;
    let transaction = connection.unchecked_transaction()?;
    for (page_id, title, target) in rows {
        let outcome = super::redirect::resolve_redirect(&normalize_search_key(&target), &redirect_map, max_depth);
        let resolved_key = match &outcome {
            RedirectOutcome::Resolved { final_title, .. } => final_title.clone(),
            other => {
                broken += 1;
                log_issue_on(
                    &transaction,
                    job_id,
                    "validation_error",
                    other.code(),
                    &format!("{} -> {}", title, target),
                    Some(page_id),
                    &title,
                    false,
                )?;
                transaction.execute(
                    "UPDATE glossary_import_staging
                        SET filter_status = 'filtered', filter_reason = ?3
                      WHERE job_id = ?1 AND page_id = ?2",
                    params![job_id, page_id, other.code()],
                )?;
                continue;
            }
        };
        // 存展示标题而不是搜索键：发布时靠它和名词主表的 term 对齐。
        let Some(resolved_target) = known.get(&resolved_key).cloned() else {
            broken += 1;
            log_issue_on(
                &transaction,
                job_id,
                "validation_error",
                "broken_redirect",
                &format!("{} -> {}（目标词条不在本批数据中）", title, resolved_key),
                Some(page_id),
                &title,
                false,
            )?;
            transaction.execute(
                "UPDATE glossary_import_staging
                    SET filter_status = 'filtered', filter_reason = 'broken_redirect'
                  WHERE job_id = ?1 AND page_id = ?2",
                params![job_id, page_id],
            )?;
            continue;
        };
        transaction.execute(
            "UPDATE glossary_import_staging
                SET redirect_title = ?3
              WHERE job_id = ?1 AND page_id = ?2",
            params![job_id, page_id, resolved_target],
        )?;
        resolved += 1;
    }
    transaction.commit()?;
    Ok((resolved, broken))
}

/// 发布：把 staging 的 accepted 行写进名词主表，人工字段绝不覆盖。
/// 分批处理并记录游标，中断后可以继续且不会重复插入。
pub fn publish_batch(
    db: &Database,
    job_id: &str,
    dump_version: &str,
    chunk: usize,
) -> Result<PublishSummary, AppError> {
    let mut summary = PublishSummary::default();
    let mut connection = db.connect()?;
    let mut cursor = 0_i64;
    loop {
        let transaction = connection.transaction()?;
        let rows = fetch_publish_chunk(&transaction, job_id, cursor, chunk)?;
        if rows.is_empty() {
            transaction.commit()?;
            break;
        }
        cursor = rows.last().map(|(id, _)| *id).unwrap_or(cursor);
        for (id, row) in rows {
            match publish_row(&transaction, job_id, &row, dump_version) {
                Ok(outcome) => {
                    match outcome {
                        PublishOutcome::Inserted => summary.inserted += 1,
                        PublishOutcome::Updated => summary.updated += 1,
                        PublishOutcome::Skipped => summary.skipped += 1,
                        PublishOutcome::Conflict => summary.conflicts += 1,
                    }
                    transaction.execute(
                        "UPDATE glossary_import_staging
                            SET publish_status = ?3, target_term_id = ?4
                          WHERE id = ?1 AND job_id = ?2",
                        params![
                            id,
                            job_id,
                            match outcome {
                                PublishOutcome::Inserted | PublishOutcome::Updated => PUBLISH_DONE,
                                PublishOutcome::Skipped => PUBLISH_SKIPPED,
                                PublishOutcome::Conflict => PUBLISH_CONFLICT,
                            },
                            match outcome {
                                PublishOutcome::Inserted | PublishOutcome::Updated | PublishOutcome::Skipped => publish_row_target_id(&transaction, id, job_id),
                                PublishOutcome::Conflict => None,
                            }
                        ],
                    )?;
                }
                Err(error) => {
                    log_issue_on(
                        &transaction,
                        job_id,
                        "validation_error",
                        "publish_failed",
                        &error.to_string(),
                        Some(row.page_id),
                        &row.normalized_title,
                        true,
                    )?;
                    transaction.execute(
                        "UPDATE glossary_import_staging SET publish_status = 'conflict' WHERE id = ?1",
                        params![id],
                    )?;
                    summary.conflicts += 1;
                }
            }
        }
        transaction.commit()?;
    }
    summary.aliases_inserted = publish_aliases(db, job_id, dump_version)?;
    Ok(summary)
}

fn publish_row_target_id(
    connection: &rusqlite::Connection,
    staging_id: i64,
    job_id: &str,
) -> Option<i64> {
    connection
        .query_row(
            "SELECT target_term_id FROM glossary_import_staging WHERE id = ?1 AND job_id = ?2",
            params![staging_id, job_id],
            |row| row.get(0),
        )
        .optional()
        .ok()
        .flatten()
}

type PublishRow = StagingRow;

fn fetch_publish_chunk(
    transaction: &Transaction<'_>,
    job_id: &str,
    cursor: i64,
    chunk: usize,
) -> Result<Vec<(i64, PublishRow)>, AppError> {
    let mut statement = transaction.prepare(
        "SELECT id, page_id, coalesce(revision_id, 0), raw_title, normalized_title,
                coalesce(summary, ''), coalesce(revision_timestamp, ''), coalesce(content_hash, '')
           FROM glossary_import_staging
          WHERE job_id = ?1 AND id > ?2 AND filter_status = 'accepted' AND publish_status = 'pending'
          ORDER BY id LIMIT ?3",
    )?;
    let rows = statement
        .query_map(params![job_id, cursor, chunk as i64], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                StagingRow {
                    page_id: row.get(1)?,
                    revision_id: row.get(2)?,
                    raw_title: row.get(3)?,
                    normalized_title: row.get(4)?,
                    redirect_title: None,
                    summary: row.get(5)?,
                    revision_timestamp: row.get(6)?,
                    content_hash: row.get(7)?,
                    filter_status: FILTER_ACCEPTED.to_string(),
                    filter_reason: None,
                    quality_flag: None,
                },
            ))
        })
        .map_err(AppError::from)?;
    let rows = rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)?;
    Ok(rows)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublishOutcome {
    Inserted,
    Updated,
    Skipped,
    Conflict,
}

/// 需求 7.8 的冲突矩阵在这里落地。
fn publish_row(
    transaction: &Transaction<'_>,
    job_id: &str,
    row: &PublishRow,
    dump_version: &str,
) -> Result<PublishOutcome, AppError> {
    let now = now_seconds();
    let existing: Option<(i64, String, i64, String, i64, String)> = transaction
        .query_row(
            "SELECT id, source, coalesce(external_page_id, 0), coalesce(source_content_hash, ''),
                    coalesce(manually_edited, 0), coalesce(wikipedia_snapshot, '')
               FROM glossary_terms WHERE term = ?1",
            params![row.normalized_title],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                ))
            },
        )
        .optional()?;

    let Some((term_id, source, external_page_id, content_hash, manually_edited, _snapshot)) = existing
    else {
        // 情况 1：没有同名记录 -> 新建待确认候选。
        transaction.execute(
            "INSERT INTO glossary_terms(
                term, canonical_name, aliases_json, definition, source, source_title, source_url,
                wikipedia_snapshot, status, updated_at, external_page_id, source_revision_id,
                source_dump_version, source_updated_at, source_synced_at, license_code,
                manually_edited, source_content_hash, published_batch_id, normalized_term)
             VALUES (?1, ?1, '[]', ?2, 'wikipedia', ?3, ?4, ?2, 'pending', ?5, ?6, ?7, ?8, ?9, ?5, ?10, 0, ?11, ?12, ?13)",
            params![
                row.normalized_title,
                row.summary,
                row.raw_title,
                super::title::page_url(&row.normalized_title),
                now,
                row.page_id,
                row.revision_id,
                dump_version,
                parse_timestamp(&row.revision_timestamp),
                WIKIPEDIA_LICENSE,
                row.content_hash,
                job_id,
                normalize_search_key(&row.normalized_title)
            ],
        )?;
        return Ok(PublishOutcome::Inserted);
    };

    let same_page = source == "wikipedia" && external_page_id == row.page_id;
    if same_page {
        // 情况 2/4：同一 page id。人工编辑过的只更新来源快照。
        if content_hash == row.content_hash {
            transaction.execute(
                "UPDATE glossary_terms
                    SET source_revision_id = ?2, source_dump_version = ?3, source_updated_at = ?4,
                        source_synced_at = ?5, published_batch_id = ?6, updated_at = ?5
                  WHERE id = ?1",
                params![
                    term_id,
                    row.revision_id,
                    dump_version,
                    parse_timestamp(&row.revision_timestamp),
                    now,
                    job_id
                ],
            )?;
            return Ok(PublishOutcome::Skipped);
        }
        if manually_edited == 1 {
            transaction.execute(
                "UPDATE glossary_terms
                    SET wikipedia_snapshot = ?2, source_content_hash = ?3, source_revision_id = ?4,
                        source_dump_version = ?5, source_updated_at = ?6, source_synced_at = ?7,
                        published_batch_id = ?8, updated_at = ?7
                  WHERE id = ?1",
                params![
                    term_id,
                    row.summary,
                    row.content_hash,
                    row.revision_id,
                    dump_version,
                    parse_timestamp(&row.revision_timestamp),
                    now,
                    job_id
                ],
            )?;
            return Ok(PublishOutcome::Updated);
        }
        transaction.execute(
            "UPDATE glossary_terms
                SET definition = ?2, wikipedia_snapshot = ?2, source_content_hash = ?3,
                    source_revision_id = ?4, source_dump_version = ?5, source_updated_at = ?6,
                    source_synced_at = ?7, source_url = ?8, license_code = ?9,
                    published_batch_id = ?10, updated_at = ?7,
                    status = CASE WHEN status = 'source_missing' THEN 'pending' ELSE status END
              WHERE id = ?1",
            params![
                term_id,
                row.summary,
                row.content_hash,
                row.revision_id,
                dump_version,
                parse_timestamp(&row.revision_timestamp),
                now,
                super::title::page_url(&row.normalized_title),
                WIKIPEDIA_LICENSE,
                job_id
            ],
        )?;
        return Ok(PublishOutcome::Updated);
    }

    // 情况 3/5：同名但不是同一个 page id，或人工创建的同名词条。
    // 一律不覆盖，进冲突队列。
    log_issue_on(
        transaction,
        job_id,
        "name_conflict",
        if source == "wikipedia" { "page_id_mismatch" } else { "manual_name_conflict" },
        &format!(
            "{} 已存在（来源 {}，外部 id {}），维基 page {} 不自动合并",
            row.normalized_title, source, external_page_id, row.page_id
        ),
        Some(row.page_id),
        &row.normalized_title,
        false,
    )?;
    Ok(PublishOutcome::Conflict)
}

/// 把重定向结果写成别名。别名唯一约束是 (term_id, normalized_alias, alias_type)，
/// 因此同一条目重复导入不会产生重复别名。
pub fn publish_aliases(db: &Database, job_id: &str, _dump_version: &str) -> Result<u64, AppError> {
    let mut connection = db.connect()?;
    let pairs: Vec<(i64, String, String)> = {
        let mut statement = connection.prepare(
            "SELECT t.id, s.normalized_title, s.redirect_title
               FROM glossary_import_staging s
               JOIN glossary_terms t ON t.term = s.redirect_title
              WHERE s.job_id = ?1 AND s.filter_status = 'redirect'
                AND s.redirect_title IS NOT NULL",
        )?;
        let rows = statement
            .query_map([job_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(AppError::from)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)?
    };
    if pairs.is_empty() {
        return Ok(0);
    }
    let transaction = connection.transaction()?;
    let now = now_seconds();
    let mut inserted = 0_u64;
    {
        let mut statement = transaction.prepare(
            "INSERT INTO glossary_term_aliases(term_id, alias, normalized_alias, alias_type, source, external_page_id, created_at, updated_at)
             VALUES (?1, ?2, ?3, 'redirect', 'wikipedia', ?4, ?5, ?5)
             ON CONFLICT(term_id, normalized_alias, alias_type) DO NOTHING",
        )?;
        for (term_id, alias, target) in pairs {
            let normalized = normalize_search_key(&alias);
            if normalized.is_empty() || normalized == normalize_search_key(&target) {
                continue;
            }
            let changed = statement.execute(params![term_id, alias, normalized, target, now])?;
            inserted += changed as u64;
        }
    }
    transaction.commit()?;
    // 别名回写进 aliases_json，AI 抽取路径读的是这一列。
    sync_aliases_json(&connection, job_id)?;
    Ok(inserted)
}

fn sync_aliases_json(connection: &rusqlite::Connection, job_id: &str) -> Result<(), AppError> {
    let targets: Vec<(i64, String)> = {
        let mut statement = connection.prepare(
            "SELECT DISTINCT t.id, t.term
               FROM glossary_import_staging s
               JOIN glossary_terms t ON t.term = s.redirect_title
              WHERE s.job_id = ?1 AND s.filter_status = 'redirect'",
        )?;
        let rows = statement
            .query_map([job_id], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)))
            .map_err(AppError::from)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)?
    };
    let mut update = connection.prepare("UPDATE glossary_terms SET aliases_json = ?2 WHERE id = ?1")?;
    for (term_id, _) in targets {
        let mut alias_statement = connection.prepare(
            "SELECT alias FROM glossary_term_aliases WHERE term_id = ?1 ORDER BY alias",
        )?;
        let mut aliases: Vec<String> = alias_statement
            .query_map([term_id], |row| row.get::<_, String>(0))
            .map_err(AppError::from)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(AppError::from)?;
        aliases.sort();
        update.execute(params![term_id, serde_json::to_string(&aliases)?])?;
    }
    Ok(())
}

/// 新 dump 里消失的维基词条只标记来源失效，不物理删除（需求 7.8 最后一行）。
/// 只有整批（非 --limit）导入才允许执行。
pub fn mark_missing_sources(db: &Database, job_id: &str) -> Result<u64, AppError> {
    let mut connection = db.connect()?;
    let transaction = connection.transaction()?;
    let affected = transaction.execute(
        "UPDATE glossary_terms
            SET status = 'source_missing', updated_at = ?2
          WHERE source = 'wikipedia'
            AND external_page_id IS NOT NULL
            AND status <> 'ignored'
            AND NOT EXISTS (SELECT 1 FROM glossary_import_staging s
                             WHERE s.job_id = ?1 AND s.filter_status = 'accepted'
                               AND s.page_id = glossary_terms.external_page_id)",
        params![job_id, now_seconds()],
    )?;
    transaction.commit()?;
    Ok(affected as u64)
}

/// 清理旧 staging：只删属于已结束任务、且超过保留天数的批次。
pub fn prune_staging(db: &Database, retention_days: i64) -> Result<u64, AppError> {
    let connection = db.connect()?;
    let affected = connection.execute(
        "DELETE FROM glossary_import_staging
          WHERE job_id IN (SELECT id FROM glossary_import_jobs
                            WHERE status IN ('completed','cancelled','failed')
                              AND finished_at IS NOT NULL
                              AND finished_at < ?1)",
        params![now_seconds() - retention_days.max(0) * 86_400],
    )?;
    Ok(affected as u64)
}

/// 词条来源失效但人工确认过的数据要捞回来（人工优先，需求 5.1）。
pub fn count_by_status(db: &Database) -> Result<Vec<(String, i64)>, AppError> {
    let connection = db.connect()?;
    let mut statement = connection.prepare(
        "SELECT status, count(*) FROM glossary_terms GROUP BY status ORDER BY count(*) DESC",
    )?;
    let rows = statement
        .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)))
        .map_err(AppError::from)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(AppError::from)
}

/// 维基修订时间 `2026-09-01T02:03:04Z` -> unix 秒。
pub fn parse_timestamp(value: &str) -> Option<i64> {
    if value.len() < 19 {
        return None;
    }
    let year: i64 = value.get(0..4)?.parse().ok()?;
    let month: i64 = value.get(5..7)?.parse().ok()?;
    let day: i64 = value.get(8..10)?.parse().ok()?;
    let hour: i64 = value.get(11..13)?.parse().ok()?;
    let minute: i64 = value.get(14..16)?.parse().ok()?;
    let second: i64 = value.get(17..19)?.parse().ok()?;
    // 以 UTC 基准构造再按天数折算，避免引入时区库。
    let days = days_from_civil(year, month, day);
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

/// Howard Hinnant 的 days_from_civil 算法。
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_prime = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub fn truncate(value: &str, limit: usize) -> String {
    if value.chars().count() <= limit {
        return value.to_string();
    }
    value.chars().take(limit).collect()
}
