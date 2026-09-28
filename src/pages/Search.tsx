import { Search as SearchIcon } from "lucide-react";
import { useState } from "react";
import { api } from "../api/tauri";
import { NoteCard } from "../components/notes/NoteCard";
import { PageHeader } from "../components/ui/PageHeader";
import type { SearchResult } from "../types/domain";
export function SearchPage(){const [q,setQ]=useState("");const [results,setResults]=useState<SearchResult[]>([]);async function submit(e:React.FormEvent){e.preventDefault();setResults(await api.search(q))}return <><PageHeader title="搜索" subtitle="在你的全部阅读记录中寻找线索。"/><form className="big-search" onSubmit={submit}><SearchIcon/><input value={q} onChange={e=>setQ(e.target.value)} placeholder="输入关键词，例如：组织管理"/><button>搜索</button></form>{q&&<p className="result-count">找到 {results.length} 条相关内容</p>}<section className="notes-list">{results.map(n=><NoteCard key={n.id} note={n}/>)}</section></>}

