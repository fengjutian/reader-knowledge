import { invoke } from "@tauri-apps/api/core";
import type { AiAnswer, AiRequest, AiSettings, Book, BookDetail, BookMetadataRow, BookMetadataSourceDetail, BookPage, BookRecommendations, DashboardStats, DatabaseOverview, DatabaseRows, EmbeddingSettings, LocalModelStatus, MetadataFetchResult, Note, ReadingPeriod, ReadingStats, RelationAnalysis, SearchResult, SemanticRelation, SyncProgress } from "../types/domain";

const isTauri = () => "__TAURI_INTERNALS__" in window;
async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri()) return invoke<T>(command, args);
  throw new Error("此功能仅在 wereader 桌面应用中可用");
}

export const api = {
  dashboard: () => call<DashboardStats>("get_dashboard"),
  databaseOverview: () => call<DatabaseOverview>("get_database_overview"),
  databaseRows: (table: string, query = "", limit = 40, offset = 0) => call<DatabaseRows>("list_database_rows", { table, query, limit, offset }),
  readingStats: (mode: ReadingPeriod) => call<ReadingStats>("get_reading_stats", { mode }),
  books: () => call<Book[]>("list_books"),
  booksPage: (query = "", limit = 500, offset = 0) => call<BookPage>("list_books_page", { query, limit, offset }),
  recommendations: (count = 12, maxIdx = 0) => call<BookRecommendations>("get_book_recommendations", { count, maxIdx }),
  bookMetadata: () => call<BookMetadataRow[]>("list_book_metadata"),
  bookMetadataDetails: (bookId: string) => call<BookMetadataSourceDetail[]>("get_book_metadata_details", { bookId }),
  fetchBookMetadata: (bookId: string, source: string, force = false) => call<MetadataFetchResult>("fetch_book_metadata", { bookId, source, force }),
  fetchDoubanBookMetadata: (bookId: string, url: string) => call<MetadataFetchResult>("fetch_douban_book_metadata", { bookId, url }),
  fetchBooksMetadata: (bookIds: string[], source: string, force = false) => call<MetadataFetchResult[]>("fetch_books_metadata", { bookIds, source, force }),
  book: (bookId: string) => call<BookDetail>("get_book", { bookId }),
  bookNotes: (bookId: string) => call<Note[]>("list_book_notes", { bookId }),
  openBook: (bookId: string) => call<void>("open_book", { bookId }),
  openExternalUrl: (url: string) => call<void>("open_external_url", { url }),
  notes: (type?: Note["type"]) => call<Note[]>("list_notes", { noteType: type }),
  search: (query: string, noteType?: Note["type"]) => call<SearchResult[]>("search_notes", { query, noteType }),
  aiSettings: () => call<AiSettings | null>("get_ai_settings"),
  saveAiSettings: (settings: AiSettings, apiKey?: string) => call<void>("save_ai_settings", { settings, apiKey }),
  testAi: (apiKey?: string) => call<boolean>("test_ai", { apiKey }),
  embeddingSettings: () => call<EmbeddingSettings | null>("get_embedding_settings"),
  saveEmbeddingSettings: (settings: EmbeddingSettings, apiKey?: string) => call<void>("save_embedding_settings", { settings, apiKey }),
  testEmbedding: () => call<boolean>("test_embedding"),
  semanticRelations: () => call<SemanticRelation[]>("build_semantic_relations"),
  localEmbeddingStatus: () => call<LocalModelStatus>("local_embedding_status"),
  downloadLocalEmbedding: () => call<LocalModelStatus>("download_local_embedding"),
  deleteLocalEmbedding: () => call<void>("delete_local_embedding"),
  ask: (request: AiRequest) => call<AiAnswer>("ask_ai", { request }),
  analyzeRelation: (leftBookId: string, rightBookId: string, keywords: string[], refresh = false) => call<RelationAnalysis>("analyze_book_relation", { request: { leftBookId, rightBookId, keywords, refresh } }),
  cachedRelation: (leftBookId: string, rightBookId: string, keywords: string[]) => call<RelationAnalysis | null>("get_cached_relation_analysis", { request: { leftBookId, rightBookId, keywords, refresh: false } }),
  sync: () => call<SyncProgress>("sync_weread"),
  saveSecret: (kind: string, value: string) => call<void>("save_secret", { kind, value }),
  hasSecret: (kind: string) => call<boolean>("has_secret", { kind }),
  testConnection: (kind: string, value?: string) => call<boolean>("test_connection", { kind, value }),
};

