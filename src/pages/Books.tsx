import { Check, Highlighter, Lightbulb, Search, SlidersHorizontal } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { api } from "../api/tauri";
import { Button } from "../components/ui/Button";
import { PageHeader } from "../components/ui/PageHeader";
import { useAppStore } from "../stores/app";
import { useSyncStore } from "../stores/sync";
import type { Book } from "../types/domain";

let booksCache: Book[] | null = null;

export function Books() {
  const [items, setItems] = useState<Book[]>(() => booksCache ?? []);
  const [q, setQ] = useState("");
  const [withHighlights, setWithHighlights] = useState(true);
  const [withThoughts, setWithThoughts] = useState(true);
  const [loading, setLoading] = useState(() => booksCache === null);
  const [error, setError] = useState("");
  const [visibleCount, setVisibleCount] = useState(80);
  const firstLoad = useRef(true);
  const openBook = useAppStore(state => state.openBook);
  const syncStatus = useSyncStore(state => state.status);

  useEffect(() => {
    if (firstLoad.current) {
      firstLoad.current = false;
      if (booksCache) return;
    }
    const timer = window.setTimeout(() => {
      api.books()
        .then(value => { booksCache = value; setItems(value); setError(""); })
        .catch(reason => setError(reason instanceof Error ? reason.message : String(reason)))
        .finally(() => setLoading(false));
    }, 0);
    return () => window.clearTimeout(timer);
  }, [syncStatus]);

  const filtered = useMemo(() => items
    .filter(book => `${book.title}${book.author}`.toLowerCase().includes(q.toLowerCase()))
    .filter(book => {
      if (!withHighlights && !withThoughts) return true;
      return (withHighlights && book.highlightCount > 0) || (withThoughts && book.thoughtCount > 0);
    })
    .sort((a, b) => b.highlightCount - a.highlightCount || b.thoughtCount - a.thoughtCount || b.updatedAt.localeCompare(a.updatedAt)), [items, q, withHighlights, withThoughts]);
  const visible = filtered.slice(0, visibleCount);
  useEffect(() => setVisibleCount(80), [q, withHighlights, withThoughts]);
  return <>
    <PageHeader title="书籍" subtitle={`${items.length} 本书，承载你的阅读轨迹。`} />
    <div className="toolbar"><label className="field field--search"><Search size={16} /><input value={q} onChange={event => setQ(event.target.value)} placeholder="搜索书名或作者" /></label><div className="book-note-filters" aria-label="笔记类型筛选"><button type="button" className={withHighlights ? "active" : ""} aria-pressed={withHighlights} onClick={() => setWithHighlights(value => !value)}><Highlighter size={14} />有划线{withHighlights && <Check size={12} />}</button><button type="button" className={withThoughts ? "active" : ""} aria-pressed={withThoughts} onClick={() => setWithThoughts(value => !value)}><Lightbulb size={14} />有想法{withThoughts && <Check size={12} />}</button></div><button className="filter-button"><SlidersHorizontal size={16} />划线优先</button></div>
    {loading && <div className="notes-loading">正在打开书架…</div>}
    {!loading && error && <div className="notes-loading">读取书架失败：{error}</div>}
    {!loading && !error && items.length === 0 && <div className="notes-loading">书架暂时为空，可返回概览同步微信读书。</div>}
    {!loading && !error && items.length > 0 && filtered.length === 0 && <div className="notes-loading">没有符合当前筛选条件的书籍。</div>}
    {!loading && !error && <section className="book-grid">{visible.map(book => <article className="book-card" key={book.id} role="button" tabIndex={0} onClick={() => openBook(book.id)} onKeyDown={event => { if (event.key === "Enter") openBook(book.id); }}><div className="book-cover"><span>{book.title}</span>{book.cover && <img src={book.cover} alt="" loading="lazy" referrerPolicy="no-referrer" onError={event => { event.currentTarget.style.display = "none"; }} />}</div><div><h3>{book.title}</h3><p>{book.author}</p><small>{book.highlightCount} 条划线 · {book.thoughtCount} 条想法</small><div className="book-progress"><i style={{ width: `${book.progress}%` }} /></div></div></article>)}</section>}
    {!loading && !error && visibleCount < filtered.length && <div className="book-load-more"><Button variant="secondary" onClick={() => setVisibleCount(count => count + 80)}>继续加载（剩余 {filtered.length - visibleCount} 本）</Button></div>}
  </>;
}
