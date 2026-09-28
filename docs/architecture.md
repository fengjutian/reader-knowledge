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

