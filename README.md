# 阅读者

> 整理你的个人思考

阅读者是一个本地优先的个人阅读知识库。它将微信读书中的书籍、划线和想法同步到本机，把散落的阅读记录整理成可搜索、可关联、可追溯的个人知识，并允许 AI 在明确引用来源的前提下协助总结与分析。

它不是另一个阅读器，而是阅读之后的工作台。

## 主要功能

### 阅读资料库

- 同步微信读书书架、划线、想法与阅读统计
- 按书名、作者和笔记内容进行全文检索
- 区分原文划线与个人想法，保留书籍、章节和时间信息
- 查看书籍详情，并跳转到微信读书 Web

### AI 阅读助手

- 面向全库提问、单书总结和跨书分析
- 基于检索结果回答，展示可点击的笔记引用
- 支持 DeepSeek、MiniMax 及自定义兼容服务
- 只发送问题和被检索出的相关内容，不上传整个数据库

### 名词库

- 为人物、地点、事件和专业概念维护稳定解释
- 优先从中文维基百科获取内容，也可以手动编辑
- 名词解释可作为 AI 问答的背景上下文
- AI 回答通过 `[W1]` 等标记引用名词来源

### 书籍关系图谱

- 根据笔记语义、共同作者及元数据构建书籍关系
- 支持观点一致、冲突、互补、因果和理论应用等关系分析
- 提供适合全库浏览的 3D 图谱与关系流动粒子
- 支持从任意书籍出发查看直接关联，并进行跨书比较

### 书籍元数据

- 从微信读书和豆瓣补全书籍资料
- 不同数据源独立保存、独立展示，不强行合并字段
- 支持 ISBN、出版社、出版时间、页数、主题、简介和目录等信息

### 本地知识基础设施

- SQLite 保存书籍、笔记、元数据、向量索引和分析缓存
- SQLite FTS5 提供全文检索
- 支持本地中文 Embedding 模型，也可连接远程 Embedding 服务
- API Key 保存在操作系统凭据库，不写入数据库或日志

## 工作方式

```text
微信读书 / 元数据源
        ↓
本地 SQLite + FTS5
        ↓
全文检索 + Embedding 语义检索
        ↓
名词解释 + 书籍关系 + 笔记证据
        ↓
AI 总结、问答与跨书分析
```

## 技术栈

- 桌面端：Tauri 2、Rust
- 前端：React 19、TypeScript、Vite、SCSS
- 状态与交互：Zustand、Radix UI、Motion、Lucide
- 数据表格：TanStack Table
- 图谱：Sigma.js、Graphology、react-force-graph-3d
- 可视化：Nivo
- 数据：SQLite、FTS5
- 向量：FastEmbed / OpenAI-compatible Embeddings API

## 开发环境

请先安装：

- Node.js 20 或更高版本
- npm
- Rust stable
- Tauri 2 所需的系统依赖
- Windows 环境下需要 WebView2

安装依赖：

```bash
npm install
```

启动桌面开发环境：

```bash
npm run tauri dev
```

仅启动前端预览：

```bash
npm run dev
```

构建前端：

```bash
npm run build
```

构建桌面应用：

```bash
npm run tauri build
```

## 初次使用

1. 打开“设置 → 微信读书”，填写 Agent Gateway API Key。
2. 测试连接并同步微信读书数据。
3. 在“设置 → AI 服务”中配置 DeepSeek、MiniMax 或自定义模型。
4. 如需语义检索和关系图谱，可下载本地 Embedding 模型，或配置兼容的远程服务。
5. 在书籍、笔记、AI 助手、知识图谱和名词库中整理你的阅读材料。

微信读书 API Key 应以 `wrk-` 开头。

## 数据与隐私

阅读者坚持本地优先：

- 阅读数据和索引默认保存在本机
- 密钥由操作系统凭据库管理
- 日志不记录 API Key、Authorization Header 或完整私人内容
- AI 请求仅包含当前问题及检索出的必要上下文
- 同步失败不会删除现有本地数据

使用远程 AI 或 Embedding 服务时，被选中的上下文仍会发送给对应服务商，请根据其隐私政策自行判断是否启用。

## 项目结构

```text
src/                    React 前端
  components/           通用组件与领域组件
  pages/                页面模块
  stores/               Zustand 状态
  workers/              图谱等后台计算
src-tauri/               Rust / Tauri 后端
  src/ai/                AI 与 Embedding Provider
  src/database/          SQLite、迁移与查询
  src/metadata/          书籍元数据来源
  src/weread/            微信读书同步
docs/                    产品与架构文档
```

## 当前状态

项目仍在持续开发中。数据库结构、接口及交互可能调整，重要数据请定期备份。

更多信息请参阅 [架构说明](docs/architecture.md) 和 [产品需求](docs/wereader-v0.1-requirements.md)。
