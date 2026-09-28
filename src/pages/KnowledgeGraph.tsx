import { BookOpen, Focus, Link2, Search, Share2, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api/tauri";
import { PageHeader } from "../components/ui/PageHeader";
import { KnowledgeGraphCanvas } from "../components/graph/KnowledgeGraphCanvas";
import { useAppStore } from "../stores/app";
import { graphCacheKey, readGraphCache, writeGraphCache } from "../utils/graphCache";
import type { Book, Note } from "../types/domain";

type Node = Book & { x: number; y: number };
type Evidence = { bookId: string; text: string; noteId: string };
type Edge = { id: string; from: string; to: string; score: number; keywords: string[]; relation: string; evidence: Evidence[] };
type Analysis = { nodes: Node[]; candidates: Edge[] };
type Strength = "all" | "strong" | "balanced" | "broad";
const W = 1000, H = 650;
const short = (value: string, size: number) => value.length > size ? `${value.slice(0, size)}…` : value;

export function KnowledgeGraph() {
  const [books, setBooks] = useState<Book[]>([]), [notes, setNotes] = useState<Note[]>([]), [loading, setLoading] = useState(true);
  const [analyzing, setAnalyzing] = useState(false), [graph, setGraph] = useState<{ nodes: Node[]; edges: Edge[] }>({ nodes: [], edges: [] });
  const [error, setError] = useState(""), [query, setQuery] = useState(""), [strength, setStrength] = useState<Strength>("all");
  const [selectedId, setSelectedId] = useState<string>(), [edgeId, setEdgeId] = useState<string>(), [focusId, setFocusId] = useState<string>();
  const openBook = useAppStore(state => state.openBook);
  const workerRef = useRef<Worker | null>(null);
  const cacheKeyRef = useRef("");
  useEffect(() => { Promise.all([api.books(), api.notes()]).then(([b, n]) => { setBooks(b); setNotes(n); }).catch(reason => setError(reason instanceof Error ? reason.message : String(reason))).finally(() => setLoading(false)); }, []);
  useEffect(() => {
    const worker = new Worker(new URL("../workers/knowledgeGraph.worker.ts", import.meta.url), { type: "module" });
    workerRef.current = worker;
    worker.onmessage = (event: MessageEvent<{ type: "nodes" | "result"; graph: { nodes: Node[]; edges: Edge[] }; analysis?: Analysis }>) => {
      setGraph(event.data.graph);
      if (event.data.type === "result") {
        setAnalyzing(false);
        setFocusId(current => current ?? event.data.graph.edges[0]?.from ?? event.data.graph.nodes[0]?.id);
      }
      if (event.data.analysis && cacheKeyRef.current) void writeGraphCache(cacheKeyRef.current, event.data.analysis);
    };
    worker.onerror = event => { setError(event.message || "关系分析失败"); setAnalyzing(false); };
    return () => worker.terminate();
  }, []);
  useEffect(() => {
    if (loading || error || !workerRef.current) return;
    let cancelled = false;
    const key = graphCacheKey(books, notes.length); cacheKeyRef.current = key; setAnalyzing(true);
    void readGraphCache<Analysis>(key).then(cached => {
      if (!cancelled) workerRef.current?.postMessage({ type: "init", books, notes: notes.map(({ id, type, bookId, chapter, content, bookTitle, createdAt }) => ({ id, type, bookId, chapter, content, bookTitle, createdAt })), strength, cached });
    });
    return () => { cancelled = true; };
  }, [books, notes, loading, error]);
  useEffect(() => { if (!loading && books.length) workerRef.current?.postMessage({ type: "filter", strength }); }, [strength, loading, books.length]);
  const viewGraph = useMemo(() => {
    if (!focusId) return { nodes: [], edges: [] as Edge[] };
    const edges = graph.edges.filter(edge => edge.from === focusId || edge.to === focusId).sort((a, b) => b.score - a.score).slice(0, 20);
    const ids = new Set([focusId, ...edges.flatMap(edge => [edge.from, edge.to])]);
    const source = graph.nodes.filter(node => ids.has(node.id));
    const nodes = source.map(node => {
      if (node.id === focusId) return { ...node, x: W / 2, y: H / 2 };
      const index = source.filter(item => item.id !== focusId).findIndex(item => item.id === node.id), count = Math.max(1, source.length - 1), ring = count > 12 && index >= 10 ? 245 : 175, ringIndex = count > 12 && index >= 10 ? index - 10 : index, ringCount = count > 12 && index >= 10 ? count - 10 : Math.min(count, 10), angle = Math.PI * 2 * ringIndex / ringCount - Math.PI / 2;
      return { ...node, x: W / 2 + Math.cos(angle) * ring * 1.45, y: H / 2 + Math.sin(angle) * ring };
    });
    return { nodes, edges };
  }, [focusId, graph]);
  const selected = graph.nodes.find(node => node.id === selectedId), selectedEdge = graph.edges.find(edge => edge.id === edgeId);
  const q = query.trim().toLowerCase();
  const searchResults = q ? books.filter(book => `${book.title}${book.author}`.toLowerCase().includes(q)).slice(0, 8) : [];
  return <div className="knowledge-graph-page"><PageHeader title="书籍关系图谱" subtitle="从你的划线与想法中，发现书与书之间的共同主题。" />
    <div className="graph-toolbar"><div className="graph-search-wrap"><label className="graph-search"><Search size={16}/><input value={query} onChange={e => setQuery(e.target.value)} placeholder="选择一本中心书籍" /></label>{searchResults.length > 0 && <div className="graph-search-results">{searchResults.map(book => <button key={book.id} onClick={() => { setFocusId(book.id); setQuery(""); setSelectedId(book.id); }}><strong>{book.title}</strong><span>{book.author || book.category || "未知作者"}</span></button>)}</div>}</div><label className="graph-strength"><Link2 size={14}/><span>每本书显示</span><select value={strength} onChange={e => setStrength(e.target.value as Strength)}><option value="strong">5 个强关联</option><option value="balanced">10 个关联</option><option value="all">12 个关联</option><option value="broad">20 个关联</option></select></label><span>{viewGraph.nodes.length > 0 ? `${viewGraph.nodes.length - 1} 本关联书籍` : "请选择一本书"}</span></div>
    <section className="graph-shell">{(loading || (analyzing && !graph.nodes.length)) && <div className="graph-state">正在后台分析书籍之间的联系…<span>你可以继续使用其他页面</span></div>}{analyzing && !!graph.nodes.length && <div className="graph-analyzing">正在补充关系…</div>}{!loading && error && <div className="graph-state"><Share2/><strong>暂时无法生成图谱</strong><span>{error}</span></div>}{!loading && !analyzing && !error && !graph.nodes.length && <div className="graph-state"><Share2/><strong>还没有发现可靠的书籍关系</strong><span>更多划线与想法会让关联分析更加准确。</span></div>}
      {!loading && !error && !!viewGraph.nodes.length && <KnowledgeGraphCanvas nodes={viewGraph.nodes} edges={viewGraph.edges} focusId={focusId} selectedId={selectedId} selectedEdgeId={edgeId} onNodeClick={id => { setSelectedId(id); setEdgeId(undefined); }} onNodeOpen={id => { setFocusId(id); setSelectedId(id); setEdgeId(undefined); }} onEdgeClick={id => { setEdgeId(id); setSelectedId(undefined); }}/>}
      {selected && <aside className="graph-detail"><button className="graph-detail__close" onClick={() => setSelectedId(undefined)}><X size={16}/></button><span className="graph-kind">书籍</span><h2>{selected.title}</h2><p>{selected.author}</p><small>{selected.highlightCount} 条划线 · {selected.thoughtCount} 条想法 · {graph.edges.filter(edge => edge.from === selected.id || edge.to === selected.id).length} 本关联书籍</small><div className="graph-detail__actions"><button onClick={() => setFocusId(selected.id)}><Focus size={14}/>查看关系网</button><button onClick={() => openBook(selected.id)}><BookOpen size={14}/>打开书籍</button></div></aside>}
      {selectedEdge && (() => { const from = graph.nodes.find(n => n.id === selectedEdge.from)!, to = graph.nodes.find(n => n.id === selectedEdge.to)!; return <aside className="graph-detail graph-relation-detail"><button className="graph-detail__close" onClick={() => setEdgeId(undefined)}><X size={16}/></button><span className="graph-kind">{selectedEdge.relation}</span><h2>《{from.title}》<br/>与《{to.title}》</h2><p>共同主题：</p><div className="relation-keywords">{selectedEdge.keywords.map(word => <span key={word}>{word}</span>)}</div>{selectedEdge.evidence.map(item => <button className="relation-evidence" key={item.noteId} onClick={() => openBook(item.bookId, item.noteId)}>“{short(item.text, 72)}”</button>)}<small>关联强度 {Math.round(selectedEdge.score * 100)}%</small></aside>; })()}
      {!!viewGraph.nodes.length && <div className="graph-legend"><span><i className="book"/>中心为当前书籍，双击周边书继续探索</span><span><i className="relation"/>线越粗，内容关联越强</span></div>}
    </section></div>;
}
