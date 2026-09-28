import type { Book, Note } from "../types/domain";

type Strength = "all" | "strong" | "balanced" | "broad";
type Node = Book & { x: number; y: number };
type Edge = { id: string; from: string; to: string; score: number; keywords: string[] };
const LIMITS = { all: { books: Infinity, edges: Infinity, neighbors: 3 }, strong: { books: 20, edges: 20, neighbors: 1 }, balanced: { books: 28, edges: 38, neighbors: 2 }, broad: { books: 36, edges: 60, neighbors: 3 } };
const STOP = new Set(["我们","你们","他们","这个","那个","一个","什么","就是","因为","所以","但是","如果","可以","没有","不是","已经","自己","这种","这样","以及","对于","进行","需要","可能","时候","这里","其中","那些","这些"]);

function terms(text: string) {
  const clean = text.toLowerCase().replace(/[\s\p{P}\p{S}\d]+/gu, " "), result: string[] = [];
  for (const block of clean.match(/[\u3400-\u9fff]{2,}/g) ?? []) for (let index = 0; index < block.length - 1; index += 1) { const word = block.slice(index, index + 2); if (!STOP.has(word)) result.push(word); }
  return result.concat(clean.match(/[a-z]{3,}/g) ?? []);
}

function vectors(books: Book[], notes: Note[]) {
  const evidence = new Map<string, string>();
  for (const note of notes) { const current = evidence.get(note.bookId) ?? ""; if (current.length < 16000) evidence.set(note.bookId, `${current} ${note.chapter} ${note.content}`.slice(0, 16000)); }
  const documents = books.map(book => terms(`${book.title} ${book.title} ${book.author} ${evidence.get(book.id) ?? ""}`));
  const frequency = new Map<string, number>(); documents.forEach(document => new Set(document).forEach(word => frequency.set(word, (frequency.get(word) ?? 0) + 1)));
  return documents.map(document => { const counts = new Map<string, number>(); document.forEach(word => counts.set(word, (counts.get(word) ?? 0) + 1)); return new Map([...counts].map(([word, count]) => [word, (1 + Math.log(count)) * Math.log(1 + books.length / (frequency.get(word) ?? 1))] as const).filter(([word]) => (frequency.get(word) ?? 1) < books.length * .55).sort((a, b) => b[1] - a[1]).slice(0, 100)); });
}

function position(books: Book[]) {
  const count = Math.max(1, books.length);
  return books.map((book, index): Node => { const angle = index * 2.399963, radius = 35 + Math.sqrt(index / count) * 270; return { ...book, x: 500 + Math.cos(angle) * radius * 1.55, y: 325 + Math.sin(angle) * radius }; });
}

function build(booksInput: Book[], notes: Note[], strength: Strength) {
  const limit = LIMITS[strength], sorted = [...booksInput].sort((a, b) => b.highlightCount + b.thoughtCount - a.highlightCount - a.thoughtCount);
  const books = Number.isFinite(limit.books) ? sorted.slice(0, limit.books) : sorted, data = vectors(books, notes), lengths = data.map(vector => Math.sqrt([...vector.values()].reduce((sum, value) => sum + value * value, 0)) || 1);
  const postings = new Map<string, { index: number; value: number }[]>(); data.forEach((vector, index) => vector.forEach((value, word) => postings.set(word, [...(postings.get(word) ?? []), { index, value }])));
  const pairs = new Map<string, { dot: number; words: { word: string; weight: number }[] }>();
  postings.forEach((items, word) => { if (items.length > 40) return; for (let a = 0; a < items.length; a += 1) for (let b = a + 1; b < items.length; b += 1) { const key = `${items[a].index}:${items[b].index}`, pair = pairs.get(key) ?? { dot: 0, words: [] }, weight = items[a].value * items[b].value; pair.dot += weight; pair.words.push({ word, weight }); pairs.set(key, pair); } });
  const candidates: Edge[] = [];
  pairs.forEach((pair, key) => { const [a, b] = key.split(":").map(Number), score = pair.dot / (lengths[a] * lengths[b]); if (score > .015 && pair.words.length >= 2) candidates.push({ id: `${books[a].id}:${books[b].id}`, from: books[a].id, to: books[b].id, score, keywords: pair.words.sort((x, y) => y.weight - x.weight).slice(0, 5).map(item => item.word) }); });
  candidates.sort((a, b) => b.score - a.score); const degree = new Map<string, number>();
  const connected = candidates.filter(edge => { if ((degree.get(edge.from) ?? 0) >= limit.neighbors && (degree.get(edge.to) ?? 0) >= limit.neighbors) return false; degree.set(edge.from, (degree.get(edge.from) ?? 0) + 1); degree.set(edge.to, (degree.get(edge.to) ?? 0) + 1); return true; });
  return { nodes: position(books), edges: Number.isFinite(limit.edges) ? connected.slice(0, limit.edges) : connected };
}

self.onmessage = (event: MessageEvent<{ books: Book[]; notes: Note[]; strength: Strength }>) => self.postMessage(build(event.data.books, event.data.notes, event.data.strength));
