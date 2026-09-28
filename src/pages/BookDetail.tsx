import { ExternalLink, X } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { api } from "../api/tauri";
import { NoteCard } from "../components/notes/NoteCard";
import { Button } from "../components/ui/Button";
import { useAppStore } from "../stores/app";
import type { BookDetail as BookDetailType, Note } from "../types/domain";

export function BookDetail() {
  const { selectedBookId: bookId, selectedNoteId, clearSelectedNote, closeBook } = useAppStore();
  const [book, setBook] = useState<BookDetailType>();
  const [notes, setNotes] = useState<Note[]>([]);
  const [error, setError] = useState("");
  const notesByChapter = useMemo(() => {
    const groups = new Map<string, Note[]>();
    for (const note of notes) {
      const chapter = note.chapter.trim() || "未分章节";
      const items = groups.get(chapter) ?? [];
      items.push(note);
      groups.set(chapter, items);
    }
    return [...groups.entries()];
  }, [notes]);
  useEffect(() => {
    if (!bookId) return;
    setError("");
    Promise.all([api.book(bookId), api.bookNotes(bookId)])
      .then(([detail, items]) => { setBook(detail); setNotes(items); })
      .catch(reason => setError(reason instanceof Error ? reason.message : String(reason)));
  }, [bookId]);
  useEffect(() => {
    if (!selectedNoteId || !notes.length) return;
    const frame = requestAnimationFrame(() => document.getElementById(`note-${selectedNoteId}`)?.scrollIntoView({ behavior: "smooth", block: "center" }));
    const timer = window.setTimeout(clearSelectedNote, 2600);
    return () => { cancelAnimationFrame(frame); window.clearTimeout(timer); };
  }, [selectedNoteId, notes, clearSelectedNote]);
  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => { if (event.key === "Escape") closeBook(); };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [closeBook]);
  useEffect(() => {
    const previousOverflow = document.body.style.overflow;
    const previousPaddingRight = document.body.style.paddingRight;
    const scrollbarWidth = window.innerWidth - document.documentElement.clientWidth;
    document.body.style.overflow = "hidden";
    if (scrollbarWidth > 0) document.body.style.paddingRight = `${scrollbarWidth}px`;
    return () => {
      document.body.style.overflow = previousOverflow;
      document.body.style.paddingRight = previousPaddingRight;
    };
  }, []);
  if (!bookId) return null;
  return <div className="book-drawer-layer" role="dialog" aria-modal="true" aria-label={book?.title || "书籍详情"}>
    <button className="book-drawer-backdrop" aria-label="关闭书籍详情" onClick={closeBook}/>
    <aside className="book-drawer">
      <div className="book-drawer__top"><span>书籍详情</span><button className="icon-button" aria-label="关闭" onClick={closeBook}><X size={19}/></button></div>
      <div className="book-drawer__content">{error ? <div className="empty-state"><p>{error}</p><Button variant="secondary" onClick={closeBook}>关闭</Button></div> : !book ? <div className="empty-state">正在读取书籍…</div> : <>
    <header className="book-detail-header">
      {book.cover ? <img src={book.cover} alt="" referrerPolicy="no-referrer"/> : <div className="book-cover"><span>{book.title}</span></div>}
      <div><span className="eyebrow">{book.category || "微信读书"}</span><h1>{book.title}</h1><p>{book.author}</p>
        <div className="book-detail-meta"><span>{book.highlightCount} 条划线</span><span>{book.thoughtCount} 条想法</span>{book.updatedAt && <span>最近阅读 {book.updatedAt}</span>}</div>
        <Button icon={<ExternalLink size={15}/>} onClick={() => api.openBook(book.id)}>在微信读书 Web 中打开</Button>
      </div>
    </header>
    <div className="section-heading"><h2>全部笔记</h2><span>{notes.length} 条</span></div>
    {notes.length ? <div className="chapter-groups">{notesByChapter.map(([chapter, chapterNotes]) => <section className="chapter-group" key={chapter}><header className="chapter-group__header"><h3>{chapter}</h3><span>{chapterNotes.length} 条</span></header><div className="notes-list">{chapterNotes.map(note => <NoteCard key={`${note.type}-${note.id}`} note={note} focused={note.id === selectedNoteId}/>)}</div></section>)}</div> : <div className="empty-state">这本书还没有可导出的划线或想法</div>}
      </>}
      </div>
    </aside>
  </div>;
}
