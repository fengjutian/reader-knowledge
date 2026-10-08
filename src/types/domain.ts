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
export interface BookPage { total: number; books: Book[]; categories: string[] }
export interface RecommendedBook {
  bookId: string;
  title: string;
  author: string;
  cover: string;
  intro: string;
  category: string;
  reason: string;
  readingCount: number;
  searchIdx: number;
  newRating: number;
  newRatingCount: number;
  newRatingDetail?: { title?: string };
  deepLink?: string;
}
export interface BookRecommendations { books: RecommendedBook[] }
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
  authorName:string; authorAvatar:string; authorUrl:string; authorBio:string; tableOfContents:string;
}
/** 名词来源：manual 人工创建/编辑，wikipedia 维基导入，other 其他。 */
export type GlossarySource = "manual"|"wikipedia"|"other";
/** 审核状态：pending 待确认，confirmed 已确认，ignored 已忽略，conflict 同名冲突，source_missing 来源失效。 */
export type GlossaryStatus = "pending"|"confirmed"|"ignored"|"conflict"|"source_missing";
export interface GlossaryTerm { id:number;term:string;canonicalName:string;aliases:string[];definition:string;source:GlossarySource;sourceTitle:string;sourceUrl:string;wikipediaSnapshot:string;status:GlossaryStatus;updatedAt:number;externalPageId:number;sourceRevisionId:number;sourceDumpVersion:string;sourceUpdatedAt:number;sourceSyncedAt:number;licenseCode:string;manuallyEdited:boolean;sourceContentHash:string;publishedBatchId:string }
export interface WikipediaCandidate { title:string;description:string;excerpt:string;url:string }
export type GlossaryImportJobStatus = "pending"|"downloading"|"verifying"|"parsing"|"resolving_redirects"|"validating"|"ready_to_publish"|"publishing"|"completed"|"paused"|"cancelled"|"failed";
export interface GlossaryImportJob { id:string;sourceType:string;dumpVersion:string;sourceUrl:string;mode:string;status:GlossaryImportJobStatus;totalBytes:number;downloadedBytes:number;scannedCount:number;acceptedCount:number;redirectCount:number;filteredCount:number;insertedCount:number;updatedCount:number;skippedCount:number;conflictCount:number;errorCount:number;currentFile:string;bytesPerSecond:number;errorMessage:string;autoPublish:boolean;startedAt:number;finishedAt:number;createdAt:number;updatedAt:number }
export interface GlossaryImportIssue { id:number;pageId:number|null;title:string;recordType:string;code:string;message:string;retryable:boolean;createdAt:number }
export interface GlossaryImportRequest { sourceUrl?:string;dumpVersion?:string;mode?:string;handleRedirects?:boolean;filterDisambiguation?:boolean;filterListPages?:boolean;autoPublish?:boolean;maxItems?:number;batchSize?:number;concurrency?:number;maxRetries?:number;tempDir?:string;localFile?:string }
export interface GlossaryPublishSummary { inserted:number;updated:number;skipped:number;conflicts:number;aliasesInserted:number;sourceMissing:number }

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
export interface MetadataOverview { weread: number; douban: number; missing: number; vectors: number; relationCache: number }
export interface DatabaseOverview { sizeBytes: number; tables: DatabaseTableStat[]; categories: DatabaseCategoryStat[]; lastSyncedAt?: string; metadata: MetadataOverview }
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
export type SearchEntityType = "book" | "highlight" | "thought" | "source";
/** 全局搜索结果：书籍 / 划线 / 想法共用一个列表，靠 type 区分。 */
export interface GlobalSearchResult {
  id: string;
  type: SearchEntityType;
  bookId: string;
  title: string;
  /** 书籍结果是「作者 · 分类」，笔记结果是章节名。 */
  subtitle: string;
  /** 后端已截断的纯文本摘要。 */
  snippet: string;
  score: number;
  updatedAt: string;
}
export interface GlobalSearchRequest {
  query?: string;
  /** 空表示全部类型。 */
  types?: SearchEntityType[];
  bookId?: string;
  limit?: number;
  offset?: number;
}
export interface GlobalSearchPage { results: GlobalSearchResult[]; hasMore: boolean }
export interface Citation { index: number; note: Note }
export interface GlossaryCitation { index:number;term:string;definition:string;source:string;sourceUrl:string }
export interface AiAnswer {
  content: string;
  citations: Citation[];
  glossaryCitations?: GlossaryCitation[];
  sourcesConsidered: number;
  /** 非阻断提示：rerank 降级、或服务不支持流式而回退。回答本身是完整的。 */
  rerankNote?: string;
  /** 来自导入资料的引用。编号接在 citations 之后。 */
  sourceCitations?: SourceCitation[];
}
export interface SourceCitation {
  index: number; sourceId: string; sourceType: SourceType; title: string;
  locator: SourceLocator; quote: string;
}
export type AiMode = "ask" | "summary" | "compare";
export interface AiTurn { question: string; answer: AiAnswer }
export interface AiRequest { question: string; mode: AiMode; bookIds: string[]; history?: { question: string; answer: string }[] }
/** 后端 `ai-stream` 事件的 TS 形态，与 `src-tauri/src/ai/provider.rs` 的 `AiStreamEvent` 一一对应。 */
export type AiStreamEvent =
  | { requestId: string; type: "started" }
  | { requestId: string; type: "delta"; content: string }
  | { requestId: string; type: "completed"; answer: AiAnswer }
  | { requestId: string; type: "failed"; message: string }
  | { requestId: string; type: "cancelled" };
export interface AiSettings { provider: string; endpoint: string; model: string }
export interface RerankerSettings { provider: string; endpoint: string; model: string; topN: number; enabled: boolean }

// 概念级知识图谱
export type EntityKind = "concept" | "topic" | "idea";
export type EntityStatus = "suggested" | "confirmed" | "hidden";
export type ConceptRelationKind = "broader" | "narrower" | "related" | "supports" | "conflicts" | "causes" | "applies";
export interface ConceptEvidence { noteId: string; bookId: string; quote: string; confidence: number }
export interface KnowledgeEntity {
  id: string; kind: EntityKind; canonicalName: string; description: string;
  aliases: string[]; status: EntityStatus; evidenceCount: number;
  updatedAt: string; evidence: ConceptEvidence[];
}
export interface KnowledgeRelation {
  id: string; fromEntityId: string; toEntityId: string;
  relation: ConceptRelationKind; summary: string;
  confidence: number; evidence: ConceptEvidence[];
}
export interface ConceptGraph {
  entities: KnowledgeEntity[]; relations: KnowledgeRelation[];
  /** 节点过多时后端只返回高置信度子图 */
  truncated: boolean; totalEntities: number;
}
export interface ConceptGraphQuery {
  kinds?: EntityKind[]; bookId?: string; minConfidence?: number;
  centerId?: string; limit?: number; includeHidden?: boolean;
}
export interface ConceptScanResult {
  booksScanned: number; booksFailed: number; notesScanned: number; notesSkipped: number;
  entitiesCreated: number; relationsCreated: number; rejected: number; failures: string[];
}
export interface EntityCorrection {
  id: string; canonicalName?: string; description?: string;
  aliases?: string[]; status?: EntityStatus;
}

// 导入资料（网页 / PDF / EPUB）
export type SourceType = "web" | "pdf" | "epub";
export interface SourceLocator { page?: number; chapter?: number; heading?: string }
export interface LibrarySource {
  id: string; sourceType: SourceType; title: string; author?: string;
  /** 网页是原 URL；PDF / EPUB 是原始文件路径 */
  origin?: string; pageCount: number; cover?: string;
  documentCount: number; importedAt: string; deleted: boolean;
}
export interface SourceDocumentItem {
  id: string; position: number; heading: string; content: string; locator: SourceLocator;
}
export interface SourceDetail extends LibrarySource { documents: SourceDocumentItem[] }
export interface ImportPreview {
  sourceType: SourceType; title: string; author?: string; origin?: string; pageCount: number;
  sample: SourceDocumentItem[]; documentCount: number; warnings: string[];
  /** 同一内容已导入过时为 true，confirm 时不会重复入库 */
  duplicate: boolean; duplicateOf?: string;
  /** 本地文件的内容指纹；确认导入时回传，后端发现文件变了会拒绝 */
  fileFingerprint?: string;
}
export interface ConfirmImportRequest {
  sourceType: SourceType; title: string; author?: string; origin?: string; pageCount: number;
  url?: string; path?: string;
  fileFingerprint?: string;
}
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
