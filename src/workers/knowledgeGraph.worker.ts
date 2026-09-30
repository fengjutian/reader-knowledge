import type { Book, Note } from "../types/domain";

type Strength = "all" | "strong" | "balanced" | "broad";
type Node = Book & { x: number; y: number };
type Evidence = { bookId: string; text: string; noteId: string };
type Edge = { id: string; from: string; to: string; score: number; keywords: string[]; relation: string; evidence: Evidence[] };
type Analysis = { nodes: Node[]; candidates: Edge[] };
const LIMITS = { all: { books: Infinity, edges: Infinity, neighbors: 12 }, strong: { books: Infinity, edges: Infinity, neighbors: 5 }, balanced: { books: Infinity, edges: Infinity, neighbors: 10 }, broad: { books: Infinity, edges: Infinity, neighbors: 20 } };
const STOP = new Set([
  "我们","你们","他们","这个","那个","一个","什么","就是","因为","所以","但是","如果","可以","没有","不是","已经","自己","这种","这样","以及","对于","进行","需要","可能","时候","这里","其中","那些","这些","问题","认为","关系","之后","之前","只是","还是","很多","一些","一种","如何","为什么","其实","非常","现在","发现","开始","能够","通过","方式",
  "部分","一部","根据","有关","相关","方面","一定","不同","而且","并且","或者","应该","虽然","由于","因此","比如","例如","说明","表示","事实","情况","内容","作者","本书","书中","来说","关于","之间","主要","基本","一般","整个","所有","其他","同时","甚至","仍然","如此","当时","后来","最后","第一","第二",
]);
// Chinese bigram vectors are sparse, so meaningful relations can have a low
// cosine score. Noise is controlled by STOP terms and two-sided evidence,
// rather than by an aggressive score threshold.
const MIN_CONTENT_SCORE = .02;
let analysis: Analysis = { nodes: [], candidates: [] };

function words(text: string) {
  const clean = text.toLowerCase().replace(/[\s\p{P}\p{S}\d]+/gu, " "), result: string[] = [];
  for (const block of clean.match(/[\u3400-\u9fff]{2,}/g) ?? []) for (let index = 0; index < block.length - 1; index += 1) { const word = block.slice(index, index + 2); if (!STOP.has(word)) result.push(word); }
  return result.concat(clean.match(/[a-z]{3,}/g) ?? []);
}

function addTerms(counts: Map<string, number>, text: string, weight: number) {
  words(text).forEach(word => counts.set(word, (counts.get(word) ?? 0) + weight));
}

function noteMatches(notes: Note[], keywords: string[]) {
  return notes.map(note => {
    const terms = new Set(words(`${note.chapter} ${note.content.slice(0, 1200)}`));
    const matched = keywords.filter(word => terms.has(word));
    return { note, matched, score: matched.length + (note.type === "thought" ? .2 : 0) };
  }).filter(item => item.matched.length > 0).sort((a, b) => b.score - a.score);
}

function titleKey(title: string) {
  return title.toLowerCase()
    .replace(/[《》〈〉「」『』【】()（）\[\]\s·:：,，.。!！?？\-—_]/g, "")
    .replace(/第?[一二三四五六七八九十百千万\d]+[册卷部]?/g, "")
    .replace(/全[一二三四五六七八九十百千万\d]+册|全集|全本|珍藏版|典藏版|精装版|新版|修订版|插图版|青少版|少儿版|人民文学版|电子书/g, "");
}

function sameWork(left: Book, right: Book) {
  const a = titleKey(left.title), b = titleKey(right.title);
  return a.length >= 3 && b.length >= 3 && (a === b || (Math.min(a.length, b.length) >= 4 && (a.includes(b) || b.includes(a))));
}

function position(books: Book[]) {
  const count = Math.max(1, books.length);
  return books.map((book, index): Node => { const angle = index * 2.399963, radius = 35 + Math.sqrt(index / count) * 270; return { ...book, x: 500 + Math.cos(angle) * radius * 1.55, y: 325 + Math.sin(angle) * radius }; });
}

function analyze(booksInput: Book[], notes: Note[]) {
  const books = [...booksInput].sort((a, b) => b.highlightCount + b.thoughtCount - a.highlightCount - a.thoughtCount);
  const notesByBook = new Map<string, Note[]>();
  notes.forEach(note => { const list = notesByBook.get(note.bookId) ?? []; if (list.length < 250) list.push(note); notesByBook.set(note.bookId, list); });
  const counts = books.map(book => {
    const map = new Map<string, number>(); addTerms(map, book.title, 1.2); addTerms(map, book.author, 2.5); addTerms(map, book.category, 4);
    for (const note of notesByBook.get(book.id) ?? []) { addTerms(map, note.chapter, 1.5); addTerms(map, note.content.slice(0, 800), note.type === "thought" ? 2.2 : 1); }
    return map;
  });
  const frequency = new Map<string, number>(); counts.forEach(map => map.forEach((_, word) => frequency.set(word, (frequency.get(word) ?? 0) + 1)));
  const vectors = counts.map(map => new Map([...map].filter(([word]) => (frequency.get(word) ?? 1) < books.length * .45).map(([word, count]) => [word, (1 + Math.log(count)) * Math.log(1 + books.length / (frequency.get(word) ?? 1))] as const).sort((a, b) => b[1] - a[1]).slice(0, 120)));
  const lengths = vectors.map(vector => Math.sqrt([...vector.values()].reduce((sum, value) => sum + value * value, 0)) || 1);
  const postings = new Map<string, { index: number; value: number }[]>();
  vectors.forEach((vector, index) => vector.forEach((value, word) => { const list = postings.get(word) ?? []; list.push({ index, value }); postings.set(word, list); }));
  const pairs = new Map<string, { dot: number; shared: { word: string; weight: number }[] }>();
  postings.forEach((items, word) => { if (items.length > 50) return; for (let a = 0; a < items.length; a += 1) for (let b = a + 1; b < items.length; b += 1) { const key = `${items[a].index}:${items[b].index}`, pair = pairs.get(key) ?? { dot: 0, shared: [] }, weight = items[a].value * items[b].value; pair.dot += weight; pair.shared.push({ word, weight }); pairs.set(key, pair); } });
  const candidates: Edge[] = [];
  pairs.forEach((pair, key) => {
    const [a, b] = key.split(":").map(Number), lexicalScore = pair.dot / (lengths[a] * lengths[b]);
    if (lexicalScore < MIN_CONTENT_SCORE || pair.shared.length < 2 || sameWork(books[a], books[b])) return;
    const shared = pair.shared.sort((x, y) => y.weight - x.weight).slice(0, 16).map(item => item.word);
    const leftMatches = noteMatches(notesByBook.get(books[a].id) ?? [], shared);
    const rightMatches = noteMatches(notesByBook.get(books[b].id) ?? [], shared);
    const leftTerms = new Set(leftMatches.flatMap(item => item.matched));
    const rightTerms = new Set(rightMatches.flatMap(item => item.matched));
    const keywords = shared.filter(word => leftTerms.has(word) && rightTerms.has(word)).slice(0, 5);
    if (keywords.length < 2 || !leftMatches.length || !rightMatches.length) return;
    const relevantLeft = leftMatches.filter(item => item.matched.some(word => keywords.includes(word)));
    const relevantRight = rightMatches.filter(item => item.matched.some(word => keywords.includes(word)));
    const matchingNotes = relevantLeft.length + relevantRight.length;
    const chapters = new Set([...relevantLeft, ...relevantRight].map(item => `${item.note.bookId}:${item.note.chapter}`)).size;
    const keywordQuality = Math.min(1, keywords.length / 5);
    const evidenceCoverage = Math.min(1, matchingNotes / 6);
    const chapterCoverage = Math.min(1, chapters / 4);
    const score = lexicalScore * .55 + keywordQuality * .2 + evidenceCoverage * .15 + chapterCoverage * .1;
    const evidence = [
      ...relevantLeft.slice(0, 2).map(({ note }) => ({ bookId: books[a].id, noteId: note.id, text: note.content.slice(0, 150) })),
      ...relevantRight.slice(0, 2).map(({ note }) => ({ bookId: books[b].id, noteId: note.id, text: note.content.slice(0, 150) })),
    ];
    const relation = books[a].author && books[a].author === books[b].author ? "共同作者 · 微信读书书架" : score >= .28 ? "高度主题相似" : score >= .12 ? "主题相近" : "潜在关联";
    candidates.push({ id: `${books[a].id}:${books[b].id}`, from: books[a].id, to: books[b].id, score, keywords, relation, evidence });
  });
  candidates.sort((a, b) => b.score - a.score);
  return { nodes: position(books), candidates };
}

function filter(strength: Strength) {
  const limit = LIMITS[strength], nodes = Number.isFinite(limit.books) ? analysis.nodes.slice(0, limit.books) : analysis.nodes, ids = new Set(nodes.map(node => node.id)), degree = new Map<string, number>();
  const selected = analysis.candidates.filter(edge => ids.has(edge.from) && ids.has(edge.to)).filter(edge => { if ((degree.get(edge.from) ?? 0) >= limit.neighbors && (degree.get(edge.to) ?? 0) >= limit.neighbors) return false; degree.set(edge.from, (degree.get(edge.from) ?? 0) + 1); degree.set(edge.to, (degree.get(edge.to) ?? 0) + 1); return true; });
  return { nodes, edges: Number.isFinite(limit.edges) ? selected.slice(0, limit.edges) : selected };
}

self.onmessage = (event: MessageEvent<{ type: "init"; books: Book[]; notes: Note[]; strength: Strength; cached?: Analysis } | { type: "filter"; strength: Strength }>) => {
  if (event.data.type === "filter") { self.postMessage({ type: "result", graph: filter(event.data.strength) }); return; }
  if (event.data.cached) { analysis = event.data.cached; self.postMessage({ type: "result", graph: filter(event.data.strength), cached: true }); return; }
  const nodes = position([...event.data.books].sort((a, b) => b.highlightCount + b.thoughtCount - a.highlightCount - a.thoughtCount));
  self.postMessage({ type: "nodes", graph: { nodes, edges: [] } });
  analysis = analyze(event.data.books, event.data.notes);
  self.postMessage({ type: "result", graph: filter(event.data.strength), analysis });
};
