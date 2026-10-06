import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { api } from "../api/tauri";
import type { SyncProgress } from "../types/domain";

/** 同步成功后递增的单调版本号。页面订阅它来决定何时重新拉取数据。 */
export const LIBRARY_UPDATED_EVENT = "library-updated";

export interface LibraryUpdatedDetail {
  source: string;
  books: number;
  highlights: number;
  thoughts: number;
  completedAt: number;
}

interface SyncState extends SyncProgress {
  /** 最近一次成功同步的版本号。0 表示本次会话还没有成功同步过。 */
  revision: number;
  /** 最近一次成功同步完成的时间戳。 */
  completedAt?: number;
  run: () => Promise<void>;
}

function syncErrorMessage(error: unknown) {
  const message = typeof error === "string" ? error : error instanceof Error ? error.message : "同步失败";
  return message.includes("No matching entry found in secure storage")
    ? "请先在设置中填写并保存微信读书 API Key"
    : message;
}

export const useSyncStore = create<SyncState>((set, get) => ({
  status: "idle", progress: 0, books: 0, highlights: 0, thoughts: 0, processedBooks: 0, totalBooks: 0,
  revision: 0,
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
      const result = await api.sync();
      // 只有成功完成后才推进 revision 并广播事件，失败时保留上一次成功数据。
      const completedAt = Date.now();
      const revision = get().revision + 1;
      set({ ...result, revision, completedAt });
      if (typeof window !== "undefined") {
        window.dispatchEvent(new CustomEvent<LibraryUpdatedDetail>(LIBRARY_UPDATED_EVENT, {
          detail: {
            source: "weread",
            books: result.books,
            highlights: result.highlights,
            thoughts: result.thoughts,
            completedAt,
          },
        }));
      }
    } catch (error) {
      set({ status: "failed", progress: 0, message: syncErrorMessage(error) });
    } finally {
      unlisten();
    }
  },
}));
