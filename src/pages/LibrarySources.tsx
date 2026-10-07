import { FileText, Globe2, Import, Loader2, RotateCcw, Trash2 } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { api } from "../api/tauri";
import { ImportSourceDialog, SourceDetailPanel } from "../components/import/ImportSourceDialog";
import { Button } from "../components/ui/Button";
import { PageHeader } from "../components/ui/PageHeader";
import { useAppStore } from "../stores/app";
import type { LibrarySource, SourceDetail, SourceType } from "../types/domain";

const typeLabels: Record<SourceType, string> = { web: "网页", pdf: "PDF", epub: "EPUB" };
const typeIcons: Record<SourceType, typeof Globe2> = { web: Globe2, pdf: FileText, epub: FileText };
const readError = (error: unknown) => (error instanceof Error ? error.message : String(error));

export function LibrarySources() {
  const { sourceDetailId, setSourceDetail } = useAppStore();
  const [sources, setSources] = useState<LibrarySource[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const [detail, setDetail] = useState<SourceDetail | null>(null);
  /** 等待二次确认的删除目标；为空表示不在确认态。 */
  const [confirmPurge, setConfirmPurge] = useState<LibrarySource | null>(null);
  const load = useCallback(async () => {
    setLoading(true); setError("");
    try { setSources(await api.librarySources()); }
    catch (reason) { setError(readError(reason)); }
    finally { setLoading(false); }
  }, []);

  useEffect(() => { void load(); }, [load]);
  useEffect(() => {
    const onChanged = () => { void load(); };
    window.addEventListener("library-sources-changed", onChanged);
    return () => window.removeEventListener("library-sources-changed", onChanged);
  }, [load]);

  // 从全局搜索点进来的资料详情：切到本页并拉取内容
  useEffect(() => {
    if (!sourceDetailId) { setDetail(null); return; }
    let active = true;
    setError("");
    api.librarySource(sourceDetailId)
      .then(value => { if (active) setDetail(value); })
      .catch(reason => { if (active) { setError(readError(reason)); setSourceDetail(undefined); } });
    return () => { active = false; };
  }, [sourceDetailId, setSourceDetail]);

  async function softDelete(source: LibrarySource) {
    setBusy(true); setError("");
    try { await api.deleteLibrarySource(source.id); setDetail(null); await load(); }
    catch (reason) { setError(readError(reason)); }
    finally { setBusy(false); }
  }

  async function purge() {
    if (!confirmPurge) return;
    setBusy(true); setError("");
    try {
      await api.purgeLibrarySource(confirmPurge.id);
      setConfirmPurge(null); setDetail(null);
      await load();
    } catch (reason) { setError(readError(reason)); }
    finally { setBusy(false); }
  }

  async function openDetail(source: LibrarySource) {
    setError("");
    try { setDetail(await api.librarySource(source.id)); setSourceDetail(source.id); }
    catch (reason) { setError(readError(reason)); }
  }

  return <>
    <PageHeader title="导入资料" subtitle="网页、PDF 与 EPUB 的本地副本，可被搜索与 AI 引用。" />
    <div className="library-sources">
      <div className="library-sources__toolbar">
        <Button variant="primary" onClick={() => setImportOpen(true)}><Import size={15} />导入资料</Button>
        <Button variant="secondary" onClick={() => void load()} disabled={loading}>{loading ? <Loader2 size={14} className="spin" /> : <RotateCcw size={14} />}刷新</Button>
        <span>{sources.length} 份资料</span>
      </div>

      {error && <div className="graph-refresh-error">操作失败：{error}</div>}

      {loading && !sources.length && <div className="graph-state">正在加载资料列表…</div>}
      {!loading && !sources.length && !error && <div className="graph-state"><FileText size={22} /><strong>还没有导入任何资料</strong><span>点击「导入资料」，把常读的网页、PDF 或 EPUB 收进本地，之后就能在搜索与 AI 回答里找到它们。</span></div>}

      <div className="library-sources__list">
        {sources.map(source => {
          const Icon = typeIcons[source.sourceType];
          return <article key={source.id} className="library-source">
            <button type="button" className="library-source__open" onClick={() => void openDetail(source)}>
              <Icon size={17} />
              <div>
                <strong>{source.title}</strong>
                <small>{typeLabels[source.sourceType]}{source.author ? ` · ${source.author}` : ""}{source.pageCount > 0 ? ` · ${source.pageCount} 页` : ""} · {source.documentCount} 个文档块</small>
                {source.origin && <em title={source.origin}>{source.origin}</em>}
              </div>
            </button>
            <div className="library-source__actions">
              <button type="button" onClick={() => void softDelete(source)} disabled={busy}><Trash2 size={14} />移除</button>
              <button type="button" className="danger" onClick={() => setConfirmPurge(source)} disabled={busy}>彻底删除</button>
            </div>
          </article>;
        })}
      </div>

      {confirmPurge && <div className="purge-confirm" role="alertdialog" aria-label="确认彻底删除">
        <div>
          <strong>彻底删除《{confirmPurge.title}》？</strong>
          <p>这会删除本地副本、全部文档块与搜索索引，无法撤销。如果只是想从列表隐藏，请用「移除」。</p>
        </div>
        <div>
          <Button variant="secondary" onClick={() => setConfirmPurge(null)} disabled={busy}>取消</Button>
          <Button variant="primary" onClick={() => void purge()} disabled={busy}>{busy ? <Loader2 size={15} className="spin" /> : "确认删除"}</Button>
        </div>
      </div>}

      {detail && <>
        <div className="library-source__reader" onClick={() => setDetail(null)} />
        <SourceDetailPanel source={detail} onClose={() => setDetail(null)} />
      </>}

      <ImportSourceDialog open={importOpen} onOpenChange={setImportOpen} />
    </div>
  </>;
}
