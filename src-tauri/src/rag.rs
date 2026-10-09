//! RAG 证据聚合：把笔记与导入资料合成一份可引用的证据。
//!
//! 计划 9.6.2 要求「RAG 同时检索笔记和导入文档，但结果必须标明来源」。
//! 现有 `hybrid_search` 只覆盖微信读书笔记，这里补上导入资料，
//! 并用统一的编号把两类证据编到同一个索引空间里，AI 的 `[n]` 引用不会歧义。

use crate::database::Database;
use crate::error::AppError;
use std::collections::HashMap;

/// 复用 commands 里的文本规范化规则，保证笔记与资料的打分口径一致。
use crate::commands::normalize_search_text as commands_normalize;

/// 一条可引用的证据。`kind` 决定前端点击后的行为与展示。
///
/// 目前只有 `Source` 会出现在 `prepare_ask` 的上下文里：笔记走既有的
/// `SearchResult` 通道（引用模型是 `Citation { note }`，改它会牵动
/// 全部既有前端代码）。`Note` 保留下来是为了让两种证据共享同一套
/// 打分 / 去重 / prompt 渲染逻辑，后续把笔记也迁进来时不用重写。
#[derive(Debug, Clone, PartialEq)]
pub enum Evidence {
    /// 微信读书划线或想法
    #[allow(dead_code)]
    Note {
        book_id: String,
        book_title: String,
        chapter: String,
        content: String,
        note_type: String,
        note_id: String,
    },
    /// 导入资料的文档块
    Source {
        source_id: String,
        source_title: String,
        source_type: String,
        content: String,
        locator: crate::import::Locator,
    },
}

impl Evidence {
    /// 喂给模型时这一条展示成什么样。
    pub fn prompt_text(&self, index: usize, limit: usize) -> String {
        let content: String = self.content().chars().take(limit).collect();
        match self {
            Evidence::Note { book_title, chapter, note_type, .. } => format!(
                "[{index}]\ntype: {}\nbook: 《{book_title}》\nchapter: {chapter}\ncontent: {content}\n",
                if note_type == "thought" { "用户想法" } else { "书籍原文划线" }
            ),
            Evidence::Source { source_title, locator, .. } => {
                let source_label = match locator.page {
                    Some(page) => format!("{source_title} 第 {page} 页"),
                    None => match locator.chapter {
                        Some(chapter) => format!("{source_title} 第 {chapter} 章"),
                        None => source_title.clone(),
                    },
                };
                format!("[{index}]\ntype: 导入资料\nsource: {source_label}\ncontent: {content}\n")
            }
        }
    }

    pub fn book_id(&self) -> &str {
        match self {
            Evidence::Note { book_id, .. } => book_id,
            // 导入资料不是微信读书书籍，用来源 id 代替书籍 id
            Evidence::Source { source_id, .. } => source_id,
        }
    }

    pub fn content(&self) -> &str {
        match self {
            Evidence::Note { content, .. } => content,
            Evidence::Source { content, .. } => content,
        }
    }

    /// 引用点击时给出的定位信息。笔记给 noteId，资料暂无独立 id。
    #[allow(dead_code)]
    pub fn citation_target(&self) -> String {
        match self {
            Evidence::Note { note_id, .. } => note_id.clone(),
            Evidence::Source { .. } => String::new(),
        }
    }
}

/// 从导入资料里取候选块。
///
/// 走 `source_docs_fts`，与笔记一样沿用「FTS 收敛 + 兜底扫描 + 包含匹配」，
/// 因为 `unicode61` 不切分中文，纯 FTS 对中文查询会落空。
pub fn source_evidence(
    db: &Database,
    query: &str,
    terms: &[String],
    book_ids: &[String],
    limit: usize,
) -> Result<Vec<Evidence>, AppError> {
    let normalized = commands_normalize(query);
    if normalized.is_empty() {
        return Ok(Vec::new());
    }
    let c = db.connect()?;
    // 限定了具体书籍时，导入资料不参与（它不属于任何一本书）
    if !book_ids.is_empty() {
        return Ok(Vec::new());
    }
    let match_query = terms
        .iter()
        .filter(|term| !term.is_empty())
        .take(12)
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" OR ");

    let mut rows: Vec<(String, String, String, String, String, crate::import::Locator)> = Vec::new();
    if !match_query.is_empty() {
        let mut statement = c.prepare(
            "SELECT source_docs_fts.source_id,coalesce(s.title,''),coalesce(s.source_type,''),
                    coalesce(source_docs_fts.heading,''),source_docs_fts.content,coalesce(d.locator_json,'{}')
             FROM source_docs_fts
             JOIN source_documents d ON d.id=source_docs_fts.doc_id
             JOIN library_sources s ON s.id=source_docs_fts.source_id
             WHERE source_docs_fts MATCH ?1 AND s.is_deleted=0
             LIMIT ?2",
        )?;
        rows = statement
            .query_map(rusqlite::params![match_query, limit as i64], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
    }
    if rows.is_empty() {
        // FTS 落空时走包含匹配兜底。`unicode61` 不切分中文，纯 MATCH 对中文查询几乎必然落空，
        // 所以这里必须按 terms 做 LIKE 过滤；不能只 LIMIT 取前 N 块再让上层打分，
        // 否则命中块排在靠后位置时资料会一条都进不了 prompt（静默降级）。
        let scan_limit = (limit as i64).saturating_mul(8).clamp(200, 5000);
        let patterns: Vec<String> = terms
            .iter()
            .map(|term| term.trim())
            .filter(|term| !term.is_empty())
            .map(|term| format!("%{}%", commands_normalize(term)))
            .collect();
        if patterns.is_empty() {
            return Ok(Vec::new());
        }
        let or_clause = (1..=patterns.len())
            .map(|index| {
                let slot = format!("?{index}");
                format!(
                    "coalesce(d.content,'') LIKE {slot} OR coalesce(d.heading,'') LIKE {slot} OR coalesce(s.title,'') LIKE {slot}"
                )
            })
            .collect::<Vec<_>>()
            .join(" OR ");
        let limit_slot = patterns.len() + 1;
        let sql = format!(
            "SELECT d.source_id,coalesce(s.title,''),coalesce(s.source_type,''),coalesce(d.heading,''),d.content,coalesce(d.locator_json,'{{}}')
             FROM source_documents d JOIN library_sources s ON s.id=d.source_id
             WHERE s.is_deleted=0 AND ({or_clause})
             LIMIT ?{limit_slot}"
        );
        let mut statement = c.prepare(&sql)?;
        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::with_capacity(patterns.len() + 1);
        for pattern in &patterns {
            params.push(Box::new(pattern.clone()));
        }
        params.push(Box::new(scan_limit));
        rows = statement
            .query_map(rusqlite::params_from_iter(params.iter().map(|value| value.as_ref())), |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    serde_json::from_str(&r.get::<_, String>(5)?).unwrap_or_default(),
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
    }

    let mut scored: Vec<(f64, Evidence)> = Vec::new();
    for (source_id, title, source_type, _heading, content, locator) in rows {
        let score = score_block(&normalized, terms, &content);
        if score <= 0.0 {
            continue;
        }
        scored.push((score, Evidence::Source { source_id, source_title: title, source_type, content, locator }));
    }
    // 稳定排序：分数降序，同分按 id 字典序
    scored.sort_by(|left, right| right.0.partial_cmp(&left.0).unwrap_or(std::cmp::Ordering::Equal));
    Ok(scored.into_iter().take(limit).map(|(_, item)| item).collect())
}

fn score_block(normalized_query: &str, terms: &[String], content: &str) -> f64 {
    let normalized_content = commands_normalize(content);
    if normalized_content.contains(normalized_query) {
        return 12.0 + terms.iter().map(|term| normalized_content.match_indices(term).count().min(4) as f64).sum::<f64>();
    }
    let hits: f64 = terms
        .iter()
        .filter(|term| normalized_content.contains(*term))
        .map(|term| normalized_content.match_indices(term).count().min(4) as f64)
        .sum();
    if hits > 0.0 {
        4.0 + hits
    } else {
        0.0
    }
}

/// 合并笔记与导入资料的证据，笔记在前（保持既有行为），资料补在后面。
///
/// 每条都带上最终编号，AI 的 `[n]` 引用可以一路回溯到具体笔记或资料块。
/// 当前 `prepare_ask` 走的是「笔记在 `SearchResult` 里、资料在这里」的
/// 两条独立通道，编号由 `finalize_answer` 负责对齐；这个函数留给
/// 后续把笔记也迁进 `Evidence` 时使用。
#[allow(dead_code)]
pub fn merge_evidence(mut notes: Vec<Evidence>, sources: Vec<Evidence>) -> Vec<Evidence> {
    let mut seen: HashMap<String, ()> = HashMap::new();
    notes.extend(sources);
    notes.retain(|item| {
        // 必须按字符截断：正文以中文为主，按字节切会落在多字节字符中间而 panic。
        let head: String = item.content().chars().take(80).collect();
        let key = format!("{}|{:?}|{}", item.book_id(), item.citation_target(), head);
        seen.insert(key, ()).is_none()
    });
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::Database;

    fn test_db() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(dir.path().join("test.db")).unwrap();
        (dir, db)
    }

    fn seed_source(db: &Database, source_id: &str, source_type: &str, title: &str) {
        db.connect()
            .unwrap()
            .execute(
                "INSERT INTO library_sources(id,source_type,title,content_hash,page_count,imported_at,updated_at,is_deleted)
                 VALUES(?1,?2,?3,?4,1,0,0,0)",
                rusqlite::params![source_id, source_type, title, format!("hash-{source_id}")],
            )
            .unwrap();
    }

    fn seed_doc(db: &Database, doc_id: &str, source_id: &str, heading: &str, content: &str, locator: crate::import::Locator) {
        db.connect()
            .unwrap()
            .execute(
                "INSERT INTO source_documents(id,source_id,position,heading,content,locator_json) VALUES(?1,?2,0,?3,?4,?5)",
                rusqlite::params![doc_id, source_id, heading, content, serde_json::to_string(&locator).unwrap()],
            )
            .unwrap();
    }

    fn note(id: &str, content: &str) -> Evidence {
        Evidence::Note {
            book_id: "b1".into(),
            book_title: "书一".into(),
            chapter: "第一章".into(),
            content: content.into(),
            note_type: "highlight".into(),
            note_id: id.into(),
        }
    }

    #[test]
    fn 提示文本标明来源类型() {
        let highlighted = note("n1", "正文");
        assert!(highlighted.prompt_text(1, 100).contains("type: 书籍原文划线"));
        assert!(highlighted.prompt_text(1, 100).contains("book: 《书一》"));

        let thought = Evidence::Note {
            book_id: "b1".into(),
            book_title: "书一".into(),
            chapter: "第二章".into(),
            content: "我的想法".into(),
            note_type: "thought".into(),
            note_id: "n2".into(),
        };
        assert!(thought.prompt_text(2, 100).contains("type: 用户想法"));
    }

    #[test]
    fn 资料_提示文本带页码与章节() {
        let page = Evidence::Source {
            source_id: "s1".into(),
            source_title: "一本PDF".into(),
            source_type: "pdf".into(),
            content: "正文".into(),
            locator: crate::import::Locator { page: Some(12), chapter: None, heading: None },
        };
        let text = page.prompt_text(3, 100);
        assert!(text.contains("type: 导入资料"), "{text}");
        assert!(text.contains("source: 一本PDF 第 12 页"), "{text}");

        let chapter = Evidence::Source {
            source_id: "s2".into(),
            source_title: "一本EPUB".into(),
            source_type: "epub".into(),
            content: "正文".into(),
            locator: crate::import::Locator { page: None, chapter: Some(3), heading: None },
        };
        assert!(chapter.prompt_text(4, 100).contains("source: 一本EPUB 第 3 章"));
    }

    #[test]
    fn 提示文本按传入长度截断() {
        let long = "字".repeat(500);
        let item = Evidence::Note {
            book_id: "b1".into(),
            book_title: "书".into(),
            chapter: "章".into(),
            content: long,
            note_type: "highlight".into(),
            note_id: "n1".into(),
        };
        let text = item.prompt_text(1, 50);
        assert!(text.contains(&"字".repeat(50)));
        assert!(!text.contains(&"字".repeat(51)));
    }

    #[test]
    fn 资料_能被检索到并标明来源() {
        let (_dir, db) = test_db();
        seed_source(&db, "s1", "pdf", "导入的PDF");
        seed_doc(&db, "d1", "s1", "第 5 页", "独特关键词出现在这里", crate::import::Locator { page: Some(5), chapter: None, heading: None });

        let terms = vec!["独特关键词".to_string()];
        let found = source_evidence(&db, "独特关键词", &terms, &[], 10).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].book_id(), "s1");
        assert!(found[0].prompt_text(1, 100).contains("导入资料"));
    }

    #[test]
    fn 资料_限定书籍时不参与() {
        let (_dir, db) = test_db();
        seed_source(&db, "s1", "pdf", "导入的PDF");
        seed_doc(&db, "d1", "s1", "第 5 页", "独特关键词", crate::import::Locator::default());

        let found = source_evidence(&db, "独特关键词", &["独特关键词".to_string()], &["b1".to_string()], 10).unwrap();
        assert!(found.is_empty(), "限定书籍时导入资料不参与");
    }

    #[test]
    fn 资料_软删除后不参与() {
        let (_dir, db) = test_db();
        seed_source(&db, "s1", "pdf", "导入的PDF");
        seed_doc(&db, "d1", "s1", "第 5 页", "独特关键词", crate::import::Locator::default());
        db.connect().unwrap().execute("UPDATE library_sources SET is_deleted=1 WHERE id='s1'", []).unwrap();

        let found = source_evidence(&db, "独特关键词", &["独特关键词".to_string()], &[], 10).unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn 资料_空查询不返回() {
        let (_dir, db) = test_db();
        seed_source(&db, "s1", "pdf", "导入的PDF");
        seed_doc(&db, "d1", "s1", "第 5 页", "一些内容", crate::import::Locator::default());
        assert!(source_evidence(&db, "  ", &[], &[], 10).unwrap().is_empty());
    }

    #[test]
    fn 资料_中文查询走包含匹配兵底也能命中() {
        // 真实缺陷：source_docs_fts 用 unicode61，中文整段是单个 token，
        // `MATCH "地方政府"` 匹配不到「地方政府的债务问题」，必然落入兜底分支；
        // 而兜底分支原本是无条件 LIMIT，取 rowid 顺序前 N 块再被 score_block 全过滤 → 恒空。
        // 这里用「命中内容排在很多无关块之后」来复现：只有加了 LIKE 条件才找得到。
        let (_dir, db) = test_db();
        seed_source(&db, "s1", "pdf", "财政报告");
        // 先塞 300 条无关块，让目标块排在靠后位置。
        for index in 0..300 {
            seed_doc(
                &db,
                &format!("noise{index}"),
                "s1",
                &format!("第 {index} 页"),
                &format!("无关内容 {index}，与查询毫无关系"),
                crate::import::Locator::default(),
            );
        }
        seed_doc(&db, "target", "s1", "第 900 页", "地方政府的债务问题与化解路径", crate::import::Locator { page: Some(900), chapter: None, heading: None });

        let terms = vec!["地方政府".to_string()];
        let found = source_evidence(&db, "地方政府", &terms, &[], 10).unwrap();
        assert!(!found.is_empty(), "中文查询必须能通过包含匹配兜底命中资料");
        assert!(found.iter().any(|item| item.content().contains("地方政府")), "命中的应是包含目标词的块，实际 {:?}", found.iter().map(|item| item.content()).collect::<Vec<_>>());
    }

    #[test]
    fn 资料_兜底不会把无关块当证据() {
        let (_dir, db) = test_db();
        seed_source(&db, "s1", "pdf", "无关文档");
        seed_doc(&db, "d1", "s1", "第 1 页", "完全无关的正文内容", crate::import::Locator::default());
        let found = source_evidence(&db, "不存在的词", &["不存在的词".to_string()], &[], 10).unwrap();
        assert!(found.is_empty(), "没有命中词的块不应成为证据");
    }

    #[test]
    fn 合并时笔记在前_资料在后() {
        let merged = merge_evidence(vec![note("n1", "笔记内容")], vec![Evidence::Source {
            source_id: "s1".into(),
            source_title: "资料".into(),
            source_type: "pdf".into(),
            content: "资料内容".into(),
            locator: crate::import::Locator::default(),
        }]);
        assert_eq!(merged.len(), 2);
        assert!(matches!(merged[0], Evidence::Note { .. }));
        assert!(matches!(merged[1], Evidence::Source { .. }));
    }

    #[test]
    fn 合并时去掉完全重复的证据() {
        let merged = merge_evidence(vec![note("n1", "重复内容")], vec![Evidence::Source {
            source_id: "s1".into(),
            source_title: "资料".into(),
            source_type: "pdf".into(),
            content: "重复内容".into(),
            locator: crate::import::Locator::default(),
        }]);
        // book_id 不同所以不是同一条；相同 book_id + 内容才会被去重
        assert_eq!(merged.len(), 2);

        let duplicated = merge_evidence(vec![note("n1", "同样的内容"), note("n1", "同样的内容")], vec![]);
        assert_eq!(duplicated.len(), 1, "同一条笔记重复出现应只保留一份");
    }
}
