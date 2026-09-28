import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api/tauri";
import type { SyncProgress } from "../types/domain";

interface SyncState extends SyncProgress { run: () => Promise<void> }

function syncErrorMessage(error: unknown) {
  const message = typeof error === "string" ? error : error instanceof Error ? error.message : "同步失败";
  return message.includes("No matching entry found in secure storage")
    ? "请先在设置中填写并保存微信读书 API Key"
    : message;
}

export const useSyncStore = create<SyncState>((set) => ({
  status: "idle", progress: 0, books: 0, highlights: 0, thoughts: 0, processedBooks: 0, totalBooks: 0,
  run: async () => {
    set({ status: "reading", progress: 0, processedBooks: 0, totalBooks: 0, message: "正在读取微信读书…" });
    const unlisten = await listen<SyncProgress>("sync-progress", event => {
      const progress = event.payload;
      set({
        ...progress,
        message: progress.status === "processing"
          ? progress.processedBooks === 0
            ? `书架已载入，共 ${progress.books} 本书`
            : `已同步 ${progress.processedBooks}/${progress.totalBooks} 本`
          : progress.status === "complete" ? "所有内容已是最新" : undefined,
      });
    });
    try {
      set(await api.sync());
    } catch (error) {
      set({ status: "failed", progress: 0, message: syncErrorMessage(error) });
    } finally {
      unlisten();
    }
  },
}));

