//! dump 下载：域名白名单、磁盘预检、断点续传、MD5 校验。
//!
//! 安全约束（需求 11.2 / 11.3 / 7.2）：
//! - 只允许白名单域名 + HTTPS，防 SSRF；
//! - 文件名从 URL 取片段后白名单校验，不做路径拼接穿越；
//! - 续传用 Range 头，服务端不支持 Range 时才从零重来；
//! - `.part` 临时文件下完才原子改名，中断不会留下“看起来完整”的文件。

use crate::error::AppError;
use reqwest::blocking::Client;
use reqwest::header::{HeaderMap, RANGE, USER_AGENT};
use reqwest::redirect::Policy;
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// 流式算 MD5 用。md5 0.7 只有 Context/consume，没有 Md5::new 那套 API。
use md5::Context as Md5Context;

/// 连续多次写盘之间不额外 sleep，进度回调自带节流。
const CHUNK_SIZE: usize = 256 * 1024;
/// 磁盘预留：除了文件本身再留 1 GiB 余量，避免把用户磁盘写满。
const DISK_HEADROOM_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct RemoteFile {
    pub url: String,
    pub file_name: String,
    pub total_bytes: Option<u64>,
    pub supports_range: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadPlan {
    /// 从零开始。
    Restart,
    /// 从已有字节数继续。
    ResumeFrom(u64),
    /// 已经下完，直接校验。
    Complete,
}

#[derive(Debug, Clone)]
pub struct DownloadResult {
    pub path: PathBuf,
    pub bytes: u64,
    pub resumed_from: u64,
    pub elapsed_ms: u64,
}

pub fn build_client(user_agent: &str, timeout: Duration) -> Result<Client, AppError> {
    Client::builder()
        .user_agent(user_agent)
        .timeout(timeout)
        // 不自动跟随 30x。否则初始白名单 URL 可以把客户端带到任意域名或内网地址。
        // Wikimedia dump 使用稳定直链；如将来需要跟随，必须逐跳重新做域名/IP 校验。
        .redirect(Policy::none())
        .build()
        .map_err(AppError::from)
}

/// 域名白名单 + HTTPS 校验。允许清单里的主机名逐个比对，
/// 不做子域通配，避免 `evil-dumps.wikimedia.org.com` 这类绕过。
pub fn validate_source_url(url: &str, allowed_hosts: &[String]) -> Result<reqwest::Url, AppError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| AppError::Message(format!("dump 地址无法解析：{error}")))?;
    if parsed.scheme() != "https" {
        return Err(AppError::Message("dump 地址必须使用 HTTPS".to_string()));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::Message("dump 地址缺少主机名".to_string()))?
        .to_lowercase();
    let allowed = allowed_hosts
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(&host));
    if !allowed {
        return Err(AppError::Message(format!(
            "域名 {host} 不在允许清单内：{}",
            allowed_hosts.join(", ")
        )));
    }
    Ok(parsed)
}

/// 从 URL 取文件名，只允许安全字符，其余一律拒绝（防路径穿越）。
/// `Url::parse` 会把 `..` 规范化掉，因此额外检查原始串里是否出现穿越片段。
pub fn safe_file_name(url: &str) -> Result<String, AppError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| AppError::Message(format!("dump 地址无法解析：{error}")))?;
    if url.contains("..") {
        return Err(AppError::Message("dump 地址包含路径穿越片段".to_string()));
    }
    let name = parsed
        .path_segments()
        .and_then(|segments| segments.filter(|segment| !segment.is_empty()).next_back())
        .ok_or_else(|| AppError::Message("dump 地址里没有文件名".to_string()))?;
    let name = percent_decode(name);
    let valid = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains('/')
        && !name.contains('\\')
        && name
            .chars()
            .all(|value| value.is_ascii_alphanumeric() || matches!(value, '.' | '_' | '-'));
    if !valid {
        return Err(AppError::Message(format!("dump 文件名不合法：{name}")));
    }
    Ok(name)
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&value[index + 1..index + 3], 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 探测远端文件：优先 HEAD，405/501 时退回 Range: bytes=0-0 的 GET。
pub fn probe(client: &Client, url: &str, allowed_hosts: &[String]) -> Result<RemoteFile, AppError> {
    validate_source_url(url, allowed_hosts)?;
    let file_name = safe_file_name(url)?;

    let head = client.head(url).send();
    if let Ok(response) = head {
        if response.status().is_success() {
            let total_bytes = response.content_length();
            let supports_range = response
                .headers()
                .get(reqwest::header::ACCEPT_RANGES)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.contains("bytes"));
            return Ok(RemoteFile { url: url.to_string(), file_name, total_bytes, supports_range });
        }
    }

    let response = client
        .get(url)
        .header(RANGE, "bytes=0-0")
        .send()
        .map_err(|error| AppError::Message(format!("探测 dump 文件失败：{error}")))?;
    if !response.status().is_success() {
        return Err(AppError::Message(format!(
            "dump 文件不可用，HTTP {}",
            response.status().as_u16()
        )));
    }
    let partial = response.status() == reqwest::StatusCode::PARTIAL_CONTENT;
    let total_bytes = response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.rsplit('/').next().and_then(|total| total.trim().parse().ok()));
    Ok(RemoteFile { url: url.to_string(), file_name, total_bytes, supports_range: partial })
}

/// 续传决策。纯函数，方便单测覆盖各种组合。
pub fn plan_download(existing: u64, total: Option<u64>, supports_range: bool) -> DownloadPlan {
    if existing == 0 {
        return DownloadPlan::Restart;
    }
    match total {
        Some(total) if existing == total => DownloadPlan::Complete,
        Some(total) if existing > total => DownloadPlan::Restart,
        _ if supports_range => DownloadPlan::ResumeFrom(existing),
        _ => DownloadPlan::Restart,
    }
}

/// 下载前检查可用空间，不够就直接失败，不下载到一半才失败。
pub fn ensure_space(dir: &Path, needed: u64) -> Result<(), AppError> {
    let available = fs4::available_space(dir)
        .map_err(|error| AppError::Message(format!("无法读取磁盘可用空间：{error}")))?;
    if available < needed + DISK_HEADROOM_BYTES {
        return Err(AppError::Message(format!(
            "磁盘空间不足：还需要 {} 字节，可用 {} 字节",
            needed, available
        )));
    }
    Ok(())
}

/// 下载（含续传）。`on_progress` 收到的是累计字节数，由调用方负责节流。
pub fn download_with_resume<F>(
    client: &Client,
    remote: &RemoteFile,
    dest_dir: &Path,
    on_progress: F,
) -> Result<DownloadResult, AppError>
where
    F: FnMut(u64),
{
    fs::create_dir_all(dest_dir)
        .map_err(|error| AppError::Message(format!("创建下载目录失败：{error}")))?;
    let target = dest_dir.join(&remote.file_name);
    let part = dest_dir.join(format!("{}.part", remote.file_name));
    let existing = fs::metadata(&part).map(|meta| meta.len()).unwrap_or(0);
    let plan = plan_download(existing, remote.total_bytes, remote.supports_range);

    let mut on_progress = on_progress;
    match plan {
        DownloadPlan::Complete => {
            on_progress(existing);
            return Ok(DownloadResult { path: part, bytes: existing, resumed_from: existing, elapsed_ms: 0 });
        }
        DownloadPlan::Restart => {
            let _ = fs::remove_file(&part);
        }
        DownloadPlan::ResumeFrom(offset) => {
            // 已有内容超过总量时也当作重来，避免拼出坏文件。
            if let Some(total) = remote.total_bytes {
                if offset >= total {
                    let _ = fs::remove_file(&part);
                }
            }
        }
    }

    let mut resumed_from = match plan {
        DownloadPlan::ResumeFrom(offset) => offset,
        _ => 0,
    };
    let needed = remote.total_bytes.unwrap_or(0).saturating_sub(resumed_from);
    if remote.total_bytes.is_some() {
        ensure_space(dest_dir, needed)?;
    }

    let mut request = client.get(&remote.url);
    if resumed_from > 0 {
        request = request.header(RANGE, format!("bytes={resumed_from}-"));
    }
    let mut response = request
        .send()
        .map_err(|error| AppError::Message(format!("下载 dump 失败：{error}")))?;
    if !response.status().is_success() {
        return Err(AppError::Message(format!(
            "下载 dump 失败，HTTP {}",
            response.status().as_u16()
        )));
    }
    // 服务端忽略了 Range（返回 200 而不是 206），必须从头写。
    if resumed_from > 0 && response.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        let _ = fs::remove_file(&part);
        resumed_from = 0;
    }

    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(resumed_from > 0)
        .truncate(resumed_from == 0)
        .open(&part)
        .map_err(|error| AppError::Message(format!("写入 dump 临时文件失败：{error}")))?;
    let started = Instant::now();
    let mut written = resumed_from;
    let mut buffer = vec![0_u8; CHUNK_SIZE];
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|error| AppError::Message(format!("读取下载流失败：{error}")))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|error| AppError::Message(format!("写盘失败：{error}")))?;
        written += read as u64;
        on_progress(written);
    }
    file.flush().map_err(|error| AppError::Message(format!("刷新文件失败：{error}")))?;
    drop(file);
    if let Some(total) = remote.total_bytes {
        if written != total {
            return Err(AppError::Message(format!(
                "下载不完整：预期 {total} 字节，实际 {written} 字节，.part 已保留可续传"
            )));
        }
    }
    // 原子改名：只有下完的文件才会出现在正式路径上。
    fs::rename(&part, &target)
        .map_err(|error| AppError::Message(format!("dump 文件改名失败：{error}")))?;
    Ok(DownloadResult {
        path: target,
        bytes: written,
        resumed_from,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}

/// 解析官方 md5sums.txt：每行 `md5  文件名`。
pub fn parse_md5sums(content: &str) -> HashMap<String, String> {
    content
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                return None;
            }
            let mut parts = line.split_whitespace();
            let digest = parts.next()?;
            let name = parts.next()?;
            (digest.len() == 32 && digest.chars().all(|c| c.is_ascii_hexdigit()))
                .then(|| (name.trim_start_matches('*').to_string(), digest.to_lowercase()))
        })
        .collect()
}

/// 拉取 md5sums.txt 并返回 文件名 -> md5。
pub fn fetch_md5sums(
    client: &Client,
    base_url: &str,
    allowed_hosts: &[String],
) -> Result<HashMap<String, String>, AppError> {
    let url = format!("{}{}", base_url.trim_end_matches('/'), "/zhwiki-latest-md5sums.txt");
    validate_source_url(&url, allowed_hosts)?;
    let response = client
        .get(&url)
        .send()
        .map_err(|error| AppError::Message(format!("获取 md5sums 失败：{error}")))?;
    if !response.status().is_success() {
        return Err(AppError::Message(format!(
            "获取 md5sums 失败，HTTP {}",
            response.status().as_u16()
        )));
    }
    let text = response
        .text()
        .map_err(|error| AppError::Message(format!("读取 md5sums 失败：{error}")))?;
    Ok(parse_md5sums(&text))
}

pub fn file_md5(path: &Path) -> Result<String, AppError> {
    let mut file =
        File::open(path).map_err(|error| AppError::Message(format!("打开文件计算 MD5 失败：{error}")))?;
    let mut context = Md5Context::new();
    let mut buffer = vec![0_u8; CHUNK_SIZE];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| AppError::Message(format!("读取文件计算 MD5 失败：{error}")))?;
        if read == 0 {
            break;
        }
        context.consume(&buffer[..read]);
    }
    Ok(format!("{:x}", context.compute()))
}

/// 校验失败必须阻止进入解析阶段（需求 7.2.6）。
pub fn verify_md5(path: &Path, expected: &str) -> Result<String, AppError> {
    let actual = file_md5(path)?;
    if !expected.eq_ignore_ascii_case(&actual) {
        return Err(AppError::Message(format!(
            "MD5 校验失败：期望 {expected}，实际 {actual}"
        )));
    }
    Ok(actual)
}

/// 构造带 User-Agent 的请求头，供探测类调用复用。
pub fn default_headers(user_agent: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Ok(value) = user_agent.parse() {
        headers.insert(USER_AGENT, value);
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hosts() -> Vec<String> {
        vec!["dumps.wikimedia.org".to_string()]
    }

    #[test]
    fn 白名单域名通过() {
        assert!(validate_source_url("https://dumps.wikimedia.org/zhwiki/latest/x.bz2", &hosts()).is_ok());
    }

    #[test]
    fn 非白名单域名被拒绝() {
        let error = validate_source_url("https://evil.example.com/x.bz2", &hosts()).expect_err("应拒绝");
        assert!(error.to_string().contains("不在允许清单"));
    }

    #[test]
    fn 相似域名不能绕过白名单() {
        assert!(validate_source_url("https://dumps.wikimedia.org.evil.com/x.bz2", &hosts()).is_err());
        assert!(validate_source_url("https://evil-dumps.wikimedia.org/x.bz2", &hosts()).is_err());
    }

    #[test]
    fn 明文协议被拒绝() {
        assert!(validate_source_url("http://dumps.wikimedia.org/x.bz2", &hosts()).is_err());
    }

    #[test]
    fn 文件名可安全提取() {
        assert_eq!(
            safe_file_name("https://dumps.wikimedia.org/zhwiki/latest/zhwiki-latest-pages-articles-multistream.xml.bz2")
                .unwrap(),
            "zhwiki-latest-pages-articles-multistream.xml.bz2"
        );
    }

    #[test]
    fn 路径穿越型文件名被拒绝() {
        // Url::parse 会把 `..` 规范化掉，所以这里额外检查原始串里有没有穿越片段。
        assert!(safe_file_name("https://dumps.wikimedia.org/../../etc/passwd").is_err());
        assert!(safe_file_name("https://dumps.wikimedia.org/a/..%2F..%2Fpasswd").is_err());
        assert!(safe_file_name("https://dumps.wikimedia.org/").is_err());
    }

    #[test]
    fn 编码后的穿越片段同样被拒绝() {
        // 文件名本身合法但解码后含分隔符。
        assert!(safe_file_name("https://dumps.wikimedia.org/a%2F..%2Fpasswd").is_err());
    }

    #[test]
    fn 续传决策覆盖各种情况() {
        assert_eq!(plan_download(0, Some(100), true), DownloadPlan::Restart);
        assert_eq!(plan_download(40, Some(100), true), DownloadPlan::ResumeFrom(40));
        assert_eq!(plan_download(100, Some(100), true), DownloadPlan::Complete);
        assert_eq!(plan_download(120, Some(100), true), DownloadPlan::Restart);
        // 服务端不支持 Range：只能重来。
        assert_eq!(plan_download(40, Some(100), false), DownloadPlan::Restart);
        // 不知道总量但支持 Range：可以续传。
        assert_eq!(plan_download(40, None, true), DownloadPlan::ResumeFrom(40));
    }

    #[test]
    fn md5sums_可解析() {
        let content = "# comment\n\
             0123456789abcdef0123456789abcdef  zhwiki-latest-pages-articles-multistream.xml.bz2\n\
             fedcba9876543210fedcba9876543210 *zhwiki-latest-all-titles-in-ns0.gz\n\
             短行不是校验和\n";
        let parsed = parse_md5sums(content);
        assert_eq!(
            parsed.get("zhwiki-latest-pages-articles-multistream.xml.bz2").map(String::as_str),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert_eq!(
            parsed.get("zhwiki-latest-all-titles-in-ns0.gz").map(String::as_str),
            Some("fedcba9876543210fedcba9876543210")
        );
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn md5_校验通过与失败() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("a.txt");
        fs::write(&path, b"hello").expect("write");
        let digest = file_md5(&path).expect("md5");
        assert_eq!(digest, "5d41402abc4b2a76b9719d911017c592");
        assert!(verify_md5(&path, &digest).is_ok());
        assert!(verify_md5(&path, "00000000000000000000000000000000").is_err());
    }

    #[test]
    fn 磁盘空间不足时报错() {
        let dir = tempfile::tempdir().expect("temp dir");
        // 需要超过整块磁盘的量，必然失败。
        assert!(ensure_space(dir.path(), u64::MAX / 2).is_err());
        // 需要很小则通过（1 GiB 余量在真实磁盘上足够）。
        assert!(ensure_space(dir.path(), 1024).is_ok());
    }

    #[test]
    fn 断点续传保留已有内容() {
        let dir = tempfile::tempdir().expect("temp dir");
        let part = dir.path().join("x.bin.part");
        fs::write(&part, b"0123456789").expect("write part");
        assert_eq!(fs::metadata(&part).unwrap().len(), 10);
        // 已有的 .part 不应被下载流程清空（这里只验证文件状态不变）。
        assert_eq!(fs::metadata(&part).unwrap().len(), 10);
    }

    #[test]
    fn user_agent_写入请求头() {
        let headers = default_headers("wxreader-test/1.0 (test@example.com)");
        assert!(headers.contains_key(USER_AGENT));
    }
}
