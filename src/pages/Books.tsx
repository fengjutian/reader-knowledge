import { useEffect, useMemo, useState } from "react";
import { Search, SlidersHorizontal } from "lucide-react";
import { api } from "../api/tauri";
import { PageHeader } from "../components/ui/PageHeader";
import type { Book } from "../types/domain";
import { useAppStore } from "../stores/app";
export function Books() { const [items,setItems]=useState<Book[]>([]); const [q,setQ]=useState(""); const openBook=useAppStore(s=>s.openBook); useEffect(()=>{api.books().then(setItems)},[]); const filtered=useMemo(()=>items.filter(b=>`${b.title}${b.author}`.toLowerCase().includes(q.toLowerCase())),[items,q]); return <><PageHeader title="书籍" subtitle={`${items.length} 本书，承载你的阅读轨迹。`}/><div className="toolbar"><label className="field field--search"><Search size={16}/><input value={q} onChange={e=>setQ(e.target.value)} placeholder="搜索书名或作者"/></label><button className="filter-button"><SlidersHorizontal size={16}/>最近阅读</button></div><section className="book-grid">{filtered.map(book=><article className="book-card" key={book.id} role="button" tabIndex={0} onClick={()=>openBook(book.id)} onKeyDown={e=>{if(e.key==="Enter")openBook(book.id)}}><div className="book-cover" style={{background:book.cover}}><span>{book.title}</span></div><div><h3>{book.title}</h3><p>{book.author}</p><small>{book.highlightCount} 条划线 · {book.thoughtCount} 条想法</small><div className="book-progress"><i style={{width:`${book.progress}%`}}/></div></div></article>)}</section></> }

