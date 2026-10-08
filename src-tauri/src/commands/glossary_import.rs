//! 名词库维基导入的管理命令。
//!
//! 长任务绝不在命令生命周期内跑：命令只负责建任务、查进度、暂停/取消/发布，
//! 实际流水线在后台线程里执行，与 CLI 走的是同一套 store 逻辑。
//!
//! 权限说明：这是本地单用户桌面应用，没有账号体系。等价于“管理员”的边界是
//! 本机用户本身，因此所有写操作都写审计日志，且导入的写库内容永远不覆盖人工字段。

use crate::database::Database;
use crate::error::AppError;
use crate::wikipedia::config::WikipediaConfig;
use crate::wikipedia::report;
use crate::wikipedia::runner;
use crate::wikipedia::store::{self, ImportIssue, ImportJob, ImportRequest, JobStatus};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Manager, State};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryImportRequest {
    pub source_url: Option<String>,
    pub dump_version: Option<String>,
    pub mode: Option<String>,
    pub handle_redirects: Option<bool>,
    pub filter_disambiguation: Option<bool>,
    pub filter_list_pages: Option<bool>,
    pub auto_publish: Option<bool>,
    pub max_items: Option<u64>,
    pub batch_size: Option<usize>,
    pub concurrency: Option<usize>,
    pub max_retries: Option<u32>,
    pub temp_dir: Option<String>,
    /// 本地 dump 文件路径：离线与试跑用，跳过下载与 MD5。
    pub local_file: Option<String>,
}

impl GlossaryImportRequest {
    fn into_request(self) -> Result<ImportRequest, AppError> {
        let mode = match self.mode.as_deref() {
            None | Some("") => None,
            Some(value) => Some(crate::wikipedia::config::ImportMode::parse(value)?),
        };
        Ok(ImportRequest {
            source_url: self.source_url,
            dump_version: self.dump_version,
            mode,
            handle_redirects: self.handle_redirects,
            filter_disambiguation: self.filter_disambiguation,
            filter_list_pages: self.filter_list_pages,
            auto_publish: self.auto_publish,
            max_items: self.max_items,
            batch_size: self.batch_size,
            concurrency: self.concurrency,
            max_retries: self.max_retries,
            temp_dir: self.temp_dir.map(std::path::PathBuf::from),
            local_file: self.local_file.map(std::path::PathBuf::from),
        })
    }
}

fn import_config(app: &AppHandle) -> Result<WikipediaConfig, AppError> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|error| AppError::Message(format!("无法定位应用数据目录：{error}")))?;
    WikipediaConfig::from_env(&dir)
}

#[tauri::command]
pub fn create_glossary_import(
    app: AppHandle,
    db: State<'_, Database>,
    request: Option<GlossaryImportRequest>,
) -> Result<ImportJob, AppError> {
    let config = import_config(&app)?;
    config.ensure_temp_dir()?;
    let request = request.unwrap_or_default().into_request()?;
    let job = store::create_job(db.inner(), &request, &config)?;
    spawn_pipeline(app, db.inner(), job.id.clone(), config);
    Ok(job)
}

/// 后台线程里跑流水线。CLI 用同一个 run_pipeline，行为一致。
fn spawn_pipeline(app: AppHandle, db: &Database, job_id: String, config: WikipediaConfig) {
    let handle = match db.reopen() {
        Ok(handle) => Arc::new(handle),
        Err(_) => return,
    };
    // 线程内的配置已经带上了应用数据目录，app handle 只用于保持窗口存活。
    let _ = app;
    std::thread::spawn(move || {
        let _ = runner::run_pipeline(handle, job_id, config);
    });
}

#[tauri::command]
pub fn list_glossary_imports(db: State<'_, Database>, limit: Option<usize>) -> Result<Vec<ImportJob>, AppError> {
    store::list_jobs(db.inner(), limit.unwrap_or(50))
}

#[tauri::command]
pub fn get_glossary_import(db: State<'_, Database>, id: String) -> Result<ImportJob, AppError> {
    store::get_job(db.inner(), &id)?
        .ok_or_else(|| AppError::Message("导入任务不存在".to_string()))
}

#[tauri::command]
pub fn pause_glossary_import(db: State<'_, Database>, id: String) -> Result<ImportJob, AppError> {
    store::pause_job(db.inner(), &id)?;
    store::get_job(db.inner(), &id)?
        .ok_or_else(|| AppError::Message("导入任务不存在".to_string()))
}

#[tauri::command]
pub fn resume_glossary_import(
    app: AppHandle,
    db: State<'_, Database>,
    id: String,
) -> Result<ImportJob, AppError> {
    let config = import_config(&app)?;
    store::resume_job(db.inner(), &id)?;
    spawn_pipeline(app, db.inner(), id.clone(), config);
    store::get_job(db.inner(), &id)?
        .ok_or_else(|| AppError::Message("导入任务不存在".to_string()))
}

#[tauri::command]
pub fn cancel_glossary_import(db: State<'_, Database>, id: String) -> Result<ImportJob, AppError> {
    store::cancel_job(db.inner(), &id)?;
    store::get_job(db.inner(), &id)?
        .ok_or_else(|| AppError::Message("导入任务不存在".to_string()))
}

#[tauri::command]
pub fn publish_glossary_import(app: AppHandle, db: State<'_, Database>, id: String) -> Result<ImportJob, AppError> {
    let job = store::get_job(db.inner(), &id)?
        .ok_or_else(|| AppError::Message("导入任务不存在".to_string()))?;
    let status = JobStatus::parse(&job.status)?;
    if status != JobStatus::ReadyToPublish {
        return Err(AppError::Message(format!(
            "只有 ready_to_publish 的任务可以发布，当前是 {}",
            status.as_str()
        )));
    }
    store::transition(db.inner(), &id, JobStatus::Publishing)?;
    let config = import_config(&app)?;
    spawn_pipeline(app, db.inner(), id.clone(), config);
    store::get_job(db.inner(), &id)?
        .ok_or_else(|| AppError::Message("导入任务不存在".to_string()))
}

#[tauri::command]
pub fn glossary_import_errors(
    db: State<'_, Database>,
    id: String,
    record_type: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<ImportIssue>, AppError> {
    store::list_issues(db.inner(), &id, record_type.as_deref(), limit.unwrap_or(200))
}

#[tauri::command]
pub fn glossary_import_report(db: State<'_, Database>, id: String) -> Result<serde_json::Value, AppError> {
    report::build_report(db.inner(), &id)
}

/// 清理结束任务的 staging。返回删除行数。
#[tauri::command]
pub fn cleanup_glossary_import(
    app: AppHandle,
    db: State<'_, Database>,
    retention_days: Option<i64>,
) -> Result<u64, AppError> {
    let config = import_config(&app)?;
    let days = retention_days.unwrap_or(config.staging_retention_days);
    let removed = store::prune_staging(db.inner(), days)?;
    store::audit(&db.connect()?, "staging_pruned", None, &format!("retention_days={days} rows={removed}"))?;
    Ok(removed)
}

#[tauri::command]
pub fn list_glossary_review_queue(
    db: State<'_, Database>,
    status: Option<String>,
    source: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<crate::models::GlossaryTerm>, AppError> {
    let mut terms =
        super::glossary_terms_for_review(db.inner(), None, source.as_deref(), status.as_deref())?;
    terms.truncate(limit.unwrap_or(200));
    Ok(terms)
}

/// 单条确认/忽略/恢复待确认。
#[tauri::command]
pub fn set_glossary_term_status(
    db: State<'_, Database>,
    id: i64,
    status: String,
) -> Result<(), AppError> {
    super::set_glossary_term_status(db.inner(), id, &status)
}

/// 批量确认/忽略，用于列表页的“全部确认”。
#[tauri::command]
pub fn bulk_set_glossary_term_status(
    db: State<'_, Database>,
    ids: Vec<i64>,
    status: String,
) -> Result<u64, AppError> {
    super::bulk_set_glossary_term_status(db.inner(), &ids, &status)
}
