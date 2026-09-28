import { Search as SearchIcon } from "lucide-react";
import { useState } from "react";
import { api } from "../api/tauri";
import { NoteCard } from "../components/notes/NoteCard";
import { PageHeader } from "../components/ui/PageHeader";
import type { NoteType, SearchResult } from "../types/domain";

type Filter = "all" | NoteType;
export function SearchPage() {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [error, setError] = useState("");
  async function search(nextFilter = filter) {
    if (!query.trim()) { setResults([]); return; }
    setError("");
    try { setResults(await api.search(query, nextFilter === "all" ? undefined : nextFilter)); }
    catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); }
  }
  return <>
    <PageHeader title="搜索" subtitle="在全部阅读记录中寻找线索，支持中文语义片段召回。"/>
    <form className="big-search" onSubmit={event => { event.preventDefault(); void search(); }}><SearchIcon/><input value={query} onChange={event => setQuery(event.target.value)} placeholder="输入关键词，例如：组织管理"/><button>搜索</button></form>
    <div className="search-filters">{([['all','全部'],['highlight','划线'],['thought','想法']] as const).map(([value,label]) => <button key={value} className={filter === value ? "active" : ""} onClick={() => { setFilter(value); void search(value); }}>{label}</button>)}</div>
    {error && <p className="field-error">{error}</p>}{query && <p className="result-count">找到 {results.length} 条相关内容</p>}
    <section className="notes-list">{results.map(note => <NoteCard key={`${note.type}-${note.id}`} note={note} query={query}/>)}</section>
  </>;
}
