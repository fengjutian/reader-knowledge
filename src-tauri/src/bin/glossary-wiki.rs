//! 名词库维基导入的独立命令行入口。
//!
//! 桌面端的导入跑在后台线程里；服务器或无人值守场景用这个二进制，
//! 两者共用 `wereader_lib::wikipedia` 的同一套 store/runner 逻辑。
//!
//! 用法示例：
//!   glossary-wiki inspect --source https://dumps.wikimedia.org/zhwiki/latest/
//!   glossary-wiki import --mode summary --local-file D:\dump.xml.bz2 --limit 1000
//!   glossary-wiki resume --job <id>
//!   glossary-wiki validate --job <id>
//!   glossary-wiki publish --job <id>
//!   glossary-wiki cleanup --job <id>
//!
//! 所有子命令都支持 --help，结束时输出一行机器可读 JSON 摘要。

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use wereader_lib::database::Database;
use wereader_lib::wikipedia::config::{ImportMode, WikipediaConfig};
use wereader_lib::wikipedia::report;
use wereader_lib::wikipedia::runner;
use wereader_lib::wikipedia::store::{self, ImportRequest, JobStatus};

const HELP: &str = r#"名词库维基导入 CLI

用法：
  glossary-wiki <子命令> [参数]

子命令：
  inspect    探测 dump 源，打印可用文件与大小（不下载正文）
  import     创建并运行一个导入任务
  resume     从检查点继续一个已暂停的任务
  validate   只跑批次校验并打印报告
  publish    发布一个 ready_to_publish 的任务
  report     打印任务报告
  cleanup    清理已结束任务的 staging
  list       列出最近的导入任务

通用参数：
  --db <路径>            数据库文件，默认取应用数据目录
  --job <id>            任务 id
  --mode <summary>      导入模式：summary（默认）/ titles_only / full_text
  --source <url>        dump 基地址，默认官方 zhwiki/latest
  --local-file <路径>    本地 dump 文件，跳过下载与 MD5
  --limit <n>           最多处理 n 个页面（测试用）
  --batch-size <n>      staging 批量写入大小
  --auto-publish        校验通过后自动发布
  --retention-days <n>  staging 保留天数（cleanup 用）
  --help                显示帮助
"#;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|value| value == "--help" || value == "-h") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let command = args[0].clone();
    let flags = match parse_flags(&args[1..]) {
        Ok(flags) => flags,
        Err(error) => return fail(&error),
    };
    match run(&command, &flags) {
        Ok(summary) => {
            println!("{}", serde_json::to_string(&summary).unwrap_or_default());
            ExitCode::SUCCESS
        }
        Err(error) => fail(&error),
    }
}

fn fail(error: &str) -> ExitCode {
    let payload = serde_json::json!({ "ok": false, "error": error });
    println!("{payload}");
    eprintln!("{error}");
    ExitCode::FAILURE
}

fn parse_flags(args: &[String]) -> Result<HashMap<String, String>, String> {
    let mut flags: HashMap<String, String> = HashMap::new();
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if !arg.starts_with("--") {
            return Err(format!("无法识别的参数：{arg}"));
        }
        let key = arg.trim_start_matches("--").to_string();
        // 布尔开关没有值。
        if matches!(key.as_str(), "auto-publish" | "help") {
            flags.insert(key, "true".to_string());
            index += 1;
            continue;
        }
        let value = args
            .get(index + 1)
            .ok_or_else(|| format!("参数 --{key} 缺少取值"))?;
        flags.insert(key, value.clone());
        index += 2;
    }
    Ok(flags)
}

fn flag<'a>(flags: &'a HashMap<String, String>, key: &str) -> Option<&'a str> {
    flags.get(key).map(String::as_str)
}

fn flag_u64(flags: &HashMap<String, String>, key: &str) -> Result<Option<u64>, String> {
    match flag(flags, key) {
        None => Ok(None),
        Some(value) => value
            .parse()
            .map(Some)
            .map_err(|_| format!("参数 --{key} 需要数字：{value}")),
    }
}

fn flag_usize(flags: &HashMap<String, String>, key: &str) -> Result<Option<usize>, String> {
    Ok(flag_u64(flags, key)?.map(|value| value as usize))
}

fn open_db(flags: &HashMap<String, String>) -> Result<Database, String> {
    let path = match flag(flags, "db") {
        Some(value) => PathBuf::from(value),
        None => default_db_path()?,
    };
    Database::open(path).map_err(|error| error.to_string())
}

fn default_db_path() -> Result<PathBuf, String> {
    // 优先取 Tauri 约定的应用数据目录；取不到就退回用户主目录。
    let base = std::env::var("APPDATA")
        .or_else(|_| std::env::var("HOME").map(|home| format!("{home}/.local/share")))
        .map_err(|_| "无法定位应用数据目录，请用 --db 指定数据库路径".to_string())?;
    Ok(PathBuf::from(base).join("com.wereader.app").join("readflow.db"))
}

fn config_from(flags: &HashMap<String, String>) -> Result<WikipediaConfig, String> {
    let base = default_db_path()
        .and_then(|path| {
            path.parent()
                .map(|parent| parent.to_path_buf())
                .ok_or_else(|| "无法推导应用数据目录".to_string())
        })
        .unwrap_or_else(|_| PathBuf::from("."));
    let mut config = WikipediaConfig::from_env(&base).map_err(|error| error.to_string())?;
    if let Some(source) = flag(flags, "source") {
        config.base_url = source.to_string();
        config.validate().map_err(|error| error.to_string())?;
    }
    if let Some(dir) = flag(flags, "temp-dir") {
        config.temp_dir = PathBuf::from(dir);
    }
    if let Some(size) = flag_usize(flags, "batch-size")? {
        config.batch_size = size.max(1);
    }
    if let Some(value) = flag_u64(flags, "retention-days")? {
        config.staging_retention_days = value as i64;
    }
    Ok(config)
}

fn run(command: &str, flags: &HashMap<String, String>) -> Result<serde_json::Value, String> {
    let db = open_db(flags)?;
    match command {
        "inspect" => inspect(&db, flags),
        "import" => import(&db, flags),
        "resume" => resume(&db, flags),
        "validate" => validate(&db, flags),
        "publish" => publish(&db, flags),
        "report" => report_command(&db, flags),
        "cleanup" => cleanup(&db, flags),
        "list" => list(&db),
        other => Err(format!("未知子命令：{other}")),
    }
}

/// 探测官方 dump 目录：只发 HEAD 请求，不下载正文。
fn inspect(db: &Database, flags: &HashMap<String, String>) -> Result<serde_json::Value, String> {
    let config = config_from(flags)?;
    let client = wereader_lib::wikipedia::download::build_client(&config.user_agent, std::time::Duration::from_secs(30))
        .map_err(|error| error.to_string())?;
    let base = format!("{}/", config.base_url.trim_end_matches('/'));
    let mut files = Vec::new();
    for name in [
        wereader_lib::wikipedia::config::DUMP_ARTICLES_FILENAME,
        "zhwiki-latest-pages-articles-multistream-index.txt.bz2",
        wereader_lib::wikipedia::config::DUMP_MD5SUMS_FILENAME,
        "zhwiki-latest-all-titles-in-ns0.gz",
    ] {
        let url = format!("{base}{name}");
        match wereader_lib::wikipedia::download::probe(&client, &url, &config.allowed_hosts) {
            Ok(remote) => files.push(serde_json::json!({
                "file": remote.file_name,
                "bytes": remote.total_bytes,
                "supportsRange": remote.supports_range,
            })),
            Err(error) => files.push(serde_json::json!({ "file": name, "error": error.to_string() })),
        }
    }
    let _ = db;
    Ok(serde_json::json!({ "ok": true, "baseUrl": base, "files": files }))
}

fn import(db: &Database, flags: &HashMap<String, String>) -> Result<serde_json::Value, String> {
    let config = config_from(flags)?;
    config.ensure_temp_dir().map_err(|error| error.to_string())?;
    let mode = match flag(flags, "mode") {
        None => ImportMode::Summary,
        Some(value) => ImportMode::parse(value).map_err(|error| error.to_string())?,
    };
    let request = ImportRequest {
        source_url: flag(flags, "source").map(str::to_string),
        dump_version: flag(flags, "dump-version").map(str::to_string),
        mode: Some(mode),
        auto_publish: Some(flag(flags, "auto-publish").is_some()),
        max_items: flag_u64(flags, "limit")?,
        batch_size: flag_usize(flags, "batch-size")?,
        temp_dir: flag(flags, "temp-dir").map(PathBuf::from),
        local_file: flag(flags, "local-file").map(PathBuf::from),
        ..ImportRequest::default()
    };
    let job = store::create_job(db, &request, &config).map_err(|error| error.to_string())?;
    let summary = runner::run_pipeline(Arc::new(db.reopen().map_err(|error| error.to_string())?), job.id.clone(), config)
        .map_err(|error| error.to_string())?;
    Ok(serde_json::json!({
        "ok": true,
        "jobId": summary.job_id,
        "status": summary.status.as_str(),
        "scanned": summary.scanned,
        "message": summary.message,
        "published": summary.published,
    }))
}

fn job_id(flags: &HashMap<String, String>) -> Result<String, String> {
    flag(flags, "job")
        .map(str::to_string)
        .ok_or_else(|| "缺少 --job <id>".to_string())
}

fn resume(db: &Database, flags: &HashMap<String, String>) -> Result<serde_json::Value, String> {
    let config = config_from(flags)?;
    let id = job_id(flags)?;
    let status = store::resume_job(db, &id).map_err(|error| error.to_string())?;
    let summary = runner::run_pipeline(Arc::new(db.reopen().map_err(|error| error.to_string())?), id.clone(), config)
        .map_err(|error| error.to_string())?;
    Ok(serde_json::json!({
        "ok": true,
        "jobId": id,
        "resumedFrom": status.as_str(),
        "status": summary.status.as_str(),
        "scanned": summary.scanned,
        "message": summary.message,
    }))
}

fn validate(db: &Database, flags: &HashMap<String, String>) -> Result<serde_json::Value, String> {
    let id = job_id(flags)?;
    let validation = store::validate_batch(db, &id).map_err(|error| error.to_string())?;
    let report = report::build_report(db, &id).map_err(|error| error.to_string())?;
    Ok(serde_json::json!({ "ok": true, "validation": validation, "report": report }))
}

fn publish(db: &Database, flags: &HashMap<String, String>) -> Result<serde_json::Value, String> {
    let id = job_id(flags)?;
    let job = store::get_job(db, &id)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "导入任务不存在".to_string())?;
    let status = JobStatus::parse(&job.status).map_err(|error| error.to_string())?;
    if status != JobStatus::ReadyToPublish {
        return Err(format!(
            "只有 ready_to_publish 的任务可以发布，当前是 {}",
            status.as_str()
        ));
    }
    store::transition(db, &id, JobStatus::Publishing).map_err(|error| error.to_string())?;
    let config = config_from(flags)?;
    let summary = runner::run_pipeline(std::sync::Arc::new(db.reopen().map_err(|e| e.to_string())?), id.clone(), config)
        .map_err(|error| error.to_string())?;
    Ok(serde_json::json!({ "ok": true, "jobId": id, "status": summary.status.as_str(), "published": summary.published }))
}

fn report_command(db: &Database, flags: &HashMap<String, String>) -> Result<serde_json::Value, String> {
    let id = job_id(flags)?;
    let value = report::build_report(db, &id).map_err(|error| error.to_string())?;
    Ok(value)
}

fn cleanup(db: &Database, flags: &HashMap<String, String>) -> Result<serde_json::Value, String> {
    let config = config_from(flags)?;
    // 指定 --job 时只清理该任务：需要先确认它已经结束。
    if let Some(id) = flag(flags, "job") {
        let job = store::get_job(db, id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "导入任务不存在".to_string())?;
        let status = JobStatus::parse(&job.status).map_err(|error| error.to_string())?;
        if !status.is_terminal() {
            return Err(format!("任务还在 {} 状态，拒绝清理", status.as_str()));
        }
    }
    let removed = store::prune_staging(db, config.staging_retention_days).map_err(|error| error.to_string())?;
    Ok(serde_json::json!({
        "ok": true,
        "removedStagingRows": removed,
        "retentionDays": config.staging_retention_days,
    }))
}

fn list(db: &Database) -> Result<serde_json::Value, String> {
    let jobs = store::list_jobs(db, 50).map_err(|error| error.to_string())?;
    Ok(serde_json::json!({ "ok": true, "jobs": jobs }))
}

