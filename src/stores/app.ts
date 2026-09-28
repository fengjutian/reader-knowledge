import { create } from "zustand";

export type Page = "dashboard" | "books" | "bookDetail" | "highlights" | "thoughts" | "search" | "ai" | "settings";
interface AppState {
  page: Page;
  theme: "light" | "dark";
  searchOpen: boolean;
  selectedBookId?: string;
  setPage: (page: Page) => void;
  setSearchOpen: (open: boolean) => void;
  openBook: (bookId: string) => void;
  toggleTheme: () => void;
}

export const useAppStore = create<AppState>((set) => ({
  page: "dashboard",
  theme: (localStorage.getItem("readflow-theme") as "light" | "dark") || "light",
  searchOpen: false,
  setPage: page => set({ page }),
  setSearchOpen: searchOpen => set({ searchOpen }),
  openBook: selectedBookId => set({ selectedBookId, page: "bookDetail" }),
  toggleTheme: () => set(state => {
    const theme = state.theme === "light" ? "dark" : "light";
    localStorage.setItem("readflow-theme", theme);
    return { theme };
  }),
}));

