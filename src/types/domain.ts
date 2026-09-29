export type NoteType = "highlight" | "thought";

export interface Book {
  id: string;
  title: string;
  author: string;
  category: string;
  cover: string;
  highlightCount: number;
  thoughtCount: number;
  progress: number;
  updatedAt: string;
}
export interface BookDetail extends Book {
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
export interface AiAnswer { content: string; citations: Citation[]; sourcesConsidered: number }
export type AiMode = "ask" | "summary" | "compare";
export interface AiTurn { question: string; answer: AiAnswer }
export interface AiRequest { question: string; mode: AiMode; bookIds: string[]; history?: { question: string; answer: string }[] }
export interface AiSettings { provider: string; endpoint: string; model: string }
export type RelationKind = "same_concept" | "agreement" | "conflict" | "complementary" | "causal" | "application" | "uncertain";
export interface RelationEvidence { bookId: string; noteId: string; noteType: NoteType; text: string }
export interface RelationClaim { relation: RelationKind; summary: string; confidence: number; evidence: RelationEvidence[] }
export interface RelationAnalysis { relation: RelationKind; concepts: string[]; summary: string; confidence: number; claims: RelationClaim[]; cached: boolean }
export interface SyncProgress {
  status: "idle" | "reading" | "processing" | "complete" | "failed";
  progress: number;
  books: number;
  highlights: number;
  thoughts: number;
  processedBooks: number;
  totalBooks: number;
  message?: string;
}
