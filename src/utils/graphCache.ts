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

export function graphCacheKey(
  books: { id: string; updatedAt: string; highlightCount: number; thoughtCount: number }[],
  notes: { id: string; bookId: string; chapter: string; content: string; type: string; createdAt: string }[],
) {
  let hash = 2166136261;
  const update = (value: string) => { for (let index = 0; index < value.length; index += 1) hash = Math.imul(hash ^ value.charCodeAt(index), 16777619); };
  books.forEach(book => update(`${book.id}:${book.updatedAt}:${book.highlightCount}:${book.thoughtCount}|`));
  notes.forEach(note => update(`${note.id}:${note.bookId}:${note.type}:${note.createdAt}:${note.chapter}:${note.content}|`));
  return `v8:${books.length}:${notes.length}:${(hash >>> 0).toString(36)}`;
}
