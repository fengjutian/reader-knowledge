import { Search } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../api/tauri";
import { PageHeader } from "../components/ui/PageHeader";
import { useAppStore } from "../stores/app";
import { useSyncStore } from "../stores/sync";
import type { Book } from "../types/domain";

const pageSize = 500;

export function Books() {
  const [items, setItems] = useState<Book[]>([]);
  const [total, setTotal] = useState(0);
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const openBook = useAppStore(state => state.openBook);
  const syncStatus = useSyncStore(state => state.status);
  const pageCount = Math.max(1, Math.ceil(total / pageSize));

  useEffect(() => { setPage(0); }, [query]);
  useEffect(() => {
    setLoading(true);
    setError("");
    const timer = window.setTimeout(() => {
      api.booksPage(query, pageSize, page * pageSize)
        .then(result => { setItems(result.books); setTotal(result.total); })
        .catch(reason => setError(reason instanceof Error ? reason.message : String(reason)))
        .finally(() => setLoading(false));
    }, query ? 250 : 0);
    return () => window.clearTimeout(timer);
  }, [page, query, syncStatus]);

  return <>
    <PageHeader title="书籍" subtitle={`${total} 本书，承载你的阅读轨迹。`}/>
    <div className="toolbar books-toolbar"><label className="field field--search"><Search size={16}/><input value={query} onChange={event => setQuery(event.target.value)} placeholder="搜索书名或作者"/></label></div>
    <div className="books-scroll">
      {loading ? <div className="notes-loading">正在打开书架…</div> : error ? <div className="notes-loading">读取书架失败：{error}</div> : <section className="book-grid">{items.map(book => <article className="book-card" key={book.id} role="button" tabIndex={0} onClick={() => openBook(book.id)} onKeyDown={event => event.key === "Enter" && openBook(book.id)}><div className="book-cover"><span>{book.title}</span>{book.cover && <img src={book.cover} alt="" loading="lazy" referrerPolicy="no-referrer"/>}</div><div><h3>{book.title}</h3><p>{book.author}</p><small>{book.highlightCount} 条划线 · {book.thoughtCount} 条想法</small></div></article>)}</section>}
    </div>
    {!loading && !error && <footer className="books-pagination"><span>第 {page + 1} / {pageCount} 页 · 本页 {items.length} 本 · 共 {total} 本</span><div><button disabled={page === 0} onClick={() => setPage(value => value - 1)}>上一页</button><button disabled={page + 1 >= pageCount} onClick={() => setPage(value => value + 1)}>下一页</button></div></footer>}
  </>;
}
