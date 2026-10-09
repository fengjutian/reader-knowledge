const DB_NAME = "readflow-graph";
const STORE = "analyses";

/**
 * 本地图谱关系的算法版本。改动 worker 的关系计算逻辑时必须递增，
 * 否则旧缓存会被当成新结果复用。
 */
export const GRAPH_ALGORITHM_VERSION = "v9";
/** 语义关系缓存的命名空间前缀，用于按前缀批量失效。 */
export const SEMANTIC_CACHE_PREFIX = "semantic-relations:v4";
/** 本地图谱分析缓存的命名空间前缀。 */
export const LOCAL_CACHE_PREFIX = `local-relations:${GRAPH_ALGORITHM_VERSION}`;

/**
 * 单例连接：每次读写都新开一条 IDBDatabase 会让连接无界累积（且易触发
 * versionchange 阻塞）。这里复用同一条，事务用完立即 end，不依赖 close()。
 */
let connection: Promise<IDBDatabase> | null = null;

function database() {
  if (connection) return connection;
  connection = new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, 1);
    request.onupgradeneeded = () => request.result.createObjectStore(STORE);
    request.onsuccess = () => {
      const db = request.result;
      // 别的标签页升级版本时主动让路，避免本连接永久阻塞对方的 versionchange。
      db.onversionchange = () => {
        db.close();
        connection = null;
      };
      resolve(db);
    };
    request.onerror = () => {
      connection = null;
      reject(request.error);
    };
    request.onblocked = () => {
      connection = null;
      reject(new Error("IndexedDB 升级被其他标签页阻塞"));
    };
  });
  return connection;
}

/** 在一个只读事务里读一个键，事务结束即释放。 */
export async function readGraphCache<T>(key: string): Promise<T | undefined> {
  try {
    const db = await database();
    return await new Promise<T | undefined>((resolve, reject) => {
      const transaction = db.transaction(STORE, "readonly");
      const request = transaction.objectStore(STORE).get(key);
      request.onsuccess = () => resolve(request.result as T | undefined);
      request.onerror = () => reject(request.error);
      transaction.onabort = () => reject(transaction.error);
    });
  } catch { return undefined; }
}

export async function writeGraphCache(key: string, value: unknown) {
  try {
    const db = await database();
    await new Promise<void>((resolve, reject) => {
      const transaction = db.transaction(STORE, "readwrite");
      transaction.objectStore(STORE).put(value, key);
      transaction.oncomplete = () => resolve();
      transaction.onerror = () => reject(transaction.error);
      transaction.onabort = () => reject(transaction.error);
    });
  } catch { /* Cache failure must not block the graph. */ }
}

async function deleteKeys(predicate: (key: string) => boolean) {
  try {
    const db = await database();
    await new Promise<void>((resolve, reject) => {
      const transaction = db.transaction(STORE, "readwrite");
      const store = transaction.objectStore(STORE);
      const request = store.getAllKeys();
      request.onsuccess = () => {
        for (const key of request.result) if (typeof key === "string" && predicate(key)) store.delete(key);
      };
      request.onerror = () => reject(request.error);
      transaction.oncomplete = () => resolve();
      transaction.onerror = () => reject(transaction.error);
      transaction.onabort = () => reject(transaction.error);
    });
  } catch { /* Cache cleanup is best-effort. */ }
}

/** 按命名空间删除缓存，避免无条件清空整个 store。 */
export async function clearGraphCacheByPrefix(prefix: string) {
  await deleteKeys(key => key.startsWith(prefix));
}

/** 只清理本地图谱分析缓存，保留语义关系缓存。 */
export async function clearLocalGraphCache() {
  await clearGraphCacheByPrefix(LOCAL_CACHE_PREFIX);
}

/** 只清理语义关系缓存，保留本地图谱分析缓存。 */
export async function clearSemanticGraphCache() {
  await clearGraphCacheByPrefix(SEMANTIC_CACHE_PREFIX);
}

export async function clearGraphCache() {
  try {
    const db = await database();
    await new Promise<void>((resolve, reject) => {
      const transaction = db.transaction(STORE, "readwrite");
      transaction.objectStore(STORE).clear();
      transaction.oncomplete = () => resolve();
      transaction.onerror = () => reject(transaction.error);
      transaction.onabort = () => reject(transaction.error);
    });
  } catch { /* Cache cleanup is best-effort. */ }
}

function fnv1a(parts: string[]) {
  let hash = 2166136261;
  const update = (value: string) => { for (let index = 0; index < value.length; index += 1) hash = Math.imul(hash ^ value.charCodeAt(index), 16777619); };
  parts.forEach(update);
  return (hash >>> 0).toString(36);
}

export interface GraphCacheInput {
  books: { id: string; updatedAt: string; highlightCount: number; thoughtCount: number }[];
  notes: { id: string; bookId: string; chapter: string; content: string; type: string; createdAt: string }[];
}

/**
 * 本地图谱缓存 key：包含算法版本、活跃书籍 ID 及其更新时间/笔记数、笔记 ID 及其内容摘要。
 * 任何一项变化都会得到不同的 key，从而自动失效旧缓存。
 */
export function graphCacheKey({ books, notes }: GraphCacheInput) {
  const bookPart = books
    .map(book => `${book.id}:${book.updatedAt}:${book.highlightCount}:${book.thoughtCount}`)
    .join("|");
  const notePart = notes
    .map(note => `${note.id}:${note.bookId}:${note.type}:${note.createdAt}:${note.chapter}:${note.content.length}:${fnv1a([note.content])}`)
    .join("|");
  return `${LOCAL_CACHE_PREFIX}:${books.length}:${notes.length}:${fnv1a([bookPart, notePart])}`;
}

/**
 * 语义关系缓存 key：在本地图谱 key 之上再叠加 embedding 模型名，
 * 使切换模型或安装/删除本地模型时缓存自然失效。
 */
export function semanticCacheKey(input: GraphCacheInput, embeddingModel: string, metadataSources: string[] = []) {
  const metadataPart = metadataSources.slice().sort().join(",");
  return `${SEMANTIC_CACHE_PREFIX}:${embeddingModel || "none"}:${graphCacheKey(input)}:${fnv1a([metadataPart])}`;
}
