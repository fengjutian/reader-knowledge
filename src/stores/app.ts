import { create } from "zustand";

export type Page = "dashboard" | "books" | "highlights" | "thoughts" | "search" | "settings";
interface AppState {
  page: Page;
  theme: "light" | "dark";
  searchOpen: boolean;
  setPage: (page: Page) => void;
  setSearchOpen: (open: boolean) => void;
  toggleTheme: () => void;
}

export const useAppStore = create<AppState>((set) => ({
  page: "dashboard",
  theme: (localStorage.getItem("readflow-theme") as "light" | "dark") || "light",
  searchOpen: false,
  setPage: page => set({ page }),
  setSearchOpen: searchOpen => set({ searchOpen }),
  toggleTheme: () => set(state => {
    const theme = state.theme === "light" ? "dark" : "light";
    localStorage.setItem("readflow-theme", theme);
    return { theme };
  }),
}));

