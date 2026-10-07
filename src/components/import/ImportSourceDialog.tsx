import * as Dialog from "@radix-ui/react-dialog";
import { AlertTriangle, FileText, Globe2, Loader2, Search, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { api } from "../../api/tauri";
import { Button } from "../ui/Button";
import type { ImportPreview, SourceDetail, SourceType } from "../../types/domain";

const typeLabels: Record<SourceType, string> = { web: "网页", pdf: "PDF", epub: "EPUB" };
const readError = (error: unknown) => (error instanceof Error ? error.message : String(error));

/**
 * 导入资料对话框。
 *
 * 流程固定为「填地址 / 选文件 → 预览 → 确认入库」：
 * 预览阶段就把标题、正文样本、警告和重复提示摆出来，
 * 用户不会在点确认之后才发现抓错了页面。
 */
export function ImportSourceDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (open: boolean) => void }) {
  const [tab, setTab] = useState<"web" | "file">("web");
  const [url, setUrl] = useState("");
  const [path, setPath] = useState("");
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [title, setTitle] = useState("");
  const [author, setAuthor] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [done, setDone] = useState("");

  const reset = useCallback(() => {
    setPreview(null); setTitle(""); setAuthor(""); setError(""); setDone("");
  }, []);

  async function previewWeb() {
    if (!url.trim()) { setError("请输入网页地址"); return; }
    setBusy(true); setError(""); setDone("");
    try {
      const result = await api.previewWebImport(url.trim());
      setPreview(result); setTitle(result.title); setAuthor(result.author ?? "");
    } catch (reason) { setError(readError(reason)); setPreview(null); }
    finally { setBusy(false); }
  }

  async function previewFile() {
    if (!path.trim()) { setError("请选择文件路径"); return; }
    setBusy(true); setError(""); setDone("");
    try {
      const result = await api.previewFileImport(path.trim());
      setPreview(result); setTitle(result.title); setAuthor(result.author ?? "");
    } catch (reason) { setError(readError(reason)); setPreview(null); }
    finally { setBusy(false); }
  }

  async function confirm() {
    if (!preview) return;
    setBusy(true); setError(""); setDone("");
    try {
      const result = await api.confirmImport({
        sourceType: preview.sourceType,
        title: title.trim() || preview.title,
        author: author.trim() || undefined,
        origin: preview.origin,
        pageCount: preview.pageCount,
        // 网页用 url 重新抓取；PDF / EPUB 用本地文件路径重新读取
        url: preview.sourceType === "web" ? url.trim() : undefined,
        path: preview.sourceType === "web" ? undefined : path.trim() || undefined,
      });
      if (result.duplicate) {
        // 重复内容不重复入库，把已有资料指回去
        setError("这份内容已经导入过了，没有重复添加");
        setDone("");
        return;
      }
      setDone(`已导入《${result.title}》，共 ${result.documentCount} 个文档块`);
      setPreview(null);
      window.dispatchEvent(new Event("library-sources-changed"));
    } catch (reason) { setError(readError(reason)); }
    finally { setBusy(false); }
  }

  useEffect(() => { if (!open) reset(); }, [open, reset]);

  return <Dialog.Root open={open} onOpenChange={value => { if (!value) reset(); onOpenChange(value); }}>
    <Dialog.Portal>
      <Dialog.Overlay className="dialog-overlay" />
      <Dialog.Content className="import-dialog">
        <div className="import-dialog__head">
          <div>
            <Dialog.Title>导入资料</Dialog.Title>
            <Dialog.Description>把网页、PDF 或 EPUB 导入本地，之后可以搜索并用于 AI 回答。</Dialog.Description>
          </div>
          <Dialog.Close className="icon-button" aria-label="关闭"><X size={18} /></Dialog.Close>
        </div>

        <div className="import-dialog__tabs" role="tablist" aria-label="导入方式">
          <button type="button" role="tab" aria-selected={tab === "web"} className={tab === "web" ? "active" : ""} onClick={() => { setTab("web"); reset(); }}><Globe2 size={15} />网页</button>
          <button type="button" role="tab" aria-selected={tab === "file"} className={tab === "file" ? "active" : ""} onClick={() => { setTab("file"); reset(); }}><FileText size={15} />本地文件</button>
        </div>

        {tab === "web" ? (
          <label className="import-dialog__field">
            <span>网页地址（仅支持 HTTPS）</span>
            <div className="import-dialog__url">
              <Search size={15} />
              <input value={url} onChange={event => setUrl(event.target.value)} placeholder="https://example.com/article" onKeyDown={event => { if (event.key === "Enter") void previewWeb(); }} />
              <Button variant="secondary" onClick={() => void previewWeb()} disabled={busy || !url.trim()}>{busy ? <Loader2 size={14} className="spin" /> : "预览"}</Button>
            </div>
            <small>出于安全考虑，不支持抓取本机与内网地址；跟随重定向时也会逐跳校验。</small>
          </label>
        ) : (
          <label className="import-dialog__field">
            <span>本地文件（PDF / EPUB）</span>
            <div className="import-dialog__url">
              <FileText size={15} />
              <input value={path} onChange={event => setPath(event.target.value)} placeholder="C:\\Users\\you\\book.epub" onKeyDown={event => { if (event.key === "Enter") void previewFile(); }} />
              <Button variant="secondary" onClick={() => void previewFile()} disabled={busy || !path.trim()}>{busy ? <Loader2 size={14} className="spin" /> : "预览"}</Button>
            </div>
            <small>扫描版 PDF 没有文字层，会明确提示暂不支持 OCR，不会导入空资料。</small>
          </label>
        )}

        {error && <div className="import-dialog__error"><AlertTriangle size={14} />{error}</div>}
        {done && <div className="import-dialog__done">{done}</div>}

        {preview && (
          <div className="import-dialog__preview">
            <div className="import-dialog__fields">
              <label><span>标题</span><input value={title} onChange={event => setTitle(event.target.value)} /></label>
              <label><span>作者</span><input value={author} onChange={event => setAuthor(event.target.value)} placeholder="可留空" /></label>
            </div>
            <div className="import-dialog__meta">
              <span>{typeLabels[preview.sourceType]}</span>
              {preview.pageCount > 0 && <span>共 {preview.pageCount} 页</span>}
              <span>{preview.documentCount} 个文档块</span>
              {preview.origin && <span title={preview.origin}>{preview.origin}</span>}
            </div>
            {preview.warnings.length > 0 && <ul className="import-dialog__warnings">{preview.warnings.map((warning, index) => <li key={index}>{warning}</li>)}</ul>}
            {preview.duplicate && <div className="import-dialog__error"><AlertTriangle size={14} />这份内容已经导入过，确认后不会重复添加。</div>}
            <div className="import-dialog__sample">
              <h3>正文样本</h3>
              {preview.sample.map((item, index) => <article key={`${item.position}-${index}`}><strong>{item.heading || "正文"}</strong><p>{item.content}</p></article>)}
            </div>
            <div className="import-dialog__footer">
              <Button variant="secondary" onClick={() => setPreview(null)} disabled={busy}>重新选择</Button>
              <Button variant="primary" onClick={() => void confirm()} disabled={busy || preview.duplicate || !title.trim()}>{busy ? <Loader2 size={15} className="spin" /> : "确认导入"}</Button>
            </div>
          </div>
        )}
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>;
}

export function SourceDetailPanel({ source, onClose }: { source: SourceDetail; onClose: () => void }) {
  return <div className="source-detail">
    <div className="source-detail__head">
      <div>
        <span className="graph-kind">{typeLabels[source.sourceType]}{source.pageCount > 0 ? ` · ${source.pageCount} 页` : ""}</span>
        <h2>{source.title}</h2>
        {source.author && <p>{source.author}</p>}
        {source.origin && <small className="source-detail__origin" title={source.origin}>{source.origin}</small>}
      </div>
      <button type="button" className="icon-button" aria-label="关闭资料详情" onClick={onClose}><X size={17} /></button>
    </div>
    <div className="source-detail__docs">
      {source.documents.map(item => <article key={item.id}>
        <strong>
          {item.heading || "正文"}
          {item.locator.page !== undefined && <em>第 {item.locator.page} 页</em>}
          {item.locator.chapter !== undefined && <em>第 {item.locator.chapter} 章</em>}
        </strong>
        <p>{item.content}</p>
      </article>)}
      {source.documents.length === 0 && <p className="graph-detail__empty">这份资料没有可显示的文档块。</p>}
    </div>
  </div>;
}
