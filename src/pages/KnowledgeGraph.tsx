import { BookOpen, Focus, Highlighter, Lightbulb, Search, Share2, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { api } from "../api/tauri";
import { PageHeader } from "../components/ui/PageHeader";
import { useAppStore } from "../stores/app";
import type { Book, Note, NoteType } from "../types/domain";

type GraphNode = {
  id: string;
  kind: "book" | NoteType;
  title: string;
  subtitle: string;
  x: number;
  y: number;
  bookId: string;
  note?: Note;
};

const WIDTH = 1000;
const HEIGHT = 650;

function shorten(value: string, size: number) {
  return value.length > size ? `${value.slice(0, size)}…` : value;
}

function buildGraph(books: Book[], notes: Note[], visibleTypes: Set<NoteType>) {
  const usefulBooks = books
    .filter(book => notes.some(note => note.bookId === book.id))
    .sort((a, b) => (b.highlightCount + b.thoughtCount) - (a.highlightCount + a.thoughtCount))
    .slice(0, 10);
  const nodes: GraphNode[] = [];
  const edges: { from: string; to: string }[] = [];
  const centerX = WIDTH / 2;
  const centerY = HEIGHT / 2;
  const ring = Math.min(265, 145 + usefulBooks.length * 13);

  usefulBooks.forEach((book, index) => {
    const angle = (Math.PI * 2 * index / usefulBooks.length) - Math.PI / 2;
    const bx = centerX + Math.cos(angle) * ring;
    const by = centerY + Math.sin(angle) * ring * .72;
    nodes.push({ id: `book:${book.id}`, kind: "book", title: book.title, subtitle: book.author, x: bx, y: by, bookId: book.id });
    const children = notes
      .filter(note => note.bookId === book.id && visibleTypes.has(note.type))
      .slice(0, 4);
    children.forEach((note, childIndex) => {
      const spread = (childIndex - (children.length - 1) / 2) * .32;
      const childAngle = angle + spread;
      const distance = 76 + (childIndex % 2) * 13;
      const id = `note:${note.id}`;
      nodes.push({ id, kind: note.type, title: note.content, subtitle: note.chapter, x: bx + Math.cos(childAngle) * distance, y: by + Math.sin(childAngle) * distance, bookId: book.id, note });
      edges.push({ from: `book:${book.id}`, to: id });
    });
  });
  return { nodes, edges };
}

export function KnowledgeGraph() {
  const [books, setBooks] = useState<Book[]>([]);
  const [notes, setNotes] = useState<Note[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [query, setQuery] = useState("");
  const [types, setTypes] = useState<Set<NoteType>>(new Set(["highlight", "thought"]));
  const [selectedId, setSelectedId] = useState<string>();
  const [focusedBookId, setFocusedBookId] = useState<string>();
  const openBook = useAppStore(state => state.openBook);

  useEffect(() => {
    Promise.all([api.books(), api.notes()])
      .then(([bookList, noteList]) => { setBooks(bookList); setNotes(noteList); })
      .catch(reason => setError(reason instanceof Error ? reason.message : String(reason)))
      .finally(() => setLoading(false));
  }, []);

  const graph = useMemo(() => buildGraph(books, notes, types), [books, notes, types]);
  const matchingIds = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    if (!normalized) return new Set(graph.nodes.map(node => node.id));
    return new Set(graph.nodes.filter(node => `${node.title}${node.subtitle}`.toLowerCase().includes(normalized)).map(node => node.id));
  }, [graph.nodes, query]);
  const selected = graph.nodes.find(node => node.id === selectedId);
  const visibleNodes = focusedBookId ? graph.nodes.filter(node => node.bookId === focusedBookId) : graph.nodes;
  const visibleIds = new Set(visibleNodes.map(node => node.id));
  const toggleType = (type: NoteType) => setTypes(current => {
    const next = new Set(current);
    if (next.has(type)) next.delete(type); else next.add(type);
    return next;
  });

  return <div className="knowledge-graph-page">
    <PageHeader title="知识图谱" subtitle="探索书籍、划线与想法之间的联系。" />
    <div className="graph-toolbar">
      <label className="graph-search"><Search size={16}/><input value={query} onChange={event => setQuery(event.target.value)} placeholder="搜索节点" /></label>
      <button className={types.has("highlight") ? "active" : ""} onClick={() => toggleType("highlight")}><Highlighter size={14}/>划线</button>
      <button className={types.has("thought") ? "active" : ""} onClick={() => toggleType("thought")}><Lightbulb size={14}/>想法</button>
      {focusedBookId && <button className="graph-reset" onClick={() => setFocusedBookId(undefined)}><X size={14}/>退出聚焦</button>}
      <span>{visibleNodes.length} 个节点</span>
    </div>
    <section className="graph-shell">
      {loading && <div className="graph-state">正在梳理知识脉络…</div>}
      {!loading && error && <div className="graph-state"><Share2/><strong>暂时无法生成图谱</strong><span>{error}</span></div>}
      {!loading && !error && graph.nodes.length === 0 && <div className="graph-state"><Share2/><strong>还没有可以连接的知识</strong><span>同步微信读书后，书籍、划线与想法会在这里形成关联。</span></div>}
      {!loading && !error && graph.nodes.length > 0 && <svg className="knowledge-graph" viewBox={`0 0 ${WIDTH} ${HEIGHT}`} role="img" aria-label="知识图谱">
        <g className="graph-edges">{graph.edges.filter(edge => visibleIds.has(edge.from) && visibleIds.has(edge.to)).map(edge => {
          const from = graph.nodes.find(node => node.id === edge.from)!;
          const to = graph.nodes.find(node => node.id === edge.to)!;
          return <line key={`${edge.from}-${edge.to}`} x1={from.x} y1={from.y} x2={to.x} y2={to.y}/>;
        })}</g>
        {visibleNodes.map(node => {
          const active = selectedId === node.id;
          const dimmed = !matchingIds.has(node.id);
          return <g key={node.id} className={`graph-node graph-node--${node.kind}${active ? " selected" : ""}${dimmed ? " dimmed" : ""}`} transform={`translate(${node.x} ${node.y})`} onClick={() => setSelectedId(node.id)} role="button" tabIndex={0} onKeyDown={event => { if (event.key === "Enter") setSelectedId(node.id); }}>
            <circle r={node.kind === "book" ? 29 : 14}/>
            {node.kind === "book" ? <BookOpen size={18} x={-9} y={-9}/> : node.kind === "highlight" ? <Highlighter size={11} x={-5.5} y={-5.5}/> : <Lightbulb size={11} x={-5.5} y={-5.5}/>}
            <text y={node.kind === "book" ? 44 : 28} textAnchor="middle">{shorten(node.title, node.kind === "book" ? 9 : 7)}</text>
          </g>;
        })}
      </svg>}
      {selected && <aside className="graph-detail">
        <button className="graph-detail__close" aria-label="关闭详情" onClick={() => setSelectedId(undefined)}><X size={16}/></button>
        <span className={`graph-kind graph-kind--${selected.kind}`}>{selected.kind === "book" ? "书籍" : selected.kind === "highlight" ? "划线" : "想法"}</span>
        <h2>{selected.kind === "book" ? selected.title : shorten(selected.title, 120)}</h2>
        <p>{selected.subtitle || "暂无章节信息"}</p>
        {selected.note && <small>来源：《{selected.note.bookTitle}》</small>}
        <div className="graph-detail__actions">
          {selected.kind === "book" && <button onClick={() => setFocusedBookId(selected.bookId)}><Focus size={14}/>聚焦关联</button>}
          <button onClick={() => openBook(selected.bookId, selected.note?.id)}><BookOpen size={14}/>查看原文</button>
        </div>
      </aside>}
      {!loading && !error && graph.nodes.length > 0 && <div className="graph-legend"><span><i className="book"/>书籍</span><span><i className="highlight"/>划线</span><span><i className="thought"/>想法</span></div>}
    </section>
  </div>;
}
