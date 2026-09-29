# wereader-knowledge v0.1 MVP 产品需求

> 本文档由用户提供的需求定稿落库。项目定位：微信读书个人笔记知识库 + AI 阅读助手。

## 产品目标

wereader-knowledge 不做新的阅读器。第一阶段不实现 PDF/EPUB 阅读、微信读书 WebView、在线阅读或移动端，而是打通：微信读书 → 本地知识库 → 搜索 → AI → 理解自己的阅读记录。

## MVP 范围

- 概览：书籍、划线、想法数量，最近同步时间与同步状态。
- 书籍：按标题/作者搜索，按最近阅读/笔记数量排序，查看书籍详情。
- 划线与想法：明确区分“作者说了什么”和“我想到了什么”，支持来源、章节、日期与复制。
- 全局搜索：`Ctrl/Cmd + K`，SQLite FTS5 检索书籍、划线和想法，支持书籍与类型过滤。
- AI 阅读助手：DeepSeek、MiniMax 统一 Provider 接口；FTS5 Top 20 RAG；回答需附可点击引用；不得虚构笔记。
- 设置：微信读书与 AI API Key 仅存 OS Credential Store，支持连接测试与模型切换。
- 同步：分页拉取、UPSERT、Sync Session、Soft Delete。仅在所有分页完整成功且无 API 错误后执行软删除。

## 技术约束

- Tauri 2 + React + TypeScript + Vite + SCSS
- Zustand；Radix UI 负责交互；Lucide React 负责图标；Motion 负责状态动画
- Rust Core 负责微信读书 Gateway、SQLite/FTS5、AI Gateway 和密钥存储，React 仅通过 Tauri IPC 调用
- SQLite 表：`books`、`chapters`、`highlights`、`thoughts`、`weread_raw`、`sync_sessions`、`sync_state`；原始响应先进入 `weread_raw`
- AI 只发送用户问题及检索出的相关笔记，不发送整个数据库
- 日志不得记录 API Key、Authorization Header、完整 Prompt 或完整隐私数据

## 数据与同步原则

- 外部主键：`bookId`、`bookmarkId`、`reviewId`
- 数据流：微信读书 API → `weread_raw` → 规范化表 → FTS5 → RAG → AI
- 同步流：创建 session → 拉取并 UPSERT books/highlights/thoughts → 更新 FTS → 完整性校验 → soft delete → 提交事务 → 更新 sync state
- 同步失败必须保留已有数据，并展示阶段性计数与可重试错误。

## UI 规范

- 风格：克制、留白、内容优先、轻量、桌面感；参考 Linear + Notion + Readwise + AI Native。
- 支持明暗主题。基础色：背景 `#f7f7f5`、卡片 `#fff`、正文 `#1f1f1f`、边框 `#e6e6e3`、强调色 `#6d5dfc`。
- 侧栏入口：概览、书籍、划线、想法、搜索、AI、设置。
- 动画只表达页面切换、同步进度、搜索结果、AI 流式状态等状态变化。

## 验收标准

- 微信读书：密钥配置与连接测试、三类数据分页同步、增量 UPSERT、软删除、失败不破坏数据。
- 本地知识库：SQLite/FTS5、全文搜索、按书籍/类型过滤。
- AI：DeepSeek/MiniMax、Provider 抽象、RAG Top-K、上下文、流式响应、来源引用、防止伪造。
- UI：Light/Dark、Radix、Lucide、Motion、完整侧栏及 Dashboard/Books/Highlights/Thoughts/Search/AI/Settings 页面。

## 后续路线

v0.2 引入 Embedding、Vector Search、Hybrid Search 和 Reranker；之后再引入 Concept/Topic/Idea/Relation 知识图谱。微信读书只是第一个数据源，未来可扩展网页与 PDF/EPUB。

