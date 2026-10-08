import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { AiAnswer, AiRequest, AiSettings, AiStreamEvent, Book, ConceptGraph, ConceptGraphQuery, ConceptScanResult, ConfirmImportRequest, EntityCorrection, GlobalSearchPage, GlobalSearchRequest, ImportPreview, KnowledgeEntity, LibrarySource, RerankerSettings, SourceDetail, BookDetail, BookMetadataRow, BookMetadataSourceDetail, BookPage, BookRecommendations, DashboardStats, DatabaseOverview, DatabaseRows, EmbeddingSettings, GlossaryImportIssue, GlossaryImportJob, GlossaryImportRequest, GlossaryPublishSummary, GlossaryStatus, GlossaryTerm, LocalModelStatus, MetadataFetchResult, Note, ReadingPeriod, ReadingStats, RecommendedBook, RelationAnalysis, SearchResult, SemanticRelation, SyncProgress, WikipediaCandidate } from "../types/domain";

const isTauri = () => "__TAURI_INTERNALS__" in window;
async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (isTauri()) return invoke<T>(command, args);
  throw new Error("此功能仅在 wereader 桌面应用中可用");
}

const AI_STREAM_EVENT = "ai-stream";

export interface AiStreamSession {
  requestId: string;
  /** 监听器注册完成并且后端已接受这次请求后 resolve；被拒绝时 reject。 */
  ready: Promise<void>;
  /** 让后端停止生成。 */
  cancel(): void;
  /** 解除事件监听。组件卸载或请求结束时必须调用。 */
  dispose(): void;
}

/**
 * 发起一次流式问答。
 *
 * 监听器必须在 `ask_ai_stream` 之前注册好：后端一收到请求就会立刻发 `started`，
 * 先 invoke 再 listen 会丢掉第一段内容。
 */
export function startAiStream(request: AiRequest, requestId: string, onEvent: (event: AiStreamEvent) => void): AiStreamSession {
  if (!isTauri()) {
    const failed = Promise.reject(new Error("此功能仅在 wereader 桌面应用中可用"));
    failed.catch(() => undefined);
    return { requestId, ready: failed, cancel: () => undefined, dispose: () => undefined };
  }
  let unlisten: (() => void) | null = null;
  let disposed = false;
  const ready = (async () => {
    const off = await listen<AiStreamEvent>(AI_STREAM_EVENT, ({ payload }) => {
      if (!disposed) onEvent(payload);
    });
    if (disposed) {
      off();
      return;
    }
    unlisten = off;
    await invoke<string>("ask_ai_stream", { request, requestId });
  })();
  return {
    requestId,
    ready,
    cancel: () => {
      if (disposed) return;
      invoke<boolean>("cancel_ai_stream", { requestId }).catch(() => undefined);
    },
    dispose: () => {
      if (disposed) return;
      disposed = true;
      unlisten?.();
      unlisten = null;
    },
  };
}

export const api = {
  dashboard: () => call<DashboardStats>("get_dashboard"),
  databaseOverview: () => call<DatabaseOverview>("get_database_overview"),
  databaseRows: (table: string, query = "", limit = 40, offset = 0) => call<DatabaseRows>("list_database_rows", { table, query, limit, offset }),
  readingStats: (mode: ReadingPeriod) => call<ReadingStats>("get_reading_stats", { mode }),
  books: () => call<Book[]>("list_books"),
  booksPage: (query = "", limit = 500, offset = 0, category = "all", readingStatus = "all", withHighlights = true, withThoughts = true, sortBy = "recent") => call<BookPage>("list_books_page", { query, limit, offset, category, readingStatus, withHighlights, withThoughts, sortBy }),
  recommendations: (count = 12, maxIdx = 0) => call<BookRecommendations>("get_book_recommendations", { count, maxIdx }),
  recommendationDetail: (bookId: string, title: string) => call<RecommendedBook>("get_book_recommendation_detail", { bookId, title }),
  bookMetadata: (source = "weread") => call<BookMetadataRow[]>("list_book_metadata", { source }),
  bookMetadataDetails: (bookId: string) => call<BookMetadataSourceDetail[]>("get_book_metadata_details", { bookId }),
  fetchBookMetadata: (bookId: string, source: string, force = false) => call<MetadataFetchResult>("fetch_book_metadata", { bookId, source, force }),
  fetchDoubanBookMetadata: (bookId: string, url: string) => call<MetadataFetchResult>("fetch_douban_book_metadata", { bookId, url }),
  fetchBooksMetadata: (bookIds: string[], source: string, force = false) => call<MetadataFetchResult[]>("fetch_books_metadata", { bookIds, source, force }),
  glossaryTerms: (query = "", source = "", status = "") => call<GlossaryTerm[]>("list_glossary_terms", { query, source, status }),
  saveGlossaryTerm: (term: GlossaryTerm) => call<void>("save_glossary_term", { term }),
  deleteGlossaryTerm: (id: number) => call<void>("delete_glossary_term", { id }),
  setGlossaryTermStatus: (id: number, status: GlossaryStatus) => call<void>("set_glossary_term_status", { id, status }),
  bulkGlossaryTermStatus: (ids: number[], status: GlossaryStatus) => call<number>("bulk_set_glossary_term_status", { ids, status }),
  searchWikipedia: (term: string) => call<WikipediaCandidate[]>("search_wikipedia", { term }),
  createGlossaryImport: (request?: GlossaryImportRequest) => call<GlossaryImportJob>("create_glossary_import", { request }),
  glossaryImports: (limit = 30) => call<GlossaryImportJob[]>("list_glossary_imports", { limit }),
  glossaryImport: (id: string) => call<GlossaryImportJob>("get_glossary_import", { id }),
  pauseGlossaryImport: (id: string) => call<GlossaryImportJob>("pause_glossary_import", { id }),
  resumeGlossaryImport: (id: string) => call<GlossaryImportJob>("resume_glossary_import", { id }),
  cancelGlossaryImport: (id: string) => call<GlossaryImportJob>("cancel_glossary_import", { id }),
  publishGlossaryImport: (id: string) => call<GlossaryPublishSummary>("publish_glossary_import", { id }),
  glossaryImportErrors: (id: string, recordType?: string, limit = 100) => call<GlossaryImportIssue[]>("glossary_import_errors", { id, recordType: recordType ?? null, limit }),
  glossaryImportReport: (id: string) => call<Record<string, unknown>>("glossary_import_report", { id }),
  cleanupGlossaryImport: (retentionDays?: number) => call<number>("cleanup_glossary_import", { retentionDays: retentionDays ?? null }),
  book: (bookId: string) => call<BookDetail>("get_book", { bookId }),
  bookNotes: (bookId: string) => call<Note[]>("list_book_notes", { bookId }),
  openBook: (bookId: string) => call<void>("open_book", { bookId }),
  openExternalUrl: (url: string) => call<void>("open_external_url", { url }),
  notes: (type?: Note["type"]) => call<Note[]>("list_notes", { noteType: type }),
  search: (query: string, noteType?: Note["type"]) => call<SearchResult[]>("search_notes", { query, noteType }),
  globalSearch: (request: GlobalSearchRequest) => call<GlobalSearchPage>("global_search", { request }),
  aiSettings: () => call<AiSettings | null>("get_ai_settings"),
  saveAiSettings: (settings: AiSettings, apiKey?: string) => call<void>("save_ai_settings", { settings, apiKey }),
  testAi: (apiKey?: string) => call<boolean>("test_ai", { apiKey }),
  embeddingSettings: () => call<EmbeddingSettings | null>("get_embedding_settings"),
  rerankerSettings: () => call<RerankerSettings | null>("get_reranker_settings"),
  saveRerankerSettings: (settings: RerankerSettings, apiKey?: string) => call<void>("save_reranker_settings", { settings, apiKey }),
  testReranker: (settings: RerankerSettings, apiKey?: string) => call<boolean>("test_reranker", { settings, apiKey }),
  conceptGraph: (query: ConceptGraphQuery) => call<ConceptGraph>("list_concept_graph", { query }),
  conceptEntity: (entityId: string) => call<KnowledgeEntity>("get_concept_entity", { entityId }),
  correctConceptEntity: (correction: EntityCorrection) => call<KnowledgeEntity>("correct_concept_entity", { correction }),
  mergeConceptEntities: (sourceId: string, targetId: string) => call<void>("merge_concept_entities", { request: { sourceId, targetId } }),
  clearSuggestedConcepts: () => call<number>("clear_suggested_concepts"),
  scanConcepts: (request: { bookId?: string; changedOnly?: boolean } = {}) => call<ConceptScanResult>("scan_concepts", { request }),
  previewWebImport: (url: string) => call<ImportPreview>("preview_web_import", { request: { url } }),
  previewFileImport: (path: string) => call<ImportPreview>("preview_file_import", { request: { path } }),
  confirmImport: (request: ConfirmImportRequest) => call<ImportPreview>("confirm_import", { request }),
  librarySources: (includeDeleted = false) => call<LibrarySource[]>("list_library_sources", { includeDeleted }),
  librarySource: (sourceId: string) => call<SourceDetail>("get_library_source", { sourceId }),
  deleteLibrarySource: (sourceId: string) => call<void>("delete_library_source", { sourceId }),
  purgeLibrarySource: (sourceId: string) => call<void>("purge_library_source", { sourceId }),
  saveEmbeddingSettings: (settings: EmbeddingSettings, apiKey?: string) => call<void>("save_embedding_settings", { settings, apiKey }),
  testEmbedding: () => call<boolean>("test_embedding"),
  semanticRelations: () => call<SemanticRelation[]>("build_semantic_relations"),
  metadataRelations: (source: "weread" | "douban") => call<SemanticRelation[]>("build_metadata_relations", { source }),
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

