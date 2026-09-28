import { Search as SearchIcon } from "lucide-react";
import { useState } from "react";
import { api } from "../api/tauri";
import { NoteCard } from "../components/notes/NoteCard";
import { PageHeader } from "../components/ui/PageHeader";
import type { SearchResult } from "../types/domain";
import type { NoteType } from "../types/domain";
type Filter = "all" | NoteType;
export function SearchPage(){const [q,setQ]=useState("");const [filter,setFilter]=useState<Filter>("all");const [results,setResults]=useState<SearchResult[]>([]);async function search(nextFilter=filter){if(!q.trim()){setResults([]);return}setResults(await api.search(q,nextFilter==="all"?undefined:nextFilter))}async function submit(e:React.FormEvent){e.preventDefault();await search()}async function choose(next:Filter){setFilter(next);await search(next)}return <><PageHeader title="搜索" subtitle="在你的全部阅读记录中寻找线索。"/><form className="big-search" onSubmit={submit}><SearchIcon/><input value={q} onChange={e=>setQ(e.target.value)} placeholder="输入关键词，例如：组织管理"/><button>搜索</button></form><div className="search-filters">{([['all','全部'],['highlight','划线'],['thought','想法']] as const).map(([value,label])=><button key={value} className={filter===value?"active":""} onClick={()=>choose(value)}>{label}</button>)}</div>{q&&<p className="result-count">找到 {results.length} 条相关内容</p>}<section className="notes-list">{results.map(n=><NoteCard key={n.id} note={n}/>)}</section></>}

