import { Check, Highlighter, Lightbulb, Search, SlidersHorizontal } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../api/tauri";
import { PageHeader } from "../components/ui/PageHeader";
import { useAppStore } from "../stores/app";
import { useSyncStore } from "../stores/sync";
import type { Book } from "../types/domain";

const pageSize = 500;
type SortBy = "recent" | "highlights" | "thoughts" | "title";
type ReadingStatus = "all" | Book["readingStatus"];

export function Books() {
  const [items, setItems] = useState<Book[]>([]);
  const [categories, setCategories] = useState<string[]>([]);
  const [total, setTotal] = useState(0);
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState("all");
  const [readingStatus, setReadingStatus] = useState<ReadingStatus>("all");
  const [withHighlights, setWithHighlights] = useState(true);
  const [withThoughts, setWithThoughts] = useState(true);
  const [sortBy, setSortBy] = useState<SortBy>("recent");
  const [page, setPage] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const openBook = useAppStore(state => state.openBook);
  const syncStatus = useSyncStore(state => state.status);
  const pageCount = Math.max(1, Math.ceil(total / pageSize));

  useEffect(() => { setPage(0); }, [query, category, readingStatus, withHighlights, withThoughts, sortBy]);
  useEffect(() => {
    setLoading(true);
    setError("");
    const timer = window.setTimeout(() => {
      api.booksPage(query, pageSize, page * pageSize, category, readingStatus, withHighlights, withThoughts, sortBy)
        .then(result => { setItems(result.books); setTotal(result.total); setCategories(result.categories); })
        .catch(reason => setError(reason instanceof Error ? reason.message : String(reason)))
        .finally(() => setLoading(false));
    }, query ? 250 : 0);
    return () => window.clearTimeout(timer);
  }, [page, query, category, readingStatus, withHighlights, withThoughts, sortBy, syncStatus]);

  return <>
    <PageHeader title="书籍" subtitle={`${total} 本书，承载你的阅读轨迹。`}/>
    <div className="toolbar books-toolbar">
      <label className="field field--search"><Search size={16}/><input value={query} onChange={event => setQuery(event.target.value)} placeholder="搜索书名或作者"/></label>
      <div className="book-note-filters" aria-label="笔记类型筛选">
        <button type="button" className={withHighlights ? "active" : ""} aria-pressed={withHighlights} onClick={() => setWithHighlights(value => !value)}><Highlighter size={14}/>有划线{withHighlights && <Check size={12}/>}</button>
        <button type="button" className={withThoughts ? "active" : ""} aria-pressed={withThoughts} onClick={() => setWithThoughts(value => !value)}><Lightbulb size={14}/>有想法{withThoughts && <Check size={12}/>}</button>
      </div>
      <label className="filter-button book-filter-select"><select value={category} onChange={event => setCategory(event.target.value)} aria-label="按分类筛选"><option value="all">全部分类</option>{categories.map(value => <option value={value} key={value}>{value}</option>)}</select></label>
      <label className="filter-button book-filter-select"><select value={readingStatus} onChange={event => setReadingStatus(event.target.value as ReadingStatus)} aria-label="按阅读状态筛选"><option value="all">全部状态</option><option value="unread">未开始</option><option value="reading">阅读中</option><option value="finished">已读完</option></select></label>
      <label className="filter-button book-sort"><SlidersHorizontal size={16}/><select value={sortBy} onChange={event => setSortBy(event.target.value as SortBy)} aria-label="书籍排序"><option value="recent">最近阅读</option><option value="highlights">划线最多</option><option value="thoughts">想法最多</option><option value="title">书名排序</option></select></label>
    </div>
    <div className="books-scroll">
      {loading ? <div className="notes-loading">正在打开书架…</div> : error ? <div className="notes-loading">读取书架失败：{error}</div> : items.length === 0 ? <div className="notes-loading">没有符合当前筛选条件的书籍。</div> : <section className="book-grid">{items.map(book => { const progress = Math.min(100, Math.max(0, Math.round(Number(book.progress) || 0))); return <article className="book-card" key={book.id} role="button" tabIndex={0} onClick={() => openBook(book.id)} onKeyDown={event => event.key === "Enter" && openBook(book.id)}><div className="book-cover"><span>{book.title}</span>{book.cover && <img src={book.cover} alt="" loading="lazy" referrerPolicy="no-referrer" onError={event => { event.currentTarget.style.display = "none"; }}/>}</div><div><h3>{book.title}</h3><p>{book.author}</p><small>{book.highlightCount} 条划线 · {book.thoughtCount} 条想法</small>{progress > 0 && <div className="book-progress-wrap" aria-label={`阅读进度 ${progress}%`}><div className="book-progress-label"><span>阅读进度</span><strong>{progress}%</strong></div><div className="book-progress"><i style={{ width: `${progress}%` }}/></div></div>}</div></article>; })}</section>}
    </div>
    {!loading && !error && <footer className="books-pagination"><span>第 {page + 1} / {pageCount} 页 · 本页 {items.length} 本 · 共 {total} 本</span><div><button disabled={page === 0} onClick={() => setPage(value => value - 1)}>上一页</button><button disabled={page + 1 >= pageCount} onClick={() => setPage(value => value + 1)}>下一页</button></div></footer>}
  </>;
}
