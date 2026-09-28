# 架构说明

前端采用“页面 → 领域组件 → store → IPC adapter”单向依赖；Rust 采用“command → service → repository/provider”分层。外部数据结构不会进入 UI，模型供应商也不会进入业务组件。

```text
React pages → feature components → Zustand stores → api/tauri.ts
                                                    ↓ IPC
Tauri commands → sync / AI services → repositories → SQLite/FTS5
                  ↓                   ↓
             WeRead gateway       AI providers
```

当前仓库保留 mock adapter，使 Web/Vite 模式也可独立预览；Tauri 环境下自动切换到真实 IPC。

## 微信读书官方协议

微信读书接入只以官方 [weread-skills](https://weread.qq.com/r/weread-skills) 为准：

- 统一入口：`POST https://i.weread.qq.com/api/agent/gateway`
- 鉴权：`Authorization: Bearer wrk-...`
- 每个请求上报 `skill_version: "1.0.4"`
- `api_name`、`skill_version` 和业务参数全部位于 JSON 顶层
- 书架使用 `/shelf/sync`
- 笔记概览使用 `/user/notebooks` 和 `lastSort` 游标
- 单书划线使用 `/book/bookmarklist`
- 单书个人想法使用 `/review/list/mine` 和 `synckey` 游标
- 响应包含 `upgrade_info` 时立即中止操作并提示升级

