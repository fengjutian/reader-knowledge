# 维基百科名词库导入（运维手册）

对应需求文档：`docs/wikipedia-glossary-import-requirements.md`（v1.0）。

一句话：把中文维基百科官方 dump 离线解析成名词候选，走「staging → 校验 → 发布」两阶段写入
`glossary_terms`，人工内容永远优先。

---

## 1. 技术栈与落点

| 层 | 实现 | 位置 |
|---|---|---|
| 桌面端 | Tauri v2 + Rust（rusqlite / SQLite bundled） | `src-tauri/` |
| 前端 | React 19 + TypeScript + vitest | `src/` |
| 导入管道 | 纯 Rust，无队列 | `src-tauri/src/wikipedia/` |
| 数据库迁移 | `schema.sql`（新表）+ `database/migrations.rs`（新增列） | — |
| 长任务执行 | 桌面端后台线程 / 独立 CLI 二进制 | `src/bin/glossary-wiki.rs` |

模块边界（底层不碰数据库，上层不碰 XML）：

```text
parser        bz2 → 流式 XML → WikiPage
wikitext      Wikitext → 纯文本摘要
title         标题规范化 / 搜索键 / 原文 URL
redirect      重定向链、环、断链
download      域名白名单、断点续传、MD5 校验
store         任务状态机、staging、校验、发布、冲突保护、审计
runner        把上面几层串成状态机
report        导入报告
config        环境变量装配与启动校验
```

## 2. 环境变量

全部可选，但生产环境**必须**显式设置 `WIKIPEDIA_USER_AGENT`（Wikimedia 政策要求带联系方式）
和 `WIKIPEDIA_IMPORT_TEMP_DIR`（指向有 ≥50 GB 空间的持久目录）。

| 变量 | 默认 | 说明 |
|---|---|---|
| `WIKIPEDIA_DUMP_BASE_URL` | `https://dumps.wikimedia.org/zhwiki/latest/` | 必须 HTTPS |
| `WIKIPEDIA_IMPORT_TEMP_DIR` | `<app_data>/wikipedia-import` | 启动时校验可创建、可写 |
| `WIKIPEDIA_USER_AGENT` | 项目名 + 仓库地址 | 太短会直接报错 |
| `WIKIPEDIA_IMPORT_ALLOWED_HOSTS` | `dumps.wikimedia.org` | 逗号分隔，逐个精确匹配（无子域通配） |
| `WIKIPEDIA_IMPORT_BATCH_SIZE` | `500` | staging 批量写入大小 |
| `WIKIPEDIA_IMPORT_MAX_CONCURRENCY` | `1` | 一期只做单进程解析 |
| `WIKIPEDIA_IMPORT_SUMMARY_MAX_CHARS` | `600` | 摘要硬上限，下限固定 300 |
| `WIKIPEDIA_IMPORT_MAX_RETRIES` | `3` | 下载重试 |
| `WIKIPEDIA_STAGING_RETENTION_DAYS` | `14` | 已结束任务的 staging 保留天数 |

## 3. 数据库变更

启动时自动执行，无需手工操作：

- `schema.sql` 新增 5 张表：`glossary_term_aliases`、`glossary_import_jobs`、
  `glossary_import_staging`、`glossary_import_issues`、`glossary_import_audit`。
- `database::migrations` 版本化补列（SQLite 的 DDL 没有 `ADD COLUMN IF NOT EXISTS`）：
  `external_page_id`（INTEGER）、`source_revision_id`、`source_dump_version`、
  `source_updated_at`、`source_synced_at`、`license_code`、`manually_edited`、
  `source_content_hash`、`published_batch_id`、`normalized_term`。
  迁移版本记在 `app_meta.glossary_import_migration`，重复启动不会重复 ALTER。
- 存量 `glossary_terms` 回填为 `source='manual'` / `status='confirmed'` / `manually_edited=0`，
  **不会被后续同步覆盖**。

复用既有字段，没有另造同义字段：

- `glossary_terms.source` = 需求里的 `source_type`（`manual|wikipedia|other`）
- `glossary_terms.status` = 需求里的 `review_status`
  （`pending|confirmed|ignored|conflict|source_missing`）
- `glossary_terms.wikipedia_snapshot` = 最近一次由来源写入的摘要，用于人工审核时对比

## 4. 命令行

```bash
# 探测官方 dump（只发 HEAD，不下正文）
npm run glossary:wiki -- inspect

# 小样本试跑（不自动发布，先看报告）
npm run glossary:wiki -- import --mode summary --local-file D:\dump.xml.bz2 --limit 1000

# 全量：先跑到 staging，人工确认后发布
npm run glossary:wiki -- import --mode summary
npm run glossary:wiki -- validate --job <id>
npm run glossary:wiki -- publish --job <id>

# 中断与清理
npm run glossary:wiki -- resume --job <id>
npm run glossary:wiki -- cleanup --job <id> --retention-days 14
npm run glossary:wiki -- list
```

所有子命令结尾输出一行机器可读 JSON，退出码 0 表示成功。
`--db <path>` 可指定数据库文件（默认取应用数据目录）。

界面等价入口：名词库页 → 「维基导入」。

## 5. 状态机

```text
pending → downloading → verifying → parsing → resolving_redirects
        → validating → ready_to_publish → publishing → completed
任意进行中状态 → paused（恢复时回到 pausedFrom 记录的阶段）
任意非终态    → cancelled / failed
终态（completed / cancelled / failed）不再接受任何迁移
```

- 状态迁移只能走 `store::transition`，非法迁移直接报错（集成测试覆盖）。
- 暂停/取消**不靠内存信号**：调用方改数据库状态，runner 在批次边界读状态决定停在哪，
  所以界面和 CLI 同时操作也不会打架。
- 取消是安全的：runner 在批次边界退出，staging 保持 `pending`，名词主表不出现半发布状态。

## 6. 首次导入建议流程

1. `inspect` 确认源可用。
2. `import --limit 1000`，`validate --job <id>` 看报告：过滤原因分布、冲突数、错误样例。
3. 人工抽查 ≥50 条摘要、≥20 条重定向。
4. 跑单个官方分片（推荐 `zhwiki-latest-pages-articles1.xml-p1p187712.bz2`，约 255 MB），记录吞吐与内存。
5. 全量 `import`（**不自动发布**），查看报告与异常分布。
6. **备份数据库**后再 `publish --job <id>`。
7. 发布后验证名词列表、搜索、详情页来源与许可证展示。

## 7. 容量

按 2026-10 的 dump 规模估算（程序不假设这些数字固定，下载前会探测实际大小）：

- 正文普通压缩包 `pages-articles.xml.bz2` ≈ 3.43 GB；当前程序顺序扫描，不需要更大的 multistream 包
- 建议可用空间 ≥ 50 GB（临时文件 15 GB / 业务数据 10 GB / 索引日志 10 GB / 余量 15 GB）
- 下载前会检查可用空间并保留 1 GiB 余量，不够直接失败而不是下到一半失败

## 8. 安全

| 风险 | 处理 |
|---|---|
| SSRF | 只允许白名单域名 + HTTPS；域名精确匹配，`dumps.wikimedia.org.evil.com` 这类会被拒 |
| 路径穿越 | 文件名从 URL 取片段后白名单校验；URL 含 `..` 直接拒绝 |
| XXE | 不展开任何 DTD/外部实体，只认 5 个 XML 预定义实体和数字引用 |
| 压缩炸弹 | 解压总量上限 64 GiB；压缩比上限 200 倍；单页正文上限 4 MiB |
| 覆盖人工内容 | 人工字段与来源字段分离；`manually_edited=1` 的词条只更新来源快照 |
| 敏感信息 | 错误日志截断到 500/800 字符，不写连接串与凭据 |

## 9. 可观测性

- 进度写库节流：默认 1 秒一次，不逐条写。
- 任务详情包含：阶段、字节数、扫描/有效/重定向/过滤数、增删改跳过冲突错误数、
  当前文件、速率、起止时间、最近错误。
- 任务完成时生成报告（存 `report_json`，CLI `report --job <id>` 可取）：
  输入文件与配置快照、各类计数、过滤/冲突/错误原因分布、20 条以内错误示例、
  最终结论（`success` / `partial_success` / `cancelled` / `paused` / `failed`）。

## 10. 已知限制与后续

- 一期只开放 `summary` 模式；`titles_only` / `full_text` 保留枚举与程序能力，界面不暴露。
- 搜索仍用 `LIKE` + 别名 EXISTS：项目现有 FTS 用 `unicode61` 分词，对中文整句只会切成一个
  token，摘要全文匹配走 `LIKE` 更准。名词规模到百万级后应补 trigram FTS 索引。
- 不做简繁转换：简繁只作为可选搜索辅助键，不作唯一键，避免误合并不同词条。
- 图片/音频不入库；将来导入必须逐项处理各自许可证。
- 每月检查新 dump 为人工流程，稳定后再考虑定时任务。

许可证：维基文本按 CC BY-SA 4.0 使用，详情页已展示署名与许可证链接。上线前请由项目负责人
按实际发布方式复核合规要求。
