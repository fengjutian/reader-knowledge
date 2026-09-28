import { BookOpen, Focus, Link2, Search, Share2, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { api } from "../api/tauri";
import { PageHeader } from "../components/ui/PageHeader";
import { useAppStore } from "../stores/app";
import type { Book, Note } from "../types/domain";

type Node = Book & { x: number; y: number };
type Edge = { id: string; from: string; to: string; score: number; keywords: string[] };
type Strength = "all" | "strong" | "balanced" | "broad";
const W = 1000, H = 650;
const LIMITS = { all: { books: Infinity, edges: Infinity, neighbors: 3 }, strong: { books: 20, edges: 20, neighbors: 1 }, balanced: { books: 28, edges: 38, neighbors: 2 }, broad: { books: 36, edges: 60, neighbors: 3 } };
const STOP = new Set(["我们","你们","他们","这个","那个","一个","什么","就是","因为","所以","但是","如果","可以","没有","不是","已经","自己","这种","这样","以及","对于","进行","需要","可能","时候","这里","其中","那些","这些"]);
const short = (value: string, size: number) => value.length > size ? `${value.slice(0, size)}…` : value;

function terms(text: string) {
  const clean = text.toLowerCase().replace(/[\s\p{P}\p{S}\d]+/gu, " ");
  const result: string[] = [];
  for (const block of clean.match(/[\u3400-\u9fff]{2,}/g) ?? []) for (let i = 0; i < block.length - 1; i += 1) {
    const word = block.slice(i, i + 2); if (!STOP.has(word)) result.push(word);
  }
  return result.concat(clean.match(/[a-z]{3,}/g) ?? []);
}

function vectors(books: Book[], notes: Note[]) {
  const documents = books.map(book => terms(`${book.title} ${book.title} ${book.author} ${notes.filter(note => note.bookId === book.id).map(note => `${note.chapter} ${note.content}`).join(" ").slice(0, 30000)}`));
  const frequency = new Map<string, number>();
  documents.forEach(document => new Set(document).forEach(word => frequency.set(word, (frequency.get(word) ?? 0) + 1)));
  return documents.map(document => {
    const counts = new Map<string, number>(); document.forEach(word => counts.set(word, (counts.get(word) ?? 0) + 1));
    const vector = new Map<string, number>(); counts.forEach((count, word) => { const f = frequency.get(word) ?? 1; if (f < books.length * .55) vector.set(word, (1 + Math.log(count)) * Math.log(1 + books.length / f)); });
    return vector;
  });
}

function compare(a: Map<string, number>, b: Map<string, number>) {
  let dot = 0, al = 0, bl = 0; a.forEach(v => { al += v * v; }); b.forEach(v => { bl += v * v; });
  const shared = [...a].filter(([word]) => b.has(word)).map(([word, value]) => ({ word, weight: value * (b.get(word) ?? 0) })).sort((x, y) => y.weight - x.weight);
  shared.forEach(item => { dot += item.weight; });
  return { score: dot / (Math.sqrt(al * bl) || 1), keywords: shared.slice(0, 5).map(item => item.word) };
}

function position(nodes: Node[], edges: Edge[]) {
  const points = nodes.map((_, i) => ({ x: W / 2 + Math.cos(i * 2.4) * (80 + 7 * i), y: H / 2 + Math.sin(i * 2.4) * (60 + 5 * i) }));
  const steps = nodes.length > 80 ? 55 : nodes.length > 45 ? 90 : 150;
  for (let step = 0; step < steps; step += 1) {
    const forces = points.map(() => ({ x: 0, y: 0 }));
    for (let a = 0; a < points.length; a += 1) for (let b = a + 1; b < points.length; b += 1) {
      const dx = points[a].x - points[b].x, dy = points[a].y - points[b].y, d = Math.max(24, Math.hypot(dx, dy)), push = 1100 / (d * d);
      forces[a].x += dx / d * push; forces[a].y += dy / d * push; forces[b].x -= dx / d * push; forces[b].y -= dy / d * push;
    }
    edges.forEach(edge => { const a = nodes.findIndex(n => n.id === edge.from), b = nodes.findIndex(n => n.id === edge.to); if (a < 0 || b < 0) return; const dx = points[b].x - points[a].x, dy = points[b].y - points[a].y, pull = (Math.hypot(dx, dy) - 135) * .0018; forces[a].x += dx * pull; forces[a].y += dy * pull; forces[b].x -= dx * pull; forces[b].y -= dy * pull; });
    points.forEach((p, i) => { forces[i].x += (W / 2 - p.x) * .0008; forces[i].y += (H / 2 - p.y) * .0008; p.x = Math.max(55, Math.min(W - 55, p.x + forces[i].x * 5)); p.y = Math.max(55, Math.min(H - 65, p.y + forces[i].y * 5)); });
  }
  return nodes.map((node, i) => ({ ...node, ...points[i] }));
}

function buildGraph(allBooks: Book[], notes: Note[], strength: Strength) {
  const limit = LIMITS[strength];
  const sortedBooks = [...allBooks].sort((a, b) => b.highlightCount + b.thoughtCount - a.highlightCount - a.thoughtCount);
  const books = Number.isFinite(limit.books) ? sortedBooks.slice(0, limit.books) : sortedBooks;
  const bookVectors = vectors(books, notes), candidates: Edge[] = [];
  books.forEach((book, a) => books.slice(a + 1).forEach((other, offset) => { const relation = compare(bookVectors[a], bookVectors[a + offset + 1]); if (relation.keywords.length >= 2 && relation.score > .015) candidates.push({ id: `${book.id}:${other.id}`, from: book.id, to: other.id, ...relation }); }));
  candidates.sort((a, b) => b.score - a.score); const degree = new Map<string, number>();
  const connected = candidates.filter(edge => { if ((degree.get(edge.from) ?? 0) >= limit.neighbors && (degree.get(edge.to) ?? 0) >= limit.neighbors) return false; degree.set(edge.from, (degree.get(edge.from) ?? 0) + 1); degree.set(edge.to, (degree.get(edge.to) ?? 0) + 1); return true; });
  const edges = Number.isFinite(limit.edges) ? connected.slice(0, limit.edges) : connected;
  return { nodes: position(books.map(book => ({ ...book, x: 0, y: 0 })), edges), edges };
}

export function KnowledgeGraph() {
  const [books, setBooks] = useState<Book[]>([]), [notes, setNotes] = useState<Note[]>([]), [loading, setLoading] = useState(true);
  const [error, setError] = useState(""), [query, setQuery] = useState(""), [strength, setStrength] = useState<Strength>("all");
  const [selectedId, setSelectedId] = useState<string>(), [edgeId, setEdgeId] = useState<string>(), [focusId, setFocusId] = useState<string>();
  const openBook = useAppStore(state => state.openBook);
  useEffect(() => { Promise.all([api.books(), api.notes()]).then(([b, n]) => { setBooks(b); setNotes(n); }).catch(reason => setError(reason instanceof Error ? reason.message : String(reason))).finally(() => setLoading(false)); }, []);
  const graph = useMemo(() => buildGraph(books, notes, strength), [books, notes, strength]);
  const selected = graph.nodes.find(node => node.id === selectedId), selectedEdge = graph.edges.find(edge => edge.id === edgeId);
  const focus = useMemo(() => !focusId ? new Set(graph.nodes.map(node => node.id)) : new Set([focusId, ...graph.edges.filter(edge => edge.from === focusId || edge.to === focusId).flatMap(edge => [edge.from, edge.to])]), [focusId, graph]);
  const q = query.trim().toLowerCase();
  return <div className="knowledge-graph-page"><PageHeader title="书籍关系图谱" subtitle="从你的划线与想法中，发现书与书之间的共同主题。" />
    <div className="graph-toolbar"><label className="graph-search"><Search size={16}/><input value={query} onChange={e => setQuery(e.target.value)} placeholder="搜索书名或作者" /></label><label className="graph-strength"><Link2 size={14}/><span>关联范围</span><select value={strength} onChange={e => { setStrength(e.target.value as Strength); setFocusId(undefined); }}><option value="all">全部书籍</option><option value="strong">核心关联</option><option value="balanced">均衡</option><option value="broad">更多关联</option></select></label>{focusId && <button className="graph-reset" onClick={() => setFocusId(undefined)}><X size={14}/>查看全部</button>}<span>{graph.nodes.length} / {books.length} 本书 · {graph.edges.length} 条关系</span></div>
    <section className="graph-shell">{loading && <div className="graph-state">正在分析书籍之间的联系…</div>}{!loading && error && <div className="graph-state"><Share2/><strong>暂时无法生成图谱</strong><span>{error}</span></div>}{!loading && !error && !graph.nodes.length && <div className="graph-state"><Share2/><strong>还没有发现可靠的书籍关系</strong><span>更多划线与想法会让关联分析更加准确。</span></div>}
      {!loading && !error && !!graph.nodes.length && <svg className="knowledge-graph" viewBox={`0 0 ${W} ${H}`} role="img" aria-label="书籍关系图谱"><g className="graph-edges">{graph.edges.map(edge => { const from = graph.nodes.find(n => n.id === edge.from)!, to = graph.nodes.find(n => n.id === edge.to)!; return <g key={edge.id} className={`graph-relation${edgeId === edge.id ? " selected" : ""}${!focus.has(edge.from) || !focus.has(edge.to) ? " dimmed" : ""}`} onClick={() => { setEdgeId(edge.id); setSelectedId(undefined); }}><line x1={from.x} y1={from.y} x2={to.x} y2={to.y} style={{ strokeWidth: 1.2 + edge.score * 8 }}/><line className="graph-relation__hit" x1={from.x} y1={from.y} x2={to.x} y2={to.y}/></g>; })}</g>{graph.nodes.map(node => <g key={node.id} className={`graph-node graph-node--book${selectedId === node.id ? " selected" : ""}${!focus.has(node.id) || (!!q && !`${node.title}${node.author}`.toLowerCase().includes(q)) ? " dimmed" : ""}`} transform={`translate(${node.x} ${node.y})`} onClick={() => { setSelectedId(node.id); setEdgeId(undefined); }} role="button" tabIndex={0}><circle r={24 + Math.min(9, Math.log2(node.highlightCount + node.thoughtCount + 1) * 1.8)}/><BookOpen size={18} x={-9} y={-9}/><text y="45" textAnchor="middle">{short(node.title, 10)}</text></g>)}</svg>}
      {selected && <aside className="graph-detail"><button className="graph-detail__close" onClick={() => setSelectedId(undefined)}><X size={16}/></button><span className="graph-kind">书籍</span><h2>{selected.title}</h2><p>{selected.author}</p><small>{selected.highlightCount} 条划线 · {selected.thoughtCount} 条想法 · {graph.edges.filter(edge => edge.from === selected.id || edge.to === selected.id).length} 本关联书籍</small><div className="graph-detail__actions"><button onClick={() => setFocusId(selected.id)}><Focus size={14}/>查看关系网</button><button onClick={() => openBook(selected.id)}><BookOpen size={14}/>打开书籍</button></div></aside>}
      {selectedEdge && (() => { const from = graph.nodes.find(n => n.id === selectedEdge.from)!, to = graph.nodes.find(n => n.id === selectedEdge.to)!; return <aside className="graph-detail graph-relation-detail"><button className="graph-detail__close" onClick={() => setEdgeId(undefined)}><X size={16}/></button><span className="graph-kind">关联依据</span><h2>《{from.title}》<br/>与《{to.title}》</h2><p>共同出现在两本书阅读记录中的主题词：</p><div className="relation-keywords">{selectedEdge.keywords.map(word => <span key={word}>{word}</span>)}</div><small>关联强度 {Math.round(selectedEdge.score * 100)}%</small></aside>; })()}
      {!!graph.nodes.length && <div className="graph-legend"><span><i className="book"/>圆越大，阅读记录越多</span><span><i className="relation"/>线越粗，内容关联越强</span></div>}
    </section></div>;
}
