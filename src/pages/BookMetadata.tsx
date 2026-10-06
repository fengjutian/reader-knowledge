import { CheckSquare, Eye, RefreshCw, Search, Square } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "../api/tauri";
import { BookMetadataDialog } from "../components/metadata/BookMetadataDialog";
import { Button } from "../components/ui/Button";
import { Message, type MessageValue } from "../components/ui/Message";
import { PageHeader } from "../components/ui/PageHeader";
import { useLibraryRevision } from "../hooks/useLibraryRevision";
import { useAppStore } from "../stores/app";
import type { BookMetadataRow, BookMetadataSourceDetail, MetadataFetchResult } from "../types/domain";

const labels: Record<string, string> = { weread: "微信读书", open_library: "Open Library", google_books: "Google Books", douban: "豆瓣", smart: "智能补全", manual: "手动" };
// 后端 fetch_books_metadata 单批上限为 20，前端与之一致。
const maxBatch = 20;
const pageSize = 100;
const visibleSources = ["weread", "douban", "open_library", "google_books", "smart"] as const;
type VisibleSource = typeof visibleSources[number];

function coreTitle(title: string) { return title.split(/[：:（(【[]/, 1)[0].trim(); }
function readableError(error: unknown) {
  const text = error instanceof Error ? error.message : String(error);
  if (text.includes("429")) return "请求过于频繁，请稍后再试。";
  return text.length > 180 ? `${text.slice(0, 180)}…` : text;
}

/** 汇总批量补全结果：单本失败不会让整批结果消失。 */
function summarize(results: MetadataFetchResult[]) {
  const succeeded = results.filter(result => result.status === "updated" || result.status === "cached").length;
  return { succeeded, failed: results.length - succeeded };
}

export function BookMetadata() {
  const [items, setItems] = useState<BookMetadataRow[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");
  const [source, setSource] = useState<VisibleSource>("weread");
  const [message, setMessage] = useState<MessageValue>();
  const [loading, setLoading] = useState(true);
  const [refreshing, setRefreshing] = useState(false);
  const [page, setPage] = useState(0);
  const [viewBook, setViewBook] = useState<BookMetadataRow>();
  const [details, setDetails] = useState<BookMetadataSourceDetail[]>([]);
  const openBook = useAppStore(state => state.openBook);
  const closeMessage = useCallback(() => setMessage(undefined), []);

  // 刷新时不重置 loading，也不清空 items，避免表格闪烁。
  const load = useCallback(async (options?: { quiet?: boolean }) => {
    if (options?.quiet) setRefreshing(true); else setLoading(true);
    try { const rows = await api.bookMetadata(source); setItems(rows); }
    catch (error) { setMessage({ kind: "error", text: readableError(error) }); }
    finally { setLoading(false); setRefreshing(false); }
  }, [source]);
  useEffect(() => { void load(); }, [load]);
  // 同步成功后静默重新拉取，保留搜索条件、来源选择与分页状态。
  useLibraryRevision(useCallback(() => { void load({ quiet: true }); }, [load]));

  const filtered = useMemo(() => items.filter(item => `${item.title} ${item.author} ${item.isbn} ${item.publisher}`.toLocaleLowerCase().includes(query.toLocaleLowerCase())), [items, query]);
  const pageCount = Math.max(1, Math.ceil(filtered.length / pageSize));
  const visible = filtered.slice(page * pageSize, (page + 1) * pageSize);
  const selectedVisibleCount = visible.reduce((count, item) => count + Number(selected.has(item.bookId)), 0);
  const visibleSelectionTarget = Math.min(visible.length, maxBatch);
  const allVisibleSelected = visibleSelectionTarget > 0 && selectedVisibleCount === visibleSelectionTarget;
  useEffect(() => setPage(0), [query]);
  // 数据变少导致当前页越界时，回退到最后一页。
  useEffect(() => { if (page > pageCount - 1) setPage(pageCount - 1); }, [page, pageCount]);

  function toggleAll() {
    setSelected(current => {
      const next = new Set(current);
      if (allVisibleSelected) { visible.forEach(item => next.delete(item.bookId)); return next; }
      for (const item of visible) { if (next.size >= maxBatch) break; next.add(item.bookId); }
      if (visible.some(item => !next.has(item.bookId))) setMessage({ kind: "info", text: `每次最多补全 ${maxBatch} 本书。` });
      return next;
    });
  }

  function toggle(book: BookMetadataRow) {
    setSelected(current => {
      const next = new Set(current);
      if (next.has(book.bookId)) next.delete(book.bookId);
      else if (next.size < maxBatch) next.add(book.bookId);
      else setMessage({ kind: "info", text: `每次最多补全 ${maxBatch} 本书。` });
      return next;
    });
  }

  // 豆瓣需要按书名/作者搜索或自定义 URL，仍逐本处理，但复用统一的进度与统计逻辑。
  async function pullDouban(ids: string[]) {
    if (!ids.length) return;
    if (ids.length > maxBatch) { setMessage({ kind: "error", text: `豆瓣每次最多补全 ${maxBatch} 本书。` }); return; }
    setBusy(new Set(ids));
    setMessage({ kind: "info", text: `正在从豆瓣补全 ${ids.length} 本书…` });
    const results: MetadataFetchResult[] = [];
    for (const id of ids) {
      const book = items.find(item => item.bookId === id);
      if (!book) { results.push({ bookId: id, source: "douban", status: "failed", message: "列表中找不到该书籍" }); continue; }
      try {
        results.push(await api.fetchDoubanBookMetadata(id, `${coreTitle(book.title)} ${book.author}`.trim()));
      } catch (error) { results.push({ bookId: id, source: "douban", status: "failed", message: readableError(error) }); }
    }
    const { succeeded, failed } = summarize(results);
    setMessage({ kind: failed ? "error" : "success", text: `豆瓣补全完成：成功 ${succeeded}，失败 ${failed}。` });
    await load({ quiet: true });
    setBusy(new Set());
  }

  // 微信读书走批量接口：单本失败不影响同批次其他结果。
  async function pullWeread(ids: string[]) {
    if (!ids.length) return;
    if (ids.length > maxBatch) { setMessage({ kind: "error", text: `每次最多补全 ${maxBatch} 本书。` }); return; }
    setBusy(new Set(ids));
    setMessage({ kind: "info", text: `正在从微信读书补全 ${ids.length} 本书…` });
    let results: MetadataFetchResult[];
    try {
      results = await api.fetchBooksMetadata(ids, "weread", true);
    } catch (error) {
      setMessage({ kind: "error", text: readableError(error) });
      setBusy(new Set());
      return;
    }
    const { succeeded, failed } = summarize(results);
    setMessage({ kind: failed ? "error" : "success", text: `微信读书补全完成：成功 ${succeeded}，失败 ${failed}。` });
    await load({ quiet: true });
    setBusy(new Set());
  }

  // Open Library / Google Books / 智能补全都由后端批量接口统一处理。
  async function pullRemote(ids: string[], target: Exclude<VisibleSource, "weread" | "douban">) {
    if (!ids.length) return;
    if (ids.length > maxBatch) { setMessage({ kind: "error", text: `每次最多补全 ${maxBatch} 本书。` }); return; }
    setBusy(new Set(ids));
    setMessage({ kind: "info", text: `正在从 ${labels[target]} 补全 ${ids.length} 本书…` });
    let results: MetadataFetchResult[];
    try { results = await api.fetchBooksMetadata(ids, target, true); }
    catch (error) { setMessage({ kind: "error", text: readableError(error) }); setBusy(new Set()); return; }
    const { succeeded, failed } = summarize(results);
    setMessage({ kind: failed ? "error" : "success", text: `${labels[target]} 补全完成：成功 ${succeeded}，失败 ${failed}。` });
    await load({ quiet: true });
    setBusy(new Set());
  }

  const pullSelectedSource = (ids: string[]) => {
    if (source === "weread") return pullWeread(ids);
    if (source === "douban") return pullDouban(ids);
    return pullRemote(ids, source);
  };

  async function view(book: BookMetadataRow) {
    setViewBook(book); setDetails([]);
    try { setDetails(await api.bookMetadataDetails(book.bookId)); }
    catch (error) { setViewBook(undefined); setMessage({ kind: "error", text: readableError(error) }); }
  }

  return <>
    <Message value={message} onClose={closeMessage}/>
    <PageHeader title="书籍元数据" subtitle={`${items.length} 本书；当前展示${labels[source]}字段，各来源详情独立保存。`} actions={<Button icon={<RefreshCw size={15}/>} disabled={!selected.size || busy.size > 0} onClick={() => void pullSelectedSource([...selected])}>{labels[source]}补全（{selected.size}/{maxBatch}）</Button>}/>
    <div className="toolbar metadata-toolbar"><label className="field field--search"><Search size={16}/><input value={query} onChange={event => setQuery(event.target.value)} placeholder="搜索书名、作者、ISBN 或出版社"/></label><label className="field metadata-source"><span>数据源</span><select value={source} onChange={event => { setSource(event.target.value as VisibleSource); setSelected(new Set()); }}>{visibleSources.map(item => <option key={item} value={item}>{labels[item]}</option>)}</select></label>{refreshing && <span className="field-hint">正在刷新…</span>}</div>
    <div className="douban-import"><div><strong>按来源批量补全</strong><small>微信读书、Open Library、Google Books 与智能补全走同一批量接口；豆瓣按书名与作者搜索。单个来源分别保存，不合并字段。</small></div><span className="douban-import__limit">已选 {selected.size} / {maxBatch}</span><div className="metadata-source-actions">{visibleSources.map(item => <Button key={item} variant="secondary" disabled={!selected.size || busy.size > 0} onClick={() => void pullSelectedSource(item === source ? [...selected] : [...selected])}>{labels[item]}</Button>)}</div></div>
    {loading ? <div className="notes-loading">正在读取书籍…</div> : <div className="metadata-table-wrap">
      <table className="metadata-table">
        <colgroup><col className="col-check"/><col className="col-title"/><col className="col-author"/><col className="col-isbn"/><col className="col-publisher"/><col className="col-pages"/><col className="col-subjects"/><col className="col-sources"/><col className="col-view"/><col className="col-actions"/></colgroup>
        <thead><tr><th><button className="table-check" onClick={toggleAll}>{allVisibleSelected ? <CheckSquare size={17}/> : <Square size={17}/>}</button></th><th>书名</th><th>作者</th><th>ISBN</th><th>出版社 / 时间</th><th>页数</th><th>主题</th><th>数据源</th><th>查看</th><th>操作</th></tr></thead>
        <tbody>{visible.map(book => <tr key={book.bookId}>
          <td><button className="table-check" onClick={() => toggle(book)}>{selected.has(book.bookId) ? <CheckSquare size={17}/> : <Square size={17}/>}</button></td>
          <td><button className="book-title-link" onClick={() => openBook(book.bookId)}>{book.title}</button></td>
          <td><span className="metadata-ellipsis">{book.author || "—"}</span></td><td>{book.isbn || "—"}</td>
          <td><span className="metadata-ellipsis">{book.publisher || "—"}</span><small>{book.publishedDate}</small></td><td>{book.pageCount ?? "—"}</td>
          <td><div className="metadata-tags">{book.subjects.slice(0, 2).map(subject => <span key={subject}>{subject}</span>)}</div></td>
          <td><div className="metadata-sources">{book.sources.includes(source) ? <span>{labels[source]}</span> : <em>未补全</em>}</div></td>
          <td><button className="metadata-view" onClick={() => void view(book)} aria-label="查看元数据"><Eye size={16}/></button></td>
          <td><Button variant="secondary" disabled={busy.size > 0} icon={<RefreshCw size={14}/>} onClick={() => void pullSelectedSource([book.bookId])}>{labels[source]}补全</Button></td>
        </tr>)}</tbody>
      </table>
      <footer className="metadata-pagination"><span>第 {page + 1} / {pageCount} 页 · 共 {filtered.length} 本</span><div><button disabled={page === 0} onClick={() => setPage(value => value - 1)}>上一页</button><button disabled={page + 1 >= pageCount} onClick={() => setPage(value => value + 1)}>下一页</button></div></footer>
    </div>}
    {viewBook && <BookMetadataDialog book={viewBook} details={details} onClose={() => setViewBook(undefined)}/>}
  </>;
}
