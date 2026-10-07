import { create } from "zustand";
import type { SourceLocator } from "../types/domain";

export type Page = "dashboard" | "books" | "discover" | "metadata" | "notes" | "ai" | "graph" | "glossary" | "database" | "settings" | "import";
export type Theme = "light" | "dark" | "voyage" | "minimax" | "landpack" | "parley" | "lagoon";
export type FontFamily = "system" | "source-han-sans" | "source-han-serif" | "lxgw-wenkai";
interface AppState {
  page: Page;
  theme: Theme;
  fontFamily: FontFamily;
  searchOpen: boolean;
  selectedBookId?: string;
  selectedNoteId?: string;
  /** 正在查看的导入资料 id */
  sourceDetailId?: string;
  /**
   * 从 AI 引用跳转过来时要定位到的位置（PDF 页码 / EPUB 章节 / 标题）。
   *
   * 和 `sourceDetailId` 一起设置、一起清空，避免两个状态不同步导致
   * 「换了资料却沿用旧 locator」。
   */
  sourceDetailLocator?: SourceLocator;
  aiDraft?: { mode: "compare"; bookIds: string[]; question: string };
  setPage: (page: Page) => void;
  setSearchOpen: (open: boolean) => void;
  openBook: (bookId: string, noteId?: string) => void;
  setSourceDetail: (sourceId: string | undefined, locator?: SourceLocator) => void;
  closeBook: () => void;
  clearSelectedNote: () => void;
  openAiCompare: (bookIds: string[]) => void;
  clearAiDraft: () => void;
  setTheme: (theme: Theme) => void;
  setFontFamily: (fontFamily: FontFamily) => void;
}

export const useAppStore = create<AppState>((set) => ({
  page: "dashboard",
  theme: (() => {
    const saved = localStorage.getItem("readflow-theme");
    return saved === "dark" || saved === "voyage" || saved === "minimax" || saved === "landpack" || saved === "parley" || saved === "lagoon" ? saved : "light";
  })(),
  fontFamily: (() => {
    const saved = localStorage.getItem("wereader-font-family");
    return saved === "source-han-sans" || saved === "source-han-serif" || saved === "lxgw-wenkai" ? saved : "system";
  })(),
  searchOpen: false,
  setPage: page => set({ page }),
  setSearchOpen: searchOpen => set({ searchOpen }),
  openBook: (selectedBookId, selectedNoteId) => set({ selectedBookId, selectedNoteId }),
  setSourceDetail: (sourceDetailId, sourceDetailLocator) => set({ sourceDetailId, sourceDetailLocator }),
  closeBook: () => set({ selectedBookId: undefined, selectedNoteId: undefined }),
  clearSelectedNote: () => set({ selectedNoteId: undefined }),
  openAiCompare: bookIds => set({ page: "ai", aiDraft: { mode: "compare", bookIds, question: "请比较这些书对同一主题的观点、共识、分歧与互补之处。" } }),
  clearAiDraft: () => set({ aiDraft: undefined }),
  setTheme: theme => set(() => {
    localStorage.setItem("readflow-theme", theme);
    return { theme };
  }),
  setFontFamily: fontFamily => set(() => {
    localStorage.setItem("wereader-font-family", fontFamily);
    return { fontFamily };
  }),
}));

