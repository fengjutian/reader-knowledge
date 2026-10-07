# wereader 剩余功能实现计划

> 面向开发执行者：MiniMax。本文以当前 `0.2.0` 代码为基线，要求按阶段、小批次实现并逐项验收，不进行无关重构。

## 1. 总体目标与执行原则

本计划补齐以下能力：

1. AI 真流式输出、取消与错误恢复。
2. 完整全局搜索：书籍、划线、想法、书籍过滤与类型过滤。
3. 可配置的 Reranker，使 Hybrid Search 的排序可解释、可降级。
4. Concept / Topic / Idea / Relation 概念级知识图谱。
5. Open Library 与 Google Books 元数据源，以及多来源合并策略。
6. 网页、PDF、EPUB 内容导入。阅读器、移动端与完整 Web 版暂不纳入本轮。

执行时遵守以下原则：

- 保持 React → `src/api/tauri.ts` → Tauri command → service/provider → SQLite 的现有分层。
- API Key 只能存入 OS Credential Store，不得进入 SQLite、日志、错误信息或前端持久化。
- 数据库变更必须兼容已有用户数据；只允许增量迁移，不允许删除或重建现有表。
- 外部请求设置超时、响应体上限、有限重试和清晰错误信息；不得无限重试。
- 每个阶段单独提交，先完成后端和测试，再接 UI；不得一次性实现全部阶段。
- 不得破坏微信读书同步、现有 FTS、AI 引用、书籍图谱和豆瓣元数据功能。
- 用户内容默认保存在本机；AI 请求只发送当前任务所需的最小上下文。

## 2. 建议实施顺序

| 阶段 | 功能 | 优先级 | 前置依赖 |
|---|---|---:|---|
| P0 | 基线测试与数据库迁移框架确认 | 最高 | 无 |
| P1 | AI 流式输出 | 最高 | P0 |
| P2 | 完整全局搜索 | 最高 | P0 |
| P3 | Reranker | 高 | P2 |
| P4 | 概念级知识图谱 | 高 | P3 |
| P5 | Open Library / Google Books | 中 | P0 |
| P6 | 网页、PDF、EPUB 导入 | 中/长期 | P2 |

P1 与 P2 可以分别开发，但合并时仍应逐个阶段验收。P4 不要在 P3 排序质量稳定前开始。

## 3. P0：建立可靠基线

### 3.1 开始前检查

1. 运行 `npm test`、`npm run lint`、`npm run build`。
2. 运行 Rust 测试：进入 `src-tauri` 后执行 `cargo test`。
3. 记录已有失败，不得通过删除测试或降低断言规避。
4. 检查工作区已有改动，只修改本计划涉及文件。

### 3.2 数据库迁移要求

当前 schema 位于 `src-tauri/src/database/schema.sql`。新增表时：

- 使用 `CREATE TABLE IF NOT EXISTS` 和 `CREATE INDEX IF NOT EXISTS`。
- 新增列必须提供可兼容旧库的迁移逻辑。
- schema 初始化应具备幂等性，连续启动两次不能失败。
- 为新增表增加数据库概览统计或至少保证数据库浏览页面不会报错。

### 3.3 基线验收

- 四类命令全部通过。
- 空库、已有 0.2.0 数据库都能启动。
- 微信读书同步失败时不会删除原有数据。

## 4. P1：AI 真流式输出

### 4.1 目标行为

- 提交问题后，界面逐块显示模型生成内容，而不是等待完整响应。
- 同一时间只允许当前会话存在一个活动请求。
- 用户可以点击“停止生成”。停止后保留已生成内容，但标记为未完成。
- 流结束后再固定引用列表、名词引用和 `sourcesConsidered`。
- 网络中断、超时或非法 SSE 数据必须显示错误，不能把失败当成正常结束。
- DeepSeek、MiniMax 和自定义 OpenAI-compatible Endpoint 均走同一流式协议；若服务明确不支持流式，可显示说明并安全回退到非流式请求。

### 4.2 后端设计

在 `src-tauri/src/ai/provider.rs` 中扩展 Provider 抽象。建议新增：

```rust
pub enum AiStreamEvent {
    Started,
    Delta { content: String },
    Completed { content: String },
    Failed { message: String },
    Cancelled,
}
```

实现步骤：

1. 为每次请求生成 `request_id`，由前端传入或由 command 返回。
2. 新增 `ask_ai_stream` command；保留现有 `ask_ai` 作为兼容接口和测试入口。
3. 请求 OpenAI-compatible Chat Completions 时发送 `stream: true`。
4. 使用字节流增量解析 SSE，正确处理：
   - 一条事件跨多个网络 chunk；
   - 一个 chunk 含多条事件；
   - 空行、注释行与 `data: [DONE]`；
   - UTF-8 中文字符被拆分；
   - `choices[0].delta.content` 缺失。
5. Tauri 事件名使用固定前缀，例如 `ai-stream`，事件 payload 必须带 `requestId`，避免串流污染其他会话。
6. RAG、Prompt、引用候选仍在后端完成；模型只流式返回正文。
7. 流结束后发送 `completed` 事件，携带最终引用和统计信息。
8. 增加 `cancel_ai_stream(request_id)` command。后端保存取消令牌；组件卸载、新请求发起或用户点击停止时取消网络读取。
9. 对请求时长、累计响应大小和单事件大小设上限；延续现有错误脱敏规则。

建议事件结构：

```ts
type AiStreamEvent =
  | { requestId: string; type: "started" }
  | { requestId: string; type: "delta"; content: string }
  | { requestId: string; type: "completed"; answer: AiAnswer }
  | { requestId: string; type: "failed"; message: string }
  | { requestId: string; type: "cancelled" };
```

### 4.3 前端修改

涉及：

- `src/api/tauri.ts`
- `src/types/domain.ts`
- `src/pages/AI.tsx`
- `src/styles/global.scss` 或对应样式文件

实现步骤：

1. 在 API adapter 增加流式开始、监听、取消方法；监听器必须在结束、失败和组件卸载时解除。
2. `AI.tsx` 新增当前 `requestId`、草稿回答和流状态。
3. 收到 `delta` 时增量追加文本并保持滚动位置合理；用户手动向上滚动后不得强制拉到底部。
4. 将发送按钮在生成时切换为停止按钮。
5. `completed` 后将最终回答写入 `turns` 和历史记录；草稿不能重复入库。
6. 失败时保留已生成正文，提供“重试”按钮。重试沿用原问题和选书范围，但生成新的 request ID。
7. 切换会话或新建会话前取消当前请求。

### 4.4 测试与验收

Rust 单元测试至少覆盖：SSE 分片、多个事件、`[DONE]`、中文分片、错误 JSON、响应超限和取消。

前端测试至少覆盖：

- delta 顺序追加；
- 旧 request ID 事件被忽略；
- 停止后不再追加；
- completed 后只保存一次历史；
- 失败后展示错误并保留部分内容；
- 监听器正确清理。

验收标准：在真实 DeepSeek 或 MiniMax 配置下，首段内容无需等待完整回答即可出现；连续快速发起、取消、切换会话不会串内容。

## 5. P2：完整全局搜索

### 5.1 产品范围

全局搜索应同时返回：

- 书籍：匹配标题、作者、分类、ISBN。
- 划线：匹配正文、章节名和所属书名。
- 想法：匹配正文、章节名和所属书名。

支持过滤：全部类型、书籍、划线、想法，以及指定书籍。搜索结果按相关度优先，同分时按更新时间倒序。

### 5.2 数据结构

不要继续让 `SearchResult` 直接展开 `Note`。改为可区分实体的联合模型：

```ts
type SearchEntityType = "book" | "highlight" | "thought";

interface GlobalSearchResult {
  id: string;
  type: SearchEntityType;
  bookId: string;
  title: string;
  subtitle: string;
  snippet: string;
  score: number;
  updatedAt: string;
}
```

Rust 创建对应结构体，并新增 `GlobalSearchRequest`：`query`、`types`、`bookId`、`limit`、`offset`。

### 5.3 数据库与查询

1. 保留现有 `notes_fts`，不要破坏触发器。
2. 新增 `books_fts`，至少索引 `book_id`、`title`、`author`、`category`；ISBN 可通过元数据表补充普通查询或同步进索引。
3. 为 `books` 的插入、标题/作者/分类/删除状态更新和删除建立同步触发器。
4. 启动迁移时回填现有书籍索引。
5. 新增 `global_search` command，所有动态条件使用参数绑定；不得拼接用户输入。
6. 空查询不执行 FTS，可返回最近更新项，数量必须受限。
7. 中文查询继续使用现有 `search_terms`/bigram 策略，统一书籍和笔记查询规则。
8. 使用 FTS `bm25` 后归一化分数；书名精确匹配、标题前缀匹配可加可解释的固定 boost。
9. 结果片段在后端限制长度，前端不直接渲染服务端 HTML。
10. 保留旧 `search_notes` 一段时间，现有调用迁移完成后再决定是否删除。

### 5.4 UI

重构 `src/components/search/SearchDialog.tsx`：

1. 搜索框下增加类型 chips：全部、书籍、划线、想法。
2. 增加书籍筛选器；默认不加载巨大下拉，可复用现有书籍列表并支持输入过滤。
3. 结果按类型显示不同图标和字段。
4. 点击书籍打开书籍详情；点击笔记打开书籍详情并定位对应笔记。
5. 保留键盘上下选择、Enter 打开、Escape 关闭。
6. 请求增加 160–250ms debounce，并继续使用请求序号屏蔽过期结果。
7. 增加“加载更多”或虚拟滚动，首批建议 30 条。
8. 加载、空结果、数据库错误必须是不同状态。

### 5.5 测试与验收

- 标题、作者、ISBN、中文短语、划线和想法均可命中。
- 类型过滤和书籍过滤可以组合。
- 被软删除的书籍和笔记不返回。
- 更新书名后旧标题不再命中，新标题立即命中。
- 特殊字符和引号不会造成 FTS 语法错误或 SQL 注入。
- 键盘操作、过期请求屏蔽和点击跳转均有前端测试。

## 6. P3：可配置 Reranker

### 6.1 目标与边界

Reranker 用于 AI RAG 候选重排，不替代 FTS，也不直接影响普通全局搜索的基础可用性。没有配置、调用失败或超时时必须降级到当前本地 Hybrid Search。

### 6.2 配置模型

新增 `reranker_settings` 表：

- `id`，固定为 1；
- `provider`；
- `endpoint`；
- `model`；
- `top_n`；
- `enabled`；
- `updated_at`。

API Key 保存为 `reranker:{provider}` 的系统凭据。设置页增加启用开关、Endpoint、Model、Top N、保存并测试。

### 6.3 Provider 与排序流程

建议新增 `src-tauri/src/ai/reranker.rs`：

1. 从 Hybrid Search 取候选 50–100 条。
2. 对候选内容做长度限制和去重。
3. 调用兼容 Cohere/Jina 风格的 rerank API；如果项目决定只支持一种协议，UI 必须明确标注。
4. 验证返回 index 不越界、不重复，分数为有限数值。
5. 取 Top N 作为 AI 上下文，同时保留每本书最低覆盖规则，避免跨书分析只剩一本书。
6. 记录耗时与候选数量，但不记录问题正文、笔记正文和密钥。
7. 超时、429、5xx 或格式错误时回退本地排序，并向前端返回非阻断 warning。

### 6.4 可解释性

内部候选结构增加 `lexicalScore`、`semanticScore`、`rerankScore?`、`finalScore`。无需默认向普通用户展示全部分数，但调试日志和测试必须能验证排序来源。

### 6.5 验收

- 启用后 RAG 使用 rerank Top N。
- 禁用或服务失败时 AI 仍可回答。
- 跨书模式保持选中书籍的证据覆盖。
- 不向 reranker 发送超出候选集的其他本地数据。

## 7. P4：概念级知识图谱

### 7.1 与现有图谱的关系

现有 `KnowledgeGraph` 是书籍节点关系图。概念图谱应作为新的视图模式，不得直接把概念节点混入现有书籍图导致交互失控。

建议提供两个 tab：

- 书籍关系：保留现状。
- 概念网络：展示 Concept / Topic / Idea 及其证据。

### 7.2 数据模型

新增表：

```sql
knowledge_entities(
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,              -- concept/topic/idea
  canonical_name TEXT NOT NULL,
  description TEXT NOT NULL,
  aliases_json TEXT NOT NULL,
  status TEXT NOT NULL,            -- suggested/confirmed/hidden
  source_hash TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);

knowledge_entity_evidence(
  entity_id TEXT NOT NULL,
  note_id TEXT NOT NULL,
  book_id TEXT NOT NULL,
  quote TEXT NOT NULL,
  confidence REAL NOT NULL,
  PRIMARY KEY(entity_id, note_id)
);

knowledge_relations(
  id TEXT PRIMARY KEY,
  from_entity_id TEXT NOT NULL,
  to_entity_id TEXT NOT NULL,
  relation TEXT NOT NULL,          -- broader/narrower/related/supports/conflicts/causes/applies
  summary TEXT NOT NULL,
  confidence REAL NOT NULL,
  evidence_json TEXT NOT NULL,
  input_hash TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
```

为 canonical name、kind、book_id 和关系两端建立索引。

### 7.3 抽取管线

1. 输入只使用未删除的划线和想法，按书籍分批。
2. 使用内容 hash 判断是否需要重新抽取；未变化的笔记复用缓存。
3. 先用本地关键词/名词库生成候选，再让 AI 输出严格 JSON，以降低成本。
4. 每个实体必须至少绑定一条真实笔记证据；无证据实体不得写入。
5. 将同义词归一到 canonical name；优先复用名词库条目。
6. AI 输出需经过 schema 校验；关系两端不存在时拒绝写入。
7. 提供“扫描全部”“仅更新变化内容”“取消”“清理建议项”操作。
8. 任何失败都按书籍隔离，不回滚其他已完成书籍。

### 7.4 用户校正

概念详情支持：确认、隐藏、重命名、添加别名、合并两个实体。合并操作必须用事务更新证据和关系，并处理重复边。

### 7.5 图谱 UI

- 按实体类型着色，提供类型、书籍、最低置信度过滤。
- 点击节点展示定义、别名、关联书籍和证据笔记。
- 点击边展示关系摘要和证据。
- 双击证据打开对应书籍笔记。
- 大于 500 节点时默认只加载高置信度子图，并支持按中心实体扩展。
- 图谱计算放入 worker，沿用现有大图性能策略。

### 7.6 验收

- 每个节点和关系都能追溯到本地笔记。
- 内容变化后只重算受影响实体。
- 合并、隐藏、重命名在重启后仍保留。
- AI 返回伪造 note ID 时不会入库。
- 500 节点规模下操作无明显长时间阻塞。

## 8. P5：Open Library 与 Google Books

### 8.1 先统一 Metadata Provider

在 `src-tauri/src/metadata` 抽象统一接口：

```rust
trait MetadataProvider {
    async fn search(&self, title: &str, author: &str, isbn: Option<&str>) -> Result<Vec<MetadataCandidate>, AppError>;
    async fn fetch(&self, source_id: &str) -> Result<NormalizedMetadata, AppError>;
}
```

豆瓣、Open Library、Google Books 最终都归一化为现有 `book_metadata_sources` 与 `book_metadata_extras` 字段。原始响应继续保存在 `raw_json`，但 UI 不直接依赖外部 JSON。

### 8.2 匹配可信度

候选匹配分数建议由以下因素组成：

- ISBN 完全一致：最高权重。
- 规范化标题一致或高度相似。
- 作者交集。
- 出版社和出版年份辅助匹配。

自动保存阈值建议不低于 0.85；低于阈值时必须让用户选候选，不得静默写入可能错误的书籍。

### 8.3 来源规则

默认优先级建议：微信读书 → 豆瓣 → Google Books → Open Library。来源数据仍然独立保存；不要覆盖其他来源原值。

需要“合并视图”时按字段制定规则：

- ISBN：一致才作为统一值，冲突时展示来源。
- 作者/主题：去重合并并保留来源。
- 简介、目录、评分：不强行合并，按来源切换。
- 封面：用户选择或按清晰度选择，保存来源。

### 8.4 网络与凭据

- Open Library 无密钥时也要设置明确 User-Agent、超时和限流。
- Google Books API Key 使用 `google_books` credential namespace。
- 429 使用有限指数退避；404/无结果不是系统错误。
- 批量补全沿用现有并发限制，每本失败不影响整批。

### 8.5 UI 与验收

1. 设置页提供来源启用状态和 Google Books Key 测试。
2. 元数据页开放来源选择和“自动选择最佳来源”。
3. 候选不确定时展示选择对话框。
4. 详情页按来源展示数据，明确来源和抓取时间。
5. 单本、批量、无结果、限流、错误 Key 和字段冲突都有测试。

## 9. P6：网页、PDF、EPUB 导入

### 9.1 本轮边界

本阶段只做“导入为本地资料并进入搜索/AI”，不做完整阅读器、DRM 内容处理、浏览器扩展、云同步或移动端。

### 9.2 通用来源模型

不要把非书籍内容硬塞进微信读书 `books` 结构。新增：

```sql
library_sources(
  id TEXT PRIMARY KEY,
  source_type TEXT NOT NULL,       -- web/pdf/epub
  title TEXT NOT NULL,
  author TEXT,
  origin TEXT,
  local_path TEXT,
  content_hash TEXT NOT NULL,
  imported_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  is_deleted INTEGER NOT NULL DEFAULT 0
);

source_documents(
  id TEXT PRIMARY KEY,
  source_id TEXT NOT NULL,
  position INTEGER NOT NULL,
  heading TEXT,
  content TEXT NOT NULL,
  locator_json TEXT NOT NULL
);
```

为文档块建立 FTS 表和触发器。AI 引用模型扩展为可引用微信读书笔记或导入文档定位信息。

### 9.3 网页导入

1. 用户输入 HTTPS URL。
2. 后端校验 scheme，阻止 localhost、环回、私网和 link-local 地址，防止 SSRF。
3. 限制重定向次数、响应大小、Content-Type 和超时。
4. 提取标题、作者、正文和原 URL；移除脚本、样式、导航等噪声。
5. 显示预览，用户确认后写库。
6. 对相同规范 URL 与内容 hash 去重。

### 9.4 PDF 导入

1. 使用 Tauri 文件选择器，由用户主动选择本地文件。
2. 校验扩展名、MIME、大小和文件可读性。
3. 按页提取文本并保存页码 locator。
4. 扫描版 PDF 无文本时明确提示“暂不支持 OCR”，不要导入空资料。
5. 文本按标题/段落和 token 长度切块，保留页码范围。

### 9.5 EPUB 导入

1. 校验 ZIP/EPUB 结构，防止 Zip Slip 和解压炸弹。
2. 按 OPF spine 顺序读取章节。
3. 清理 HTML 后保留章节标题、段落和章节 locator。
4. 提取书名、作者、封面；封面复制到应用数据目录，不依赖原文件持续存在。

### 9.6 搜索、AI 和删除

- 全局搜索新增“导入资料”类型。
- RAG 同时检索笔记和导入文档，但结果必须标明来源。
- 引用点击后打开资料详情并定位页码/章节。
- 删除资料采用软删除；提供“永久删除本地副本”二次确认。
- 删除或重新导入时清理/更新对应 FTS、Embedding 和关系缓存。

### 9.7 验收

- 普通网页、文本 PDF、标准 EPUB 可导入并搜索。
- AI 回答能引用正确 URL、PDF 页码或 EPUB 章节。
- 恶意 URL、超大响应、Zip Slip、损坏文件不会导致崩溃或越权读取。
- 相同内容重复导入有明确提示。

## 10. 暂缓项目

以下项目不要与上述阶段混合开发：

- 完整 PDF/EPUB 阅读器与批注编辑。
- 微信读书内嵌 WebView。
- 在线阅读服务。
- 移动端应用。
- 完整浏览器版/Web 数据层。
- 浏览器扩展和云同步。

如后续决定开发 Web 版，需要先抽象 Tauri IPC 与 Web backend 的统一 adapter；当前 `src/api/tauri.ts` 在非 Tauri 环境会直接报错，不能仅靠 UI mock 视为 Web 版完成。

## 11. 每阶段交付清单

MiniMax 每完成一个阶段必须提供：

1. 修改文件列表及每个文件的作用。
2. 新增/变更 IPC 命令、事件和 TypeScript 类型。
3. 数据库迁移说明及旧数据兼容说明。
4. 安全与隐私检查结果。
5. 自动化测试列表及实际运行结果。
6. 手工验收步骤。
7. 已知限制和下一阶段依赖。

最终必须运行：

```powershell
npm test
npm run lint
npm run build
Set-Location src-tauri
cargo test
```

不得以“代码看起来正确”代替实际测试。若环境缺少命令或网络依赖，应明确记录未执行项、原因和用户可复现的命令。

## 12. 可直接发送给 MiniMax 的执行指令

```text
请阅读 docs/remaining-features-implementation-plan.md，并只实现其中当前指定的一个阶段。

执行要求：
1. 开始前检查现有代码、测试和 git 工作区，不覆盖用户已有改动。
2. 先给出该阶段的文件级实施计划，再开始修改。
3. 保持现有 React → Tauri IPC → Rust service/provider → SQLite 分层。
4. 所有数据库变更必须向后兼容并具有幂等性。
5. API Key 不得写入数据库、日志或前端存储。
6. 实现后补齐 Rust 与前端测试，并实际运行 test、lint、build、cargo test。
7. 不实现后续阶段，不做无关重构，不删除或弱化现有测试。
8. 完成后报告修改文件、测试结果、手工验收步骤和已知限制。

当前只执行阶段：P1（将这里替换为 P2/P3/P4/P5/P6）。
```
