import { BookOpen, Focus, Link2, Search, Share2, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api/tauri";
import { PageHeader } from "../components/ui/PageHeader";
import { useAppStore } from "../stores/app";
import type { Book, Note } from "../types/domain";

type Node = Book & { x: number; y: number };
type Edge = { id: string; from: string; to: string; score: number; keywords: string[] };
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
  useEffect(() => { Promise.all([api.books(), api.notes()]).then(([b, n]) => { setBooks(b); setNotes(n); }).catch(reason => setError(reason instanceof Error ? reason.message : String(reason))).finally(() => setLoading(false)); }, []);
  useEffect(() => {
    if (loading || error) return;
    workerRef.current?.terminate();
    const worker = new Worker(new URL("../workers/knowledgeGraph.worker.ts", import.meta.url), { type: "module" });
    workerRef.current = worker; setAnalyzing(true);
    worker.onmessage = (event: MessageEvent<{ nodes: Node[]; edges: Edge[] }>) => { setGraph(event.data); setAnalyzing(false); };
    worker.onerror = event => { setError(event.message || "关系分析失败"); setAnalyzing(false); };
    worker.postMessage({ books, notes, strength });
    return () => worker.terminate();
  }, [books, notes, strength, loading, error]);
  const selected = graph.nodes.find(node => node.id === selectedId), selectedEdge = graph.edges.find(edge => edge.id === edgeId);
  const focus = useMemo(() => !focusId ? new Set(graph.nodes.map(node => node.id)) : new Set([focusId, ...graph.edges.filter(edge => edge.from === focusId || edge.to === focusId).flatMap(edge => [edge.from, edge.to])]), [focusId, graph]);
  const q = query.trim().toLowerCase();
  return <div className="knowledge-graph-page"><PageHeader title="书籍关系图谱" subtitle="从你的划线与想法中，发现书与书之间的共同主题。" />
    <div className="graph-toolbar"><label className="graph-search"><Search size={16}/><input value={query} onChange={e => setQuery(e.target.value)} placeholder="搜索书名或作者" /></label><label className="graph-strength"><Link2 size={14}/><span>关联范围</span><select value={strength} onChange={e => { setStrength(e.target.value as Strength); setFocusId(undefined); }}><option value="all">全部书籍</option><option value="strong">核心关联</option><option value="balanced">均衡</option><option value="broad">更多关联</option></select></label>{focusId && <button className="graph-reset" onClick={() => setFocusId(undefined)}><X size={14}/>查看全部</button>}<span>{graph.nodes.length} / {books.length} 本书 · {graph.edges.length} 条关系</span></div>
    <section className="graph-shell">{(loading || analyzing) && <div className="graph-state">正在后台分析书籍之间的联系…<span>你可以继续使用其他页面</span></div>}{!loading && error && <div className="graph-state"><Share2/><strong>暂时无法生成图谱</strong><span>{error}</span></div>}{!loading && !analyzing && !error && !graph.nodes.length && <div className="graph-state"><Share2/><strong>还没有发现可靠的书籍关系</strong><span>更多划线与想法会让关联分析更加准确。</span></div>}
      {!loading && !analyzing && !error && !!graph.nodes.length && <svg className="knowledge-graph" viewBox={`0 0 ${W} ${H}`} role="img" aria-label="书籍关系图谱"><g className="graph-edges">{graph.edges.map(edge => { const from = graph.nodes.find(n => n.id === edge.from)!, to = graph.nodes.find(n => n.id === edge.to)!; return <g key={edge.id} className={`graph-relation${edgeId === edge.id ? " selected" : ""}${!focus.has(edge.from) || !focus.has(edge.to) ? " dimmed" : ""}`} onClick={() => { setEdgeId(edge.id); setSelectedId(undefined); }}><line x1={from.x} y1={from.y} x2={to.x} y2={to.y} style={{ strokeWidth: 1.2 + edge.score * 8 }}/><line className="graph-relation__hit" x1={from.x} y1={from.y} x2={to.x} y2={to.y}/></g>; })}</g>{graph.nodes.map(node => <g key={node.id} className={`graph-node graph-node--book${selectedId === node.id ? " selected" : ""}${!focus.has(node.id) || (!!q && !`${node.title}${node.author}`.toLowerCase().includes(q)) ? " dimmed" : ""}`} transform={`translate(${node.x} ${node.y})`} onClick={() => { setSelectedId(node.id); setEdgeId(undefined); }} role="button" tabIndex={0}><circle r={24 + Math.min(9, Math.log2(node.highlightCount + node.thoughtCount + 1) * 1.8)}/><BookOpen size={18} x={-9} y={-9}/><text y="45" textAnchor="middle">{short(node.title, 10)}</text></g>)}</svg>}
      {selected && <aside className="graph-detail"><button className="graph-detail__close" onClick={() => setSelectedId(undefined)}><X size={16}/></button><span className="graph-kind">书籍</span><h2>{selected.title}</h2><p>{selected.author}</p><small>{selected.highlightCount} 条划线 · {selected.thoughtCount} 条想法 · {graph.edges.filter(edge => edge.from === selected.id || edge.to === selected.id).length} 本关联书籍</small><div className="graph-detail__actions"><button onClick={() => setFocusId(selected.id)}><Focus size={14}/>查看关系网</button><button onClick={() => openBook(selected.id)}><BookOpen size={14}/>打开书籍</button></div></aside>}
      {selectedEdge && (() => { const from = graph.nodes.find(n => n.id === selectedEdge.from)!, to = graph.nodes.find(n => n.id === selectedEdge.to)!; return <aside className="graph-detail graph-relation-detail"><button className="graph-detail__close" onClick={() => setEdgeId(undefined)}><X size={16}/></button><span className="graph-kind">关联依据</span><h2>《{from.title}》<br/>与《{to.title}》</h2><p>共同出现在两本书阅读记录中的主题词：</p><div className="relation-keywords">{selectedEdge.keywords.map(word => <span key={word}>{word}</span>)}</div><small>关联强度 {Math.round(selectedEdge.score * 100)}%</small></aside>; })()}
      {!!graph.nodes.length && <div className="graph-legend"><span><i className="book"/>圆越大，阅读记录越多</span><span><i className="relation"/>线越粗，内容关联越强</span></div>}
    </section></div>;
}
