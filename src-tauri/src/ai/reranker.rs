//! Reranker：把 Hybrid Search 的候选重排后再交给 AI。
//!
//! 边界（对应实现计划的 6.1）：
//! - 只用于 AI RAG，**不参与普通全局搜索**，全局搜索的可用性不受它影响。
//! - 没配置、被禁用、超时、429/5xx、返回格式非法 —— 任何一种情况都降级到本地排序，
//!   并给前端一条非阻断 warning，绝不让 AI 因为重排失败而答不了。

use crate::error::AppError;
use crate::http::{limited_json, MAX_API_RESPONSE_BYTES};
use crate::models::{ChatMessage, RerankerSettings};
use serde::Deserialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// 单条候选送进 rerank 的最大字符数，超出截断以控制请求体。
pub const MAX_CANDIDATE_CHARS: usize = 600;
/// 交给 rerank 的候选上限，与计划一致。
pub const MAX_CANDIDATES: usize = 100;
/// rerank 请求整体超时。超时即降级，不阻塞 AI 回答。
pub const RERANK_TIMEOUT: Duration = Duration::from_secs(20);
/// Top N 的合法区间。
pub const MIN_TOP_N: i64 = 1;
pub const MAX_TOP_N: i64 = 50;

pub const RERANKER_CREDENTIAL_PREFIX: &str = "reranker";

/// 凭据名必须是 `reranker:{provider}`，避免和其他服务的 key 串用。
pub fn credential_name(provider: &str) -> String {
    format!("{RERANKER_CREDENTIAL_PREFIX}:{provider}")
}

/// 重排结果。`warning` 非空表示发生过降级，仍然是可用结果。
#[derive(Debug, Clone, Default)]
pub struct RerankOutcome {
    /// 排序后的候选下标（对应传入 `documents` 的顺序）。
    pub order: Vec<usize>,
    /// 映射回**原始候选**顺序的下标，调用方按这个顺序取候选。
    pub original_order: Vec<usize>,
    /// 远端返回的相关性分数，与 `order` 一一对应，仅用于调试。
    pub rerank_scores: Vec<f64>,
    /// 非阻断提示，直接展示给用户。
    pub warning: String,
    /// 是否真的走了远端重排。
    pub applied: bool,
    /// 送去重排的候选数，只用于排查性能，不含任何正文。
    pub candidate_count: usize,
    /// 实际耗时（毫秒），只用于排查性能，不含问题正文或笔记正文。
    pub elapsed_ms: u128,
}

/// 远端 rerank 的一行结果。
#[derive(Debug, Deserialize)]
struct RerankItem {
    index: usize,
    #[serde(default)]
    relevance_score: Option<f64>,
}

/// 远端 rerank 响应。只关心 `results`，其余字段忽略。
#[derive(Debug, Deserialize)]
struct RerankResponse {
    #[serde(default)]
    results: Option<Vec<RerankItem>>,
}

/// 校验并归一化远端返回的排序。
///
/// 拒绝条件（计划 6.3.4）：index 越界、index 重复、分数非有限数值（NaN / Inf）。
/// 任一条不满足就整份丢弃并降级，不能「部分信任」。
fn validate_order(candidate_count: usize, items: &[RerankItem]) -> Result<Vec<(usize, f64)>, String> {
    if items.is_empty() {
        return Err("重排服务没有返回任何结果".into());
    }
    let mut seen = vec![false; candidate_count];
    let mut order = Vec::with_capacity(items.len());
    for item in items {
        if item.index >= candidate_count {
            return Err(format!("重排服务返回了越界的下标 {}", item.index));
        }
        if seen[item.index] {
            return Err(format!("重排服务返回了重复的下标 {}", item.index));
        }
        seen[item.index] = true;
        let score = item.relevance_score.unwrap_or(0.0);
        if !score.is_finite() {
            return Err("重排服务返回了非有限分数".into());
        }
        order.push((item.index, score));
    }
    Ok(order)
}

/// 构造请求体。只发送候选正文，不带问题原文之外的任何本地数据。
fn request_body(model: &str, query: &str, documents: &[String], top_n: usize) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "query": query,
        "documents": documents,
        "top_n": top_n,
        "return_documents": false,
    })
}

/// 对候选正文做长度限制与去重。
///
/// 去重按「截断后的正文完全相同」判断，避免同一段划线重复占满 Top N。
/// 返回 `(原始下标, 送出的正文)`，原始下标用于回写排序。
pub fn prepare_documents<T: AsRef<str>>(
    candidates: &[T],
) -> (Vec<(usize, String)>, Vec<String>) {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut pairs = Vec::new();
    for (index, candidate) in candidates.iter().enumerate().take(MAX_CANDIDATES) {
        let text: String = candidate.as_ref().chars().take(MAX_CANDIDATE_CHARS).collect();
        let text = text.trim().to_owned();
        if text.is_empty() || !seen.insert(text.clone()) {
            continue;
        }
        pairs.push((index, text));
    }
    let documents = pairs.iter().map(|(_, text)| text.clone()).collect();
    (pairs, documents)
}

/// 保留每本书至少一条证据，避免跨书分析被单本书刷屏。
///
/// 规则：按 rerank 顺序遍历，某本书累计达到 `per_book` 条后不再收；
/// 全部收满后剩余的按原顺序补齐，保证不丢候选。
pub fn apply_book_coverage(
    order: &[usize],
    book_ids: &[String],
    top_n: usize,
    per_book: usize,
) -> Vec<usize> {
    let per_book = per_book.max(1);
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut selected = Vec::with_capacity(top_n);
    let mut overflow = Vec::new();
    for index in order {
        let Some(book_id) = book_ids.get(*index) else { continue };
        let count = counts.entry(book_id.as_str()).or_default();
        if selected.len() < top_n && *count < per_book {
            *count += 1;
            selected.push(*index);
        } else if selected.len() < top_n {
            // 名额已满但还没到 per_book 上限的继续收
            if *count < per_book {
                *count += 1;
                selected.push(*index);
            } else {
                overflow.push(*index);
            }
        } else {
            overflow.push(*index);
        }
    }
    selected.extend(overflow.into_iter().take(top_n.saturating_sub(selected.len())));
    selected
}

/// 把远端分数写回候选，产出可解释的最终排序。
///
/// 目前由测试与调试使用：`rerank` 已经在远端给出最终顺序，
/// 这个结构负责把「这个顺序到底来自哪里」显式记录下来。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CandidateScores {
    pub lexical_score: f64,
    pub semantic_score: f64,
    pub rerank_score: Option<f64>,
    pub final_score: f64,
}

/// 归一化分数，便于比较不同量纲的分项。
fn normalize(values: &[f64]) -> Vec<f64> {
    let peak = values.iter().copied().fold(0.0_f64, f64::max);
    if peak <= 0.0 {
        return values.iter().map(|_| 0.0).collect();
    }
    values.iter().map(|value| (value / peak).clamp(0.0, 1.0)).collect()
}

/// 合并分项分数。rerank 命中时以 rerank 为主，本地分数只作轻微平局裁决。
pub fn combine_scores(
    lexical: &[f64],
    semantic: &[f64],
    rerank: Option<&[f64]>,
) -> Vec<CandidateScores> {
    let lexical = normalize(lexical);
    let semantic = normalize(semantic);
    let rerank = rerank.map(normalize);
    let count = lexical.len();
    (0..count)
        .map(|index| {
            let rerank_score = rerank.as_ref().map(|values| values[index]);
            let final_score = match rerank_score {
                Some(value) => value * 0.85 + lexical[index] * 0.10 + semantic[index] * 0.05,
                None => lexical[index] * 0.70 + semantic[index] * 0.30,
            };
            CandidateScores {
                lexical_score: lexical[index],
                semantic_score: semantic[index],
                rerank_score,
                final_score,
            }
        })
        .collect()
}

/// 是否启用：开关打开 + 四项配置齐全。
pub fn is_usable(settings: &RerankerSettings) -> bool {
    settings.enabled
        && !settings.provider.trim().is_empty()
        && !settings.endpoint.trim().is_empty()
        && !settings.model.trim().is_empty()
        && (MIN_TOP_N..=MAX_TOP_N).contains(&settings.top_n)
}

/// endpoint 校验：与 AI 服务同样的规则，只允许 HTTPS 或本机 HTTP。
pub fn validate_endpoint(endpoint: &str) -> Result<(), AppError> {
    let url = reqwest::Url::parse(endpoint)
        .map_err(|_| AppError::Message("Reranker Endpoint 格式无效".into()))?;
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if url.scheme() != "https" && !(local && url.scheme() == "http") {
        return Err(AppError::Message(
            "Reranker Endpoint 必须使用 HTTPS；仅本机地址允许 HTTP".into(),
        ));
    }
    Ok(())
}

pub fn validate_settings(settings: &RerankerSettings) -> Result<(), AppError> {
    if settings.provider.trim().is_empty() || settings.model.trim().is_empty() {
        return Err(AppError::Message("Reranker Provider 和模型不能为空".into()));
    }
    if !(MIN_TOP_N..=MAX_TOP_N).contains(&settings.top_n) {
        return Err(AppError::Message(format!(
            "Top N 必须在 {MIN_TOP_N} 到 {MAX_TOP_N} 之间"
        )));
    }
    validate_endpoint(&settings.endpoint)
}

/// 调用远端 rerank 并返回排序。任何失败都降级，`Ok` 里带 warning。
pub async fn rerank<T: AsRef<str>>(
    http: &reqwest::Client,
    settings: &RerankerSettings,
    api_key: &str,
    query: &str,
    candidates: &[T],
    cancelled: &AtomicBool,
) -> RerankOutcome {
    let started = Instant::now();
    let (pairs, documents) = prepare_documents(candidates);
    let mut outcome = RerankOutcome { candidate_count: documents.len(), ..Default::default() };
    if documents.is_empty() {
        outcome.warning = "没有可用于重排的候选笔记".into();
        outcome.elapsed_ms = started.elapsed().as_millis();
        return outcome;
    }
    let top_n = (settings.top_n as usize).min(documents.len());
    let body = request_body(&settings.model, query, &documents, top_n);
    let send = http
        .post(&settings.endpoint)
        .bearer_auth(api_key)
        .json(&body)
        .timeout(RERANK_TIMEOUT)
        .send();
    let response = match send.await {
        Ok(response) => response,
        Err(error) => {
            outcome.warning = rerank_failure_message(&error);
            outcome.elapsed_ms = started.elapsed().as_millis();
            return outcome;
        }
    };
    let status = response.status();
    if !status.is_success() {
        // 错误响应体可能含服务内部信息，不回传给前端。
        outcome.warning = match status.as_u16() {
            429 => "重排服务限流，本次沿用本地排序".into(),
            code if (500..600).contains(&code) => {
                format!("重排服务暂时不可用（HTTP {code}），本次沿用本地排序")
            }
            code => format!("重排服务请求失败（HTTP {code}），本次沿用本地排序"),
        };
        outcome.elapsed_ms = started.elapsed().as_millis();
        return outcome;
    }
    if cancelled.load(Ordering::Relaxed) {
        outcome.warning = "重排已取消，沿用本地排序".into();
        outcome.elapsed_ms = started.elapsed().as_millis();
        return outcome;
    }
    let value: RerankResponse = match limited_json(response, MAX_API_RESPONSE_BYTES, "Reranker").await {
        Ok(value) => value,
        Err(error) => {
            outcome.warning = format!("重排服务响应无法解析（{error}），本次沿用本地排序");
            outcome.elapsed_ms = started.elapsed().as_millis();
            return outcome;
        }
    };
    match validate_order(documents.len(), &value.results.unwrap_or_default()) {
        Ok(order) => {
            // 下标从 documents 映射回原始候选，同时保留远端分数用于调试。
            outcome.order = order.iter().map(|(index, _)| *index).collect();
            outcome.rerank_scores = order.iter().map(|(_, score)| *score).collect();
            outcome.original_order = order
                .iter()
                .map(|(index, _)| pairs.get(*index).map(|(original, _)| *original).unwrap_or(*index))
                .collect();
            outcome.applied = true;
        }
        Err(reason) => {
            outcome.warning = format!("{reason}，本次沿用本地排序");
        }
    }
    outcome.elapsed_ms = started.elapsed().as_millis();
    outcome
}

/// 打印排序来源的调试信息。
///
/// 只输出候选标识与分项分数，**不输出任何正文或问题内容**。
pub fn log_score_breakdown<F>(outcome: &RerankOutcome, lexical: &[f64], label: F)
where
    F: Fn(usize) -> String,
{
    let semantic = vec![0.0; outcome.order.len()];
    let scores = combine_scores(lexical, &semantic, Some(&outcome.rerank_scores));
    for (position, index) in outcome.order.iter().enumerate() {
        if let Some(scores) = scores.get(position) {
            eprintln!(
                "[rerank] candidate={} lexical={:.3} rerank={:.3} final={:.3}",
                label(*index),
                scores.lexical_score,
                scores.rerank_score.unwrap_or(0.0),
                scores.final_score
            );
        }
    }
}

fn rerank_failure_message(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "重排服务超时，本次沿用本地排序".into()
    } else if error.is_connect() {
        "无法连接重排服务，本次沿用本地排序".into()
    } else {
        "重排请求失败，本次沿用本地排序".into()
    }
}

/// 连接测试：用两个几乎相同但可区分的短文本，成功即返回 true。
pub async fn test_connection(
    http: &reqwest::Client,
    settings: &RerankerSettings,
    api_key: &str,
) -> Result<bool, AppError> {
    validate_settings(settings)?;
    let body = serde_json::json!({
        "model": settings.model,
        "query": "局部政府的债务风险",
        "documents": ["地方政府通过融资平台举债。", "这本书讨论的是别的内容。"],
        "top_n": 2,
        "return_documents": false,
    });
    let response = http
        .post(&settings.endpoint)
        .bearer_auth(api_key)
        .json(&body)
        .timeout(RERANK_TIMEOUT)
        .send()
        .await?;
    let status = response.status();
    if !status.is_success() {
        // 与 test_ai 一样：文案里只出现状态码，不出现响应体。
        let message = if status.as_u16() == 401 || status.as_u16() == 403 {
            "Reranker 认证失败，请检查 API Key".to_owned()
        } else if status.as_u16() == 429 {
            "Reranker 限流，请稍后再试".to_owned()
        } else {
            format!("Reranker 连接测试失败（HTTP {}）", status.as_u16())
        };
        return Err(AppError::Message(message));
    }
    let value: RerankResponse = limited_json(response, MAX_API_RESPONSE_BYTES, "Reranker").await?;
    let items = value.results.unwrap_or_default();
    validate_order(2, &items).map_err(AppError::Message)?;
    Ok(true)
}

/// 便于测试注入的最小聊天消息构造（rerank 测试不走对话）。
#[allow(dead_code)]
fn test_message() -> ChatMessage {
    ChatMessage { role: "user".into(), content: "ping".into() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn items(pairs: &[(usize, Option<f64>)]) -> Vec<RerankItem> {
        pairs
            .iter()
            .map(|(index, score)| RerankItem { index: *index, relevance_score: *score })
            .collect()
    }

    #[test]
    fn 拒绝越界下标() {
        let error = validate_order(3, &items(&[(0, Some(0.5)), (5, Some(0.4))])).unwrap_err();
        assert!(error.contains("越界"), "{error}");
    }

    #[test]
    fn 拒绝重复下标() {
        let error = validate_order(3, &items(&[(1, Some(0.5)), (1, Some(0.4))])).unwrap_err();
        assert!(error.contains("重复"), "{error}");
    }

    #[test]
    fn 拒绝非有限分数() {
        let error = validate_order(2, &items(&[(0, Some(f64::NAN))])).unwrap_err();
        assert!(error.contains("非有限"), "{error}");
        let error = validate_order(2, &items(&[(0, Some(f64::INFINITY))])).unwrap_err();
        assert!(error.contains("非有限"), "{error}");
    }

    #[test]
    fn 拒绝空结果() {
        assert!(validate_order(3, &[]).unwrap_err().contains("没有返回"));
    }

    #[test]
    fn 接受合法排序并保留分数() {
        let order = validate_order(3, &items(&[(2, Some(0.9)), (0, Some(0.1))])).unwrap();
        assert_eq!(order, vec![(2, 0.9), (0, 0.1)]);
    }

    #[test]
    fn 远端下标会映射回原始候选() {
        // 候选 0 与 1 正文相同被去重，documents 下标 0 其实对应原始候选 1
        let candidates = vec!["重复内容".to_string(), "重复内容".to_string(), "独特内容".to_string()];
        let (pairs, documents) = prepare_documents(&candidates);
        assert_eq!(documents.len(), 2);
        assert_eq!(pairs, vec![(0, "重复内容".to_string()), (2, "独特内容".to_string())]);

        // 远端说 documents 下标 1 最相关
        let order = validate_order(documents.len(), &items(&[(1, Some(0.9)), (0, Some(0.2))])).unwrap();
        let original: Vec<usize> = order
            .iter()
            .map(|(index, _)| pairs.get(*index).map(|(original, _)| *original).unwrap_or(*index))
            .collect();
        assert_eq!(original, vec![2, 0], "必须映射回原始候选而不是 documents 下标");
    }

    #[test]
    fn 缺少分数字段时按零分处理而不是报错() {
        // 某些服务不返回 relevance_score，此时只按 index 排序即可
        let order = validate_order(2, &items(&[(1, None), (0, None)])).unwrap();
        assert_eq!(order, vec![(1, 0.0), (0, 0.0)]);
    }

    #[test]
    fn 候选正文会截断并去重() {
        let long = "字".repeat(MAX_CANDIDATE_CHARS + 50);
        let candidates = vec![
            "第一条候选笔记".to_string(),
            "第一条候选笔记".to_string(), // 完全重复
            long.clone(),
            "   ".to_string(), // 空白直接丢弃
        ];
        let (pairs, documents) = prepare_documents(&candidates);
        assert_eq!(documents.len(), 2, "重复项与空白项都应被去掉");
        assert_eq!(pairs[0].0, 0);
        assert_eq!(pairs[1].0, 2, "原始下标必须保留，供回写排序");
        assert_eq!(documents[1].chars().count(), MAX_CANDIDATE_CHARS);
    }

    #[test]
    fn 候选数量有硬上限() {
        let candidates: Vec<String> = (0..MAX_CANDIDATES + 30).map(|i| format!("候选 {i}")).collect();
        let (_, documents) = prepare_documents(&candidates);
        assert_eq!(documents.len(), MAX_CANDIDATES);
    }

    #[test]
    fn 每本书都有最低覆盖() {
        let book_ids: Vec<String> = ["b1", "b1", "b1", "b2", "b3"]
            .iter()
            .map(|value| value.to_string())
            .collect();
        // rerank 顺序把 b1 的三条排在最前
        let selected = apply_book_coverage(&[0, 1, 2, 3, 4], &book_ids, 3, 1);
        assert!(selected.contains(&0) && selected.contains(&3) && selected.contains(&4), "{selected:?}");
        assert!(!selected.contains(&1) && !selected.contains(&2), "b1 不应超过每本 1 条");
        assert_eq!(selected.len(), 3);
    }

    #[test]
    fn 覆盖规则不会丢候选() {
        let book_ids: Vec<String> = vec!["b1".into(); 5];
        let order: Vec<usize> = (0..5).collect();
        let selected = apply_book_coverage(&order, &book_ids, 4, 1);
        assert_eq!(selected.len(), 4, "名额没满时应把溢出项补回来");
    }

    #[test]
    fn 覆盖规则忽略越界下标() {
        let book_ids: Vec<String> = vec!["b1".into()];
        let selected = apply_book_coverage(&[0, 9], &book_ids, 2, 1);
        assert_eq!(selected, vec![0]);
    }

    #[test]
    fn 分项分数可解释且归一化() {
        let lexical = vec![10.0, 5.0, 0.0];
        let semantic = vec![1.0, 0.5, 0.0];
        let rerank = vec![0.1, 0.9, 0.5];
        let scores = combine_scores(&lexical, &semantic, Some(&rerank));
        // rerank 命中时由 rerank 主导
        assert!(scores[1].final_score > scores[0].final_score);
        assert_eq!(scores[1].rerank_score, Some(1.0));
        assert_eq!(scores[0].lexical_score, 1.0);
        // 没有 rerank 时退回本地加权
        let local = combine_scores(&lexical, &semantic, None);
        assert!(local.iter().all(|item| item.rerank_score.is_none()));
        assert!(local[0].final_score > local[2].final_score);
    }

    #[test]
    fn 只有开关和配置齐全才算可用() {
        let base = RerankerSettings { provider: "Cohere".into(), endpoint: "https://api.cohere.com/v1/rerank".into(), model: "rerank-multilingual-v3.0".into(), top_n: 8, enabled: true };
        assert!(is_usable(&base));
        let mut disabled = base.clone();
        disabled.enabled = false;
        assert!(!is_usable(&disabled));
        let mut bad_top = base.clone();
        bad_top.top_n = 0;
        assert!(!is_usable(&bad_top));
        let mut no_model = base;
        no_model.model = String::new();
        assert!(!is_usable(&no_model));
    }

    #[test]
    fn endpoint_只允许_https_或本机_http() {
        assert!(validate_endpoint("https://api.cohere.com/v1/rerank").is_ok());
        assert!(validate_endpoint("http://localhost:8080/rerank").is_ok());
        assert!(validate_endpoint("http://api.example.com/rerank").is_err());
        assert!(validate_endpoint("not a url").is_err());
    }

    #[test]
    fn 配置校验覆盖_top_n_与空字段() {
        let base = RerankerSettings { provider: "Jina".into(), endpoint: "https://api.jina.ai/v1/rerank".into(), model: "rerank-v2".into(), top_n: 8, enabled: true };
        assert!(validate_settings(&base).is_ok());
        let mut too_big = base.clone();
        too_big.top_n = 500;
        assert!(validate_settings(&too_big).unwrap_err().to_string().contains("Top N"));
        let mut empty = base;
        empty.provider = " ".into();
        assert!(validate_settings(&empty).is_err());
    }

    #[test]
    fn 凭据名带_reranker_前缀() {
        assert_eq!(credential_name("Cohere"), "reranker:Cohere");
    }

    #[test]
    fn 请求体不带本地其他数据() {
        let body = request_body("m", "问题", &["候选".to_string()], 1);
        // 只能出现 query / documents / model 这些字段
        let object = body.as_object().unwrap();
        assert_eq!(object.len(), 5);
        assert!(object.contains_key("query"));
        assert!(object.contains_key("documents"));
        // return_documents 必须为 false，避免把原文再要回来
        assert_eq!(object["return_documents"], false);
    }
}
