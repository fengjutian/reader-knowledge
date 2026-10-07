import { AlertTriangle, FileText, FolderOpen, Globe2, Loader2, Search, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
// 组件本身有个 `open` prop（控制对话框开关），这里必须取别名，
// 否则函数体里的 `open(...)` 会解析到那个布尔值上。
import { open as openFileDialog } from "@tauri-apps/plugin-dialog";
import { api } from "../../api/tauri";
import { Button } from "../ui/Button";
import type { ImportPreview, SourceDetail, SourceDocumentItem, SourceLocator, SourceType } from "../../types/domain";

const typeLabels: Record<SourceType, string> = { web: "网页", pdf: "PDF", epub: "EPUB" };
const readError = (error: unknown) => (error instanceof Error ? error.message : String(error));
/** 文件选择器只接受这两种，其余文件在系统对话框里就看不到。 */
const IMPORT_FILE_FILTERS = [{ name: "支持的资料", extensions: ["pdf", "epub"] }];

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

  async function previewFile(selectedPath = path) {
    const target = selectedPath.trim();
    if (!target) { setError("请先选择要导入的 PDF 或 EPUB 文件"); return; }
    setBusy(true); setError(""); setDone("");
    try {
      const result = await api.previewFileImport(target);
      setPreview(result); setTitle(result.title); setAuthor(result.author ?? "");
    } catch (reason) { setError(readError(reason)); setPreview(null); }
    finally { setBusy(false); }
  }

  /**
   * 调起系统原生文件选择框。
   *
   * `open()` 在用户取消时返回 null（不是抛错），这时什么都不做：
   * 不能弹错误，更不能把上一次的预览清掉。
   */
  async function chooseFile() {
    setBusy(true); setError(""); setDone("");
    try {
      const selected = await openFileDialog({ multiple: false, directory: false, filters: IMPORT_FILE_FILTERS });
      if (selected === null || selected === undefined) return;
      const picked = typeof selected === "string" ? selected : String(selected);
      // 换文件就是换内容：先清掉旧预览，避免拿着 A 的标题导入 B
      setPreview(null); setTitle(""); setAuthor("");
      setPath(picked);
      await previewFile(picked);
    } catch (reason) { setError(readError(reason)); }
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
        // 本地文件把预览时的指纹带回去：文件在这期间被换掉时后端会拒绝
        fileFingerprint: preview.sourceType === "web" ? undefined : preview.fileFingerprint,
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

  useEffect(() => {
    if (!open) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") onOpenChange(false);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [open, onOpenChange]);

  if (!open) return null;

  return <div className="import-dialog import-dialog--inline" role="dialog" aria-modal="false" aria-labelledby="import-dialog-title" aria-describedby="import-dialog-description">
        <div className="import-dialog__head">
          <div>
            <h2 id="import-dialog-title">导入资料</h2>
            <p id="import-dialog-description">把网页、PDF 或 EPUB 导入本地，之后可以搜索并用于 AI 回答。</p>
          </div>
          <button type="button" className="icon-button" aria-label="关闭" onClick={() => onOpenChange(false)}><X size={18} /></button>
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
          <div className="import-dialog__field">
            <span>本地文件（PDF / EPUB）</span>
            <div className="import-dialog__url">
              <FileText size={15} />
              <input value={path} readOnly placeholder="点击「选择文件」挑选要导入的 PDF 或 EPUB" tabIndex={-1} />
              <Button variant="secondary" onClick={() => void chooseFile()} disabled={busy} aria-busy={busy}>
                {busy ? <Loader2 size={14} className="spin" /> : <FolderOpen size={14} />}选择文件
              </Button>
            </div>
            <small>扫描版 PDF 没有文字层，会明确提示暂不支持 OCR，不会导入空资料。</small>
          </div>
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
  </div>;
}

/** 高亮持续的毫秒数：看得清，但不至于让人以为界面卡住。 */
const HIGHLIGHT_MS = 2_600;

/**
 * 按 locator 找出该定位到的文档块。
 *
 * 规则：页码优先 → 章节 → 标题（先精确后规范化）。同一页拆成多块时取第一块。
 * 定位不到就返回 undefined：定位是锦上添花，不能因为它让资料打不开。
 */
export function findLocatorTarget(documents: SourceDocumentItem[], locator?: SourceLocator): SourceDocumentItem | undefined {
  if (!locator) return undefined;
  if (locator.page !== undefined) {
    const byPage = documents.find(item => item.locator.page === locator.page);
    if (byPage) return byPage;
  }
  if (locator.chapter !== undefined) {
    const byChapter = documents.find(item => item.locator.chapter === locator.chapter);
    if (byChapter) return byChapter;
  }
  if (locator.heading) {
    const heading = locator.heading;
    const exact = documents.find(item => item.heading === heading || item.locator.heading === heading);
    if (exact) return exact;
    const normalized = normalizeHeading(heading);
    if (normalized) return documents.find(item => normalizeHeading(item.heading) === normalized || normalizeHeading(item.locator.heading) === normalized);
  }
  return undefined;
}

/** 标题比对用的宽松形式：去掉空白与常见的装饰符号，忽略大小写。 */
function normalizeHeading(value?: string): string {
  return (value ?? "").toLowerCase().replace(/[\s·・—\-–_、。，,.:：!！?？"'“”‘’()（）《》【】]/g, "");
}

export function SourceDetailPanel({ source, locator, onClose }: { source: SourceDetail; locator?: SourceLocator; onClose: () => void }) {
  const [targetId, setTargetId] = useState<string | null>(null);

  useEffect(() => {
    const target = findLocatorTarget(source.documents, locator);
    if (!target) { setTargetId(null); return; }
    setTargetId(target.id);
    // 渲染完成后再滚动：详情是异步取回的，直接滚会滚到一个还没画出来的元素
    const raf = requestAnimationFrame(() => {
      // 用 getElementById 而不是选择器拼接：文档 id 来自后端，拼进选择器要额外处理转义
      const node = document.getElementById(`source-document-${target.id}`);
      if (!node) return;
      const reduce = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches ?? false;
      node.scrollIntoView({ behavior: reduce ? "auto" : "smooth", block: "center" });
    });
    const timer = window.setTimeout(() => setTargetId(null), HIGHLIGHT_MS);
    return () => { cancelAnimationFrame(raf); clearTimeout(timer); };
  }, [source.documents, locator, source.id]);

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
      {source.documents.map(item => <article
        key={item.id}
        id={`source-document-${item.id}`}
        data-page={item.locator.page}
        data-chapter={item.locator.chapter}
        data-heading={item.locator.heading}
        className={item.id === targetId ? "source-detail__document--target" : undefined}
      >
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
