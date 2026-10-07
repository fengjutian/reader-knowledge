import { ArrowLeft, ArrowRight, BookOpen, Check, ChevronDown, Eraser, Focus, GitCompareArrows, Globe2, Link2, Maximize2, Network, RotateCw, Search, Share2, Sparkles, Trash2, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api/tauri";
import { ConceptGraphPanel } from "../components/graph/ConceptGraphPanel";
import { KnowledgeGraphCanvas } from "../components/graph/KnowledgeGraphCanvas";
import { KnowledgeGraph3D } from "../components/graph/KnowledgeGraph3D";
import { Button } from "../components/ui/Button";
import { useLibraryRevision } from "../hooks/useLibraryRevision";
import { useAppStore } from "../stores/app";
import { clearGraphCache, clearLocalGraphCache, clearSemanticGraphCache, graphCacheKey, readGraphCache, semanticCacheKey as buildSemanticCacheKey, writeGraphCache } from "../utils/graphCache";
import type { Book, Note, RelationAnalysis, RelationKind, SemanticRelation } from "../types/domain";

type Node = Book & { x: number; y: number };
type Evidence = { bookId: string; text: string; noteId: string };
type Edge = { id: string; from: string; to: string; score: number; keywords: string[]; relation: string; evidence: Evidence[] };
type Analysis = { nodes: Node[]; candidates: Edge[] };
type Strength = "all" | "strong" | "balanced" | "standard" | "broad";
type WorkerStrength = Exclude<Strength, "standard">;
type RelationFilter = "all" | "content" | "author" | "analyzable";
type RelationSource = "all" | "local" | "semantic" | "weread" | "douban";
const W = 1000, H = 650;
const short = (value: string, size: number) => value.length > size ? `${value.slice(0, size)}…` : value;
const relationLabels: Record<RelationKind, string> = { same_concept: "同义概念", agreement: "观点一致", conflict: "观点冲突", complementary: "观点互补", causal: "因果关系", application: "理论与应用", uncertain: "证据不足" };
const workerStrength = (strength: Strength): WorkerStrength => strength === "standard" ? "all" : strength;
/** 元数据关系目前只支持微信读书与豆瓣，图谱中据此明确说明。 */
const METADATA_SOURCES = ["weread", "douban"] as const;

type GraphView = "books" | "concepts";

export function KnowledgeGraph() {
  // 概念网络与书籍关系是两种视图，切换时不卸载书籍图的数据，
  // 避免来回切换反复重算本地关系。
  const [view, setView] = useState<GraphView>("books");
  const [books, setBooks] = useState<Book[]>([]), [notes, setNotes] = useState<Note[]>([]), [loading, setLoading] = useState(true);
  const [analyzing, setAnalyzing] = useState(false), [graph, setGraph] = useState<{ nodes: Node[]; edges: Edge[] }>({ nodes: [], edges: [] });
  const [semanticLoading, setSemanticLoading] = useState(false);
  const [embeddingModel, setEmbeddingModel] = useState("");
  const [embeddingReady, setEmbeddingReady] = useState(false);
  const [semanticError, setSemanticError] = useState("");
  const [error, setError] = useState(""), [query, setQuery] = useState(""), [strength, setStrength] = useState<Strength>("all");
  const [relationFilter, setRelationFilter] = useState<RelationFilter>("all");
  const [relationSource, setRelationSource] = useState<RelationSource>("all");
  const [minimumScore, setMinimumScore] = useState(0);
  const [cacheNotice, setCacheNotice] = useState("");
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
  const lexicalEdgesRef = useRef<Edge[]>([]);
  const semanticEdgesRef = useRef<Edge[] | null>(null);
  const cacheKeyRef = useRef("");
  const recomputingRef = useRef(false);
  const pickerRef = useRef<HTMLDivElement>(null);
  const wasActiveRef = useRef(false);
  const combinedEdges = (external: Edge[] | null = semanticEdgesRef.current, lexical = lexicalEdgesRef.current) => {
    const merged = new Map(lexical.map(edge => [edge.id, edge]));
    external?.forEach(edge => merged.set(edge.id, edge));
    return [...merged.values()];
  };
  useEffect(() => {
    if (isActivePage && !wasActiveRef.current) {
      setSelectedId(undefined);
      setEdgeId(undefined);
      setPickerOpen(false);
    }
    wasActiveRef.current = isActivePage;
  }, [isActivePage]);
  const mergeExternalEdges = useCallback((edges: Edge[]) => {
    const merged = new Map((semanticEdgesRef.current ?? []).map(edge => [edge.id, edge]));
    edges.forEach(edge => merged.set(edge.id, edge));
    const values = [...merged.values()];
    semanticEdgesRef.current = values.length ? values : null;
    setGraph(current => ({ ...current, edges: combinedEdges(values.length ? values : null) }));
  }, []);
  /**
   * 重新拉取语义关系与元数据关系。
   * 只有本地 Embedding 已安装时才请求向量关系，否则只拉取元数据关系，
   * 避免同步后无谓地触发远程 Embedding。
   */
  const refreshSemanticRelations = useCallback((showError = false) => {
    // 从 ref 读取，保证事件处理器里拿到的是最新状态而不是闭包快照。
    const ready = embeddingStateRef.current.ready;
    setSemanticLoading(true); setSemanticError("");
    const metadataCalls = METADATA_SOURCES.map(source => api.metadataRelations(source));
    const requests = ready ? [api.semanticRelations(), ...metadataCalls] : metadataCalls;
    Promise.allSettled(requests)
      .then(results => {
        const [semanticResult, ...metadataResults] = ready ? results : [];
        const semantic = semanticResult && semanticResult.status === "fulfilled" ? semanticResult.value.map(toEdge) : [];
        const metadata = (ready ? metadataResults : results).flatMap(result => (result.status === "fulfilled" ? result.value.map(toEdge) : []));
        // 语义关系来自后端全新计算，替换旧的语义缓存内容；元数据关系与本地词法关系保持不变。
        const kept = (semanticEdgesRef.current ?? []).filter(edge => edge.id.startsWith("metadata:"));
        semanticEdgesRef.current = null;
        const mergedAll = new Map([...kept, ...metadata, ...semantic].map(edge => [edge.id, edge]));
        const values = [...mergedAll.values()];
        semanticEdgesRef.current = values.length ? values : null;
        setGraph(current => ({ ...current, edges: combinedEdges(semanticEdgesRef.current) }));
        if (semantic.length) void writeGraphCache(buildSemanticCacheKey({ books, notes }, embeddingStateRef.current.model, [...METADATA_SOURCES]), semantic);
        else void clearSemanticGraphCache();
        const failures = results.filter(result => result.status === "rejected").length;
        setSemanticError(failures ? `${failures} 个关系来源刷新失败，已保留其他来源结果` : "");
        if (!failures && !showError) setSemanticError("");
      })
      .finally(() => setSemanticLoading(false));
  }, [books, notes]);
  /** SemanticRelation（后端 camelCase）与图谱 Edge 结构一致，转换保持显式以防后端结构变化。 */
  function toEdge(relation: SemanticRelation): Edge {
    return {
      id: relation.id, from: relation.from, to: relation.to, score: relation.score,
      keywords: relation.keywords ?? [], relation: relation.relation,
      evidence: (relation.evidence ?? []).map(item => ({ bookId: item.bookId, noteId: item.noteId, text: item.text })),
    };
  }
  // 用 ref 读取 embedding 状态：让 loadLibrary 保持稳定引用，
  // 避免 embeddingReady 变化时重复拉取书籍与笔记。
  const embeddingStateRef = useRef({ ready: false, model: "" });
  useEffect(() => { embeddingStateRef.current = { ready: embeddingReady, model: embeddingModel }; }, [embeddingReady, embeddingModel]);
  const loadLibrary = useCallback(async () => {
    setLoading(true);
    try {
      const [b, n] = await Promise.all([api.books(), api.notes()]);
      const { ready, model } = embeddingStateRef.current;
      if (ready) {
        const cachedEdges = await readGraphCache<Edge[]>(buildSemanticCacheKey({ books: b, notes: n }, model, [...METADATA_SOURCES]));
        if (cachedEdges?.length) mergeExternalEdges(cachedEdges);
      }
      setBooks(b); setNotes(n);
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally { setLoading(false); }
  }, [mergeExternalEdges]);
  useEffect(() => {
    // 先读取本地 Embedding 状态，决定后续是否生成语义关系。
    let active = true;
    void api.localEmbeddingStatus().then(status => {
      if (!active) return;
      embeddingStateRef.current = { ready: status.installed, model: status.model };
      setEmbeddingReady(status.installed);
      setEmbeddingModel(status.model);
    }).catch(() => undefined);
    return () => { active = false; };
  }, []);
  useEffect(() => { void loadLibrary(); }, [loadLibrary]);
  // 首屏拉取一次元数据关系；已配置本地 Embedding 时同时生成语义关系。
  useEffect(() => {
    let active = true;
    setSemanticLoading(true); setSemanticError("");
    const metadataCalls = METADATA_SOURCES.map(source => api.metadataRelations(source));
    Promise.allSettled(metadataCalls).then(results => {
      if (!active) return;
      const edges = results.flatMap(result => (result.status === "fulfilled" ? result.value.map(toEdge) : []));
      if (edges.length) mergeExternalEdges(edges);
      setSemanticLoading(false);
    });
    return () => { active = false; };
  }, [mergeExternalEdges]);
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
  useEffect(() => {
    const nodeIds = new Set(graph.nodes.map(node => node.id)), edgeIds = new Set(graph.edges.filter(edge => nodeIds.has(edge.from) && nodeIds.has(edge.to)).map(edge => edge.id));
    setSelectedId(current => current && nodeIds.has(current) ? current : undefined);
    setEdgeId(current => current && edgeIds.has(current) ? current : undefined);
    setCompareIds(current => current.filter(id => nodeIds.has(id)));
  }, [graph.nodes, graph.edges]);
  // refreshSemanticRelations 依赖 books/notes，会随数据变化重建。
  // 用 ref 持有最新引用，让事件监听和 revision 回调保持稳定绑定。
  const refreshSemanticRef = useRef(refreshSemanticRelations);
  useEffect(() => { refreshSemanticRef.current = refreshSemanticRelations; }, [refreshSemanticRelations]);
  useEffect(() => {
    // 本地 Embedding 下载完成：自动生成语义关系并写入对应版本的缓存。
    let active = true;
    const ready = () => {
      if (!active) return;
      // 先同步写入 ref，refreshSemanticRelations 才会走语义关系分支。
      embeddingStateRef.current = { ready: true, model: embeddingStateRef.current.model };
      setEmbeddingReady(true);
      setSemanticError("");
      setCacheNotice("本地模型已就绪，正在生成语义关系…");
      void api.localEmbeddingStatus().then(status => {
        if (!active) return;
        embeddingStateRef.current = { ready: true, model: status.model };
        setEmbeddingModel(status.model);
      }).catch(() => undefined);
      refreshSemanticRef.current(true);
    };
    // 删除模型：移除内存中的语义边并清空语义缓存，保留本地词法关系与元数据关系。
    const removed = () => {
      if (!active) return;
      embeddingStateRef.current = { ready: false, model: embeddingStateRef.current.model };
      setEmbeddingReady(false);
      const kept = (semanticEdgesRef.current ?? []).filter(edge => edge.id.startsWith("metadata:"));
      semanticEdgesRef.current = kept.length ? kept : null;
      setGraph(current => ({ ...current, edges: combinedEdges(semanticEdgesRef.current) }));
      void clearSemanticGraphCache();
      setCacheNotice("本地模型已删除，语义关系已移除");
      window.setTimeout(() => setCacheNotice(""), 2200);
      workerRef.current?.postMessage({ type: "filter", strength: workerStrength(strength) });
    };
    window.addEventListener("local-embedding-ready", ready);
    window.addEventListener("local-embedding-removed", removed);
    return () => { active = false; window.removeEventListener("local-embedding-ready", ready); window.removeEventListener("local-embedding-removed", removed); };
  }, [strength]);
  // 同步成功后重新拉取书籍与笔记，并刷新元数据/语义关系
  // （本地图谱缓存 key 随数据变化自动失效旧缓存）。
  useLibraryRevision(useCallback(() => {
    lexicalEdgesRef.current = [];
    void loadLibrary();
    refreshSemanticRef.current(true);
  }, [loadLibrary]));
  useEffect(() => {
    const worker = new Worker(new URL("../workers/knowledgeGraph.worker.ts", import.meta.url), { type: "module" });
    workerRef.current = worker;
    worker.onmessage = (event: MessageEvent<{ type: "nodes" | "result"; graph: { nodes: Node[]; edges: Edge[] }; analysis?: Analysis }>) => {
      lexicalEdgesRef.current = event.data.graph.edges;
      setGraph({ ...event.data.graph, edges: combinedEdges() });
      if (event.data.type === "result") {
        setAnalyzing(false);
        if (recomputingRef.current) { recomputingRef.current = false; setCacheNotice("本地关系已重新计算"); window.setTimeout(() => setCacheNotice(""), 2200); }
        setFocusId(current => {
          if (current) return current;
          const connected = new Set(event.data.graph.edges.flatMap(edge => [edge.from, edge.to]));
          return [...event.data.graph.nodes].filter(node => connected.has(node.id)).sort((a, b) => (b.highlightCount + b.thoughtCount) - (a.highlightCount + a.thoughtCount))[0]?.id ?? event.data.graph.nodes[0]?.id;
        });
      }
      if (event.data.analysis && cacheKeyRef.current) void writeGraphCache(cacheKeyRef.current, event.data.analysis);
    };
    worker.onerror = event => { recomputingRef.current = false; setError(event.message || "关系分析失败"); setAnalyzing(false); };
    return () => worker.terminate();
  }, []);
  useEffect(() => {
    if (loading || error || !workerRef.current) return;
    let cancelled = false;
    const key = graphCacheKey({ books, notes }); cacheKeyRef.current = key; setAnalyzing(true);
    void readGraphCache<Analysis>(key).then(cached => {
      const usableCached = cached?.nodes?.length ? cached : undefined;
      if (!cancelled) workerRef.current?.postMessage({ type: "init", books, notes: notes.map(({ id, type, bookId, chapter, content, bookTitle, createdAt }) => ({ id, type, bookId, chapter, content, bookTitle, createdAt })), strength: workerStrength(strength), cached: usableCached });
    });
    return () => { cancelled = true; };
    // strength 刻意不参与：切换强度只通过下面的 filter 消息生效，不触发全量重算。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [books, notes, loading, error]);
  useEffect(() => { if (!loading && books.length) workerRef.current?.postMessage({ type: "filter", strength: workerStrength(strength) }); }, [strength, loading, books.length]);
  useEffect(() => { const close = (event: MouseEvent) => { if (!pickerRef.current?.contains(event.target as globalThis.Node)) setPickerOpen(false); }; document.addEventListener("mousedown", close); return () => document.removeEventListener("mousedown", close); }, []);
  useEffect(() => { const shortcuts = (event: KeyboardEvent) => { const target = event.target as HTMLElement; if (target.matches("input,select,textarea")) return; if (event.key === "Escape") { setSelectedId(undefined); setEdgeId(undefined); } if (event.key.toLowerCase() === "f") setFitRequest(value => value + 1); }; window.addEventListener("keydown", shortcuts); return () => window.removeEventListener("keydown", shortcuts); }, []);
  const viewGraph = useMemo(() => {
    const bookById = new Map(graph.nodes.map(node => [node.id, node]));
    const filteredEdges = graph.edges.filter(edge => bookById.has(edge.from) && bookById.has(edge.to) && edge.score >= minimumScore).filter(edge => {
      if (relationSource === "local" && (edge.id.startsWith("semantic:") || edge.id.startsWith("metadata:"))) return false;
      if (relationSource === "semantic" && !edge.id.startsWith("semantic:")) return false;
      if (relationSource === "weread" && !edge.id.startsWith("metadata:weread:")) return false;
      if (relationSource === "douban" && !edge.id.startsWith("metadata:douban:")) return false;
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
  }, [focusId, graph, minimumScore, relationFilter, relationSource, strength]);
  const renderedEdges = useMemo(() => viewGraph.edges.map(edge => ({ ...edge, semanticRelation: relationAnalyses[edge.id]?.relation })), [viewGraph.edges, relationAnalyses]);
  const selected = graph.nodes.find(node => node.id === selectedId), selectedEdge = graph.edges.find(edge => edge.id === edgeId && graph.nodes.some(node => node.id === edge.from) && graph.nodes.some(node => node.id === edge.to));
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
    if (strength !== "all" && focusId && focusId !== id) setBackStack(stack => [...stack, focusId]);
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
    setRelationSource("all");
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
  async function clearCaches() {
    // 用户手动清理时才清空全部图谱缓存；其他失效场景按命名空间删除。
    await clearGraphCache();
    setRelationAnalyses({}); setDeepAnalysis(undefined); setCacheNotice("缓存已清理");
    window.setTimeout(() => setCacheNotice(""), 2200);
  }
  async function recomputeLocalRelations() {
    if (!workerRef.current || analyzing) return;
    // 只清本地图谱分析缓存，保留语义关系缓存。
    await clearLocalGraphCache();
    lexicalEdgesRef.current = [];
    recomputingRef.current = true;
    setAnalyzing(true); setCacheNotice("正在重新计算本地关系…");
    const key = graphCacheKey({ books, notes }); cacheKeyRef.current = key;
    workerRef.current.postMessage({ type: "init", books, notes: notes.map(({ id, type, bookId, chapter, content, bookTitle, createdAt }) => ({ id, type, bookId, chapter, content, bookTitle, createdAt })), strength: workerStrength(strength) });
  }
  const q = query.trim().toLowerCase();
  const pickerBooks = useMemo(() => q ? books.filter(book => `${book.title}${book.author}${book.category}`.toLowerCase().includes(q)) : books, [books, q]);
  const pickerStart = Math.max(0, Math.floor(pickerScroll / 46) - 2), pickerItems = pickerBooks.slice(pickerStart, pickerStart + 12);
  if (view === "concepts") return <div className="knowledge-graph-page">
    <div className="graph-view-tabs" role="tablist" aria-label="图谱视图">
      <button type="button" role="tab" aria-selected={false} onClick={() => setView("books")}><BookOpen size={15}/>书籍关系</button>
      <button type="button" role="tab" aria-selected onClick={() => setView("concepts")}><Network size={15}/>概念网络</button>
    </div>
    <ConceptGraphPanel/>
  </div>;
  return <div className="knowledge-graph-page">
    <div className="graph-view-tabs" role="tablist" aria-label="图谱视图">
      <button type="button" role="tab" aria-selected onClick={() => setView("books")}><BookOpen size={15}/>书籍关系</button>
      <button type="button" role="tab" aria-selected={false} onClick={() => setView("concepts")}><Network size={15}/>概念网络</button>
    </div>
    <div className="graph-source-toolbar"><label><span>关系来源</span><select value={relationSource} onChange={e => { setRelationSource(e.target.value as RelationSource); setEdgeId(undefined); }}><option value="all">全部来源</option><option value="local">本地笔记</option><option value="semantic">语义向量</option><option value="weread">微信读书</option><option value="douban">豆瓣</option></select></label><label className="graph-score-filter"><span>最低关联度</span><input type="range" min="0" max="90" step="5" value={Math.round(minimumScore * 100)} onChange={event => { setMinimumScore(Number(event.target.value) / 100); setEdgeId(undefined); }}/><strong>{Math.round(minimumScore * 100)}%</strong></label><button type="button" onClick={() => void clearCaches()}><Eraser size={14}/>清理缓存</button><button type="button" disabled={analyzing} onClick={() => void recomputeLocalRelations()}><RotateCw className={analyzing ? "spin" : ""} size={14}/>重新计算本地关系</button>{cacheNotice && <em>{cacheNotice}</em>}</div>
    <div className="graph-toolbar"><div className="graph-search-wrap" ref={pickerRef}><label className="graph-search"><Search size={16}/><input value={query} onFocus={() => setPickerOpen(true)} onChange={e => { setQuery(e.target.value); setPickerOpen(true); setPickerScroll(0); }} onKeyDown={e => { if (e.key === "Escape") setPickerOpen(false); }} placeholder="选择一本中心书籍" /><button type="button" aria-label="展开全部书籍" onClick={() => setPickerOpen(open => !open)}><ChevronDown size={15}/></button></label>{pickerOpen && <div className="graph-book-picker"><div className="graph-book-picker__count">{q ? `找到 ${pickerBooks.length} 本` : `全部 ${pickerBooks.length} 本书`}</div><div className="graph-book-picker__scroll" onScroll={event => setPickerScroll(event.currentTarget.scrollTop)}><div style={{ height: pickerBooks.length * 46 }}>{pickerItems.map((book, index) => <button style={{ transform: `translateY(${(pickerStart + index) * 46}px)` }} key={book.id} onClick={() => { navigateToBook(book.id); setQuery(""); setPickerOpen(false); }}><strong>{book.title}</strong><span>{book.author || book.category || "未知作者"}</span></button>)}</div></div></div>}</div><label className="graph-strength"><Link2 size={14}/><span>每本书显示</span><select value={strength} onChange={e => setStrength(e.target.value as Strength)}>{strength === "all" && <option value="all" disabled>全局预览</option>}<option value="strong">5 个强关联</option><option value="balanced">10 个关联</option><option value="standard">12 个关联</option><option value="broad">20 个关联</option></select></label><label className="graph-strength"><span>关系</span><select value={relationFilter} onChange={e => { setRelationFilter(e.target.value as RelationFilter); setEdgeId(undefined); }}><option value="all">全部</option><option value="content">内容关联</option><option value="author">共同作者</option><option value="analyzable">可深度分析</option></select></label><label className="graph-connected-filter"><input type="checkbox" checked={connectedOnly} onChange={event => setConnectedOnly(event.target.checked)}/><span>仅显示有关联</span></label><div className="graph-nav"><button onClick={goBack} disabled={!backStack.length} title="返回上一本中心书" aria-label="返回"><ArrowLeft size={15}/></button><button onClick={goForward} disabled={!forwardStack.length} title="前进到下一本中心书" aria-label="前进"><ArrowRight size={15}/></button><button onClick={resetToGlobal} disabled={!graph.nodes.length} title="重置到全局视图" aria-label="重置到全局视图"><Globe2 size={15}/></button><button onClick={() => setFitRequest(value => value + 1)} disabled={!viewGraph.nodes.length} title="适应画布" aria-label="适应画布"><Maximize2 size={15}/></button><button onClick={() => refreshSemanticRelations(true)} disabled={semanticLoading} title="刷新语义关系" aria-label="刷新语义关系"><RotateCw className={semanticLoading ? "spin" : ""} size={15}/></button></div><span>{semanticLoading ? "正在刷新关系…" : `${viewGraph.edges.length} 个关系`}</span></div>
    <section className={`graph-shell${selected || selectedEdge ? " has-detail" : ""}`}>{(loading || (analyzing && !graph.nodes.length)) && <div className="graph-state">正在后台分析书籍之间的联系…<span>你可以继续使用其他页面</span></div>}{(analyzing || semanticLoading) && !!graph.nodes.length && <div className="graph-analyzing">{semanticLoading ? "正在生成语义向量并计算关系…" : "正在补充关系…"}</div>}{semanticError && <div className="graph-refresh-error">刷新失败：{semanticError}</div>}{!loading && error && <div className="graph-state"><Share2/><strong>暂时无法生成图谱</strong><span>{error}</span></div>}
      {/* 空状态分级：没有任何书籍 / 有书籍无笔记 / 有节点无关系 / Embedding 未配置 */}
      {!loading && !error && !graph.nodes.length && !analyzing && books.length === 0 && <div className="graph-state"><Share2/><strong>书架里还没有书籍</strong><span>先在首页同步微信读书，同步完成后这里会出现节点。</span><div className="graph-state__actions"><Button variant="secondary" onClick={() => useAppStore.getState().setPage("books")}>前往书籍页</Button></div></div>}
      {!loading && !error && !analyzing && books.length > 0 && notes.length === 0 && <div className="graph-state"><Share2/><strong>还没有笔记，无法计算内容关系</strong><span>{books.length} 本书已同步。添加划线或想法后，节点和关系会自动出现。</span><div className="graph-state__actions"><Button variant="secondary" onClick={() => useAppStore.getState().setPage("notes")}>前往笔记页</Button></div></div>}
      {!loading && !error && !analyzing && books.length > 0 && notes.length > 0 && !graph.nodes.length && !graph.edges.length && <div className="graph-state"><Share2/><strong>暂时没有发现书籍之间的关系</strong><span>共同作者、相同主题或相似的笔记内容会生成关系；也可以在书籍元数据页补全元数据来增加关系来源。</span><div className="graph-state__actions"><Button variant="secondary" onClick={() => void recomputeLocalRelations()}>重新计算本地关系</Button><Button variant="secondary" onClick={() => useAppStore.getState().setPage("metadata")}>前往书籍元数据</Button></div></div>}
      {!loading && !error && !analyzing && books.length > 0 && notes.length > 0 && !!graph.nodes.length && !viewGraph.edges.length && connectedOnly && <div className="graph-refresh-hint">当前"仅显示有关联"已开启，但这些书籍之间还没有找到关系。关闭该选项可以查看全部书籍，或前往书籍元数据补全共同作者与主题。</div>}
      {!!graph.nodes.length && !embeddingReady && !semanticLoading && <div className="graph-embedding-hint"><Sparkles size={13}/>尚未配置 Embedding，语义关系不可用。本地关系与元数据关系仍然有效。</div>}
      {isActivePage && strength === "all" && !loading && !error && !!viewGraph.nodes.length && <KnowledgeGraph3D
        nodes={viewGraph.nodes} edges={renderedEdges} focusId={focusId} selectedId={selectedId} showIsolated={!connectedOnly}
        fitRequest={fitRequest}
        onNodeClick={navigateToBook} onNodeOpen={navigateToBook}
        onEdgeClick={id => { setEdgeId(id); setSelectedId(undefined); }} />}
      {isActivePage && strength !== "all" && !loading && !error && !!viewGraph.nodes.length && <KnowledgeGraphCanvas
        nodes={viewGraph.nodes} edges={renderedEdges} focusId={focusId} selectedId={selectedId}
        selectedEdgeId={edgeId} fitRequest={fitRequest}
        onNodeClick={id => { setSelectedId(id); setEdgeId(undefined); }} onNodeOpen={navigateToBook}
        onEdgeClick={id => { setEdgeId(id); setSelectedId(undefined); }} />}
      {selected && <aside className="graph-detail"><button className="graph-detail__close" onClick={() => setSelectedId(undefined)}><X size={16}/></button><span className="graph-kind">书籍</span><h2>《{selected.title}》</h2><p>{selected.author}</p><small>{selected.highlightCount} 条划线 · {selected.thoughtCount} 条想法 · {graph.edges.filter(edge => edge.from === selected.id || edge.to === selected.id).length} 本关联书籍</small><button className={`graph-compare-toggle${compareIds.includes(selected.id) ? " selected" : ""}`} onClick={() => toggleCompare(selected.id)} disabled={selected.highlightCount + selected.thoughtCount === 0 || (!compareIds.includes(selected.id) && compareIds.length >= 4)}>{compareIds.includes(selected.id) ? <Check size={14}/> : <GitCompareArrows size={14}/>} {compareIds.includes(selected.id) ? "已加入比较" : "加入比较"}</button><div className="graph-detail__actions"><button onClick={() => navigateToBook(selected.id)}><Focus size={14}/>设为中心书</button><button onClick={() => openBook(selected.id)}><BookOpen size={14}/>打开书籍</button></div></aside>}
      {selectedEdge && (() => { const from = graph.nodes.find(n => n.id === selectedEdge.from)!, to = graph.nodes.find(n => n.id === selectedEdge.to)!, missing = [from, to].filter(book => book.highlightCount + book.thoughtCount === 0), canAnalyze = missing.length === 0, authorRelation = selectedEdge.relation.includes("作者"); return <aside className="graph-detail graph-relation-detail"><button className="graph-detail__close" onClick={() => setEdgeId(undefined)}><X size={16}/></button><span className="graph-kind">{deepAnalysis ? relationLabels[deepAnalysis.relation] : selectedEdge.relation}</span><h2>《{from.title}》<br/>与《{to.title}》</h2><p>{authorRelation && !deepAnalysis ? "共同作者：" : "共同主题："}</p><div className="relation-keywords">{(deepAnalysis?.concepts.length ? deepAnalysis.concepts : selectedEdge.keywords).map(word => <span key={word}>{word}</span>)}</div>{!deepAnalysis && selectedEdge.evidence.map(item => <button className="relation-evidence" key={item.noteId} onClick={() => openBook(item.bookId, item.noteId)}>“{short(item.text, 72)}”</button>)}{deepAnalysis && <div className="deep-relation"><p>{deepAnalysis.summary}</p>{deepAnalysis.claims.map((claim, index) => <section key={`${claim.relation}-${index}`}><strong>{relationLabels[claim.relation]} · {Math.round(claim.confidence * 100)}%</strong><p>{claim.summary}</p>{claim.evidence.map(item => <button className="relation-evidence" key={item.noteId} onClick={() => openBook(item.bookId, item.noteId)}><i>{item.noteType === "thought" ? "我的想法" : "原文划线"}</i>“{short(item.text, 90)}”</button>)}</section>)}</div>}<small>{deepAnalysis ? `AI 判断置信度 ${Math.round(deepAnalysis.confidence * 100)}%${deepAnalysis.cached ? " · 已缓存" : ""}` : `综合关联度 ${Math.round(selectedEdge.score * 100)}%`}</small>{deepError && <p className="relation-analysis-error">{deepError}</p>}{!canAnalyze && <p className="relation-analysis-notice">《{missing.map(book => book.title).join("》《")}》暂无划线或想法，不能进行有证据的观点分析。</p>}<div className="relation-analysis-actions"><button onClick={() => openAiCompare([from.id, to.id])} disabled={!canAnalyze}><GitCompareArrows size={14}/>比较这两本</button><button onClick={() => void analyzeRelation(!!deepAnalysis)} disabled={deepLoading || !canAnalyze}>{deepAnalysis ? <RotateCw size={14}/> : <Sparkles size={14}/>} {deepLoading ? "正在分析…" : deepAnalysis ? "重新分析" : "AI 深度分析"}</button></div></aside>; })()}
      {!!viewGraph.nodes.length && <div className="graph-legend graph-legend--help"><span>单击节点查看相邻关系</span><span>拖动平移画布</span><span>滚轮缩放</span></div>}
      {!!viewGraph.nodes.length && <div className="graph-color-legend"><strong>图例</strong><div><i className="node-center"/>中心书<i className="node-related"/>关联书<i className="node-isolated"/>无关联</div><div><i className="edge-content"/>内容关系<i className="edge-author"/>共同作者<i className="edge-agreement"/>观点一致<i className="edge-conflict"/>观点冲突<i className="edge-complementary"/>观点互补<i className="edge-causal"/>因果关系</div></div>}
      {!!compareIds.length && <div className="graph-compare-tray"><div><GitCompareArrows size={15}/><strong>已选 {compareIds.length}/4 本</strong><span>{compareIds.map(id => graph.nodes.find(node => node.id === id)?.title).filter(Boolean).join("、")}</span></div><button className="clear" onClick={() => setCompareIds([])} title="清空比较"><Trash2 size={14}/></button><button onClick={() => openAiCompare(compareIds)} disabled={compareIds.length < 2}>进入 AI 比较</button></div>}
    </section></div>;
}
