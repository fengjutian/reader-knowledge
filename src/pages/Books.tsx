import { Search, SlidersHorizontal } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { api } from "../api/tauri";
import { PageHeader } from "../components/ui/PageHeader";
import { useAppStore } from "../stores/app";
import { useSyncStore } from "../stores/sync";
import type { Book } from "../types/domain";

export function Books() {
  const [items, setItems] = useState<Book[]>([]);
  const [q, setQ] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const openBook = useAppStore(state => state.openBook);
  const syncStatus = useSyncStore(state => state.status);

  useEffect(() => {
    const timer = window.setTimeout(() => {
      api.books()
        .then(value => { setItems(value); setError(""); })
        .catch(reason => setError(reason instanceof Error ? reason.message : String(reason)))
        .finally(() => setLoading(false));
    }, 0);
    return () => window.clearTimeout(timer);
  }, [syncStatus]);

  const filtered = useMemo(() => items.filter(book => `${book.title}${book.author}`.toLowerCase().includes(q.toLowerCase())), [items, q]);
  return <>
    <PageHeader title="书籍" subtitle={`${items.length} 本书，承载你的阅读轨迹。`} />
    <div className="toolbar"><label className="field field--search"><Search size={16} /><input value={q} onChange={event => setQ(event.target.value)} placeholder="搜索书名或作者" /></label><button className="filter-button"><SlidersHorizontal size={16} />最近阅读</button></div>
    {loading && <div className="notes-loading">正在打开书架…</div>}
    {!loading && error && <div className="notes-loading">读取书架失败：{error}</div>}
    {!loading && !error && items.length === 0 && <div className="notes-loading">书架暂时为空，可返回概览同步微信读书。</div>}
    {!loading && !error && <section className="book-grid">{filtered.map(book => <article className="book-card" key={book.id} role="button" tabIndex={0} onClick={() => openBook(book.id)} onKeyDown={event => { if (event.key === "Enter") openBook(book.id); }}><div className="book-cover"><span>{book.title}</span>{book.cover && <img src={book.cover} alt="" loading="lazy" referrerPolicy="no-referrer" onError={event => { event.currentTarget.style.display = "none"; }} />}</div><div><h3>{book.title}</h3><p>{book.author}</p><small>{book.highlightCount} 条划线 · {book.thoughtCount} 条想法</small><div className="book-progress"><i style={{ width: `${book.progress}%` }} /></div></div></article>)}</section>}
  </>;
}
