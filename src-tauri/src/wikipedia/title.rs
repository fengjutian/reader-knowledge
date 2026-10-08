//! 标题规范化与稳定键。
//!
//! 规则来自需求 7.5：NFC、去首尾空白、下划线转空格、保留原始简繁、
//! 不强制做简繁转换（简繁只允许作为搜索辅助键，不能当唯一键）、
//! 首字母大小写遵循 MediaWiki 行为。

use unicode_normalization::UnicodeNormalization;

pub const WIKIPEDIA_ARTICLE_BASE: &str = "https://zh.wikipedia.org/wiki/";

/// 展示用标题：下划线转空格、NFC、压缩空白、首字母大写。
pub fn normalize_title(raw: &str) -> String {
    let replaced: String = raw
        .trim()
        .chars()
        .map(|value| if value == '_' { ' ' } else { value })
        .collect();
    let composed: String = replaced.nfc().collect();
    capitalize_first_letter(&collapse_whitespace(&composed))
}

/// 搜索/去重键：在展示标题基础上再折叠大小写。
/// 注意这里不做简繁转换，也不做全角半角转换，避免把不同词条合并。
pub fn normalize_search_key(raw: &str) -> String {
    normalize_title(raw).to_lowercase()
}

/// MediaWiki 会把标题首字母大写，只对有大小写的字符生效，中文不受影响。
fn capitalize_first_letter(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        Some(first) if first.is_lowercase() => {
            let mut out = String::with_capacity(value.len() + 2);
            out.extend(first.to_uppercase());
            out.push_str(chars.as_str());
            out
        }
        _ => value.to_string(),
    }
}

fn collapse_whitespace(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut pending_space = false;
    for value_char in value.chars() {
        if value_char.is_whitespace() {
            pending_space = !out.is_empty();
            continue;
        }
        if pending_space {
            out.push(' ');
            pending_space = false;
        }
        out.push(value_char);
    }
    out
}

/// 软重定向：zhwiki 里是 `#REDIRECT` 或 `#重定向`。
pub fn is_soft_redirect(raw: &str) -> bool {
    let trimmed = raw.trim_start();
    let upper = trimmed.to_uppercase();
    upper.starts_with("#REDIRECT") || trimmed.starts_with("#重定向")
}

/// 词条原文链接。中文标题需要逐字节百分号编码，空格按 MediaWiki 习惯写成下划线。
pub fn page_url(title: &str) -> String {
    format!(
        "{WIKIPEDIA_ARTICLE_BASE}{}",
        percent_encode(&normalize_title(title).replace(' ', "_"))
    )
}

fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.as_bytes() {
        let value = *byte as char;
        if value.is_ascii_alphanumeric() || matches!(value, '-' | '_' | '.' | '~') {
            out.push(value);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// 明显列表页：中文维基的列表/名单/年表类标题。
/// 只做保守的“结尾匹配”，避免误杀正常词条；命中后仍按可确认候选处理。
pub fn looks_like_list_page(title: &str) -> bool {
    const SUFFIXES: &[&str] = &[
        "列表",
        "名单",
        "年表",
        "一览",
        "列表页",
        "对照表",
        "一览表",
        "分类",
        "年鉴",
    ];
    SUFFIXES
        .iter()
        .any(|suffix| title.len() > suffix.len() && title.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 下划线转空格并去掉首尾空白() {
        assert_eq!(normalize_title("  Artificial_intelligence  "), "Artificial intelligence");
    }

    #[test]
    fn 连续空白压缩成一个空格() {
        assert_eq!(normalize_title("北京   大学\t医学  部门"), "北京 大学 医学 部门");
    }

    #[test]
    fn 首字母按_mediawiki_行为大写() {
        assert_eq!(normalize_title("entropY"), "EntropY");
        assert_eq!(normalize_title("北京"), "北京");
    }

    #[test]
    fn 繁体标题原样保留不做简繁转换() {
        let traditional = "電腦";
        assert_eq!(normalize_title(traditional), traditional);
        // 简体与繁体是两个不同词条，搜索键不能把它们合并。
        assert_ne!(normalize_search_key("计算机"), normalize_search_key("電腦"));
    }

    #[test]
    fn 搜索键忽略大小写与首尾空白() {
        assert_eq!(normalize_search_key("Machine Learning"), normalize_search_key("machine   learning"));
    }

    #[test]
    fn 组合字符按_nfc_合成() {
        // "e" + 组合尖音符 规范化后应与预组合字符一致。
        // 展示标题还会做首字母大写（MediaWiki 行为），所以期望值是 "Café"。
        assert_eq!(normalize_title("cafe\u{0301}"), "Café");
        // 搜索键不做大小写折叠以外的变换，组合字符同样被合成。
        assert_eq!(normalize_search_key("cafe\u{0301}"), "café");
    }

    #[test]
    fn 软重定向可识别() {
        assert!(is_soft_redirect("#REDIRECT [[目标条目]]"));
        assert!(is_soft_redirect("  #redirect [[Foo]]"));
        assert!(is_soft_redirect("#重定向[[目标条目]]"));
        assert!(!is_soft_redirect("REDIRECT"));
        assert!(!is_soft_redirect("正常词条"));
    }

    #[test]
    fn 原文链接按_mediawiki_规则编码() {
        assert_eq!(page_url("Artificial intelligence"), "https://zh.wikipedia.org/wiki/Artificial_intelligence");
        assert_eq!(page_url("人工智能"), "https://zh.wikipedia.org/wiki/%E4%BA%BA%E5%B7%A5%E6%99%BA%E8%83%BD");
        // 标题里的斜杠必须编码，否则会被当成子路径。
        assert_eq!(page_url("A/B"), "https://zh.wikipedia.org/wiki/A%2FB");
    }

    #[test]
    fn 列表页判定保守() {
        assert!(looks_like_list_page("中国电视剧列表"));
        assert!(looks_like_list_page("奥运奖牌名单"));
        assert!(!looks_like_list_page("列表"));
        assert!(!looks_like_list_page("列表的集合运算"));
        assert!(!looks_like_list_page("计算机"));
    }
}
