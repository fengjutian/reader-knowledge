# wereader-knowledge

wereader-knowledge 是一个本地优先的个人阅读知识库。它通过 Tauri 2 同步微信读书中的书籍、划线和想法，使用 SQLite/FTS5 检索，并允许 DeepSeek 或 MiniMax 基于检索结果回答问题。

## 开发

```bash
npm install
npm run tauri dev
```

产品需求见 [docs/wereader-knowledge-v0.1-requirements.md](docs/wereader-knowledge-v0.1-requirements.md)，架构说明见 [docs/architecture.md](docs/architecture.md)。

