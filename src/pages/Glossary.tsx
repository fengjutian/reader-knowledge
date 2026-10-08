import * as Dialog from "@radix-ui/react-dialog";
import { AlertTriangle, Check, ExternalLink, Eye, Filter, Pause, Play, Plus, RefreshCw, Search, Trash2, UploadCloud, X, XCircle } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api/tauri";
import { PageHeader } from "../components/ui/PageHeader";
import { DataTable, type DataTableColumn } from "../components/ui/DataTable";
import { Button } from "../components/ui/Button";
import type { GlossaryImportJob, GlossaryImportJobStatus, GlossarySource, GlossaryStatus, GlossaryTerm, WikipediaCandidate } from "../types/domain";

const empty = (): GlossaryTerm => ({
  id: 0, term: "", canonicalName: "", aliases: [], definition: "", source: "manual", sourceTitle: "", sourceUrl: "",
  wikipediaSnapshot: "", status: "confirmed", updatedAt: 0, externalPageId: 0, sourceRevisionId: 0,
  sourceDumpVersion: "", sourceUpdatedAt: 0, sourceSyncedAt: 0, licenseCode: "", manuallyEdited: false,
  sourceContentHash: "", publishedBatchId: "",
});

const SOURCE_LABEL: Record<GlossarySource, string> = { manual: "人工编辑", wikipedia: "维基百科", other: "其他来源" };
const STATUS_LABEL: Record<GlossaryStatus, string> = {
  pending: "待确认", confirmed: "已确认", ignored: "已忽略", conflict: "同名冲突", source_missing: "来源失效",
};
const JOB_STATUS_LABEL: Record<GlossaryImportJobStatus, string> = {
  pending: "待开始", downloading: "下载中", verifying: "校验中", parsing: "解析中", resolving_redirects: "解析重定向",
  validating: "批次校验", ready_to_publish: "待发布", publishing: "发布中", completed: "已完成", paused: "已暂停",
  cancelled: "已取消", failed: "失败",
};
const RUNNING: GlossaryImportJobStatus[] = ["downloading", "verifying", "parsing", "resolving_redirects", "validating", "publishing"];

const formatTime = (seconds: number) => (seconds > 0 ? new Date(seconds * 1000).toLocaleString("zh-CN") : "—");
const formatBytes = (bytes: number) => (bytes > 0 ? `${(bytes / 1024 / 1024).toFixed(1)} MB` : "—");

export function Glossary() {
  const [items, setItems] = useState<GlossaryTerm[]>([]);
  const [editing, setEditing] = useState<GlossaryTerm>(empty());
  const [query, setQuery] = useState("");
  const [sourceFilter, setSourceFilter] = useState<GlossarySource | "">("");
  const [statusFilter, setStatusFilter] = useState<GlossaryStatus | "">("");
  const [candidates, setCandidates] = useState<WikipediaCandidate[]>([]);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const termInputRef = useRef<HTMLInputElement>(null);

  const [jobs, setJobs] = useState<GlossaryImportJob[]>([]);
  const [importOpen, setImportOpen] = useState(false);
  const [importLimit, setImportLimit] = useState("1000");
  const [importLocalFile, setImportLocalFile] = useState("");
  const [importAutoPublish, setImportAutoPublish] = useState(false);

  const load = useCallback(() => api.glossaryTerms(query, sourceFilter, statusFilter).then(setItems).catch(reason => setError(String(reason))), [query, sourceFilter, statusFilter]);
  const loadImports = useCallback(() => api.glossaryImports(20).then(setJobs).catch(() => undefined), []);

  useEffect(() => { void load(); }, [load]);
  useEffect(() => { void loadImports(); }, [loadImports]);
  // 有任务在跑时才轮询进度；全部结束就停掉，不空转。
  useEffect(() => {
    if (!jobs.some(job => RUNNING.includes(job.status))) return;
    const timer = window.setInterval(() => { void loadImports(); }, 1500);
    return () => window.clearInterval(timer);
  }, [jobs, loadImports]);

  function create() { setEditing(empty()); setCandidates([]); setError(""); setDrawerOpen(true); requestAnimationFrame(() => termInputRef.current?.focus()); }
  function view(item: GlossaryTerm) { setEditing(item); setCandidates([]); setError(""); setDrawerOpen(true); }
  async function wiki() { if (!editing.term.trim()) return; setBusy(true); setError(""); try { setCandidates(await api.searchWikipedia(editing.term.trim())); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } }
  function choose(item: WikipediaCandidate) { const definition = item.excerpt.trim() || item.description.trim(); setEditing(value => ({ ...value, canonicalName: item.title, definition, source: "wikipedia", sourceTitle: item.title, sourceUrl: item.url, wikipediaSnapshot: definition, status: "confirmed" })); setCandidates([]); }
  async function save() { setBusy(true); setError(""); try { const savedTerm = editing.term.trim(); await api.saveGlossaryTerm({ ...editing, term: savedTerm, canonicalName: editing.canonicalName || savedTerm }); setItems(await api.glossaryTerms(query, sourceFilter, statusFilter)); setDrawerOpen(false); setCandidates([]); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } }
  async function remove(id: number) { setBusy(true); setError(""); try { await api.deleteGlossaryTerm(id); setDrawerOpen(false); setEditing(empty()); await load(); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } }
  async function changeStatus(id: number, status: GlossaryStatus) { setBusy(true); setError(""); try { await api.setGlossaryTermStatus(id, status); await load(); setEditing(value => value.id === id ? { ...value, status } : value); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } }
  async function confirmAllPending() {
    const ids = items.filter(item => item.status === "pending").map(item => item.id);
    if (ids.length === 0) return;
    setBusy(true); setError("");
    try { await api.bulkGlossaryTermStatus(ids, "confirmed"); await load(); } catch (reason) { setError(String(reason)); } finally { setBusy(false); }
  }

  async function startImport() {
    setBusy(true); setError("");
    try {
      const limit = Number.parseInt(importLimit, 10);
      await api.createGlossaryImport({
        mode: "summary",
        autoPublish: importAutoPublish,
        maxItems: Number.isFinite(limit) && limit > 0 ? limit : undefined,
        localFile: importLocalFile.trim() || undefined,
      });
      await loadImports();
    } catch (reason) { setError(String(reason)); } finally { setBusy(false); }
  }
  async function jobAction(action: "pause" | "resume" | "cancel" | "publish", id: string) {
    setBusy(true); setError("");
    try {
      if (action === "pause") await api.pauseGlossaryImport(id);
      if (action === "resume") await api.resumeGlossaryImport(id);
      if (action === "cancel") await api.cancelGlossaryImport(id);
      if (action === "publish") await api.publishGlossaryImport(id);
      await loadImports();
    } catch (reason) { setError(String(reason)); } finally { setBusy(false); }
  }

  const pendingCount = useMemo(() => items.filter(item => item.status === "pending").length, [items]);
  const hasFilter = Boolean(sourceFilter || statusFilter);

  const columns: DataTableColumn<GlossaryTerm>[] = [
    { key: "term", header: "名词", render: item => <div className="glossary-term-cell"><strong>{item.term}</strong><small>{item.definition}</small></div> },
    { key: "canonicalName", header: "标准名称", width: 180, render: item => item.canonicalName || "—" },
    { key: "source", header: "来源", width: 110, render: item => <span>{SOURCE_LABEL[item.source]}{item.source === "wikipedia" && item.manuallyEdited ? "（已编辑）" : ""}</span> },
    { key: "status", header: "状态", width: 96, render: item => <span className={`glossary-status glossary-status--${item.status}`}>{STATUS_LABEL[item.status]}</span> },
    { key: "actions", header: "操作", width: 150, align: "center", render: item => <div className="glossary-row-actions">
      <Button variant="secondary" className="glossary-view" icon={<Eye size={14}/>} onClick={() => view(item)}>查看</Button>
      {item.status !== "confirmed" && <Button variant="secondary" className="glossary-icon-button" title="确认" onClick={() => void changeStatus(item.id, "confirmed")}><Check size={14}/></Button>}
      {item.status !== "ignored" && <Button variant="secondary" className="glossary-icon-button" title="忽略" onClick={() => void changeStatus(item.id, "ignored")}><XCircle size={14}/></Button>}
    </div> },
  ];

  return <>
    <PageHeader title="名词库" subtitle="维基百科负责初始化，人工编辑内容优先生效；解释只作为 AI 背景。" actions={<>
      <Button variant="secondary" icon={<UploadCloud size={15}/>} onClick={() => setImportOpen(value => !value)}>维基导入</Button>
      <Button icon={<Plus size={15}/>} onClick={create}>新建名词</Button>
    </>}/>

    {importOpen && <section className="glossary-import">
      <header className="glossary-import__head">
        <strong>维基百科导入</strong>
        <button className="icon-button" aria-label="关闭导入面板" onClick={() => setImportOpen(false)}><X size={17}/></button>
      </header>
      <p className="glossary-import__note">从官方中文维基 dump 导入候选名词，默认进入待确认。填写处理上限时自动下载约 255 MB 的首个正文分片；清空上限才会下载约 3.43 GB 的完整正文包。</p>
      <div className="glossary-import__form">
        <label>最多处理<input type="number" min={1} value={importLimit} onChange={event => setImportLimit(event.target.value)}/></label>
        <label>本地 dump 文件（可选）<input value={importLocalFile} placeholder="D:\dump\zhwiki-latest-pages-articles1.xml-p1p187712.bz2" onChange={event => setImportLocalFile(event.target.value)}/></label>
        <label className="glossary-import__check"><input type="checkbox" checked={importAutoPublish} onChange={event => setImportAutoPublish(event.target.checked)}/>校验通过后自动发布</label>
        <Button icon={<UploadCloud size={15}/>} disabled={busy} onClick={() => void startImport()}>开始导入</Button>
        <Button variant="secondary" icon={<RefreshCw size={15}/>} onClick={() => void loadImports()}>刷新</Button>
      </div>
      {jobs.length > 0 && <div className="glossary-import__jobs">{jobs.map(job => <ImportJobRow key={job.id} job={job} busy={busy} onAction={(action) => void jobAction(action, job.id)}/>)}</div>}
      {jobs.length === 0 && <p className="glossary-import__empty">还没有导入任务。</p>}
    </section>}

    <section className="glossary-list glossary-list--full">
      <label className="field field--search"><Search size={16}/><input value={query} onChange={event => setQuery(event.target.value)} placeholder="搜索名词、标准名称、别名或解释"/></label>
      <div className="glossary-filters">
        <Filter size={14}/>
        <select value={sourceFilter} onChange={event => setSourceFilter(event.target.value as GlossarySource | "")} aria-label="按来源筛选">
          <option value="">全部来源</option>
          <option value="manual">人工</option>
          <option value="wikipedia">维基百科</option>
          <option value="other">其他</option>
        </select>
        <select value={statusFilter} onChange={event => setStatusFilter(event.target.value as GlossaryStatus | "")} aria-label="按状态筛选">
          <option value="">全部状态</option>
          <option value="pending">待确认</option>
          <option value="confirmed">已确认</option>
          <option value="ignored">已忽略</option>
          <option value="conflict">同名冲突</option>
          <option value="source_missing">来源失效</option>
        </select>
        {pendingCount > 0 && <Button variant="secondary" disabled={busy} icon={<Check size={14}/>} onClick={() => void confirmAllPending()}>确认当前 {pendingCount} 条待确认</Button>}
        {hasFilter && <button className="glossary-filters__clear" onClick={() => { setSourceFilter(""); setStatusFilter(""); }}>清除筛选</button>}
      </div>
      <DataTable columns={columns} rows={items} rowKey={item => item.id} empty={hasFilter ? "没有符合筛选条件的名词" : "暂无名词"}/>
    </section>

    <Dialog.Root open={drawerOpen} onOpenChange={setDrawerOpen}><Dialog.Portal>
      <Dialog.Overlay className="glossary-drawer__overlay"/>
      <Dialog.Content className="glossary-drawer">
        <header className="glossary-drawer__head"><div><Dialog.Title>{editing.id > 0 ? "查看与编辑名词" : "新建名词"}</Dialog.Title><Dialog.Description>{editing.id > 0 ? "修改后保存将立即用于 AI 问答" : "填写名词后可优先从维基百科获取解释"}</Dialog.Description></div><Dialog.Close className="icon-button" aria-label="关闭"><X size={19}/></Dialog.Close></header>
        <div className="glossary-editor">
          <label>名词<div className="glossary-term-input"><input ref={termInputRef} value={editing.term} onChange={event => setEditing({ ...editing, term: event.target.value })} placeholder="输入需要解释的名词"/><button type="button" disabled={busy || !editing.term.trim()} onClick={() => void wiki()}><Search size={14}/>{busy ? "正在搜索…" : "优先从维基百科获取"}</button></div></label>
          <label>标准名称<input value={editing.canonicalName} onChange={event => setEditing({ ...editing, canonicalName: event.target.value })}/></label>
          <label>别名<input value={editing.aliases.join("、")} onChange={event => setEditing({ ...editing, aliases: event.target.value.split(/[、,，]/).map(value => value.trim()).filter(Boolean) })}/></label>
          {candidates.length > 0 && <div className="glossary-candidates">{candidates.map(item => <button key={item.title} onClick={() => choose(item)}><strong>{item.title}</strong><span>{item.description || item.excerpt}</span></button>)}</div>}
          <label>解释<textarea rows={10} value={editing.definition} onChange={event => setEditing({ ...editing, definition: event.target.value })}/></label>
          {editing.source === "wikipedia" && <Attribution term={editing}/>}
          {editing.manuallyEdited && <p className="glossary-edited-hint"><AlertTriangle size={14}/>这条解释已被人工编辑，后续维基同步只更新来源快照，不会覆盖你写的内容。</p>}
          {error && <p className="glossary-error">{error}</p>}
        </div>
        <footer className="glossary-drawer__footer">
          {editing.id > 0 && <button className="danger" disabled={busy} onClick={() => void remove(editing.id)}><Trash2 size={14}/>删除</button>}
          {editing.id > 0 && editing.status !== "ignored" && <button disabled={busy} onClick={() => void changeStatus(editing.id, "ignored")}>忽略</button>}
          <button className="button button--primary" disabled={busy || !editing.term.trim() || !editing.definition.trim()} onClick={() => void save()}>保存并确认</button>
        </footer>
      </Dialog.Content>
    </Dialog.Portal></Dialog.Root>
  </>;
}

/** 来源与许可证展示（需求 12）。CC BY-SA 4.0 必须可见且可点。 */
function Attribution({ term }: { term: GlossaryTerm }) {
  return <div className="glossary-attribution">
    <p>本文摘要节选并整理自维基百科“{term.sourceTitle || term.term}”词条，原文由其贡献者共同创作，依据 <a href="https://creativecommons.org/licenses/by-sa/4.0/" target="_blank" rel="noreferrer">CC BY-SA 4.0</a> 许可使用。内容可能已进行格式清理或截断。</p>
    <dl>
      <div><dt>词条 ID</dt><dd>{term.externalPageId || "—"}</dd></div>
      <div><dt>修订 ID</dt><dd>{term.sourceRevisionId || "—"}</dd></div>
      <div><dt>dump 版本</dt><dd>{term.sourceDumpVersion || "—"}</dd></div>
      <div><dt>原文修订时间</dt><dd>{formatTime(term.sourceUpdatedAt)}</dd></div>
      <div><dt>本地同步时间</dt><dd>{formatTime(term.sourceSyncedAt)}</dd></div>
      <div><dt>许可证</dt><dd>{term.licenseCode || "—"}</dd></div>
    </dl>
    {term.sourceUrl && <button type="button" onClick={() => void api.openExternalUrl(term.sourceUrl)}><ExternalLink size={14}/>查看原文与贡献历史</button>}
    {term.manuallyEdited && term.wikipediaSnapshot && term.wikipediaSnapshot !== term.definition && <details className="glossary-attribution__diff">
      <summary>对比最新维基摘要与当前展示内容</summary>
      <p><strong>当前展示：</strong>{term.definition}</p>
      <p><strong>维基快照：</strong>{term.wikipediaSnapshot}</p>
    </details>}
  </div>;
}

function ImportJobRow({ job, busy, onAction }: { job: GlossaryImportJob; busy: boolean; onAction: (action: "pause" | "resume" | "cancel" | "publish") => void }) {
  const progress = job.totalBytes > 0 ? Math.min(100, Math.round((job.downloadedBytes / job.totalBytes) * 100)) : 0;
  const running = RUNNING.includes(job.status);
  return <article className="glossary-import__job">
    <header>
      <span className={`glossary-import__badge glossary-import__badge--${job.status}`}>{JOB_STATUS_LABEL[job.status]}</span>
      <code title={job.id}>{job.id.slice(0, 8)}</code>
      <time>{formatTime(job.createdAt)}</time>
    </header>
    {job.status === "downloading" && <progress className="glossary-import__progress" value={progress} max={100}/>}
    <dl>
      <div><dt>扫描</dt><dd>{job.scannedCount}</dd></div>
      <div><dt>有效</dt><dd>{job.acceptedCount}</dd></div>
      <div><dt>重定向</dt><dd>{job.redirectCount}</dd></div>
      <div><dt>过滤</dt><dd>{job.filteredCount}</dd></div>
      <div><dt>新增</dt><dd>{job.insertedCount}</dd></div>
      <div><dt>更新</dt><dd>{job.updatedCount}</dd></div>
      <div><dt>冲突</dt><dd>{job.conflictCount}</dd></div>
      <div><dt>错误</dt><dd>{job.errorCount}</dd></div>
      <div><dt>下载</dt><dd>{formatBytes(job.downloadedBytes)} / {formatBytes(job.totalBytes)}</dd></div>
      <div><dt>版本</dt><dd>{job.dumpVersion}</dd></div>
    </dl>
    {job.errorMessage && <p className="glossary-import__error">{job.errorMessage}</p>}
    <footer>
      {running && <Button variant="secondary" disabled={busy} icon={<Pause size={13}/>} onClick={() => onAction("pause")}>暂停</Button>}
      {job.status === "paused" && <Button variant="secondary" disabled={busy} icon={<Play size={13}/>} onClick={() => onAction("resume")}>继续</Button>}
      {job.status === "ready_to_publish" && <Button disabled={busy} icon={<Check size={13}/>} onClick={() => onAction("publish")}>发布本批</Button>}
      {(running || job.status === "paused" || job.status === "pending") && <Button variant="secondary" disabled={busy} icon={<X size={13}/>} onClick={() => onAction("cancel")}>取消</Button>}
    </footer>
  </article>;
}
