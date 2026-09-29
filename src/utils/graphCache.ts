const DB_NAME = "readflow-graph";
const STORE = "analyses";

function database() {
  return new Promise<IDBDatabase>((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, 1);
    request.onupgradeneeded = () => request.result.createObjectStore(STORE);
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

export async function readGraphCache<T>(key: string): Promise<T | undefined> {
  try {
    const db = await database();
    return await new Promise<T | undefined>((resolve, reject) => {
      const request = db.transaction(STORE, "readonly").objectStore(STORE).get(key);
      request.onsuccess = () => resolve(request.result as T | undefined);
      request.onerror = () => reject(request.error);
    });
  } catch { return undefined; }
}

export async function writeGraphCache(key: string, value: unknown) {
  try {
    const db = await database();
    await new Promise<void>((resolve, reject) => {
      const request = db.transaction(STORE, "readwrite").objectStore(STORE).put(value, key);
      request.onsuccess = () => resolve();
      request.onerror = () => reject(request.error);
    });
  } catch { /* Cache failure must not block the graph. */ }
}

export function graphCacheKey(books: { id: string; updatedAt: string; highlightCount: number; thoughtCount: number }[], noteCount: number) {
  let hash = 2166136261;
  const signature = `${noteCount}|${books.map(book => `${book.id}:${book.updatedAt}:${book.highlightCount}:${book.thoughtCount}`).join("|")}`;
  for (let index = 0; index < signature.length; index += 1) hash = Math.imul(hash ^ signature.charCodeAt(index), 16777619);
  return `v7:${books.length}:${noteCount}:${(hash >>> 0).toString(36)}`;
}
