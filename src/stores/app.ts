import { create } from "zustand";

export type Page = "dashboard" | "books" | "highlights" | "thoughts" | "search" | "ai" | "graph" | "settings";
interface AppState {
  page: Page;
  theme: "light" | "dark";
  searchOpen: boolean;
  selectedBookId?: string;
  selectedNoteId?: string;
  setPage: (page: Page) => void;
  setSearchOpen: (open: boolean) => void;
  openBook: (bookId: string, noteId?: string) => void;
  closeBook: () => void;
  clearSelectedNote: () => void;
  toggleTheme: () => void;
}

export const useAppStore = create<AppState>((set) => ({
  page: "dashboard",
  theme: (localStorage.getItem("readflow-theme") as "light" | "dark") || "light",
  searchOpen: false,
  setPage: page => set({ page }),
  setSearchOpen: searchOpen => set({ searchOpen }),
  openBook: (selectedBookId, selectedNoteId) => set({ selectedBookId, selectedNoteId }),
  closeBook: () => set({ selectedBookId: undefined, selectedNoteId: undefined }),
  clearSelectedNote: () => set({ selectedNoteId: undefined }),
  toggleTheme: () => set(state => {
    const theme = state.theme === "light" ? "dark" : "light";
    localStorage.setItem("readflow-theme", theme);
    return { theme };
  }),
}));

