export type NoteType = "highlight" | "thought";

export interface Book {
  id: string;
  title: string;
  author: string;
  cover: string;
  highlightCount: number;
  thoughtCount: number;
  progress: number;
  updatedAt: string;
}
export interface BookDetail extends Book {
  category: string;
  deepLink?: string;
  finished: boolean;
}

export interface Note {
  id: string;
  type: NoteType;
  bookId: string;
  bookTitle: string;
  chapter: string;
  content: string;
  createdAt: string;
}

export interface DashboardStats {
  books: number;
  highlights: number;
  thoughts: number;
  lastSyncedAt?: string;
}

export interface SearchResult extends Note { score: number }
export interface Citation { index: number; note: Note }
export interface AiAnswer { content: string; citations: Citation[] }
export interface AiSettings { provider: string; endpoint: string; model: string }
export interface SyncProgress {
  status: "idle" | "reading" | "processing" | "complete" | "failed";
  progress: number;
  books: number;
  highlights: number;
  thoughts: number;
  message?: string;
}

