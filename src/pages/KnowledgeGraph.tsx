import { ArrowLeft, ArrowRight, BookOpen, Check, ChevronDown, Focus, GitCompareArrows, Globe2, Link2, Maximize2, RotateCw, Search, Share2, Sparkles, Trash2, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api/tauri";
import { KnowledgeGraphCanvas } from "../components/graph/KnowledgeGraphCanvas";
import { KnowledgeGraph3D } from "../components/graph/KnowledgeGraph3D";
import { useAppStore } from "../stores/app";
import { graphCacheKey, readGraphCache, writeGraphCache } from "../utils/graphCache";
import type { Book, Note, RelationAnalysis, RelationKind } from "../types/domain";

type Node = Book & { x: number; y: number };
type Evidence = { bookId: string; text: string; noteId: string };
type Edge = { id: string; from: string; to: string; score: number; keywords: string[]; relation: string; evidence: Evidence[] };
type Analysis = { nodes: Node[]; candidates: Edge[] };
type Strength = "all" | "strong" | "balanced" | "standard" | "broad";
type WorkerStrength = Exclude<Strength, "standard">;
type RelationFilter = "all" | "content" | "author" | "analyzable";
const W = 1000, H = 650;
const SEMANTIC_RELATIONS_CACHE_KEY = "semantic-relations:v1";
const short = (value: string, size: number) => value.length > size ? `${value.slice(0, size)}…` : value;
const relationLabels: Record<RelationKind, string> = { same_concept: "同义概念", agreement: "观点一致", conflict: "观点冲突", complementary: "观点互补", causal: "因果关系", application: "理论与应用", uncertain: "证据不足" };
const workerStrength = (strength: Strength): WorkerStrength => strength === "standard" ? "all" : strength;

export function KnowledgeGraph() {
  const [books, setBooks] = useState<Book[]>([]), [notes, setNotes] = useState<Note[]>([]), [loading, setLoading] = useState(true);
  const [analyzing, setAnalyzing] = useState(false), [graph, setGraph] = useState<{ nodes: Node[]; edges: Edge[] }>({ nodes: [], edges: [] });
  const [semanticLoading, setSemanticLoading] = useState(false), [semanticEnabled, setSemanticEnabled] = useState(false);
  const [semanticError, setSemanticError] = useState("");
  const [error, setError] = useState(""), [query, setQuery] = useState(""), [strength, setStrength] = useState<Strength>("all");
  const [relationFilter, setRelationFilter] = useState<RelationFilter>("all");
  const [connectedOnly, setConnectedOnly] = useState(true);
  const [pickerOpen, setPickerOpen] = useState(false), [pickerScroll, setPickerScroll] = useState(0);
  const [selectedId, setSelectedId] = useState<string>(), [edgeId, setEdgeId] = useState<string>(), [focusId, setFocusId] = useState<string>();
  const [backStack, setBackStack] = useState<string[]>([]), [forwardStack, setForwardStack] = useState<string[]>([]), [fitRequest, setFitRequest] = useState(0);
  const [deepAnalysis, setDeepAnalysis] = useState<RelationAnalysis>(), [deepLoading, setDeepLoading] = useState(false), [deepError, setDeepError] = useState("");
  const [relationAnalyses, setRelationAnalyses] = useState<Record<string, RelationAnalysis>>({});
  const [compareIds, setCompareIds] = useState<string[]>([]);
  const openBook = useAppStore(state => state.openBook);
  const openAiCompare = useAppStore(state => state.openAiCompare);
  const isActivePage = useAppStore(state => state.page === "graph");
  const workerRef = useRef<Worker | null>(null);
  const semanticEdgesRef = useRef<Edge[] | null>(null);
  const cacheKeyRef = useRef("");
  const pickerRef = useRef<HTMLDivElement>(null);
  const wasActiveRef = useRef(false);
  useEffect(() => {
    if (isActivePage && !wasActiveRef.current) {
      setSelectedId(undefined);
      setEdgeId(undefined);
      setPickerOpen(false);
    }
    wasActiveRef.current = isActivePage;
  }, [isActivePage]);
  function mergeExternalEdges(edges: Edge[]) {
    const merged = new Map((semanticEdgesRef.current ?? []).map(edge => [edge.id, edge]));
    edges.forEach(edge => merged.set(edge.id, edge));
    const values = [...merged.values()];
    semanticEdgesRef.current = values.length ? values : null;
    setGraph(current => ({ ...current, edges: values.length ? values : current.edges }));
  }
  function refreshSemanticRelations(showError = false) { setSemanticLoading(true); setSemanticError(""); Promise.all([api.semanticRelations(), api.metadataRelations("weread").catch(() => []), api.metadataRelations("douban").catch(() => [])]).then(([semantic, weread, douban]) => { semanticEdgesRef.current = null; mergeExternalEdges([...semantic, ...weread, ...douban]); setSemanticEnabled(true); void writeGraphCache(SEMANTIC_RELATIONS_CACHE_KEY, semantic); }).catch(reason => { if (showError) setSemanticError(reason instanceof Error ? reason.message : String(reason)); }).finally(() => setSemanticLoading(false)); }
  useEffect(() => {
    Promise.all([api.books(), api.notes(), readGraphCache<Edge[]>(SEMANTIC_RELATIONS_CACHE_KEY)]).then(([b, n, cachedEdges]) => { if (cachedEdges?.length) { mergeExternalEdges(cachedEdges); setSemanticEnabled(true); } setBooks(b); setNotes(n); }).catch(reason => setError(reason instanceof Error ? reason.message : String(reason))).finally(() => setLoading(false));
    Promise.all([api.metadataRelations("weread").catch(() => []), api.metadataRelations("douban").catch(() => [])]).then(([weread, douban]) => mergeExternalEdges([...weread, ...douban]));
  }, []);
  useEffect(() => {
    if (books.length) setFocusId(current => current ?? books[0].id);
  }, [books]);
  useEffect(() => {
    if (!graph.nodes.length || !graph.edges.length) return;
    const connected = new Set(graph.edges.flatMap(edge => [edge.from, edge.to]));
    setFocusId(current => {
      if (current && connected.has(current) && graph.nodes.some(node => node.id === current)) return current;
      return [...graph.nodes]
        .filter(node => connected.has(node.id))
        .sort((left, right) => (right.highlightCount + right.thoughtCount) - (left.highlightCount + left.thoughtCount))[0]?.id ?? current;
    });
  }, [graph.nodes, graph.edges]);
  useEffect(() => { const removed = () => { semanticEdgesRef.current = null; setSemanticEnabled(false); workerRef.current?.postMessage({ type: "filter", strength: workerStrength(strength) }); }; window.addEventListener("local-embedding-removed", removed); return () => window.removeEventListener("local-embedding-removed", removed); }, [strength]);
  useEffect(() => {
    const worker = new Worker(new URL("../workers/knowledgeGraph.worker.ts", import.meta.url), { type: "module" });
    workerRef.current = worker;
    worker.onmessage = (event: MessageEvent<{ type: "nodes" | "result"; graph: { nodes: Node[]; edges: Edge[] }; analysis?: Analysis }>) => {
      setGraph(semanticEdgesRef.current ? { ...event.data.graph, edges: semanticEdgesRef.current } : event.data.graph);
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
      const usableCached = cached?.nodes?.length ? cached : undefined;
      if (!cancelled) workerRef.current?.postMessage({ type: "init", books, notes: notes.map(({ id, type, bookId, chapter, content, bookTitle, createdAt }) => ({ id, type, bookId, chapter, content, bookTitle, createdAt })), strength: workerStrength(strength), cached: usableCached });
    });
    return () => { cancelled = true; };
  }, [books, notes, loading, error]);
  useEffect(() => { if (!loading && books.length) workerRef.current?.postMessage({ type: "filter", strength: workerStrength(strength) }); }, [strength, loading, books.length]);
  useEffect(() => { const close = (event: MouseEvent) => { if (!pickerRef.current?.contains(event.target as globalThis.Node)) setPickerOpen(false); }; document.addEventListener("mousedown", close); return () => document.removeEventListener("mousedown", close); }, []);
  useEffect(() => { const shortcuts = (event: KeyboardEvent) => { const target = event.target as HTMLElement; if (target.matches("input,select,textarea")) return; if (event.key === "Escape") { setSelectedId(undefined); setEdgeId(undefined); } if (event.key.toLowerCase() === "f") setFitRequest(value => value + 1); }; window.addEventListener("keydown", shortcuts); return () => window.removeEventListener("keydown", shortcuts); }, []);
  const viewGraph = useMemo(() => {
    const bookById = new Map(graph.nodes.map(node => [node.id, node]));
    const filteredEdges = graph.edges.filter(edge => {
      if (relationFilter === "author") return edge.relation.includes("作者");
      if (relationFilter === "content") return !edge.relation.includes("作者");
      if (relationFilter === "analyzable") return [edge.from, edge.to].every(id => { const book = bookById.get(id); return !!book && book.highlightCount + book.thoughtCount > 0; });
      return true;
    }).sort((a, b) => b.score - a.score);
    if (strength === "all") return { nodes: graph.nodes, edges: filteredEdges };
    if (!focusId) return { nodes: [], edges: [] as Edge[] };
    const edges = filteredEdges.filter(edge => edge.from === focusId || edge.to === focusId);
    const visibleEdges = edges.slice(0, strength === "strong" ? 5 : strength === "balanced" ? 10 : strength === "standard" ? 12 : 20);
    const ids = new Set([focusId, ...visibleEdges.flatMap(edge => [edge.from, edge.to])]);
    const source = graph.nodes.filter(node => ids.has(node.id));
    const nodes = source.map(node => {
      if (node.id === focusId) return { ...node, x: W / 2, y: H / 2 };
      const index = source.filter(item => item.id !== focusId).findIndex(item => item.id === node.id), count = Math.max(1, source.length - 1), ring = count > 12 && index >= 10 ? 245 : 175, ringIndex = count > 12 && index >= 10 ? index - 10 : index, ringCount = count > 12 && index >= 10 ? count - 10 : Math.min(count, 10), angle = Math.PI * 2 * ringIndex / ringCount - Math.PI / 2;
      return { ...node, x: W / 2 + Math.cos(angle) * ring * 1.12, y: H / 2 + Math.sin(angle) * ring };
    });
    return { nodes, edges: visibleEdges };
  }, [focusId, graph, relationFilter, strength]);
  const renderedEdges = useMemo(() => viewGraph.edges.map(edge => ({ ...edge, semanticRelation: relationAnalyses[edge.id]?.relation })), [viewGraph.edges, relationAnalyses]);
  const selected = graph.nodes.find(node => node.id === selectedId), selectedEdge = graph.edges.find(edge => edge.id === edgeId);
  useEffect(() => {
    setDeepError(""); setDeepLoading(false);
    if (!selectedEdge || selectedEdge.id.startsWith("metadata:")) { setDeepAnalysis(undefined); return; }
    const inMemory = relationAnalyses[selectedEdge.id];
    if (inMemory) { setDeepAnalysis(inMemory); return; }
    let cancelled = false; setDeepAnalysis(undefined);
    void api.cachedRelation(selectedEdge.from, selectedEdge.to, selectedEdge.keywords).then(result => {
      if (!cancelled && result) {
        setDeepAnalysis(result);
      }
    }).catch(reason => { if (!cancelled) setDeepError(reason instanceof Error ? reason.message : String(reason)); });
    return () => { cancelled = true; };
  }, [edgeId, selectedEdge, relationAnalyses]);
  function navigateToBook(id: string) {
    if (focusId && focusId !== id) setBackStack(stack => [...stack, focusId]);
    const title = books.find(book => book.id === id)?.title ?? "";
    if (strength === "all") {
      setStrength("standard");
    }
    setForwardStack([]); setFocusId(id); setSelectedId(id); setEdgeId(undefined); setFitRequest(value => value + 1);
    window.setTimeout(() => setQuery(title), 0);
  }
  function goBack() {
    const target = backStack.at(-1); if (!target) return;
    if (focusId) setForwardStack(stack => [focusId, ...stack]);
    setBackStack(stack => stack.slice(0, -1)); setFocusId(target); setSelectedId(target); setEdgeId(undefined); setFitRequest(value => value + 1);
  }
  function goForward() {
    const [target, ...rest] = forwardStack; if (!target) return;
    if (focusId) setBackStack(stack => [...stack, focusId]);
    setForwardStack(rest); setFocusId(target); setSelectedId(target); setEdgeId(undefined); setFitRequest(value => value + 1);
  }
  function resetToGlobal() {
    setStrength("all");
    setRelationFilter("all");
    setFocusId(undefined);
    setSelectedId(undefined);
    setEdgeId(undefined);
    setQuery("");
    setPickerOpen(false);
    setBackStack([]);
    setForwardStack([]);
    setFitRequest(value => value + 1);
  }
  function toggleCompare(id: string) { setCompareIds(ids => ids.includes(id) ? ids.filter(item => item !== id) : ids.length < 4 ? [...ids, id] : ids); }
  async function analyzeRelation(refresh = false) {
    if (!selectedEdge || deepLoading) return;
    setDeepLoading(true); setDeepError("");
    try { const result = await api.analyzeRelation(selectedEdge.from, selectedEdge.to, selectedEdge.keywords, refresh); setDeepAnalysis(result); setRelationAnalyses(values => ({ ...values, [selectedEdge.id]: result })); }
    catch (reason) { setDeepError(reason instanceof Error ? reason.message : String(reason)); }
    finally { setDeepLoading(false); }
  }
  const q = query.trim().toLowerCase();
  const pickerBooks = useMemo(() => q ? books.filter(book => `${book.title}${book.author}${book.category}`.toLowerCase().includes(q)) : books, [books, q]);
  const pickerStart = Math.max(0, Math.floor(pickerScroll / 46) - 2), pickerItems = pickerBooks.slice(pickerStart, pickerStart + 12);
  return <div className="knowledge-graph-page">
    <div className="graph-toolbar"><div className="graph-search-wrap" ref={pickerRef}><label className="graph-search"><Search size={16}/><input value={query} onFocus={() => setPickerOpen(true)} onChange={e => { setQuery(e.target.value); setPickerOpen(true); setPickerScroll(0); }} onKeyDown={e => { if (e.key === "Escape") setPickerOpen(false); }} placeholder="选择一本中心书籍" /><button type="button" aria-label="展开全部书籍" onClick={() => setPickerOpen(open => !open)}><ChevronDown size={15}/></button></label>{pickerOpen && <div className="graph-book-picker"><div className="graph-book-picker__count">{q ? `找到 ${pickerBooks.length} 本` : `全部 ${pickerBooks.length} 本书`}</div><div className="graph-book-picker__scroll" onScroll={event => setPickerScroll(event.currentTarget.scrollTop)}><div style={{ height: pickerBooks.length * 46 }}>{pickerItems.map((book, index) => <button style={{ transform: `translateY(${(pickerStart + index) * 46}px)` }} key={book.id} onClick={() => { navigateToBook(book.id); setQuery(""); setPickerOpen(false); }}><strong>{book.title}</strong><span>{book.author || book.category || "未知作者"}</span></button>)}</div></div></div>}</div><label className="graph-strength"><Link2 size={14}/><span>每本书显示</span><select value={strength} onChange={e => setStrength(e.target.value as Strength)}>{strength === "all" && <option value="all" disabled>全局预览</option>}<option value="strong">5 个强关联</option><option value="balanced">10 个关联</option><option value="standard">12 个关联</option><option value="broad">20 个关联</option></select></label><label className="graph-strength"><span>关系</span><select value={relationFilter} onChange={e => { setRelationFilter(e.target.value as RelationFilter); setEdgeId(undefined); }}><option value="all">全部</option><option value="content">内容关联</option><option value="author">共同作者</option><option value="analyzable">可深度分析</option></select></label><label className="graph-connected-filter"><input type="checkbox" checked={connectedOnly} onChange={event => setConnectedOnly(event.target.checked)}/><span>仅显示有关联</span></label><div className="graph-nav"><button onClick={goBack} disabled={!backStack.length} title="返回上一本中心书" aria-label="返回"><ArrowLeft size={15}/></button><button onClick={goForward} disabled={!forwardStack.length} title="前进到下一本中心书" aria-label="前进"><ArrowRight size={15}/></button><button onClick={resetToGlobal} disabled={!graph.nodes.length} title="重置到全局视图" aria-label="重置到全局视图"><Globe2 size={15}/></button><button onClick={() => setFitRequest(value => value + 1)} disabled={!viewGraph.nodes.length} title="适应画布" aria-label="适应画布"><Maximize2 size={15}/></button><button onClick={() => refreshSemanticRelations(true)} disabled={semanticLoading} title="刷新语义关系" aria-label="刷新语义关系"><RotateCw className={semanticLoading ? "spin" : ""} size={15}/></button></div><span>{semanticLoading ? "正在刷新关系…" : viewGraph.nodes.length > 0 ? `发现 ${viewGraph.nodes.length - 1} 个关系` : "请选择一本书"}</span></div>
    <section className={`graph-shell${selected || selectedEdge ? " has-detail" : ""}`}>{(loading || (analyzing && !graph.nodes.length)) && <div className="graph-state">正在后台分析书籍之间的联系…<span>你可以继续使用其他页面</span></div>}{(analyzing || semanticLoading) && !!graph.nodes.length && <div className="graph-analyzing">{semanticLoading ? "正在生成语义向量并计算关系…" : "正在补充关系…"}</div>}{semanticError && <div className="graph-refresh-error">刷新失败：{semanticError}</div>}{!loading && error && <div className="graph-state"><Share2/><strong>暂时无法生成图谱</strong><span>{error}</span></div>}{!loading && !analyzing && !error && !graph.nodes.length && <div className="graph-state"><Share2/><strong>还没有发现可靠的书籍关系</strong><span>更多划线与想法会让关联分析更加准确。</span></div>}
      {isActivePage && strength === "all" && !loading && !error && !!viewGraph.nodes.length && <KnowledgeGraph3D
        nodes={viewGraph.nodes} edges={renderedEdges} focusId={focusId} selectedId={selectedId} showIsolated={!connectedOnly}
        fitRequest={fitRequest}
        onNodeClick={id => { setSelectedId(id); setEdgeId(undefined); }} onNodeOpen={navigateToBook}
        onEdgeClick={id => { setEdgeId(id); setSelectedId(undefined); }} />}
      {isActivePage && strength !== "all" && !loading && !error && !!viewGraph.nodes.length && <KnowledgeGraphCanvas
        nodes={viewGraph.nodes} edges={renderedEdges} focusId={focusId} selectedId={selectedId}
        selectedEdgeId={edgeId} fitRequest={fitRequest}
        onNodeClick={id => { setSelectedId(id); setEdgeId(undefined); }} onNodeOpen={navigateToBook}
        onEdgeClick={id => { setEdgeId(id); setSelectedId(undefined); }} />}
      {selected && <aside className="graph-detail"><button className="graph-detail__close" onClick={() => setSelectedId(undefined)}><X size={16}/></button><span className="graph-kind">书籍</span><h2>《{selected.title}》</h2><p>{selected.author}</p><small>{selected.highlightCount} 条划线 · {selected.thoughtCount} 条想法 · {graph.edges.filter(edge => edge.from === selected.id || edge.to === selected.id).length} 本关联书籍</small><button className={`graph-compare-toggle${compareIds.includes(selected.id) ? " selected" : ""}`} onClick={() => toggleCompare(selected.id)} disabled={selected.highlightCount + selected.thoughtCount === 0 || (!compareIds.includes(selected.id) && compareIds.length >= 4)}>{compareIds.includes(selected.id) ? <Check size={14}/> : <GitCompareArrows size={14}/>} {compareIds.includes(selected.id) ? "已加入比较" : "加入比较"}</button><div className="graph-detail__actions"><button onClick={() => navigateToBook(selected.id)}><Focus size={14}/>设为中心书</button><button onClick={() => openBook(selected.id)}><BookOpen size={14}/>打开书籍</button></div></aside>}
      {selectedEdge && (() => { const from = graph.nodes.find(n => n.id === selectedEdge.from)!, to = graph.nodes.find(n => n.id === selectedEdge.to)!, missing = [from, to].filter(book => book.highlightCount + book.thoughtCount === 0), canAnalyze = missing.length === 0, authorRelation = selectedEdge.relation.includes("作者"); return <aside className="graph-detail graph-relation-detail"><button className="graph-detail__close" onClick={() => setEdgeId(undefined)}><X size={16}/></button><span className="graph-kind">{deepAnalysis ? relationLabels[deepAnalysis.relation] : selectedEdge.relation}</span><h2>《{from.title}》<br/>与《{to.title}》</h2><p>{authorRelation && !deepAnalysis ? "共同作者：" : "共同主题："}</p><div className="relation-keywords">{(deepAnalysis?.concepts.length ? deepAnalysis.concepts : selectedEdge.keywords).map(word => <span key={word}>{word}</span>)}</div>{!deepAnalysis && selectedEdge.evidence.map(item => <button className="relation-evidence" key={item.noteId} onClick={() => openBook(item.bookId, item.noteId)}>“{short(item.text, 72)}”</button>)}{deepAnalysis && <div className="deep-relation"><p>{deepAnalysis.summary}</p>{deepAnalysis.claims.map((claim, index) => <section key={`${claim.relation}-${index}`}><strong>{relationLabels[claim.relation]} · {Math.round(claim.confidence * 100)}%</strong><p>{claim.summary}</p>{claim.evidence.map(item => <button className="relation-evidence" key={item.noteId} onClick={() => openBook(item.bookId, item.noteId)}><i>{item.noteType === "thought" ? "我的想法" : "原文划线"}</i>“{short(item.text, 90)}”</button>)}</section>)}</div>}<small>{deepAnalysis ? `AI 判断置信度 ${Math.round(deepAnalysis.confidence * 100)}%${deepAnalysis.cached ? " · 已缓存" : ""}` : `综合关联度 ${Math.round(selectedEdge.score * 100)}%`}</small>{deepError && <p className="relation-analysis-error">{deepError}</p>}{!canAnalyze && <p className="relation-analysis-notice">《{missing.map(book => book.title).join("》《")}》暂无划线或想法，不能进行有证据的观点分析。</p>}<div className="relation-analysis-actions"><button onClick={() => openAiCompare([from.id, to.id])} disabled={!canAnalyze}><GitCompareArrows size={14}/>比较这两本</button><button onClick={() => void analyzeRelation(!!deepAnalysis)} disabled={deepLoading || !canAnalyze}>{deepAnalysis ? <RotateCw size={14}/> : <Sparkles size={14}/>} {deepLoading ? "正在分析…" : deepAnalysis ? "重新分析" : "AI 深度分析"}</button></div></aside>; })()}
      {!!viewGraph.nodes.length && <div className="graph-legend graph-legend--help"><span>单击节点查看相邻关系</span><span>拖动平移画布</span><span>滚轮缩放</span></div>}
      {!!compareIds.length && <div className="graph-compare-tray"><div><GitCompareArrows size={15}/><strong>已选 {compareIds.length}/4 本</strong><span>{compareIds.map(id => graph.nodes.find(node => node.id === id)?.title).filter(Boolean).join("、")}</span></div><button className="clear" onClick={() => setCompareIds([])} title="清空比较"><Trash2 size={14}/></button><button onClick={() => openAiCompare(compareIds)} disabled={compareIds.length < 2}>进入 AI 比较</button></div>}
    </section></div>;
}
