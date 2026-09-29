import { CheckSquare, Database, RefreshCw, Search, Square } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { api } from "../api/tauri";
import { Button } from "../components/ui/Button";
import { PageHeader } from "../components/ui/PageHeader";
import { useAppStore } from "../stores/app";
import type { BookMetadataRow, MetadataFetchResult } from "../types/domain";

const labels:Record<string,string>={open_library:"Open Library",google_books:"Google Books",manual:"手动"};
export function BookMetadata(){
 const [items,setItems]=useState<BookMetadataRow[]>([]),[selected,setSelected]=useState<Set<string>>(new Set()),[busy,setBusy]=useState<Set<string>>(new Set());
 const [query,setQuery]=useState(""),[source,setSource]=useState("open_library"),[message,setMessage]=useState(""),[loading,setLoading]=useState(true); const openBook=useAppStore(s=>s.openBook);
 const load=()=>api.bookMetadata().then(setItems).catch(e=>setMessage(String(e))).finally(()=>setLoading(false)); useEffect(()=>{void load()},[]);
 const filtered=useMemo(()=>items.filter(x=>`${x.title} ${x.author} ${x.isbn} ${x.publisher}`.toLowerCase().includes(query.toLowerCase())),[items,query]);
 const all=filtered.length>0&&filtered.every(x=>selected.has(x.bookId));
 const toggleAll=()=>setSelected(old=>{const n=new Set(old);filtered.forEach(x=>all?n.delete(x.bookId):n.add(x.bookId));return n});
 const toggle=(id:string)=>setSelected(old=>{const n=new Set(old);n.has(id)?n.delete(id):n.add(id);return n});
 const pull=async(ids:string[])=>{if(!ids.length)return;setBusy(new Set(ids));setMessage(`正在从 ${labels[source]} 拉取 ${ids.length} 本书…`);try{const r:MetadataFetchResult[]=ids.length===1?[await api.fetchBookMetadata(ids[0],source)]:await api.fetchBooksMetadata(ids,source);const count=(s:string)=>r.filter(x=>x.status===s).length;setMessage(`完成：更新 ${count("updated")}，缓存 ${count("cached")}，未匹配 ${count("not_found")}，失败 ${count("failed")}`);await load()}catch(e){setMessage(String(e))}finally{setBusy(new Set())}};
 return <><PageHeader title="书籍元数据" subtitle={`${items.length} 本书；每个来源独立保存。`} actions={<Button icon={<RefreshCw size={15}/>} disabled={!selected.size||busy.size>0} onClick={()=>void pull([...selected])}>批量拉取（{selected.size}）</Button>}/><div className="toolbar"><label className="field field--search"><Search size={16}/><input value={query} onChange={e=>setQuery(e.target.value)} placeholder="搜索书名、作者、ISBN 或出版社"/></label><label className="filter-button"><Database size={15}/><select value={source} onChange={e=>setSource(e.target.value)}><option value="open_library">Open Library</option><option value="google_books">Google Books</option></select></label></div>{message&&<div className="metadata-message">{message}</div>}{loading?<div className="notes-loading">正在读取书籍…</div>:<div className="metadata-table-wrap"><table className="metadata-table"><thead><tr><th><button className="table-check" onClick={toggleAll}>{all?<CheckSquare size={17}/>:<Square size={17}/>}</button></th><th>书名</th><th>作者</th><th>ISBN</th><th>出版社 / 时间</th><th>页数</th><th>主题</th><th>数据源</th><th>操作</th></tr></thead><tbody>{filtered.map(b=><tr key={b.bookId}><td><button className="table-check" onClick={()=>toggle(b.bookId)}>{selected.has(b.bookId)?<CheckSquare size={17}/>:<Square size={17}/>}</button></td><td><button className="book-title-link" onClick={()=>openBook(b.bookId)}>{b.title}</button></td><td>{b.author||"—"}</td><td>{b.isbn||"—"}</td><td>{b.publisher||"—"}<small>{b.publishedDate}</small></td><td>{b.pageCount??"—"}</td><td><div className="metadata-tags">{b.subjects.slice(0,3).map(x=><span key={x}>{x}</span>)}</div></td><td><div className="metadata-sources">{b.sources.length?b.sources.map(x=><span key={x}>{labels[x]??x}</span>):<em>未补全</em>}</div></td><td><Button variant="secondary" disabled={busy.size>0} icon={<RefreshCw className={busy.has(b.bookId)?"spin":""} size={14}/>} onClick={()=>void pull([b.bookId])}>拉取</Button></td></tr>)}</tbody></table></div>}</>;
}
