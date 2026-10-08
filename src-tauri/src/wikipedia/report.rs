//! 导入报告（需求 14）。任务结束时生成，结构化存进任务的 report_json。

use crate::database::Database;
use crate::error::AppError;
use crate::wikipedia::store::{self, JobStatus};
use serde_json::{json, Value};

pub fn build_report(db: &Database, job_id: &str) -> Result<Value, AppError> {
    let job = store::get_job(db, job_id)?
        .ok_or_else(|| AppError::Message(format!("任务不存在：{job_id}")))?;
    let config = store::job_config(db, job_id)?;
    let (filters, issue_kinds, issue_codes) = store::reason_distribution(db, job_id)?;
    let validation = store::validate_batch(db, job_id)?;
    let examples: Vec<Value> = store::list_issues(db, job_id, None, 20)?
        .into_iter()
        .map(|issue| {
            json!({
                "code": issue.code,
                "recordType": issue.record_type,
                "title": issue.title,
                "message": issue.message,
            })
        })
        .collect();
    let before = job.inserted_count + job.skipped_count;
    let status = JobStatus::parse(&job.status)?;
    // 只有冲突/发布类问题才算部分成功：过滤掉消歧义、断链这类正常损耗
    // 不代表导入不完整。
    let hard_errors = store::list_issues(db, job_id, Some("parse_error"), 1)
        .map(|issues| issues.len())
        .unwrap_or(0);
    let conclusion = match status {
        JobStatus::Completed if job.conflict_count > 0 || hard_errors > 0 => "partial_success",
        JobStatus::Completed => "success",
        JobStatus::Cancelled => "cancelled",
        JobStatus::Paused => "paused",
        JobStatus::Failed => "failed",
        _ => "in_progress",
    };
    Ok(json!({
        "jobId": job_id,
        "status": job.status,
        "conclusion": conclusion,
        "source": {
            "url": job.source_url,
            "dumpVersion": job.dump_version,
            "license": crate::wikipedia::config::WIKIPEDIA_LICENSE,
        },
        "config": config,
        "counts": {
            "scanned": job.scanned_count,
            "accepted": job.accepted_count,
            "redirects": job.redirect_count,
            "filtered": job.filtered_count,
            "inserted": job.inserted_count,
            "updated": job.updated_count,
            "skipped": job.skipped_count,
            "conflicts": job.conflict_count,
            "errors": job.error_count,
        },
        "validation": {
            "accepted": validation.accepted,
            "redirects": validation.redirects,
            "filtered": validation.filtered,
            "titleConflicts": validation.title_conflicts,
            "aliasConflicts": validation.alias_conflicts,
            "publishable": validation.publishable,
        },
        "filterReasons": filters,
        "issueKinds": issue_kinds,
        "issueCodes": issue_codes,
        "termCountBeforePublish": before,
        "bytes": {
            "total": job.total_bytes,
            "downloaded": job.downloaded_bytes,
        },
        "timing": {
            "startedAt": job.started_at,
            "finishedAt": job.finished_at,
            "bytesPerSecond": job.bytes_per_second,
        },
        "errorExamples": examples,
        "errorMessage": job.error_message,
    }))
}

/// 报告转成一行 JSON，供 CLI 输出机器可读摘要。
pub fn to_compact_json(report: &Value) -> String {
    serde_json::to_string(report).unwrap_or_else(|_| "{}".to_string())
}
