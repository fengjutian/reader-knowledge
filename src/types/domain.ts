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
  readingStatus: "unread" | "reading" | "finished";
}
export interface BookDetail extends Book {
  deepLink?: string;
  finished: boolean;
}
export interface BookMetadataRow {
  bookId: string; title: string; author: string; cover: string; isbn: string;
  publisher: string; publishedDate: string; pageCount?: number; subjects: string[];
  sources: string[]; lastFetchedAt?: string; metadataStatus: "missing" | "ready";
}
export interface MetadataFetchResult {
  bookId: string; source: string; status: "updated" | "cached" | "not_found" | "failed"; message: string;
}
export interface BookMetadataSourceDetail {
  source:string; sourceId:string; sourceUrl?:string; title:string; authors:string[]; isbn:string;
  publisher:string; publishedDate:string; pageCount?:number; subjects:string[]; coverUrl:string;
  description:string; rating?:number; ratingCount?:number; fetchedAt:string;
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
export interface DatabaseTableStat { name: string; label: string; rows: number }
export interface DatabaseCategoryStat { label: string; count: number }
export interface DatabaseOverview { sizeBytes: number; tables: DatabaseTableStat[]; categories: DatabaseCategoryStat[]; lastSyncedAt?: string }
export interface DatabaseRow { id: string; primary: string; secondary: string; detail: string; createdAt: string }
export interface DatabaseRows { total: number; rows: DatabaseRow[] }

export type ReadingPeriod = "weekly" | "monthly" | "annually" | "overall";
export interface ReadingCategory {
  categoryTitle: string;
  readingTime: number;
  readingCount: number;
  val: number;
}
export interface ReadingLongestItem {
  book?: { bookId: string; title: string; author?: string; cover?: string };
  albumInfo?: { albumId?: string; title?: string; author?: string; cover?: string };
  readTime: number;
  tags?: string[];
}
export interface ReadingStatItem { stat: string; counts: string; scheme?: string }
export interface ReadingAuthor { authorId?: string; name: string; count: number; readTime?: string }
export interface ReadingPublisher { name: string; count: number }
export interface ReadingStats {
  baseTime?: number;
  readTimes?: Record<string, number>;
  dailyReadTimes?: Record<string, number>;
  readDays?: number;
  totalReadTime?: number;
  dayAverageReadTime?: number;
  compare?: number;
  preferCategory?: ReadingCategory[];
  preferCategoryWord?: string;
  preferTime?: number[];
  preferTimeWord?: string;
  preferAuthor?: ReadingAuthor[];
  preferPublisher?: ReadingPublisher[];
  readLongest?: ReadingLongestItem[];
  readStat?: ReadingStatItem[];
  readRate?: number;
  wrReadTime?: number;
  wrListenTime?: number;
}

export interface SearchResult extends Note { score: number }
export interface Citation { index: number; note: Note }
export interface AiAnswer { content: string; citations: Citation[]; sourcesConsidered: number }
export type AiMode = "ask" | "summary" | "compare";
export interface AiTurn { question: string; answer: AiAnswer }
export interface AiRequest { question: string; mode: AiMode; bookIds: string[]; history?: { question: string; answer: string }[] }
export interface AiSettings { provider: string; endpoint: string; model: string }
export interface EmbeddingSettings { provider: string; endpoint: string; model: string }
export interface SemanticRelation { id: string; from: string; to: string; score: number; keywords: string[]; relation: string; evidence: { bookId: string; noteId: string; text: string }[] }
export interface LocalModelStatus { installed: boolean; sizeBytes: number; model: string }
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
