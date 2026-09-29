import { Highlighter, Lightbulb, NotebookText, RefreshCw, Search } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { api } from "../api/tauri";
import { NoteCard } from "../components/notes/NoteCard";
import { Button } from "../components/ui/Button";
import { useSyncStore } from "../stores/sync";
import type { Note, NoteType } from "../types/domain";

type NoteFilter = "all" | NoteType;
const tabs: { value: NoteFilter; label: string; icon: typeof NotebookText }[] = [
  { value: "all", label: "全部", icon: NotebookText },
  { value: "highlight", label: "划线", icon: Highlighter },
  { value: "thought", label: "想法", icon: Lightbulb },
];

export function Notes() {
  const [notes, setNotes] = useState<Note[]>([]);
  const [filter, setFilter] = useState<NoteFilter>("all");
  const [query, setQuery] = useState("");
  const [bookId, setBookId] = useState("all");
  const [visibleCount, setVisibleCount] = useState(80);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const sync = useSyncStore();
  const syncing = sync.status === "reading" || sync.status === "processing";
  const refreshBatch = Math.floor(sync.processedBooks / 5);

  useEffect(() => {
    setLoading(true);
    setError("");
    const timer = window.setTimeout(() => {
      api.notes().then(setNotes).catch(reason => setError(reason instanceof Error ? reason.message : String(reason))).finally(() => setLoading(false));
    }, 0);
    return () => window.clearTimeout(timer);
  }, [sync.status, refreshBatch]);

  const counts = useMemo(() => ({ all: notes.length, highlight: notes.filter(note => note.type === "highlight").length, thought: notes.filter(note => note.type === "thought").length }), [notes]);
  const books = useMemo(() => {
    const unique = new Map<string, string>();
    notes.forEach(note => unique.set(note.bookId, note.bookTitle));
    return [...unique].sort((a, b) => a[1].localeCompare(b[1], "zh-CN"));
  }, [notes]);
  const filtered = useMemo(() => {
    const keyword = query.trim().toLocaleLowerCase();
    return notes.filter(note => filter === "all" || note.type === filter)
      .filter(note => bookId === "all" || note.bookId === bookId)
      .filter(note => !keyword || `${note.content}\n${note.bookTitle}\n${note.chapter}`.toLocaleLowerCase().includes(keyword));
  }, [notes, filter, bookId, query]);
  useEffect(() => setVisibleCount(80), [filter, bookId, query]);

  return <>
    <div className="notes-toolbar">
      <nav className="note-tabs" aria-label="笔记类型">{tabs.map(tab => <button key={tab.value} className={filter === tab.value ? "active" : ""} onClick={() => setFilter(tab.value)}><tab.icon size={15}/>{tab.label}<span>{counts[tab.value].toLocaleString()}</span></button>)}</nav>
      <div className="notes-toolbar__filters"><label className="notes-search"><Search size={15}/><input value={query} onChange={event => setQuery(event.target.value)} placeholder="搜索笔记、书名或章节"/></label><label className="notes-book-filter"><select value={bookId} onChange={event => setBookId(event.target.value)} aria-label="按书籍筛选"><option value="all">全部书籍</option>{books.map(([id, title]) => <option value={id} key={id}>{title}</option>)}</select></label></div>
    </div>
    <div className="notes-content">
      {loading && <div className="notes-loading">正在整理你的阅读痕迹…</div>}
      {!loading && error && <section className="notes-empty"><div className="notes-empty__icon"><NotebookText /></div><h2>暂时无法读取内容</h2><p>{error}</p></section>}
      {!loading && !error && notes.length === 0 && <section className="notes-empty"><div className="notes-empty__icon"><NotebookText /></div><span className="notes-empty__eyebrow">READING ARCHIVE</span><h2>还没有留下笔记</h2><p>同步微信读书后，你的划线与想法会汇聚在这里。</p><Button icon={<RefreshCw size={15} className={syncing ? "spin" : ""} />} onClick={sync.run} disabled={syncing}>{syncing ? "正在同步…" : "同步微信读书"}</Button>{sync.status === "failed" && <small>{sync.message}</small>}</section>}
      {!loading && !error && notes.length > 0 && filtered.length === 0 && <div className="notes-no-results">没有符合当前筛选条件的笔记</div>}
      {!loading && !error && filtered.length > 0 && <><p className="notes-result-count">显示 {Math.min(visibleCount, filtered.length).toLocaleString()} / {filtered.length.toLocaleString()} 条笔记</p><section className="notes-list">{filtered.slice(0, visibleCount).map(note => <NoteCard key={`${note.type}-${note.id}`} note={note} query={query}/>)}</section>{visibleCount < filtered.length && <div className="notes-load-more"><Button variant="secondary" onClick={() => setVisibleCount(count => count + 80)}>继续加载（剩余 {(filtered.length - visibleCount).toLocaleString()} 条）</Button></div>}</>}
    </div>
  </>;
}
