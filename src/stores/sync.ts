import { create } from "zustand";
import { api } from "../api/tauri";
import type { SyncProgress } from "../types/domain";

interface SyncState extends SyncProgress { run: () => Promise<void> }
export const useSyncStore = create<SyncState>((set) => ({
  status: "idle", progress: 0, books: 0, highlights: 0, thoughts: 0,
  run: async () => {
    set({ status: "reading", progress: 0, message: "正在读取微信读书…" });
    try {
      set(await api.sync());
    } catch (error) {
      set({ status: "failed", progress: 0, message: typeof error === "string" ? error : error instanceof Error ? error.message : "同步失败" });
    }
  },
}));

