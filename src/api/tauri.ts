import { invoke } from "@tauri-apps/api/core";
import type { AiAnswer, Book, DashboardStats, Note, SearchResult, SyncProgress } from "../types/domain";

const isTauri = () => "__TAURI_INTERNALS__" in window;
async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri()) return invoke<T>(command, args);
  throw new Error("此功能仅在 ReadFlow 桌面应用中可用");
}

export const api = {
  dashboard: () => call<DashboardStats>("get_dashboard"),
  books: () => call<Book[]>("list_books"),
  notes: (type?: Note["type"]) => call<Note[]>("list_notes", { noteType: type }),
  search: (query: string) => call<SearchResult[]>("search_notes", { query }),
  ask: (question: string) => call<AiAnswer>("ask_ai", { question }),
  sync: () => call<SyncProgress>("sync_weread"),
  saveSecret: (kind: string, value: string) => call<void>("save_secret", { kind, value }),
  testConnection: (kind: string) => call<boolean>("test_connection", { kind }),
};

