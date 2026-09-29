import { BookOpen, ChevronDown, Focus, Link2, RotateCw, Search, Share2, Sparkles, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api/tauri";
import { PageHeader } from "../components/ui/PageHeader";
import { KnowledgeGraphCanvas } from "../components/graph/KnowledgeGraphCanvas";
import { useAppStore } from "../stores/app";
import { graphCacheKey, readGraphCache, writeGraphCache } from "../utils/graphCache";
import type { Book, Note, RelationAnalysis, RelationKind } from "../types/domain";

type Node = Book & { x: number; y: number };
type Evidence = { bookId: string; text: string; noteId: string };
type Edge = { id: string; from: string; to: string; score: number; keywords: string[]; relation: string; evidence: Evidence[] };
type Analysis = { nodes: Node[]; candidates: Edge[] };
type Strength = "all" | "strong" | "balanced" | "broad";
const W = 1000, H = 650;
const short = (value: string, size: number) => value.length > size ? `${value.slice(0, size)}…` : value;
const relationLabels: Record<RelationKind, string> = { same_concept: "同义概念", agreement: "观点一致", conflict: "观点冲突", complementary: "观点互补", causal: "因果关系", application: "理论与应用", uncertain: "证据不足" };

export function KnowledgeGraph() {
  const [books, setBooks] = useState<Book[]>([]), [notes, setNotes] = useState<Note[]>([]), [loading, setLoading] = useState(true);
  const [analyzing, setAnalyzing] = useState(false), [graph, setGraph] = useState<{ nodes: Node[]; edges: Edge[] }>({ nodes: [], edges: [] });
  const [error, setError] = useState(""), [query, setQuery] = useState(""), [strength, setStrength] = useState<Strength>("all");
  const [pickerOpen, setPickerOpen] = useState(false), [pickerScroll, setPickerScroll] = useState(0);
  const [selectedId, setSelectedId] = useState<string>(), [edgeId, setEdgeId] = useState<string>(), [focusId, setFocusId] = useState<string>();
  const [deepAnalysis, setDeepAnalysis] = useState<RelationAnalysis>(), [deepLoading, setDeepLoading] = useState(false), [deepError, setDeepError] = useState("");
  const openBook = useAppStore(state => state.openBook);
  const workerRef = useRef<Worker | null>(null);
  const cacheKeyRef = useRef("");
  const pickerRef = useRef<HTMLDivElement>(null);
  useEffect(() => { Promise.all([api.books(), api.notes()]).then(([b, n]) => { setBooks(b); setNotes(n); }).catch(reason => setError(reason instanceof Error ? reason.message : String(reason))).finally(() => setLoading(false)); }, []);
  useEffect(() => {
    const worker = new Worker(new URL("../workers/knowledgeGraph.worker.ts", import.meta.url), { type: "module" });
    workerRef.current = worker;
    worker.onmessage = (event: MessageEvent<{ type: "nodes" | "result"; graph: { nodes: Node[]; edges: Edge[] }; analysis?: Analysis }>) => {
      setGraph(event.data.graph);
      if (event.data.type === "result") {
        setAnalyzing(false);
        setFocusId(current => {
          if (current) return current;
          const connected = new Set(event.data.graph.edges.flatMap(edge => [edge.from, edge.to]));
          return [...event.data.graph.nodes].filter(node => connected.has(node.id)).sort((a, b) => (b.highlightCount + b.thoughtCount) - (a.highlightCount + a.thoughtCount))[0]?.id ?? event.data.graph.nodes[0]?.id;
        });
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
  useEffect(() => { const close = (event: MouseEvent) => { if (!pickerRef.current?.contains(event.target as globalThis.Node)) setPickerOpen(false); }; document.addEventListener("mousedown", close); return () => document.removeEventListener("mousedown", close); }, []);
  useEffect(() => { setDeepAnalysis(undefined); setDeepError(""); setDeepLoading(false); }, [edgeId]);
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
  async function analyzeRelation(refresh = false) {
    if (!selectedEdge || deepLoading) return;
    setDeepLoading(true); setDeepError("");
    try { setDeepAnalysis(await api.analyzeRelation(selectedEdge.from, selectedEdge.to, selectedEdge.keywords, refresh)); }
    catch (reason) { setDeepError(reason instanceof Error ? reason.message : String(reason)); }
    finally { setDeepLoading(false); }
  }
  const q = query.trim().toLowerCase();
  const pickerBooks = useMemo(() => q ? books.filter(book => `${book.title}${book.author}${book.category}`.toLowerCase().includes(q)) : books, [books, q]);
  const pickerStart = Math.max(0, Math.floor(pickerScroll / 46) - 2), pickerItems = pickerBooks.slice(pickerStart, pickerStart + 12);
  return <div className="knowledge-graph-page"><PageHeader title="书籍关系图谱" subtitle="从你的划线与想法中，发现书与书之间的共同主题。" />
    <div className="graph-toolbar"><div className="graph-search-wrap" ref={pickerRef}><label className="graph-search"><Search size={16}/><input value={query} onFocus={() => setPickerOpen(true)} onChange={e => { setQuery(e.target.value); setPickerOpen(true); setPickerScroll(0); }} onKeyDown={e => { if (e.key === "Escape") setPickerOpen(false); }} placeholder="选择一本中心书籍" /><button type="button" aria-label="展开全部书籍" onClick={() => setPickerOpen(open => !open)}><ChevronDown size={15}/></button></label>{pickerOpen && <div className="graph-book-picker"><div className="graph-book-picker__count">{q ? `找到 ${pickerBooks.length} 本` : `全部 ${pickerBooks.length} 本书`}</div><div className="graph-book-picker__scroll" onScroll={event => setPickerScroll(event.currentTarget.scrollTop)}><div style={{ height: pickerBooks.length * 46 }}>{pickerItems.map((book, index) => <button style={{ transform: `translateY(${(pickerStart + index) * 46}px)` }} key={book.id} onClick={() => { setFocusId(book.id); setQuery(""); setSelectedId(book.id); setPickerOpen(false); }}><strong>{book.title}</strong><span>{book.author || book.category || "未知作者"}</span></button>)}</div></div></div>}</div><label className="graph-strength"><Link2 size={14}/><span>每本书显示</span><select value={strength} onChange={e => setStrength(e.target.value as Strength)}><option value="strong">5 个强关联</option><option value="balanced">10 个关联</option><option value="all">12 个关联</option><option value="broad">20 个关联</option></select></label><span>{viewGraph.nodes.length > 0 ? `${viewGraph.nodes.length - 1} 本关联书籍` : "请选择一本书"}</span></div>
    <section className="graph-shell">{(loading || (analyzing && !graph.nodes.length)) && <div className="graph-state">正在后台分析书籍之间的联系…<span>你可以继续使用其他页面</span></div>}{analyzing && !!graph.nodes.length && <div className="graph-analyzing">正在补充关系…</div>}{!loading && error && <div className="graph-state"><Share2/><strong>暂时无法生成图谱</strong><span>{error}</span></div>}{!loading && !analyzing && !error && !graph.nodes.length && <div className="graph-state"><Share2/><strong>还没有发现可靠的书籍关系</strong><span>更多划线与想法会让关联分析更加准确。</span></div>}
      {!loading && !error && !!viewGraph.nodes.length && <KnowledgeGraphCanvas nodes={viewGraph.nodes} edges={viewGraph.edges} focusId={focusId} selectedId={selectedId} selectedEdgeId={edgeId} onNodeClick={id => { setSelectedId(id); setEdgeId(undefined); }} onNodeOpen={id => { setFocusId(id); setSelectedId(id); setEdgeId(undefined); }} onEdgeClick={id => { setEdgeId(id); setSelectedId(undefined); }}/>}
      {selected && <aside className="graph-detail"><button className="graph-detail__close" onClick={() => setSelectedId(undefined)}><X size={16}/></button><span className="graph-kind">书籍</span><h2>{selected.title}</h2><p>{selected.author}</p><small>{selected.highlightCount} 条划线 · {selected.thoughtCount} 条想法 · {graph.edges.filter(edge => edge.from === selected.id || edge.to === selected.id).length} 本关联书籍</small><div className="graph-detail__actions"><button onClick={() => setFocusId(selected.id)}><Focus size={14}/>查看关系网</button><button onClick={() => openBook(selected.id)}><BookOpen size={14}/>打开书籍</button></div></aside>}
      {selectedEdge && (() => { const from = graph.nodes.find(n => n.id === selectedEdge.from)!, to = graph.nodes.find(n => n.id === selectedEdge.to)!; return <aside className="graph-detail graph-relation-detail"><button className="graph-detail__close" onClick={() => setEdgeId(undefined)}><X size={16}/></button><span className="graph-kind">{deepAnalysis ? relationLabels[deepAnalysis.relation] : selectedEdge.relation}</span><h2>《{from.title}》<br/>与《{to.title}》</h2><p>共同主题：</p><div className="relation-keywords">{(deepAnalysis?.concepts.length ? deepAnalysis.concepts : selectedEdge.keywords).map(word => <span key={word}>{word}</span>)}</div>{!deepAnalysis && selectedEdge.evidence.map(item => <button className="relation-evidence" key={item.noteId} onClick={() => openBook(item.bookId, item.noteId)}>“{short(item.text, 72)}”</button>)}{deepAnalysis && <div className="deep-relation"><p>{deepAnalysis.summary}</p>{deepAnalysis.claims.map((claim, index) => <section key={`${claim.relation}-${index}`}><strong>{relationLabels[claim.relation]} · {Math.round(claim.confidence * 100)}%</strong><p>{claim.summary}</p>{claim.evidence.map(item => <button className="relation-evidence" key={item.noteId} onClick={() => openBook(item.bookId, item.noteId)}><i>{item.noteType === "thought" ? "我的想法" : "原文划线"}</i>“{short(item.text, 90)}”</button>)}</section>)}</div>}<small>{deepAnalysis ? `AI 判断置信度 ${Math.round(deepAnalysis.confidence * 100)}%${deepAnalysis.cached ? " · 已缓存" : ""}` : `词面关联强度 ${Math.round(selectedEdge.score * 100)}%`}</small>{deepError && <p className="relation-analysis-error">{deepError}</p>}<div className="relation-analysis-actions"><button onClick={() => void analyzeRelation(!!deepAnalysis)} disabled={deepLoading}>{deepAnalysis ? <RotateCw size={14}/> : <Sparkles size={14}/>} {deepLoading ? "正在分析…" : deepAnalysis ? "重新分析" : "AI 深度分析"}</button></div></aside>; })()}
      {!!viewGraph.nodes.length && <div className="graph-legend"><span><i className="book"/>中心为当前书籍，双击周边书继续探索</span><span><i className="relation"/>线越粗，内容关联越强</span></div>}
    </section></div>;
}
