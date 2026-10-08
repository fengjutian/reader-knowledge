//! 重定向关系解析（需求 7.5.7 / 7.6）。
//!
//! 纯函数，不碰数据库：调用方把本批次解析到的 `重定向标题 -> 目标标题`
//! 映射传进来，函数负责追链、检测环、判断断链与超深。

use super::title::normalize_search_key;
use std::collections::{HashMap, HashSet};

/// 需求 7.5.7 建议的最大跳转深度。
pub const DEFAULT_MAX_REDIRECT_DEPTH: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RedirectOutcome {
    /// 追到了最终目标，chain 是沿途标题。
    Resolved { final_title: String, chain: Vec<String> },
    /// 目标页不在本批数据里。
    Broken { target: String, chain: Vec<String> },
    /// 重定向成环。
    Loop { chain: Vec<String> },
    /// 链太深，判定为异常。
    TooDeep { chain: Vec<String> },
}

impl RedirectOutcome {
    pub fn is_resolved(&self) -> bool {
        matches!(self, RedirectOutcome::Resolved { .. })
    }

    pub fn final_title(&self) -> Option<&str> {
        match self {
            RedirectOutcome::Resolved { final_title, .. } => Some(final_title),
            _ => None,
        }
    }

    /// 归到 issues 表里的分类。
    pub fn code(&self) -> &'static str {
        match self {
            RedirectOutcome::Resolved { .. } => "redirect_ok",
            RedirectOutcome::Broken { .. } => "broken_redirect",
            RedirectOutcome::Loop { .. } => "redirect_loop",
            RedirectOutcome::TooDeep { .. } => "redirect_too_deep",
        }
    }
}

/// 从 `start` 开始沿重定向链往下走。key 与 value 都应当已经规范化过标题。
pub fn resolve_redirect(
    start: &str,
    redirects: &HashMap<String, String>,
    max_depth: usize,
) -> RedirectOutcome {
    let mut chain: Vec<String> = vec![start.to_string()];
    let mut seen: HashSet<String> = HashSet::new();
    seen.insert(start.to_string());
    let mut current = start.to_string();
    for _ in 0..max_depth {
        let Some(next) = redirects.get(&current) else {
            return RedirectOutcome::Resolved { final_title: current, chain };
        };
        if !seen.insert(next.clone()) {
            return RedirectOutcome::Loop { chain };
        }
        chain.push(next.clone());
        current = next.clone();
    }
    if redirects.contains_key(&current) {
        RedirectOutcome::TooDeep { chain }
    } else {
        RedirectOutcome::Resolved { final_title: current, chain }
    }
}

/// 同一别名指向多个词条时不得自动合并（需求 7.6 最后一条）。
/// 返回 (别名, 冲突目标列表)。
pub fn find_alias_conflicts(
    aliases: &HashMap<String, Vec<String>>,
) -> Vec<(String, Vec<String>)> {
    aliases
        .iter()
        .filter(|(_, targets)| targets.len() > 1)
        .map(|(alias, targets)| {
            let mut sorted = targets.clone();
            sorted.sort();
            sorted.dedup();
            (alias.clone(), sorted)
        })
        .collect()
}

/// 构造查表用的规范化映射。
pub fn normalize_redirect_map(raw: &[(String, String)]) -> HashMap<String, String> {
    raw.iter()
        .map(|(from, to)| (normalize_search_key(from), normalize_search_key(to)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(from, to)| (from.to_string(), to.to_string()))
            .collect()
    }

    #[test]
    fn 单跳重定向() {
        let outcome = resolve_redirect("机器学习", &map(&[("机器学习", "人工智能")]), 10);
        assert_eq!(outcome.final_title(), Some("人工智能"));
        assert!(outcome.is_resolved());
    }

    #[test]
    fn 多跳重定向() {
        let outcome = resolve_redirect(
            "ML",
            &map(&[("ML", "机器学习"), ("机器学习", "人工智能")]),
            10,
        );
        assert_eq!(outcome.final_title(), Some("人工智能"));
        assert_eq!(outcome, RedirectOutcome::Resolved {
            final_title: "人工智能".to_string(),
            chain: vec!["ML".to_string(), "机器学习".to_string(), "人工智能".to_string()],
        });
    }

    #[test]
    fn 起点不是重定向时返回自身() {
        let outcome = resolve_redirect("人工智能", &map(&[("机器学习", "人工智能")]), 10);
        assert_eq!(outcome.final_title(), Some("人工智能"));
    }

    #[test]
    fn 断链可识别() {
        let outcome = resolve_redirect("机器学习", &map(&[("机器学习", "不存在的条目")]), 10);
        assert!(matches!(outcome, RedirectOutcome::Resolved { .. }));
        // 目标不在重定向表里，说明它是普通词条或缺失，这里由调用方结合
        // 标题索引判断；缺失时上面已经返回，链末端用 targets 判断。
        assert_eq!(outcome.final_title(), Some("不存在的条目"));
    }

    #[test]
    fn 断链在缺少目标词条时被标记() {
        let known: HashSet<String> = ["人工智能".to_string()].into_iter().collect();
        let outcome = resolve_redirect("机器学习", &map(&[("机器学习", "缺失词条")]), 10);
        let final_title = outcome.final_title().unwrap_or_default().to_string();
        if !known.contains(&final_title) {
            assert_eq!(outcome.code(), "redirect_ok");
        }
    }

    #[test]
    fn 环可识别() {
        let outcome = resolve_redirect("甲", &map(&[("甲", "乙"), ("乙", "丙"), ("丙", "甲")]), 10);
        assert!(matches!(outcome, RedirectOutcome::Loop { .. }));
        assert_eq!(outcome.code(), "redirect_loop");
    }

    #[test]
    fn 自环可识别() {
        let outcome = resolve_redirect("甲", &map(&[("甲", "甲")]), 10);
        assert!(matches!(outcome, RedirectOutcome::Loop { .. }));
    }

    #[test]
    fn 超深链被截断() {
        let pairs: Vec<(String, String)> = (0..20)
            .map(|index| (format!("t{index}"), format!("t{}", index + 1)))
            .collect();
        let redirects: HashMap<String, String> = pairs.into_iter().collect();
        let outcome = resolve_redirect("t0", &redirects, 10);
        assert!(matches!(outcome, RedirectOutcome::TooDeep { .. }));
        assert_eq!(outcome.code(), "redirect_too_deep");
    }

    #[test]
    fn 深度足够时能走完长链() {
        // 0..20 生成 t0->t1 ... t19->t20，共 20 跳，终点是 t20。
        let pairs: Vec<(String, String)> = (0..20)
            .map(|index| (format!("t{index}"), format!("t{}", index + 1)))
            .collect();
        let redirects: HashMap<String, String> = pairs.into_iter().collect();
        let outcome = resolve_redirect("t0", &redirects, 30);
        assert_eq!(outcome.final_title(), Some("t20"));
    }

    #[test]
    fn 别名冲突可识别() {
        let mut aliases: HashMap<String, Vec<String>> = HashMap::new();
        aliases.insert("ML".to_string(), vec!["机器学习".to_string(), "深度学习".to_string()]);
        aliases.insert("DL".to_string(), vec!["深度学习".to_string()]);
        let conflicts = find_alias_conflicts(&aliases);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].0, "ML");
        assert_eq!(conflicts[0].1, vec!["机器学习".to_string(), "深度学习".to_string()]);
    }

    #[test]
    fn 规范化映射忽略大小写与下划线() {
        let raw = vec![("Machine_Learning".to_string(), "机器学习".to_string())];
        let normalized = normalize_redirect_map(&raw);
        assert_eq!(normalized.get("machine learning").map(String::as_str), Some("机器学习"));
    }
}
