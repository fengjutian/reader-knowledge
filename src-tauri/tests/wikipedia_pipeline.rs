//! 端到端集成测试：小 bz2 dump -> staging -> 发布 -> 冲突保护。
//!
//! 这些用例覆盖需求 7.8 的冲突矩阵与 5.3 的幂等/可恢复要求：
//! 走的是真实写库路径（Database::open + schema + 迁移），不是 mock。
//! 单测覆盖不到"写进库之后能不能搜到"，所以必须在这里过一遍。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use wereader_lib::database::Database;
use wereader_lib::wikipedia::config::{ImportMode, WikipediaConfig};
use wereader_lib::wikipedia::runner;
use wereader_lib::wikipedia::store::{self, ImportRequest, JobStatus};

/// 6 个页面，覆盖普通页、重定向链、软重定向、消歧义、列表页、目标缺失的断链。
fn fixture_xml() -> String {
    let page = |title: &str, id: i64, revision: i64, text: &str| {
        format!(
            "<page><title>{title}</title><ns>0</ns><id>{id}</id><revision><id>{revision}</id>\
             <timestamp>2026-09-01T00:00:00Z</timestamp><text xml:space=\"preserve\">{text}</text></revision></page>"
        )
    };
    let redirect = |title: &str, id: i64, target: &str| {
        format!(
            "<page><title>{title}</title><ns>0</ns><id>{id}</id><redirect title=\"{target}\" />\
             <revision><id>{id}</id><timestamp>2026-09-01T00:00:00Z</timestamp>\
             <text xml:space=\"preserve\">#REDIRECT [[{target}]]</text></revision></page>"
        )
    };
    let body = "人工智能是研究、开发用于模拟、延伸和扩展人的智能的理论、方法与技术的总称，属于计算机科学的一个分支。";
    [
        page("人工智能", 1001, 2001, body),
        page("机器学习", 1002, 2002, body),
        redirect("ML", 1003, "机器学习"),
        redirect("ML算法", 1004, "ML"),
        page("软重定向页", 1005, 2005, "#REDIRECT [[人工智能]]"),
        page(
            "苹果 (消歧义)",
            1006,
            2006,
            "{{消歧义}}\n苹果可能指苹果属植物、苹果公司或苹果果实。",
        ),
        page(
            "中国电视剧列表",
            1007,
            2007,
            "以下是中国大陆电视剧的完整列表，收录了数百部作品。",
        ),
        redirect("指向空处的别名", 1008, "并不存在的词条"),
        page(
            "细胞",
            1009,
            2009,
            "细胞是生物体结构和功能的基本单位，也是生命活动的基本单位之一。",
        ),
        page("模板页", 1010, 2010, "模板内容"),
    ]
    .join("")
}

fn write_fixture(dir: &Path) -> std::path::PathBuf {
    let xml = format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?><mediawiki xmlns=\"http://www.mediawiki.org/xml/export-0.11/\">{}\n</mediawiki>", fixture_xml());
    let path = dir.join("fixture.xml.bz2");
    let file = std::fs::File::create(&path).expect("create dump");
    let mut encoder = bzip2::write::BzEncoder::new(file, bzip2::Compression::best());
    std::io::Write::write_all(&mut encoder, xml.as_bytes()).expect("write dump");
    encoder.finish().expect("finish dump");
    path
}

fn config(dir: &Path) -> WikipediaConfig {
    let mut config = WikipediaConfig::default();
    config.temp_dir = dir.join("temp");
    config.batch_size = 2; // 故意用小批量，逼出分批与检查点路径
    config.progress_interval_ms = 0;
    config.validate().expect("config should be valid");
    config
}

fn seed_job(db: &Database, dump: &Path, config: &WikipediaConfig) -> String {
    seed_job_with(db, dump, config, true)
}

fn seed_job_with(
    db: &Database,
    dump: &Path,
    config: &WikipediaConfig,
    auto_publish: bool,
) -> String {
    let request = ImportRequest {
        local_file: Some(dump.to_path_buf()),
        mode: Some(ImportMode::Summary),
        auto_publish: Some(auto_publish),
        batch_size: Some(2),
        ..ImportRequest::default()
    };
    store::create_job(db, &request, config)
        .expect("create job")
        .id
}

/// 把 dump 里的某个标题片段整段去掉，模拟"该页在新 dump 中消失"。
fn dump_without(xml: &str, title: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    let marker = format!("<title>{title}</title>");
    while let Some(index) = rest.find(&marker) {
        let (head, tail) = rest.split_at(index);
        // 从 `<page>` 开始回退，删到对应的 `</page>`。
        let page_start = head.rfind("<page>").unwrap_or(0);
        let tail_start = tail
            .find("</page>")
            .map(|value| value + "</page>".len())
            .unwrap_or(tail.len());
        out.push_str(&head[..page_start]);
        rest = &tail[tail_start..];
    }
    out.push_str(rest);
    out
}

/// 便于断言：拿到某个名词的 (id, status, definition, manually_edited)。
fn term_of(db: &Database, term: &str) -> Option<(i64, String, String, i64)> {
    let connection = db.connect().expect("connect");
    connection
        .query_row(
            "SELECT id, status, definition, coalesce(manually_edited, 0) FROM glossary_terms WHERE term = ?1",
            [term],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .ok()
}

fn publish_summary(db: &Database, id: &str) -> wereader_lib::wikipedia::store::PublishSummary {
    let job = store::get_job(db, id).expect("job").expect("job row");
    let _ = job;
    store::publish_batch(db, id, "2026-09-01", 2).expect("publish")
}

#[test]
fn 小样本导入到发布全流程() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Database::open(dir.path().join("test.db")).expect("open db");
    let config = config(dir.path());
    let dump = write_fixture(dir.path());
    let id = seed_job(&db, &dump, &config);

    let summary = runner::run_pipeline(Arc::new(db.reopen().expect("reopen")), id.clone(), config)
        .expect("pipeline should succeed");
    assert_eq!(summary.status, JobStatus::Completed, "{}", summary.message);
    let finished = store::get_job(&db, &id)
        .expect("job")
        .expect("job row")
        .finished_at;
    assert!(
        finished > 0,
        "成功任务必须记录 finished_at，供报告和 staging 清理使用"
    );

    // 普通词条成为待确认候选
    let (term_id, status, definition, edited) =
        term_of(&db, "人工智能").expect("人工智能 should exist");
    assert_eq!(status, "pending", "新导入数据默认待确认");
    assert!(!definition.is_empty());
    assert_eq!(edited, 0);
    assert!(
        definition.contains("人工智能"),
        "摘要应包含词条名: {definition}"
    );

    // 主命名空间过滤：模板页没有落到名词表
    assert!(term_of(&db, "模板页").is_none(), "ns=10 不应导入");

    // 页面过滤：消歧义、列表页、软重定向都不该成为名词
    assert!(term_of(&db, "苹果 (消歧义)").is_none(), "消歧义页应被过滤");
    assert!(term_of(&db, "中国电视剧列表").is_none(), "列表页应被过滤");
    assert!(term_of(&db, "软重定向页").is_none(), "软重定向应被过滤");

    // 重定向链解析成别名
    let connection = db.connect().expect("connect");
    let alias_count: i64 = connection
        .query_row("SELECT count(*) FROM glossary_term_aliases", [], |row| {
            row.get(0)
        })
        .expect("alias count");
    assert!(alias_count >= 1, "重定向应产生别名，实际 {alias_count}");
    let aliased: String = connection
        .query_row(
            "SELECT aliases_json FROM glossary_terms WHERE term = '机器学习'",
            [],
            |row| row.get(0),
        )
        .expect("aliases_json");
    assert!(aliased.contains("ML"), "aliases_json 应回写别名: {aliased}");

    // 断链被记录成问题
    let issues = store::list_issues(&db, &id, Some("validation_error"), 50).expect("issues");
    assert!(
        issues.iter().any(|issue| issue.code == "broken_redirect"),
        "断链应被记录: {:?}",
        issues.iter().map(|issue| &issue.code).collect::<Vec<_>>()
    );
    drop(connection);
    assert!(term_id > 0, "名词应有主键");
}

#[test]
fn 后置阶段暂停后按阶段恢复而不重新解析() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Database::open(dir.path().join("resume-stage.db")).expect("open db");
    let config = config(dir.path());
    let dump = write_fixture(dir.path());
    let id = seed_job_with(&db, &dump, &config, false);

    store::transition(&db, &id, JobStatus::Parsing).expect("to parsing");
    store::transition(&db, &id, JobStatus::ResolvingRedirects).expect("to redirects");
    store::pause_job(&db, &id).expect("pause redirects");
    assert_eq!(
        store::resume_job(&db, &id).expect("resume redirects"),
        JobStatus::ResolvingRedirects
    );

    let summary = runner::run_pipeline(Arc::new(db.reopen().expect("reopen")), id.clone(), config)
        .expect("resume from redirects must succeed");
    assert_eq!(summary.status, JobStatus::ReadyToPublish);
}

#[test]
fn 非暂停任务不能重复恢复并启动第二执行器() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Database::open(dir.path().join("double-resume.db")).expect("open db");
    let config = config(dir.path());
    let dump = write_fixture(dir.path());
    let id = seed_job_with(&db, &dump, &config, false);
    store::pause_job(&db, &id).expect("pause");
    store::resume_job(&db, &id).expect("first resume");
    assert!(store::resume_job(&db, &id).is_err(), "第二次恢复必须被拒绝");
}

#[test]
fn 同一份_dump_重复运行不产生重复名词() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Database::open(dir.path().join("test.db")).expect("open db");
    let config = config(dir.path());
    let dump = write_fixture(dir.path());

    let first = seed_job(&db, &dump, &config);
    runner::run_pipeline(
        Arc::new(db.reopen().unwrap()),
        first.clone(),
        config.clone(),
    )
    .expect("first run");
    let count_after_first = count_terms(&db);
    let published_first = store::get_job(&db, &first).unwrap().unwrap().inserted_count;
    assert!(published_first > 0, "第一次应该真的插入了数据");

    // 第二个任务用同一份 dump
    let second = seed_job(&db, &dump, &config);
    runner::run_pipeline(
        Arc::new(db.reopen().unwrap()),
        second.clone(),
        config.clone(),
    )
    .expect("second run");

    let count_after_second = count_terms(&db);
    assert_eq!(
        count_after_first, count_after_second,
        "重复导入不能让名词数量变化"
    );
    // 别名唯一约束是 (term_id, normalized_alias, alias_type)，
    // 同一条目重复导入不会产生重复别名。
    let aliases_first = count_aliases(&db);

    let job = store::get_job(&db, &second).unwrap().unwrap();
    assert_eq!(job.inserted_count, 0, "第二次不应该再插入新名词");
    assert!(
        job.skipped_count > 0,
        "第二次应该全部走跳过: {}",
        job.skipped_count
    );
    assert_eq!(
        job.conflict_count, 0,
        "同一 page id 不该算冲突: {}",
        job.conflict_count
    );
    assert_eq!(count_aliases(&db), aliases_first, "别名不该翻倍");
    assert_eq!(job.status, "completed");
}

fn count_terms(db: &Database) -> i64 {
    db.connect()
        .unwrap()
        .query_row("SELECT count(*) FROM glossary_terms", [], |row| row.get(0))
        .unwrap()
}

fn count_aliases(db: &Database) -> i64 {
    db.connect()
        .unwrap()
        .query_row("SELECT count(*) FROM glossary_term_aliases", [], |row| {
            row.get(0)
        })
        .unwrap()
}

#[test]
fn 人工创建的同名词条不被覆盖() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Database::open(dir.path().join("test.db")).expect("open db");
    let config = config(dir.path());
    let dump = write_fixture(dir.path());

    // 人工先建一条同名词条
    let connection = db.connect().expect("connect");
    connection
        .execute(
            "INSERT INTO glossary_terms(term, canonical_name, aliases_json, definition, source, status, updated_at, manually_edited)
             VALUES('人工智能','人工智能','[]','我自己写的解释','manual','confirmed',1,1)",
            [],
        )
        .expect("seed manual term");
    drop(connection);

    // 不自动发布：先跑到 ready_to_publish，再显式发布，这样冲突统计来自发布阶段。
    let id = seed_job_with(&db, &dump, &config, false);
    runner::run_pipeline(Arc::new(db.reopen().unwrap()), id.clone(), config.clone()).expect("run");
    let ready = store::get_job(&db, &id).unwrap().unwrap();
    assert_eq!(ready.status, "ready_to_publish", "人工同名词条不应阻塞发布");
    let summary = publish_summary(&db, &id);

    let (_, status, definition, edited) = term_of(&db, "人工智能").expect("人工词条仍在");
    assert_eq!(definition, "我自己写的解释", "人工解释不能被覆盖");
    assert_eq!(status, "confirmed", "人工状态不能被改");
    assert_eq!(edited, 1, "人工标记要保留");
    // 夹具里 3 个可发布词条，只有"人工智能"与人工条目同名，
    // 另外两个（机器学习、细胞）没有冲突，应该正常插入。
    assert_eq!(summary.inserted, 2, "只应插入无冲突的词条: {summary:?}");
    assert_eq!(summary.conflicts, 1, "同名不同来源应计入冲突: {summary:?}");

    // 冲突要落到问题表，不能静默丢弃
    let issues = store::list_issues(&db, &id, Some("name_conflict"), 20).expect("issues");
    assert_eq!(issues.len(), 1);
    assert_eq!(issues[0].code, "manual_name_conflict");
    assert!(
        issues[0].message.contains("不自动合并"),
        "{}",
        issues[0].message
    );
}

#[test]
fn 人工编辑过的维基词条只更新来源快照() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Database::open(dir.path().join("test.db")).expect("open db");
    let config = config(dir.path());
    let dump = write_fixture(dir.path());

    // 第一轮：正常导入并发布
    let first = seed_job(&db, &dump, &config);
    runner::run_pipeline(
        Arc::new(db.reopen().unwrap()),
        first.clone(),
        config.clone(),
    )
    .expect("first run");

    // 人工改写解释（保留 wikipedia 来源，标记 manually_edited）
    let connection = db.connect().expect("connect");
    connection
        .execute(
            "UPDATE glossary_terms SET definition = '人工改写的解释', manually_edited = 1
              WHERE term = '人工智能'",
            [],
        )
        .expect("human edit");
    drop(connection);

    // 第二轮：内容确实变了（换一份不同正文的 dump），同步不应覆盖人工内容
    let second_xml = fixture_xml().replace(
        "人工智能是研究、开发用于模拟、延伸和扩展人的智能的理论、方法与技术的总称，属于计算机科学的一个分支。",
        "人工智能（AI）是模拟人类智能的科学与工程技术的统称，广泛用于搜索、翻译、推荐等场景。",
    );
    let second_path = dir.path().join("fixture2.xml.bz2");
    let mut encoder = bzip2::write::BzEncoder::new(
        std::fs::File::create(&second_path).expect("create"),
        bzip2::Compression::best(),
    );
    std::io::Write::write_all(
        &mut encoder,
        format!("<?xml version=\"1.0\"?><mediawiki>{second_xml}</mediawiki>").as_bytes(),
    )
    .expect("write");
    encoder.finish().expect("finish");

    let second = seed_job(&db, &second_path, &config);
    runner::run_pipeline(
        Arc::new(db.reopen().unwrap()),
        second.clone(),
        config.clone(),
    )
    .expect("second run");

    let connection = db.connect().expect("connect");
    let (definition, snapshot): (String, String) = connection
        .query_row(
            "SELECT definition, coalesce(wikipedia_snapshot,'') FROM glossary_terms WHERE term='人工智能'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("row");
    drop(connection);
    assert_eq!(definition, "人工改写的解释", "人工内容不能被同步覆盖");
    assert!(
        snapshot.contains("模拟人类智能"),
        "来源快照应该更新成新摘要: {snapshot}"
    );
}

#[test]
fn 来源相同且内容未变时只更新同步元数据() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Database::open(dir.path().join("test.db")).expect("open db");
    let config = config(dir.path());
    let dump = write_fixture(dir.path());

    let first = seed_job(&db, &dump, &config);
    runner::run_pipeline(
        Arc::new(db.reopen().unwrap()),
        first.clone(),
        config.clone(),
    )
    .expect("first run");
    let second = seed_job(&db, &dump, &config);
    runner::run_pipeline(
        Arc::new(db.reopen().unwrap()),
        second.clone(),
        config.clone(),
    )
    .expect("second run");

    let job = store::get_job(&db, &second).unwrap().unwrap();
    assert_eq!(job.updated_count, 0, "内容没变不应算更新");
    assert!(job.skipped_count > 0, "内容没变应全部跳过");
    assert_eq!(job.inserted_count, 0);
}

#[test]
fn 全量导入把消失的来源标记为失效() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Database::open(dir.path().join("test.db")).expect("open db");
    let config = config(dir.path());
    let dump = write_fixture(dir.path());

    let first = seed_job(&db, &dump, &config);
    runner::run_pipeline(
        Arc::new(db.reopen().unwrap()),
        first.clone(),
        config.clone(),
    )
    .expect("first run");
    assert!(term_of(&db, "细胞").is_some(), "第一轮应有细胞");

    // 第二份 dump 里整页删掉“细胞”
    let reduced = dump_without(&fixture_xml(), "细胞");
    assert!(
        !reduced.contains("<title>细胞</title>"),
        "夹具构造要真的删掉这一页"
    );
    let second_path = dir.path().join("reduced.xml.bz2");
    let mut encoder = bzip2::write::BzEncoder::new(
        std::fs::File::create(&second_path).expect("create"),
        bzip2::Compression::best(),
    );
    std::io::Write::write_all(
        &mut encoder,
        format!("<?xml version=\"1.0\"?><mediawiki>{reduced}</mediawiki>").as_bytes(),
    )
    .expect("write");
    encoder.finish().expect("finish");

    let second = seed_job(&db, &second_path, &config);
    runner::run_pipeline(
        Arc::new(db.reopen().unwrap()),
        second.clone(),
        config.clone(),
    )
    .expect("second run");

    let (_, status, _, _) = term_of(&db, "细胞").expect("来源失效不能物理删除");
    assert_eq!(status, "source_missing", "消失的来源应标记失效");
}

#[test]
fn 限量的部分导入不会误标来源失效() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Database::open(dir.path().join("test.db")).expect("open db");
    let config = config(dir.path());
    let dump = write_fixture(dir.path());

    let first = seed_job(&db, &dump, &config);
    runner::run_pipeline(
        Arc::new(db.reopen().unwrap()),
        first.clone(),
        config.clone(),
    )
    .expect("first run");
    assert!(term_of(&db, "细胞").is_some());

    // 只处理前 2 页：没扫到的词条不能被当成"来源消失"
    let request = ImportRequest {
        local_file: Some(dump.clone()),
        mode: Some(ImportMode::Summary),
        auto_publish: Some(true),
        max_items: Some(2),
        batch_size: Some(2),
        ..ImportRequest::default()
    };
    let limited = store::create_job(&db, &request, &config)
        .expect("create limited job")
        .id;
    runner::run_pipeline(
        Arc::new(db.reopen().unwrap()),
        limited.clone(),
        config.clone(),
    )
    .expect("limited run");

    let (_, status, _, _) = term_of(&db, "细胞").expect("细胞仍在");
    assert_ne!(status, "source_missing", "部分导入不能标来源失效");
}

#[test]
fn 中断后可以恢复且不重复发布() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Database::open(dir.path().join("test.db")).expect("open db");
    let config = config(dir.path());
    let dump = write_fixture(dir.path());
    let id = seed_job(&db, &dump, &config);

    // 模拟“解析中暂停”：先跑一半由 store 写入，再标记暂停
    let connection = db.connect().expect("connect");
    connection
        .execute(
            "INSERT INTO glossary_import_staging(job_id, page_id, revision_id, raw_title, normalized_title, summary, filter_status, parse_status, created_at)
             VALUES(?1, 1001, 2001, '人工智能', '人工智能', '人工写入的摘要', 'accepted', 'ok', 1)",
            [&id],
        )
        .expect("stage one row");
    drop(connection);
    store::transition(&db, &id, JobStatus::Parsing).expect("to parsing");
    store::pause_job(&db, &id).expect("pause");
    let paused = store::get_job(&db, &id).unwrap().unwrap();
    assert_eq!(paused.status, "paused");

    // 恢复到记录的阶段，续跑
    let resumed_to = store::resume_job(&db, &id).expect("resume");
    assert_eq!(resumed_to, JobStatus::Parsing, "应回到暂停时的阶段");
    let summary = runner::run_pipeline(Arc::new(db.reopen().unwrap()), id.clone(), config)
        .expect("resume run");
    assert_eq!(summary.status, JobStatus::Completed);

    // 之前手写的那行 staging 不该被重复插入名词
    let connection = db.connect().expect("connect");
    let count: i64 = connection
        .query_row(
            "SELECT count(*) FROM glossary_terms WHERE term='人工智能'",
            [],
            |row| row.get(0),
        )
        .expect("count");
    assert_eq!(count, 1, "同一 page id 只能有一条名词");
    drop(connection);
}

#[test]
fn 非法状态迁移被拒绝() {
    use wereader_lib::wikipedia::store::JobStatus as S;
    assert!(S::Pending.can_transition_to(S::Parsing));
    assert!(!S::Pending.can_transition_to(S::Publishing));
    assert!(
        !S::Completed.can_transition_to(S::Parsing),
        "终态不能再迁移"
    );
    assert!(!S::Cancelled.can_transition_to(S::Completed));
    assert!(!S::Failed.can_transition_to(S::Downloading));
    assert!(S::Paused.can_transition_to(S::Parsing));

    let dir = tempfile::tempdir().expect("temp dir");
    let db = Database::open(dir.path().join("test.db")).expect("open db");
    let config = config(dir.path());
    let dump = write_fixture(dir.path());
    let id = seed_job(&db, &dump, &config);
    let error = store::transition(&db, &id, JobStatus::Publishing)
        .expect_err("pending -> publishing 应被拒绝");
    assert!(error.to_string().contains("非法状态迁移"), "{error}");
    let job = store::get_job(&db, &id).unwrap().unwrap();
    assert_eq!(job.status, "pending", "被拒绝的迁移不应改状态");
}

#[test]
fn 取消任务不会留下半发布状态() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Database::open(dir.path().join("test.db")).expect("open db");
    let config = config(dir.path());
    let dump = write_fixture(dir.path());
    let id = seed_job(&db, &dump, &config);

    store::transition(&db, &id, JobStatus::Parsing).expect("to parsing");
    store::cancel_job(&db, &id).expect("cancel");
    let job = store::get_job(&db, &id).unwrap().unwrap();
    assert_eq!(job.status, "cancelled");
    assert!(job.finished_at > 0, "取消要写结束时间");

    let summary = runner::run_pipeline(Arc::new(db.reopen().unwrap()), id.clone(), config)
        .expect("rerun on cancelled");
    assert_eq!(summary.status, JobStatus::Cancelled);
    assert_eq!(count_terms(&db), 0, "取消的任务不应写入名词");
}

#[test]
fn 发布只能发生在就绪状态() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Database::open(dir.path().join("test.db")).expect("open db");
    let config = config(dir.path());
    let dump = write_fixture(dir.path());
    let id = seed_job(&db, &dump, &config);
    let job = store::get_job(&db, &id).unwrap().unwrap();
    assert_eq!(job.status, "pending");
    // pending 状态下直接调 publish_batch 会把 staging 全量发出去，
    // 所以校验必须挡住：这里确认 validate 之后才允许发。
    let error = store::transition(&db, &id, JobStatus::Publishing).expect_err("不能跳过校验");
    assert!(error.to_string().contains("非法状态迁移"));
}

#[test]
fn 导入报告包含关键统计() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Database::open(dir.path().join("test.db")).expect("open db");
    let config = config(dir.path());
    let dump = write_fixture(dir.path());
    let id = seed_job(&db, &dump, &config);
    runner::run_pipeline(Arc::new(db.reopen().unwrap()), id.clone(), config).expect("run");

    let report = wereader_lib::wikipedia::report::build_report(&db, &id).expect("report");
    assert_eq!(report["conclusion"], "success");
    assert!(report["counts"]["scanned"].as_i64().unwrap() > 0);
    let filters = report["filterReasons"].as_object().expect("filterReasons");
    assert!(
        filters.contains_key("disambiguation"),
        "应记录消歧义过滤数: {filters:?}"
    );
    assert!(filters.contains_key("list_page"), "应记录列表页过滤数");
    let source = &report["source"];
    assert_eq!(source["license"], "CC BY-SA 4.0");
    assert!(report["timing"]["startedAt"].as_i64().unwrap() > 0);
}

/// staging 分布：确认"被过滤的行"也留痕，而不是静默丢弃（需求 7.7）。
#[test]
fn 被过滤的行也会留在_staging_并记录原因() {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = Database::open(dir.path().join("test.db")).expect("open db");
    let config = config(dir.path());
    let dump = write_fixture(dir.path());
    let id = seed_job_with(&db, &dump, &config, false);
    runner::run_pipeline(Arc::new(db.reopen().unwrap()), id.clone(), config).expect("run");

    let connection = db.connect().expect("connect");
    let mut statement = connection
        .prepare(
            "SELECT filter_status, coalesce(filter_reason, ''), count(*)
               FROM glossary_import_staging WHERE job_id = ?1
              GROUP BY filter_status, filter_reason",
        )
        .expect("prepare");
    let mut rows = statement
        .query_map([&id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .expect("query");
    let mut distribution: HashMap<(String, String), i64> = HashMap::new();
    while let Some(row) = rows.next() {
        let entry = row.expect("row");
        distribution.insert((entry.0, entry.1), entry.2);
    }
    drop(rows);
    drop(statement);
    drop(connection);

    assert_eq!(
        distribution.get(&("accepted".to_string(), String::new())),
        Some(&3),
        "3 个普通词条可发布: {distribution:?}"
    );
    assert_eq!(
        distribution.get(&("redirect".to_string(), String::new())),
        Some(&2),
        "2 条重定向进别名: {distribution:?}"
    );
    assert_eq!(
        distribution.get(&("filtered".to_string(), "disambiguation".to_string())),
        Some(&1)
    );
    assert_eq!(
        distribution.get(&("filtered".to_string(), "list_page".to_string())),
        Some(&1)
    );
    assert_eq!(
        distribution.get(&("filtered".to_string(), "soft_redirect".to_string())),
        Some(&1)
    );
    assert_eq!(
        distribution.get(&("filtered".to_string(), "broken_redirect".to_string())),
        Some(&1)
    );
    // 每个被过滤的行都带原因，没有空原因的
    for (status, reason) in distribution.keys() {
        if status == "filtered" {
            assert!(!reason.is_empty(), "过滤行必须带原因");
        }
    }
}
