import { Search } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { api } from "../api/tauri";
import { PageHeader } from "../components/ui/PageHeader";
import { useAppStore } from "../stores/app";
import { useSyncStore } from "../stores/sync";
import type { Book } from "../types/domain";

let booksCache: Book[] | null = null;
export function Books() {
  const [items,setItems]=useState<Book[]>(booksCache??[]); const [query,setQuery]=useState(""); const [loading,setLoading]=useState(!booksCache); const [error,setError]=useState("");
  const openBook=useAppStore(s=>s.openBook); const syncStatus=useSyncStore(s=>s.status);
  useEffect(()=>{api.books().then(v=>{booksCache=v;setItems(v);setError("")}).catch(e=>setError(String(e))).finally(()=>setLoading(false));},[syncStatus]);
  const filtered=useMemo(()=>items.filter(b=>`${b.title}${b.author}`.toLowerCase().includes(query.toLowerCase())),[items,query]);
  return <><PageHeader title="书籍" subtitle={`${items.length} 本书，承载你的阅读轨迹。`}/><div className="toolbar books-toolbar"><label className="field field--search"><Search size={16}/><input value={query} onChange={e=>setQuery(e.target.value)} placeholder="搜索书名或作者"/></label></div><div className="books-scroll">{loading?<div className="notes-loading">正在打开书架…</div>:error?<div className="notes-loading">读取书架失败：{error}</div>:<section className="book-grid">{filtered.map(book=><article className="book-card" key={book.id} role="button" tabIndex={0} onClick={()=>openBook(book.id)} onKeyDown={e=>e.key==="Enter"&&openBook(book.id)}><div className="book-cover"><span>{book.title}</span>{book.cover&&<img src={book.cover} alt="" loading="lazy" referrerPolicy="no-referrer"/>}</div><div><h3>{book.title}</h3><p>{book.author}</p><small>{book.highlightCount} 条划线 · {book.thoughtCount} 条想法</small></div></article>)}</section>}</div></>;
}
