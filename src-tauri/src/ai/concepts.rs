//! 概念抽取的纯逻辑：候选生成、AI 输出校验、证据校验。
//!
//! 设计约束（对应实现计划 7.3）：
//! - 先用本地名词库/关键词生成候选，再让 AI 只做「挑选 + 归一 + 关系」，
//!   以降低成本并让输出可控。
//! - **每个实体必须至少绑定一条真实存在的笔记**；AI 伪造的 noteId 一律拒绝。
//! - 关系两端必须是本次或库中已存在的实体，否则整条关系丢弃。
//! - 一切校验都在写库之前完成，脏数据不会落盘。

use crate::models::{ConceptEvidence, RELATION_KINDS};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

/// 单条笔记送给 AI 的字符上限。
pub const MAX_NOTE_CHARS: usize = 400;
/// 一次发给 AI 的笔记条数。
pub const MAX_NOTES_PER_BATCH: usize = 20;
/// 概念名最大长度，防止模型返回整句话。
pub const MAX_NAME_CHARS: usize = 40;
/// 证据引文最大长度。
pub const MAX_QUOTE_CHARS: usize = 160;

/// AI 返回的单个实体。
#[derive(Debug, Clone, Deserialize)]
pub struct RawEntity {
    pub kind: String,
    #[serde(alias = "name")]
    pub canonical_name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// AI 认为的置信度，可能缺失或非法
    #[serde(default)]
    pub confidence: Option<f64>,
    /// 证据笔记 ID
    #[serde(default)]
    pub evidence_note_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawRelation {
    pub from: String,
    pub to: String,
    pub relation: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub confidence: Option<f64>,
    #[serde(default)]
    pub evidence_note_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawExtraction {
    #[serde(default)]
    pub entities: Vec<RawEntity>,
    #[serde(default)]
    pub relations: Vec<RawRelation>,
}

/// 校验并规范化后的实体，可以直接写库。
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedEntity {
    pub kind: String,
    pub canonical_name: String,
    pub description: String,
    pub aliases: Vec<String>,
    pub evidence: Vec<ConceptEvidence>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedRelation {
    pub from_name: String,
    pub to_name: String,
    pub relation: String,
    pub summary: String,
    pub confidence: f64,
    pub evidence: Vec<ConceptEvidence>,
}

#[derive(Debug, Clone, Default)]
pub struct ValidatedExtraction {
    pub entities: Vec<ValidatedEntity>,
    pub relations: Vec<ValidatedRelation>,
    /// 被拒绝的原因统计，便于在扫描结果里如实汇报
    pub rejected_without_evidence: usize,
    pub rejected_unknown_note: usize,
    pub rejected_unknown_endpoint: usize,
    pub rejected_bad_kind: usize,
}

/// 本地候选：已确认的名词库条目 + 从笔记里抽的高频词。
#[derive(Debug, Clone, Default)]
pub struct LocalCandidates {
    /// 名词库条目：别名（小写）→ canonical name
    pub glossary_terms: HashMap<String, String>,
    /// 高频二元组，按频次降序
    pub frequent_phrases: Vec<String>,
}

/// 从正文里抽候选词。
///
/// 中文没有空格分词，这里用「已有名词库优先 + 二元组统计」两条腿：
/// 命中名词库的词直接作为候选，其余用出现频次较高的二元组兜底。
pub fn extract_candidates(
    notes: &[(&str, &str)],
    glossary: &[(String, String, Vec<String>)],
    max_terms: usize,
) -> LocalCandidates {
    let mut glossary_terms: HashMap<String, String> = HashMap::new();
    for (canonical, _definition, aliases) in glossary {
        let canonical = canonical.trim();
        if canonical.is_empty() {
            continue;
        }
        glossary_terms.insert(canonical.to_lowercase(), canonical.to_owned());
        for alias in aliases {
            let alias = alias.trim();
            if !alias.is_empty() {
                glossary_terms.insert(alias.to_lowercase(), canonical.to_owned());
            }
        }
    }
    let mut counts: HashMap<String, usize> = HashMap::new();
    for (_id, content) in notes {
        let normalized: Vec<char> = content
            .chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect();
        for window in normalized.windows(2) {
            let phrase: String = window.iter().collect();
            // 至少要有一个汉字，避免「of」「the」这类停用词进候选
            if phrase.chars().any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)) {
                *counts.entry(phrase).or_default() += 1;
            }
        }
    }
    let mut frequent: Vec<(String, usize)> = counts.into_iter().filter(|(_, count)| *count >= 2).collect();
    // 频次高的优先；同频次按字典序保证结果稳定
    frequent.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
    let frequent_phrases = frequent
        .into_iter()
        .map(|(phrase, _)| phrase)
        .take(max_terms)
        .collect();
    LocalCandidates { glossary_terms, frequent_phrases }
}

/// 规范化概念名：去空白、限长、折叠内部连续空白。
pub fn normalize_name(raw: &str) -> Option<String> {
    let collapsed = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return None;
    }
    let trimmed: String = collapsed.chars().take(MAX_NAME_CHARS).collect();
    let trimmed = trimmed.trim().to_owned();
    (!trimmed.is_empty()).then_some(trimmed)
}

/// 证据笔记是否真实存在且未删除。
///
/// 这是防止 AI 伪造证据的关键：只有调用方传入的真实笔记集合才算数。
fn evidence_lookup(notes: &[(String, String, String, String)]) -> HashMap<&str, (&str, &str)> {
    // (note_id, book_id, book_title, content)
    notes
        .iter()
        .map(|(id, book_id, _title, content)| (id.as_str(), (book_id.as_str(), content.as_str())))
        .collect()
}

/// 校验并规范化 AI 的抽取结果。
pub fn validate_extraction(
    raw: &RawExtraction,
    notes: &[(String, String, String, String)],
    known_entity_names: &HashSet<String>,
) -> ValidatedExtraction {
    let lookup = evidence_lookup(notes);
    let mut result = ValidatedExtraction::default();
    let mut seen_names: HashSet<String> = HashSet::new();

    for entity in &raw.entities {
        let Some(name) = normalize_name(&entity.canonical_name) else { continue };
        let kind = entity.kind.trim().to_lowercase();
        if !crate::models::ENTITY_KINDS.contains(&kind.as_str()) {
            result.rejected_bad_kind += 1;
            continue;
        }
        // 同一次抽取里同名实体只保留第一个
        if !seen_names.insert(name.clone()) {
            continue;
        }
        // 证据必须落在真实笔记上
        let mut evidence: Vec<ConceptEvidence> = Vec::new();
        for note_id in &entity.evidence_note_ids {
            let note_id = note_id.trim();
            if note_id.is_empty() {
                continue;
            }
            match lookup.get(note_id) {
                Some((book_id, content)) => {
                    let quote: String = content.chars().take(MAX_QUOTE_CHARS).collect();
                    let confidence = entity.confidence.filter(|value| value.is_finite()).unwrap_or(0.5).clamp(0.0, 1.0);
                    evidence.push(ConceptEvidence {
                        note_id: note_id.to_owned(),
                        book_id: (*book_id).to_owned(),
                        quote,
                        confidence,
                    });
                }
                // AI 伪造的 noteId：直接丢证据，不写库
                None => result.rejected_unknown_note += 1,
            }
        }
        // 没有真实证据的实体不入库
        if evidence.is_empty() {
            result.rejected_without_evidence += 1;
            continue;
        }
        let mut aliases: Vec<String> = entity
            .aliases
            .iter()
            .filter_map(|alias| normalize_name(alias))
            .filter(|alias| *alias != name)
            .collect();
        aliases.dedup();
        result.entities.push(ValidatedEntity {
            kind,
            canonical_name: name,
            description: entity.description.chars().take(MAX_QUOTE_CHARS * 2).collect(),
            aliases,
            evidence,
        });
    }

    for relation in &raw.relations {
        let (Some(from), Some(to)) = (normalize_name(&relation.from), normalize_name(&relation.to)) else {
            continue;
        };
        let kind = relation.relation.trim().to_lowercase();
        if !RELATION_KINDS.contains(&kind.as_str()) {
            result.rejected_bad_kind += 1;
            continue;
        }
        // 自环没有信息量
        if from == to {
            continue;
        }
        // 两端必须都存在：本次抽取产出的，或库里已有的
        let known = seen_names.contains(&from) || known_entity_names.contains(&from);
        let known_to = seen_names.contains(&to) || known_entity_names.contains(&to);
        if !known || !known_to {
            result.rejected_unknown_endpoint += 1;
            continue;
        }
        let mut evidence: Vec<ConceptEvidence> = Vec::new();
        for note_id in &relation.evidence_note_ids {
            let note_id = note_id.trim();
            if let Some((book_id, content)) = lookup.get(note_id) {
                let quote: String = content.chars().take(MAX_QUOTE_CHARS).collect();
                let confidence = relation.confidence.filter(|value| value.is_finite()).unwrap_or(0.5).clamp(0.0, 1.0);
                evidence.push(ConceptEvidence {
                    note_id: note_id.to_owned(),
                    book_id: (*book_id).to_owned(),
                    quote,
                    confidence,
                });
            }
        }
        let confidence = relation.confidence.filter(|value| value.is_finite()).unwrap_or(0.5).clamp(0.0, 1.0);
        result.relations.push(ValidatedRelation {
            from_name: from,
            to_name: to,
            relation: kind,
            summary: relation.summary.chars().take(MAX_QUOTE_CHARS * 2).collect(),
            confidence,
            evidence,
        });
    }
    result
}

/// 概念名到已有实体 ID 的查找键：canonical name 与所有别名都算。
pub fn entity_lookup_keys(canonical_name: &str, aliases: &[String]) -> Vec<String> {
    let mut keys = vec![canonical_name.trim().to_lowercase()];
    keys.extend(aliases.iter().map(|alias| alias.trim().to_lowercase()).filter(|key| !key.is_empty()));
    keys.sort();
    keys.dedup();
    keys
}

/// 稳定的实体 ID：同名同 kind 永远是同一个 ID，避免重复建节点。
pub fn entity_id(kind: &str, canonical_name: &str) -> String {
    let digest = format!("{:x}", md5::compute(format!("{}|{kind}", canonical_name.trim().to_lowercase()).as_bytes()));
    format!("{kind}:{digest}")
}

/// 稳定的关系 ID。
pub fn relation_id(from: &str, to: &str, relation: &str) -> String {
    let (left, right) = if from <= to { (from, to) } else { (to, from) };
    let digest = format!("{:x}", md5::compute(format!("{left}|{right}|{relation}").as_bytes()));
    format!("rel:{digest}")
}

/// 把一批笔记切成不超过 `MAX_NOTES_PER_BATCH` 的小批。
pub fn batch_notes<'a, T: Clone>(notes: &'a [T]) -> Vec<Vec<T>> {
    notes
        .chunks(MAX_NOTES_PER_BATCH)
        .map(<[T]>::to_vec)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notes() -> Vec<(String, String, String, String)> {
        vec![
            ("n1".into(), "b1".into(), "书一".into(), "地方政府的债务问题非常突出".into()),
            ("n2".into(), "b1".into(), "书一".into(), "我对地方融资平台的看法".into()),
            ("n3".into(), "b2".into(), "书二".into(), "组织管理决定了效率".into()),
        ]
    }

    fn raw_entity(name: &str, kind: &str, evidence: &[&str]) -> RawEntity {
        RawEntity {
            kind: kind.into(),
            canonical_name: name.into(),
            description: "描述".into(),
            aliases: vec![],
            confidence: Some(0.8),
            evidence_note_ids: evidence.iter().map(|value| (*value).to_owned()).collect(),
        }
    }

    #[test]
    fn 保留有真实证据的实体() {
        let raw = RawExtraction { entities: vec![raw_entity("地方政府债务", "concept", &["n1"])], relations: vec![] };
        let result = validate_extraction(&raw, &notes(), &HashSet::new());
        assert_eq!(result.entities.len(), 1);
        assert_eq!(result.entities[0].canonical_name, "地方政府债务");
        assert_eq!(result.entities[0].evidence[0].note_id, "n1");
        assert_eq!(result.entities[0].evidence[0].book_id, "b1");
    }

    #[test]
    fn 拒绝没有证据的实体() {
        let raw = RawExtraction { entities: vec![raw_entity("凭空来的概念", "concept", &[])], relations: vec![] };
        let result = validate_extraction(&raw, &notes(), &HashSet::new());
        assert!(result.entities.is_empty());
        assert_eq!(result.rejected_without_evidence, 1);
    }

    #[test]
    fn 拒绝_ai_伪造的_note_id() {
        // AI 声称 n99 有证据，但库里没有这条笔记
        let raw = RawExtraction { entities: vec![raw_entity("伪造概念", "concept", &["n99"])], relations: vec![] };
        let result = validate_extraction(&raw, &notes(), &HashSet::new());
        assert!(result.entities.is_empty(), "伪造证据的实体不得入库");
        assert_eq!(result.rejected_unknown_note, 1);
        assert_eq!(result.rejected_without_evidence, 1);
    }

    #[test]
    fn 部分伪造的证据只保留真实部分() {
        let raw = RawExtraction { entities: vec![raw_entity("混合概念", "concept", &["n1", "n99"])], relations: vec![] };
        let result = validate_extraction(&raw, &notes(), &HashSet::new());
        assert_eq!(result.entities.len(), 1);
        assert_eq!(result.entities[0].evidence.len(), 1);
        assert_eq!(result.entities[0].evidence[0].note_id, "n1");
        assert_eq!(result.rejected_unknown_note, 1);
    }

    #[test]
    fn 拒绝未知类型() {
        let raw = RawExtraction { entities: vec![raw_entity("怪东西", "paragraph", &["n1"])], relations: vec![] };
        let result = validate_extraction(&raw, &notes(), &HashSet::new());
        assert!(result.entities.is_empty());
        assert_eq!(result.rejected_bad_kind, 1);
    }

    #[test]
    fn 同批次同名实体只保留一个() {
        let raw = RawExtraction {
            entities: vec![raw_entity("重复概念", "concept", &["n1"]), raw_entity("重复概念", "topic", &["n2"])],
            relations: vec![],
        };
        let result = validate_extraction(&raw, &notes(), &HashSet::new());
        assert_eq!(result.entities.len(), 1);
        assert_eq!(result.entities[0].kind, "concept");
    }

    #[test]
    fn 关系两端必须存在() {
        let raw = RawExtraction {
            entities: vec![raw_entity("甲", "concept", &["n1"]), raw_entity("乙", "concept", &["n2"])],
            relations: vec![RawRelation {
                from: "甲".into(),
                to: "不存在".into(),
                relation: "related".into(),
                summary: String::new(),
                confidence: Some(0.7),
                evidence_note_ids: vec!["n1".into()],
            }],
        };
        let result = validate_extraction(&raw, &notes(), &HashSet::new());
        assert!(result.relations.is_empty());
        assert_eq!(result.rejected_unknown_endpoint, 1);
    }

    #[test]
    fn 关系可以指向库中已有实体() {
        let known: HashSet<String> = ["历史概念".to_string()].into_iter().collect();
        let raw = RawExtraction {
            entities: vec![raw_entity("新概念", "concept", &["n1"])],
            relations: vec![RawRelation {
                from: "新概念".into(),
                to: "历史概念".into(),
                relation: "related".into(),
                summary: "有联系".into(),
                confidence: Some(0.6),
                evidence_note_ids: vec!["n1".into()],
            }],
        };
        let result = validate_extraction(&raw, &notes(), &known);
        assert_eq!(result.relations.len(), 1);
        assert_eq!(result.relations[0].to_name, "历史概念");
    }

    #[test]
    fn 拒绝自环与未知关系类型() {
        let raw = RawExtraction {
            entities: vec![raw_entity("甲", "concept", &["n1"]), raw_entity("乙", "concept", &["n2"])],
            relations: vec![
                RawRelation { from: "甲".into(), to: "甲".into(), relation: "related".into(), summary: String::new(), confidence: Some(0.5), evidence_note_ids: vec![] },
                RawRelation { from: "甲".into(), to: "乙".into(), relation: "像亲戚".into(), summary: String::new(), confidence: Some(0.5), evidence_note_ids: vec![] },
            ],
        };
        let result = validate_extraction(&raw, &notes(), &HashSet::new());
        assert!(result.relations.is_empty());
        assert_eq!(result.rejected_bad_kind, 1);
    }

    #[test]
    fn 非法置信度被夹到合法区间() {
        let mut entity = raw_entity("甲", "concept", &["n1"]);
        entity.confidence = Some(f64::NAN);
        let raw = RawExtraction { entities: vec![entity], relations: vec![] };
        let result = validate_extraction(&raw, &notes(), &HashSet::new());
        assert!(result.entities[0].evidence[0].confidence.is_finite());
        assert!((0.0..=1.0).contains(&result.entities[0].evidence[0].confidence));

        let mut entity = raw_entity("乙", "concept", &["n2"]);
        entity.confidence = Some(9.0);
        let raw = RawExtraction { entities: vec![entity], relations: vec![] };
        let result = validate_extraction(&raw, &notes(), &HashSet::new());
        assert_eq!(result.entities[0].evidence[0].confidence, 1.0);
    }

    #[test]
    fn 概念名规范化会限长与折叠空白() {
        assert_eq!(normalize_name("  组织   管理  ").as_deref(), Some("组织 管理"));
        assert_eq!(normalize_name("   "), None);
        let long = "字".repeat(100);
        assert_eq!(normalize_name(&long).unwrap().chars().count(), MAX_NAME_CHARS);
    }

    #[test]
    fn 别名不会与规范名重复() {
        let mut entity = raw_entity("组织管理", "concept", &["n3"]);
        entity.aliases = vec!["组织管理".into(), "  组织管理  ".into(), "有效别名".into()];
        let raw = RawExtraction { entities: vec![entity], relations: vec![] };
        let result = validate_extraction(&raw, &notes(), &HashSet::new());
        assert_eq!(result.entities[0].aliases, vec!["有效别名".to_string()]);
    }

    #[test]
    fn 实体_id_稳定且同名不同类型会分开() {
        assert_eq!(entity_id("concept", "组织"), entity_id("concept", " 组织 "));
        assert_ne!(entity_id("concept", "组织"), entity_id("topic", "组织"));
    }

    #[test]
    fn 关系_id_与方向无关() {
        assert_eq!(relation_id("a", "b", "related"), relation_id("b", "a", "related"));
        assert_ne!(relation_id("a", "b", "related"), relation_id("a", "b", "causes"));
    }

    #[test]
    fn 候选词优先使用名词库() {
        let glossary = vec![("组织".to_string(), "定义".to_string(), vec!["管理".to_string()])];
        let notes = vec![("n1", "组织管理与组织效率"), ("n2", "组织协作")];
        let candidates = extract_candidates(&notes, &glossary, 10);
        assert_eq!(candidates.glossary_terms.get("组织").map(String::as_str), Some("组织"));
        assert_eq!(candidates.glossary_terms.get("管理").map(String::as_str), Some("组织"));
        // 「组织」出现三次，应当进入高频候选
        assert!(candidates.frequent_phrases.contains(&"组织".to_string()));
    }

    #[test]
    fn 候选词排除纯英文停用词() {
        let notes = vec![("n1", "the of and to in")];
        let candidates = extract_candidates(&notes, &[], 10);
        assert!(candidates.frequent_phrases.is_empty());
    }

    #[test]
    fn 笔记按上限分批() {
        let notes: Vec<u8> = (0..45).collect();
        let batches = batch_notes(&notes);
        assert_eq!(batches.len(), 3);
        assert_eq!(batches[0].len(), MAX_NOTES_PER_BATCH);
        assert_eq!(batches[2].len(), 5);
        assert_eq!(batches.iter().map(Vec::len).sum::<usize>(), 45);
    }
}
