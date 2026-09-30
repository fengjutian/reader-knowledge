import { CheckSquare, Eye, RefreshCw, Search, Square } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "../api/tauri";
import { BookMetadataDialog } from "../components/metadata/BookMetadataDialog";
import { Button } from "../components/ui/Button";
import { Message, type MessageValue } from "../components/ui/Message";
import { PageHeader } from "../components/ui/PageHeader";
import { useAppStore } from "../stores/app";
import type { BookMetadataRow, BookMetadataSourceDetail, MetadataFetchResult } from "../types/domain";

const labels: Record<string, string> = { weread: "微信读书", open_library: "Open Library", google_books: "Google Books", douban: "豆瓣", manual: "手动" };
const maxDoubanBatch = 10;
const pageSize = 100;
const hiddenSources = new Set(["open_library", "google_books"]);

function coreTitle(title: string) { return title.split(/[：:（(【[]/, 1)[0].trim(); }
function readableError(error: unknown) {
  const text = error instanceof Error ? error.message : String(error);
  if (text.includes("429")) return "请求过于频繁，请稍后再试。";
  return text.length > 180 ? `${text.slice(0, 180)}…` : text;
}

export function BookMetadata() {
  const [items, setItems] = useState<BookMetadataRow[]>([]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const [query, setQuery] = useState("");
  const [message, setMessage] = useState<MessageValue>();
  const [loading, setLoading] = useState(true);
  const [page, setPage] = useState(0);
  const [viewBook, setViewBook] = useState<BookMetadataRow>();
  const [details, setDetails] = useState<BookMetadataSourceDetail[]>([]);
  const openBook = useAppStore(state => state.openBook);
  const closeMessage = useCallback(() => setMessage(undefined), []);

  const load = useCallback(async () => {
    try { setItems(await api.bookMetadata()); }
    catch (error) { setMessage({ kind: "error", text: readableError(error) }); }
    finally { setLoading(false); }
  }, []);
  useEffect(() => { void load(); }, [load]);

  const filtered = useMemo(() => items.filter(item => `${item.title} ${item.author} ${item.isbn} ${item.publisher}`.toLocaleLowerCase().includes(query.toLocaleLowerCase())), [items, query]);
  const pageCount = Math.max(1, Math.ceil(filtered.length / pageSize));
  const visible = filtered.slice(page * pageSize, (page + 1) * pageSize);
  const selectedVisibleCount = visible.reduce((count, item) => count + Number(selected.has(item.bookId)), 0);
  const visibleSelectionTarget = Math.min(visible.length, maxDoubanBatch);
  const allVisibleSelected = visibleSelectionTarget > 0 && selectedVisibleCount === visibleSelectionTarget;
  useEffect(() => setPage(0), [query]);

  function toggleAll() {
    setSelected(current => {
      const next = new Set(current);
      if (allVisibleSelected) { visible.forEach(item => next.delete(item.bookId)); return next; }
      for (const item of visible) { if (next.size >= maxDoubanBatch) break; next.add(item.bookId); }
      if (visible.some(item => !next.has(item.bookId))) setMessage({ kind: "info", text: `豆瓣每次最多补全 ${maxDoubanBatch} 本书。` });
      return next;
    });
  }

  function toggle(book: BookMetadataRow) {
    setSelected(current => {
      const next = new Set(current);
      if (next.has(book.bookId)) next.delete(book.bookId);
      else if (next.size < maxDoubanBatch) next.add(book.bookId);
      else setMessage({ kind: "info", text: `豆瓣每次最多补全 ${maxDoubanBatch} 本书。` });
      return next;
    });
  }

  async function pullDouban(ids: string[]) {
    if (!ids.length) return;
    if (ids.length > maxDoubanBatch) { setMessage({ kind: "error", text: `豆瓣每次最多补全 ${maxDoubanBatch} 本书。` }); return; }
    setBusy(new Set(ids));
    setMessage({ kind: "info", text: `正在从豆瓣补全 ${ids.length} 本书…` });
    let updated = 0;
    let failed = 0;
    for (const id of ids) {
      const book = items.find(item => item.bookId === id);
      if (!book) { failed += 1; continue; }
      try {
        const result: MetadataFetchResult = await api.fetchDoubanBookMetadata(id, `${coreTitle(book.title)} ${book.author}`.trim());
        if (result.status === "updated" || result.status === "cached") updated += 1;
        else failed += 1;
      } catch { failed += 1; }
    }
    setMessage({ kind: failed ? "error" : "success", text: `豆瓣补全完成：成功 ${updated}，失败 ${failed}。` });
    await load();
    setBusy(new Set());
  }

  async function pullWeread(ids: string[]) {
    if (!ids.length) return;
    setBusy(new Set(ids));
    setMessage({ kind: "info", text: `正在从微信读书补全 ${ids.length} 本书…` });
    let updated = 0;
    let failed = 0;
    for (const id of ids) {
      try {
        const result = await api.fetchBookMetadata(id, "weread", true);
        if (result.status === "updated" || result.status === "cached") updated += 1;
        else failed += 1;
      } catch { failed += 1; }
    }
    setMessage({ kind: failed ? "error" : "success", text: `微信读书补全完成：成功 ${updated}，失败 ${failed}。` });
    await load();
    setBusy(new Set());
  }

  async function view(book: BookMetadataRow) {
    setViewBook(book); setDetails([]);
    try { setDetails((await api.bookMetadataDetails(book.bookId)).filter(detail => !hiddenSources.has(detail.source))); }
    catch (error) { setViewBook(undefined); setMessage({ kind: "error", text: readableError(error) }); }
  }

  return <>
    <Message value={message} onClose={closeMessage}/>
    <PageHeader title="书籍元数据" subtitle={`${items.length} 本书；当前列表展示微信读书字段，各来源详情独立保存。`} actions={<Button icon={<RefreshCw size={15}/>} disabled={!selected.size || busy.size > 0} onClick={() => void pullWeread([...selected])}>微信读书补全（{selected.size}/{maxDoubanBatch}）</Button>}/>
    <div className="toolbar metadata-toolbar"><label className="field field--search"><Search size={16}/><input value={query} onChange={event => setQuery(event.target.value)} placeholder="搜索书名、作者、ISBN 或出版社"/></label></div>
    <div className="douban-import"><div><strong>按来源批量补全</strong><small>微信读书按 bookId 获取，豆瓣按书名与作者搜索；两个来源分别保存，不合并字段。</small></div><span className="douban-import__limit">已选 {selected.size} / {maxDoubanBatch}</span><div className="metadata-source-actions"><Button variant="secondary" disabled={!selected.size || busy.size > 0} onClick={() => void pullWeread([...selected])}>微信读书</Button><Button variant="secondary" disabled={!selected.size || busy.size > 0} onClick={() => void pullDouban([...selected])}>豆瓣</Button></div></div>
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
          <td><div className="metadata-sources">{book.sources.filter(source => !hiddenSources.has(source)).length ? book.sources.filter(source => !hiddenSources.has(source)).map(source => <span key={source}>{labels[source] ?? source}</span>) : <em>未补全</em>}</div></td>
          <td><button className="metadata-view" onClick={() => void view(book)} aria-label="查看元数据"><Eye size={16}/></button></td>
          <td><Button variant="secondary" disabled={busy.size > 0} icon={<RefreshCw size={14}/>} onClick={() => void pullDouban([book.bookId])}>豆瓣补全</Button></td>
        </tr>)}</tbody>
      </table>
      <footer className="metadata-pagination"><span>第 {page + 1} / {pageCount} 页 · 共 {filtered.length} 本</span><div><button disabled={page === 0} onClick={() => setPage(value => value - 1)}>上一页</button><button disabled={page + 1 >= pageCount} onClick={() => setPage(value => value + 1)}>下一页</button></div></footer>
    </div>}
    {viewBook && <BookMetadataDialog book={viewBook} details={details} onClose={() => setViewBook(undefined)}/>}
  </>;
}
