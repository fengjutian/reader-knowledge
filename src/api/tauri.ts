import { invoke } from "@tauri-apps/api/core";
import { books, notes } from "../data/mock";
import type { AiAnswer, Book, DashboardStats, Note, SearchResult, SyncProgress } from "../types/domain";

const isTauri = () => "__TAURI_INTERNALS__" in window;
async function call<T>(command: string, args?: Record<string, unknown>, fallback?: () => T | Promise<T>): Promise<T> {
  if (isTauri()) return invoke<T>(command, args);
  if (fallback) return fallback();
  throw new Error(`${command} 仅在桌面应用中可用`);
}

export const api = {
  dashboard: () => call<DashboardStats>("get_dashboard", undefined, () => ({ books: 128, highlights: 2381, thoughts: 426, lastSyncedAt: "2026-09-28 10:32" })),
  books: () => call<Book[]>("list_books", undefined, () => books),
  notes: (type?: Note["type"]) => call<Note[]>("list_notes", { noteType: type }, () => type ? notes.filter(n => n.type === type) : notes),
  search: (query: string) => call<SearchResult[]>("search_notes", { query }, () => notes.filter(n => `${n.bookTitle}${n.chapter}${n.content}`.toLowerCase().includes(query.toLowerCase())).map(n => ({ ...n, score: 1 }))),
  ask: (question: string) => call<AiAnswer>("ask_ai", { question }, async () => {
    await new Promise(resolve => setTimeout(resolve, 900));
    return { content: "从现有笔记看，你对组织的理解集中在两个层面：一是制度背后的激励与资源流向；二是系统对个人能力和行为的放大或抑制。你更倾向于把组织视为一个动态系统，而不只是静态结构。[1][2]", citations: [{ index: 1, note: notes[1] }, { index: 2, note: notes[3] }] };
  }),
  sync: () => call<SyncProgress>("sync_weread", undefined, async () => {
    await new Promise(resolve => setTimeout(resolve, 1200));
    return { status: "complete", progress: 100, books: 128, highlights: 2381, thoughts: 426 };
  }),
  saveSecret: (kind: string, value: string) => call<void>("save_secret", { kind, value }, () => undefined),
  testConnection: (kind: string) => call<boolean>("test_connection", { kind }, async () => { await new Promise(r => setTimeout(r, 600)); return true; }),
};

