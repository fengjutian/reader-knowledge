//! 导入流水线：下载 -> 校验 -> 解析 -> 重定向 -> 校验 -> 发布。
//!
//! 长任务不在 HTTP/命令调用生命周期里跑：桌面端用后台线程，CLI 用独立进程，
//! 两者都只通过数据库里的 status 通信，暂停/取消在批次边界生效。

use crate::database::Database;
use crate::error::AppError;
use crate::wikipedia::config::{ImportMode, WikipediaConfig};
use crate::wikipedia::download;
use crate::wikipedia::parser::{self, Flow, ParseIssue, WikiPage};
use crate::wikipedia::redirect::DEFAULT_MAX_REDIRECT_DEPTH;
use crate::wikipedia::store::{self, ImportRequest, JobStatus, PublishSummary, StagingRow};
use crate::wikipedia::title::{looks_like_list_page, normalize_search_key, normalize_title};
use crate::wikipedia::wikitext::{SummaryOptions, SummaryQuality, extract_summary, soft_redirect_target};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// 清洗后短于这个长度就当作信息量不足过滤掉。
const MIN_USEFUL_CHARS: usize = 12;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Signal {
    Pause,
    Cancel,
}

#[derive(Debug, Clone)]
pub struct RunSummary {
    pub job_id: String,
    pub status: JobStatus,
    pub scanned: u64,
    pub published: Option<PublishSummary>,
    pub message: String,
}

/// 一次完整流水线。`job_id` 必须是已存在的任务。
pub fn run_pipeline(
    db: Arc<Database>,
    job_id: String,
    config: WikipediaConfig,
) -> Result<RunSummary, AppError> {
    config.ensure_temp_dir()?;
    let lock_path = config.temp_dir.join(format!("wikipedia-job-{job_id}.lock"));
    let lock_file = OpenOptions::new().create(true).read(true).write(true).open(&lock_path)
        .map_err(|error| AppError::Message(format!("无法打开任务锁 {}：{error}", lock_path.display())))?;
    if lock_file.try_lock().is_err() {
        let current = store::get_job(&db, &job_id)?
            .ok_or_else(|| AppError::Message(format!("导入任务不存在：{job_id}")))?;
        return Ok(RunSummary {
            job_id,
            status: JobStatus::parse(&current.status)?,
            scanned: current.scanned_count as u64,
            published: None,
            message: "任务已有执行器运行，本次启动已安全忽略".to_string(),
        });
    }
    let outcome = execute(&db, &job_id, &config);
    match outcome {
        Ok(summary) => {
            if let Ok(report) = crate::wikipedia::report::build_report(&db, &job_id) {
                let _ = store::save_report(&db, &job_id, &report);
            }
            Ok(summary)
        }
        Err(error) => {
            let _ = store::mark_failed(&db, &job_id, &error.to_string());
            Err(error)
        }
    }
}

fn execute(db: &Database, job_id: &str, config: &WikipediaConfig) -> Result<RunSummary, AppError> {
    let job = store::get_job(db, job_id)?
        .ok_or_else(|| AppError::Message(format!("任务不存在：{job_id}")))?;
    let job_config = store::job_config(db, job_id)?;
    let mut status = JobStatus::parse(&job.status)?;
    if status.is_terminal() {
        return Ok(RunSummary {
            job_id: job_id.to_string(),
            status,
            scanned: 0,
            published: None,
            message: "任务已处于终态，无需重跑".to_string(),
        });
    }

    let request = ImportRequest {
        local_file: job_config
            .get("localFile")
            .and_then(json_string)
            .map(PathBuf::from),
        ..ImportRequest::default()
    };
    let max_items = job_config.get("maxItems").and_then(json_u64);
    let batch_size = job_config
        .get("batchSize")
        .and_then(json_u64)
        .unwrap_or(config.batch_size as u64)
        .max(1) as usize;
    let mode = ImportMode::parse(&job.mode)?;
    let filter_disambiguation = job_config
        .get("filterDisambiguation")
        .and_then(json_bool)
        .unwrap_or(true);
    let handle_redirects = job_config
        .get("handleRedirects")
        .and_then(json_bool)
        .unwrap_or(true);
    let filter_list_pages = job_config
        .get("filterListPages")
        .and_then(json_bool)
        .unwrap_or(true);
    // 建任务时把摘要长度策略快照进 config_json，续跑时用同一套阈值。
    let summary_options = SummaryOptions {
        min_chars: job_config
            .get("summaryMinChars")
            .and_then(json_u64)
            .unwrap_or(config.summary_min_chars as u64) as usize,
        max_chars: job_config
            .get("summaryMaxChars")
            .and_then(json_u64)
            .unwrap_or(config.summary_max_chars as u64) as usize,
    };

    // 只有下载/校验/解析阶段需要 dump。后续阶段恢复时直接从 staging 继续。
    let needs_dump = matches!(status, JobStatus::Pending | JobStatus::Downloading | JobStatus::Verifying | JobStatus::Parsing);
    let dump_path: Option<PathBuf> = if needs_dump { Some(match request.local_file.clone() {
        Some(path) => {
            if !path.exists() {
                return Err(AppError::Message(format!("本地 dump 不存在：{}", path.display())));
            }
            path
        }
        None => {
            if let Some(signal) = control_signal(db, job_id)? {
                return Ok(match signal {
                    Signal::Pause => paused_summary(job_id, status),
                    Signal::Cancel => cancelled_summary(job_id),
                });
            }
            if status == JobStatus::Pending { store::transition(db, job_id, JobStatus::Downloading)?; }
            let path = download_dump(db, job_id, &job, config)?;
            if status != JobStatus::Verifying { store::transition(db, job_id, JobStatus::Verifying)?; }
            verify_dump(&path, config)?;
            path
        }
    }) } else { None };

    if control_signal(db, job_id)? == Some(Signal::Cancel) {
        return Ok(cancelled_summary(job_id));
    }

    // ---------- 2. 流式解析到 staging ----------
    if needs_dump && status != JobStatus::Parsing {
        store::transition(db, job_id, JobStatus::Parsing)?;
        status = JobStatus::Parsing;
    }
    if needs_dump {
      let staged = stage_pages(
        db,
        job_id,
        dump_path.as_deref().expect("dump path is present while parsing"),
        config,
        &summary_options,
        mode,
        filter_disambiguation,
        filter_list_pages,
        handle_redirects,
        max_items,
        batch_size,
      )?;
      if staged == Some(Signal::Pause) {
        return Ok(paused_summary(job_id, JobStatus::Parsing));
      }
      if staged == Some(Signal::Cancel) {
        return Ok(cancelled_summary(job_id));
      }
      status = JobStatus::Parsing;
    }

    // ---------- 3. 重定向解析 ----------
    if status == JobStatus::Parsing {
      store::transition(db, job_id, JobStatus::ResolvingRedirects)?;
      status = JobStatus::ResolvingRedirects;
    }
    if status == JobStatus::ResolvingRedirects {
      if let Some(signal) = control_signal(db, job_id)? {
        return Ok(match signal { Signal::Pause => paused_summary(job_id, status), Signal::Cancel => cancelled_summary(job_id) });
      }
      let (resolved, broken) = store::resolve_batch_redirects(db, job_id, DEFAULT_MAX_REDIRECT_DEPTH)?;
      if resolved > 0 || broken > 0 {
        store::log_issue(
            db,
            job_id,
            "validation_error",
            "redirect_summary",
            &format!("重定向解析完成：成功 {resolved}，异常 {broken}"),
            None,
            "",
            false,
        )?;
      }
      store::transition(db, job_id, JobStatus::Validating)?;
      status = JobStatus::Validating;
    }

    // ---------- 4. 批次校验 ----------
    if status == JobStatus::Validating {
      if let Some(signal) = control_signal(db, job_id)? {
        return Ok(match signal { Signal::Pause => paused_summary(job_id, status), Signal::Cancel => cancelled_summary(job_id) });
      }
      let validation = store::validate_batch(db, job_id)?;
    store::save_checkpoint(
        db,
        job_id,
        &serde_json::json!({
            "stage": "ready_to_publish",
            "publishable": validation.publishable,
            "titleConflicts": validation.title_conflicts,
            "aliasConflicts": validation.alias_conflicts,
        }),
    )?;
      store::transition(db, job_id, JobStatus::ReadyToPublish)?;
      status = JobStatus::ReadyToPublish;
    }

    // ---------- 5. 发布 ----------
    let mut published = None;
    if job.auto_publish && status == JobStatus::ReadyToPublish {
        if let Some(signal) = control_signal(db, job_id)? {
            return Ok(match signal {
                Signal::Pause => paused_summary(job_id, JobStatus::ReadyToPublish),
                Signal::Cancel => cancelled_summary(job_id),
            });
        }
        store::transition(db, job_id, JobStatus::Publishing)?;
        status = JobStatus::Publishing;
    }
    if status == JobStatus::Publishing {
        let mut summary = store::publish_batch(db, job_id, &job.dump_version, batch_size.max(50))?;
        // 只有整批导入才允许判定“来源失效”，--limit 的部分批次不能做这件事，
        // 否则没扫到的词条会被误标成来源消失。
        if max_items.is_none() {
            summary.source_missing = store::mark_missing_sources(db, job_id)?;
        }
        let errors = count_issue_kind(db, job_id, "parse_error")?
            + count_issue_kind(db, job_id, "validation_error")?;
        store::set_publish_counts(
            db,
            job_id,
            summary.inserted as i64,
            summary.updated as i64,
            summary.skipped as i64,
            summary.conflicts as i64,
            errors,
        )?;
        store::transition(db, job_id, JobStatus::Completed)?;
        published = Some(summary);
    }

    let final_job = store::get_job(db, job_id)?;
    Ok(RunSummary {
        job_id: job_id.to_string(),
        status: final_job
            .as_ref()
            .and_then(|job| JobStatus::parse(&job.status).ok())
            .unwrap_or(JobStatus::ReadyToPublish),
        scanned: final_job.as_ref().map(|job| job.scanned_count).unwrap_or_default() as u64,
        published,
        message: if job.auto_publish {
            "已完成".to_string()
        } else {
            "已就绪，等待人工确认后发布".to_string()
        },
    })
}

fn count_issue_kind(db: &Database, job_id: &str, record_type: &str) -> Result<i64, AppError> {
    let connection = db.connect()?;
    let count: i64 = connection.query_row(
        "SELECT count(*) FROM glossary_import_issues WHERE job_id = ?1 AND record_type = ?2",
        rusqlite::params![job_id, record_type],
        |row| row.get(0),
    )?;
    Ok(count)
}

fn download_dump(
    db: &Database,
    job_id: &str,
    job: &store::ImportJob,
    config: &WikipediaConfig,
) -> Result<PathBuf, AppError> {
    let temp_dir = config.ensure_temp_dir()?;
    let client = download::build_client(&config.user_agent, Duration::from_secs(60))?;
    let remote = download::probe(&client, &job.source_url, &config.allowed_hosts)?;
    if let Some(total) = remote.total_bytes {
        store::set_total_bytes(db, job_id, total as i64)?;
    }
    let started = Instant::now();
    let mut last_flush = Instant::now();
    let mut last_bytes = 0_u64;
    let download_result = download::download_with_resume(&client, &remote, &temp_dir, |bytes| {
        // 进度写库要节流：最多每秒一次。
        if last_flush.elapsed() < Duration::from_millis(config.progress_interval_ms) {
            return;
        }
        let elapsed = started.elapsed().as_secs_f64().max(0.001);
        let speed = (bytes as f64 - last_bytes as f64) / elapsed.max(config.progress_interval_ms as f64 / 1000.0);
        let _ = store::update_progress(
            db,
            job_id,
            None,
            Some(bytes as i64),
            None,
            None,
            None,
            None,
            Some(&remote.file_name),
            Some(speed),
        );
        last_flush = Instant::now();
        last_bytes = bytes;
    })?;
    store::update_progress(
        db,
        job_id,
        None,
        Some(download_result.bytes as i64),
        None,
        None,
        None,
        None,
        Some(&remote.file_name),
        None,
    )?;
    Ok(download_result.path)
}

fn verify_dump(path: &std::path::Path, config: &WikipediaConfig) -> Result<(), AppError> {
    let client = download::build_client(&config.user_agent, Duration::from_secs(30))?;
    let sums = download::fetch_md5sums(&client, &config.base_url, &config.allowed_hosts)?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| AppError::Message("dump 文件名无效".to_string()))?;
    let expected = sums
        .get(file_name)
        .ok_or_else(|| AppError::Message(format!("md5sums.txt 里没有 {file_name} 的校验和")))?;
    if let Err(error) = download::verify_md5(path, expected) {
        // 下载目录里的已完成/断点文件校验失败时隔离掉，下一次恢复必须重新下载，
        // 避免永久重复使用同一个等长损坏文件。
        let _ = std::fs::remove_file(path);
        return Err(error);
    }
    Ok(())
}

/// 解析 + 清洗 + 过滤 + 写 staging。返回控制信号。
#[allow(clippy::too_many_arguments)]
fn stage_pages(
    db: &Database,
    job_id: &str,
    dump_path: &std::path::Path,
    config: &WikipediaConfig,
    summary_options: &SummaryOptions,
    mode: ImportMode,
    filter_disambiguation: bool,
    filter_list_pages: bool,
    handle_redirects: bool,
    max_items: Option<u64>,
    batch_size: usize,
) -> Result<Option<Signal>, AppError> {
    // 续跑时已经在本批落库的页面直接跳过清洗，只重新扫 XML。
    let already: HashMap<i64, i64> = {
        let connection = db.connect()?;
        let mut statement = connection.prepare(
            "SELECT page_id, coalesce(revision_id, 0) FROM glossary_import_staging WHERE job_id = ?1",
        )?;
        let mut rows = statement
            .query_map([job_id], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))
            .map_err(AppError::from)?;
        let mut map = HashMap::new();
        while let Some(row) = rows.next() {
            let (page_id, revision_id) = row.map_err(AppError::from)?;
            map.insert(page_id, revision_id);
        }
        map
    };

    let reader = parser::open_dump_reader(dump_path)?;
    let mut batch: Vec<StagingRow> = Vec::with_capacity(batch_size);
    let mut counters = Counters::default();
    let mut last_flush = Instant::now();
    let mut emitted = 0_u64;

    // on_issue 只写库，不碰共享计数器：否则两个闭包会同时可变借用 counters。
    let mut on_issue = |issue: ParseIssue| {
        let _ = store::log_issue(
            db,
            job_id,
            "parse_error",
            "xml_parse_error",
            &issue.message,
            issue.page_id,
            &issue.title,
            true,
        );
    };

    let mut on_page = |page: WikiPage| -> Result<Flow, AppError> {
        emitted += 1;
        if let Some(limit) = max_items {
            if emitted > limit {
                return Ok(Flow::Stop);
            }
        }
        if already.get(&page.page_id) == Some(&page.revision_id) {
            // 本批已处理过且修订一致，跳过清洗。
            counters.skipped += 1;
            return Ok(Flow::Continue);
        }
        let decision = if !handle_redirects && page.redirect_target.is_some() {
            PageDecision::Skip("redirect_disabled")
        } else {
            classify(
                &page,
                summary_options,
                mode,
                filter_disambiguation,
                filter_list_pages,
            )
        };
        counters.scanned += 1;
        match decision {
            PageDecision::Skip(reason) => {
                counters.filtered += 1;
                batch.push(StagingRow {
                    page_id: page.page_id,
                    revision_id: page.revision_id,
                    raw_title: page.title.clone(),
                    normalized_title: normalize_title(&page.title),
                    redirect_title: None,
                    summary: String::new(),
                    revision_timestamp: page.revision_timestamp,
                    content_hash: String::new(),
                    filter_status: store::FILTER_FILTERED.to_string(),
                    filter_reason: Some(reason.to_string()),
                    quality_flag: None,
                });
            }
            PageDecision::Redirect(target) => {
                counters.redirects += 1;
                batch.push(StagingRow {
                    page_id: page.page_id,
                    revision_id: page.revision_id,
                    raw_title: page.title.clone(),
                    normalized_title: normalize_title(&page.title),
                    redirect_title: Some(normalize_title(&target)),
                    summary: String::new(),
                    revision_timestamp: page.revision_timestamp,
                    content_hash: String::new(),
                    filter_status: store::FILTER_REDIRECT.to_string(),
                    filter_reason: None,
                    quality_flag: None,
                });
            }
            PageDecision::Accept { summary, quality } => {
                counters.accepted += 1;
                let normalized = normalize_title(&page.title);
                batch.push(StagingRow {
                    page_id: page.page_id,
                    revision_id: page.revision_id,
                    raw_title: page.title.clone(),
                    normalized_title: normalized.clone(),
                    redirect_title: None,
                    content_hash: content_hash(&normalized, &summary),
                    summary,
                    revision_timestamp: page.revision_timestamp,
                    filter_status: store::FILTER_ACCEPTED.to_string(),
                    filter_reason: None,
                    quality_flag: Some(quality.as_str().to_string()),
                });
            }
        }

        if batch.len() >= batch_size {
            flush(db, job_id, &mut batch, &counters)?;
            if control_signal(db, job_id)?.is_some() {
                return Ok(Flow::Stop);
            }
        }
        if last_flush.elapsed() >= Duration::from_millis(config.progress_interval_ms) {
            flush(db, job_id, &mut batch, &counters)?;
            last_flush = Instant::now();
        }
        Ok(Flow::Continue)
    };

    parser::parse_dump(reader, &mut on_page, &mut on_issue)?;
    flush(db, job_id, &mut batch, &counters)?;
    let parse_errors = count_issue_kind(db, job_id, "parse_error")?;
    store::set_publish_counts(
        db,
        job_id,
        0,
        0,
        counters.skipped as i64,
        0,
        parse_errors,
    )?;
    store::save_checkpoint(
        db,
        job_id,
        &serde_json::json!({
            "stage": "parsed",
            "scanned": counters.scanned,
            "accepted": counters.accepted,
            "redirects": counters.redirects,
            "filtered": counters.filtered,
            "skipped": counters.skipped,
        }),
    )?;
    Ok(control_signal(db, job_id)?)
}

fn flush(
    db: &Database,
    job_id: &str,
    batch: &mut Vec<StagingRow>,
    counters: &Counters,
) -> Result<(), AppError> {
    if batch.is_empty() {
        return Ok(());
    }
    store::insert_staging_batch(db, job_id, batch)?;
    batch.clear();
    store::update_progress(
        db,
        job_id,
        None,
        None,
        Some(counters.scanned as i64),
        Some(counters.accepted as i64),
        Some(counters.redirects as i64),
        Some(counters.filtered as i64),
        None,
        None,
    )
}

#[derive(Debug, Default)]
struct Counters {
    scanned: u64,
    accepted: u64,
    redirects: u64,
    filtered: u64,
    skipped: u64,
}

fn control_signal(db: &Database, job_id: &str) -> Result<Option<Signal>, AppError> {
    let connection = db.connect()?;
    let status: String = connection
        .query_row(
            "SELECT status FROM glossary_import_jobs WHERE id = ?1",
            rusqlite::params![job_id],
            |row| row.get(0),
        )
        .map_err(AppError::from)?;
    Ok(match status.as_str() {
        "paused" => Some(Signal::Pause),
        "cancelled" => Some(Signal::Cancel),
        _ => None,
    })
}

fn paused_summary(job_id: &str, _stage: JobStatus) -> RunSummary {
    RunSummary {
        job_id: job_id.to_string(),
        status: JobStatus::Paused,
        scanned: 0,
        published: None,
        message: "已在批次边界安全暂停".to_string(),
    }
}

fn cancelled_summary(job_id: &str) -> RunSummary {
    RunSummary {
        job_id: job_id.to_string(),
        status: JobStatus::Cancelled,
        scanned: 0,
        published: None,
        message: "已取消，没有留下半发布状态".to_string(),
    }
}

#[derive(Debug)]
enum PageDecision {
    Accept { summary: String, quality: SummaryQuality },
    Redirect(String),
    Skip(&'static str),
}

/// 页面分类：先看是不是重定向，再按需过滤，最后清洗出摘要。
fn classify(
    page: &WikiPage,
    summary_options: &SummaryOptions,
    mode: ImportMode,
    filter_disambiguation: bool,
    filter_list_pages: bool,
) -> PageDecision {
    let title = normalize_title(&page.title);
    if title.is_empty() {
        return PageDecision::Skip("empty_title");
    }
    if let Some(target) = &page.redirect_target {
        let target = normalize_title(target);
        if !target.is_empty() {
            return PageDecision::Redirect(target);
        }
        return PageDecision::Skip("redirect_without_target");
    }
    if soft_redirect_target(&page.text).is_some() {
        return PageDecision::Skip("soft_redirect");
    }
    if filter_disambiguation && (is_disambiguation_title(&title) || has_disambiguation_template(&page.text)) {
        return PageDecision::Skip("disambiguation");
    }
    if filter_list_pages && looks_like_list_page(&title) {
        return PageDecision::Skip("list_page");
    }
    if matches!(mode, ImportMode::TitlesOnly) {
        return PageDecision::Accept { summary: String::new(), quality: SummaryQuality::Short };
    }
    let cleaned = extract_summary(&page.text, *summary_options);
    let summary = cleaned.summary;
    if summary.is_empty() {
        return PageDecision::Skip("empty_page");
    }
    if summary.chars().count() < MIN_USEFUL_CHARS {
        return PageDecision::Skip("too_short");
    }
    PageDecision::Accept { summary, quality: cleaned.quality }
}

fn is_disambiguation_title(title: &str) -> bool {
    title.contains("(消歧义)")
        || title.ends_with("消歧义")
        || title.contains("(disambiguation)")
        || title.to_lowercase().ends_with("(disambiguation)")
}

fn has_disambiguation_template(wikitext: &str) -> bool {
    const MARKERS: &[&str] = &[
        "{{消歧义",
        "{{消歧義",
        "{{dab",
        "{{disambig",
        "{{hndis",
        "{{geodis",
    ];
    let lowered = wikitext.to_lowercase();
    MARKERS.iter().any(|marker| {
        let lowered_marker = marker.to_lowercase();
        lowered.contains(&lowered_marker)
    })
}

/// 摘要级内容哈希：用于判断“这次导入是否真的带来新内容”。
/// 哈希的是清洗后的摘要而不是原始 wikitext：模板或排版变化不应该被当成内容更新。
pub fn content_hash(normalized_title: &str, summary: &str) -> String {
    let payload = format!("{}\u{1}{}", normalize_search_key(normalized_title), summary.trim());
    format!("{:x}", md5::compute(payload.as_bytes()))
}

// ---------- serde_json 小工具（避免和 store 互相依赖） ----------

fn json_string(value: &serde_json::Value) -> Option<String> {
    value.as_str().map(str::to_string)
}

fn json_u64(value: &serde_json::Value) -> Option<u64> {
    value.as_u64()
}

fn json_bool(value: &serde_json::Value) -> Option<bool> {
    value.as_bool()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wikipedia::parser::WikiPage;

    fn page(title: &str, text: &str) -> WikiPage {
        WikiPage {
            page_id: 1,
            namespace: 0,
            title: title.to_string(),
            redirect_target: None,
            revision_id: 2,
            revision_timestamp: "2026-09-01T00:00:00Z".to_string(),
            text: text.to_string(),
        }
    }

    fn options() -> SummaryOptions {
        SummaryOptions { min_chars: 1, max_chars: 600 }
    }

    #[test]
    fn 普通词条被接受() {
        let decision = classify(
            &page("人工智能", "人工智能是研究如何让机器表现出类似人类智能的学科，广泛应用于搜索、翻译与推荐。"),
            &options(),
            ImportMode::Summary,
            true,
            true,
        );
        assert!(matches!(decision, PageDecision::Accept { .. }));
    }

    #[test]
    fn 重定向页被单独标记() {
        let mut target = page("机器学习", "");
        target.redirect_target = Some("人工智能".to_string());
        match classify(&target, &options(), ImportMode::Summary, true, true) {
            PageDecision::Redirect(value) => assert_eq!(value, "人工智能"),
            _ => panic!("应该是重定向"),
        }
    }

    #[test]
    fn 软重定向被过滤() {
        let decision = classify(
            &page("软重定向页", "#REDIRECT [[人工智能]]"),
            &options(),
            ImportMode::Summary,
            true,
            true,
        );
        assert!(matches!(decision, PageDecision::Skip("soft_redirect")));
    }

    #[test]
    fn 消歧义页被过滤() {
        let by_title = classify(&page("苹果 (消歧义)", "苹果属多种植物。"), &options(), ImportMode::Summary, true, true);
        assert!(matches!(by_title, PageDecision::Skip("disambiguation")));
        let by_template = classify(&page("苹果属", "{{消歧义}}\n苹果属是苹果属的植物。"), &options(), ImportMode::Summary, true, true);
        assert!(matches!(by_template, PageDecision::Skip("disambiguation")));
    }

    #[test]
    fn 列表页被过滤() {
        let decision = classify(&page("中国电视剧列表", "以下是中国电视剧的完整列表。"), &options(), ImportMode::Summary, true, true);
        assert!(matches!(decision, PageDecision::Skip("list_page")));
    }

    #[test]
    fn 关闭过滤后列表页仍然进入() {
        let decision = classify(&page("中国电视剧列表", "以下是中国电视剧的完整列表，收录了数百部剧集作品。"), &options(), ImportMode::Summary, false, false);
        assert!(matches!(decision, PageDecision::Accept { .. }));
    }

    #[test]
    fn 空页面被过滤() {
        let decision = classify(&page("空页面", "{{Infobox|x}}"), &options(), ImportMode::Summary, true, true);
        assert!(matches!(decision, PageDecision::Skip("empty_page")));
    }

    #[test]
    fn 过短摘要被过滤() {
        let decision = classify(&page("短条目", "太短。"), &options(), ImportMode::Summary, true, true);
        assert!(matches!(decision, PageDecision::Skip("too_short")));
    }

    #[test]
    fn 纯模板占比过高时标记_degraded() {
        // 脚手架占绝大多数、真正的正文只有一小段：仍然接受，但打上 degraded。
        // 正文必须够长（超过 MIN_USEFUL_CHARS），否则会先被 too_short 拦掉。
        let body = "这是一段真实的中文说明文字。".repeat(3);
        // 注意 format! 里 `{{` 是转义花括号，模板字面量要写成 `{{{{`。
        let text = format!("{{{{Infobox|x}}}}{}\n{}", "{{{{嵌套|内容}}}}".repeat(80), body);
        match classify(&page("复杂页面", &text), &options(), ImportMode::Summary, true, true) {
            PageDecision::Accept { quality, .. } => assert_eq!(quality, SummaryQuality::Degraded),
            PageDecision::Skip(reason) => {
                let plain = extract_summary(&text, options());
                panic!("应该被接受但带质量标记，实际被过滤为 {reason}，摘要={:?}", plain.summary);
            }
            other => panic!("应该被接受但带质量标记，实际是 {other:?}"),
        }
    }

    #[test]
    fn 正文占比高时质量正常() {
        let text = "这是一段真实的中文正文内容。".repeat(40);
        match classify(&page("正常页面", &text), &options(), ImportMode::Summary, true, true) {
            PageDecision::Accept { quality, .. } => assert_ne!(quality, SummaryQuality::Degraded),
            other => panic!("正常页面不该 degraded，实际是 {other:?}"),
        }
    }

    #[test]
    fn 内容哈希对同样的输入稳定() {
        let first = content_hash("人工智能", "  摘要内容  ");
        let second = content_hash("人工智能", "摘要内容");
        assert_eq!(first, second, "首尾空白不应影响哈希");
        assert_ne!(content_hash("人工智能", "摘要内容"), content_hash("机器学习", "摘要内容"));
    }

    #[test]
    fn 标题模式不需要正文() {
        let decision = classify(&page("仅标题", ""), &options(), ImportMode::TitlesOnly, true, true);
        match decision {
            PageDecision::Accept { summary, .. } => assert!(summary.is_empty()),
            _ => panic!("titles_only 应该接受"),
        }
    }
}
