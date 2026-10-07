import { Check, Eraser, EyeOff, Focus, Loader2, Merge, Network, RefreshCw, ScanSearch, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "../../api/tauri";
import { Button } from "../ui/Button";
import { ConceptGraphCanvas } from "./ConceptGraphCanvas";
import { useAppStore } from "../../stores/app";
import type { ConceptGraph, ConceptScanResult, EntityCorrection, EntityKind, EntityStatus, KnowledgeEntity } from "../../types/domain";

const kindLabels: Record<EntityKind, string> = { concept: "概念", topic: "主题", idea: "想法" };
const statusLabels: Record<EntityStatus, string> = { suggested: "待确认", confirmed: "已确认", hidden: "已隐藏" };
const relationLabels: Record<string, string> = {
  broader: "上位", narrower: "下位", related: "相关", supports: "支持",
  conflicts: "冲突", causes: "导致", applies: "适用于",
};
const kindOptions: { value: EntityKind; label: string }[] = [
  { value: "concept", label: "概念" },
  { value: "topic", label: "主题" },
  { value: "idea", label: "想法" },
];
const short = (value: string, size: number) => (value.length > size ? `${value.slice(0, size)}…` : value);

export function ConceptGraphPanel() {
  const [graph, setGraph] = useState<ConceptGraph>({ entities: [], relations: [], truncated: false, totalEntities: 0 });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [scanning, setScanning] = useState(false);
  const [scanResult, setScanResult] = useState<ConceptScanResult | null>(null);
  const [kinds, setKinds] = useState<EntityKind[]>([]);
  const [minConfidence, setMinConfidence] = useState(0);
  const [selectedId, setSelectedId] = useState<string>();
  const [relationId, setRelationId] = useState<string>();
  const [detail, setDetail] = useState<KnowledgeEntity | null>(null);
  const [fitRequest, setFitRequest] = useState(0);
  const [expanded, setExpanded] = useState<string | undefined>();
  const [aliasDraft, setAliasDraft] = useState("");
  const [nameDraft, setNameDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const openBook = useAppStore(state => state.openBook);
  const isActivePage = useAppStore(state => state.page === "graph");

  const load = useCallback(() => {
    setLoading(true); setError("");
    api.conceptGraph({ kinds, minConfidence: minConfidence || undefined, centerId: expanded })
      .then(setGraph)
      .catch(reason => setError(reason instanceof Error ? reason.message : String(reason)))
      .finally(() => setLoading(false));
  }, [kinds, minConfidence, expanded]);

  useEffect(() => { load(); }, [load]);

  // 选中节点时拉详情（列表接口不返回证据正文）
  useEffect(() => {
    if (!selectedId) { setDetail(null); return; }
    let active = true;
    api.conceptEntity(selectedId).then(entity => { if (active) { setDetail(entity); setNameDraft(entity.canonicalName); setAliasDraft(""); } })
      .catch(reason => { if (active) setError(reason instanceof Error ? reason.message : String(reason)); });
    return () => { active = false; };
  }, [selectedId]);

  const selectedRelation = useMemo(() => graph.relations.find(item => item.id === relationId), [graph.relations, relationId]);

  async function scan(changedOnly: boolean) {
    setScanning(true); setError(""); setScanResult(null);
    try {
      const result = await api.scanConcepts({ changedOnly });
      setScanResult(result);
      setExpanded(undefined);
      load();
    } catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); }
    finally { setScanning(false); }
  }

  async function correct(patch: Omit<EntityCorrection, "id">) {
    if (!selectedId) return;
    setBusy(true); setError("");
    try {
      const updated = await api.correctConceptEntity({ ...patch, id: selectedId });
      setDetail(updated);
      load();
    } catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); }
    finally { setBusy(false); }
  }

  async function clearSuggested() {
    setBusy(true); setError("");
    try { await api.clearSuggestedConcepts(); setSelectedId(undefined); load(); }
    catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); }
    finally { setBusy(false); }
  }

  async function mergeInto(targetId: string) {
    if (!selectedId || selectedId === targetId) return;
    setBusy(true); setError("");
    try {
      await api.mergeConceptEntities(selectedId, targetId);
      setSelectedId(undefined); setDetail(null); setExpanded(undefined);
      load();
    } catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); }
    finally { setBusy(false); }
  }

  function toggleKind(kind: EntityKind) {
    setKinds(current => current.includes(kind) ? current.filter(item => item !== kind) : [...current, kind]);
  }

  return <div className="concept-panel">
    <div className="concept-toolbar">
      <div className="concept-chips" role="group" aria-label="实体类型">
        {kindOptions.map(option => <button type="button" key={option.value} aria-pressed={kinds.includes(option.value)} className={`search-chip${kinds.includes(option.value) ? " is-active" : ""}`} onClick={() => toggleKind(option.value)}>{option.label}</button>)}
      </div>
      <label className="graph-score-filter"><span>最低置信度</span><input type="range" min="0" max="90" step="5" value={Math.round(minConfidence * 100)} onChange={event => setMinConfidence(Number(event.target.value) / 100)}/><strong>{Math.round(minConfidence * 100)}%</strong></label>
      <Button variant="secondary" onClick={() => void scan(true)} disabled={scanning}>{scanning ? <><Loader2 size={14} className="spin"/>扫描中…</> : <><RefreshCw size={14}/>仅更新变化内容</>}</Button>
      <Button variant="secondary" onClick={() => void scan(false)} disabled={scanning}><ScanSearch size={14}/>扫描全部</Button>
      <Button variant="secondary" onClick={() => void clearSuggested()} disabled={busy}><Eraser size={14}/>清理建议项</Button>
      {expanded && <Button variant="secondary" onClick={() => { setExpanded(undefined); setSelectedId(undefined); }}><Network size={14}/>返回全图</Button>}
    </div>

    {scanResult && <div className="concept-scan-result">
      <strong>扫描完成</strong>
      <span>扫描 {scanResult.booksScanned} 本书</span>
      <span>跳过未变化笔记 {scanResult.notesSkipped} 条</span>
      <span>新建概念 {scanResult.entitiesCreated} 个、关系 {scanResult.relationsCreated} 条</span>
      {scanResult.rejected > 0 && <span>因缺少真实证据被丢弃 {scanResult.rejected} 项</span>}
      {scanResult.booksFailed > 0 && <span className="field-error">{scanResult.booksFailed} 本书失败：{scanResult.failures[0]}</span>}
    </div>}

    <section className={`graph-shell${detail || selectedRelation ? " has-detail" : ""}`}>
      {loading && <div className="graph-state">正在加载概念网络…</div>}
      {error && <div className="graph-refresh-error">操作失败：{error}</div>}
      {!loading && !graph.entities.length && <div className="graph-state"><Network size={22}/><strong>还没有概念网络</strong><span>点击「扫描全部」从你的划线与想法中抽取概念、主题和观点，并建立它们之间的关系。</span></div>}
      {graph.truncated && <div className="graph-refresh-hint">共 {graph.totalEntities} 个概念，超过 500 时默认只加载高置信度子图。点击某个概念可展开它的邻居，逐步浏览全图。</div>}
      {isActivePage && !loading && !!graph.entities.length && <ConceptGraphCanvas
        entities={graph.entities} relations={graph.relations} selectedId={selectedId} selectedRelationId={relationId}
        fitRequest={fitRequest}
        onNodeClick={id => { setSelectedId(id); setRelationId(undefined); }}
        onNodeOpen={id => { setSelectedId(id); setRelationId(undefined); setExpanded(id); }}
        onEdgeClick={id => { setRelationId(id); setSelectedId(undefined); }} />}

      {detail && <aside className="graph-detail concept-detail">
        <button className="graph-detail__close" aria-label="关闭详情" onClick={() => setSelectedId(undefined)}><X size={16}/></button>
        <span className="graph-kind">{kindLabels[detail.kind]} · {statusLabels[detail.status]}</span>
        <h2>{detail.canonicalName}</h2>
        <p>{detail.description || "（暂无描述）"}</p>
        {detail.aliases.length > 0 && <div className="relation-keywords">{detail.aliases.map(alias => <span key={alias}>{alias}</span>)}</div>}
        <div className="concept-detail__edit">
          <label>重命名<input aria-label="重命名" value={nameDraft} onChange={event => setNameDraft(event.target.value)} /></label>
          <Button variant="secondary" disabled={busy || !nameDraft.trim() || nameDraft === detail.canonicalName} onClick={() => void correct({ canonicalName: nameDraft.trim() })}>保存名称</Button>
          <label>添加别名<input aria-label="添加别名" value={aliasDraft} onChange={event => setAliasDraft(event.target.value)} placeholder="同义词" /></label>
          <Button variant="secondary" disabled={busy || !aliasDraft.trim()} onClick={() => void correct({ aliases: [aliasDraft.trim()] })}>添加别名</Button>
        </div>
        <div className="graph-detail__actions">
          <button disabled={busy} onClick={() => void correct({ status: "confirmed" })}><Check size={14}/>确认</button>
          <button disabled={busy} onClick={() => void correct({ status: "hidden" })}><EyeOff size={14}/>隐藏</button>
          <button disabled={busy} onClick={() => { setExpanded(detail.id); setFitRequest(value => value + 1); }}><Focus size={14}/>展开邻居</button>
        </div>
        {detail.aliases.length + 1 < graph.entities.length && <label className="concept-merge">
          <span><Merge size={13}/>合并到</span>
          <select aria-label="合并到" value="" onChange={event => { if (event.target.value) void mergeInto(event.target.value); }} disabled={busy}>
            <option value="">选择要保留的概念…</option>
            {graph.entities.filter(item => item.id !== detail.id).map(item => <option key={item.id} value={item.id}>{item.canonicalName}</option>)}
          </select>
        </label>}
        <small>{detail.evidenceCount} 条证据 · 更新于 {detail.updatedAt || "未知"}</small>
        <div className="concept-evidence">
          <h3>证据笔记</h3>
          {detail.evidence.map(item => <button key={item.noteId} className="relation-evidence" onClick={() => openBook(item.bookId, item.noteId)} title={`置信度 ${Math.round(item.confidence * 100)}%`}>“{short(item.quote, 80)}”</button>)}
          {detail.evidence.length === 0 && <p className="graph-detail__empty">这个概念还没有证据笔记。</p>}
        </div>
      </aside>}

      {selectedRelation && (() => {
        const from = graph.entities.find(item => item.id === selectedRelation.fromEntityId);
        const to = graph.entities.find(item => item.id === selectedRelation.toEntityId);
        return <aside className="graph-detail graph-relation-detail">
          <button className="graph-detail__close" aria-label="关闭关系详情" onClick={() => setRelationId(undefined)}><X size={16}/></button>
          <span className="graph-kind">{relationLabels[selectedRelation.relation] ?? selectedRelation.relation}</span>
          <h2>{from?.canonicalName ?? "?"}<br/>{to?.canonicalName ?? "?"}</h2>
          <p>{selectedRelation.summary || "（无说明）"}</p>
          <small>置信度 {Math.round(selectedRelation.confidence * 100)}%</small>
          <div className="concept-evidence">
            <h3>关系证据</h3>
            {selectedRelation.evidence.map(item => <button key={item.noteId} className="relation-evidence" onClick={() => openBook(item.bookId, item.noteId)}>“{short(item.quote, 90)}”</button>)}
            {selectedRelation.evidence.length === 0 && <p className="graph-detail__empty">这条关系没有附带证据笔记。</p>}
          </div>
        </aside>;
      })()}

      {!!graph.entities.length && <div className="graph-color-legend concept-legend">
        <strong>图例</strong>
        <div><i className="concept-dot concept-dot--concept"/>概念<i className="concept-dot concept-dot--topic"/>主题<i className="concept-dot concept-dot--idea"/>想法</div>
        <div>双击节点展开邻居 · 单击查看定义与证据</div>
      </div>}
    </section>
  </div>;
}
