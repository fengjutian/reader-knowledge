import { ArrowLeft, ExternalLink } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../api/tauri";
import { NoteCard } from "../components/notes/NoteCard";
import { Button } from "../components/ui/Button";
import { useAppStore } from "../stores/app";
import type { BookDetail as BookDetailType, Note } from "../types/domain";

export function BookDetail() {
  const bookId = useAppStore(state => state.selectedBookId);
  const setPage = useAppStore(state => state.setPage);
  const [book, setBook] = useState<BookDetailType>();
  const [notes, setNotes] = useState<Note[]>([]);
  const [error, setError] = useState("");

  useEffect(() => {
    if (!bookId) return;
    setError("");
    Promise.all([api.book(bookId), api.bookNotes(bookId)])
      .then(([detail, items]) => { setBook(detail); setNotes(items); })
      .catch(reason => setError(typeof reason === "string" ? reason : "无法读取书籍详情"));
  }, [bookId]);

  if (!bookId) return <div className="empty-state">没有选择书籍</div>;
  if (error) return <div className="empty-state"><p>{error}</p><Button variant="secondary" onClick={() => setPage("books")}>返回书籍</Button></div>;
  if (!book) return <div className="empty-state">正在读取书籍…</div>;

  return <>
    <button className="back-button" onClick={() => setPage("books")}><ArrowLeft size={16}/>返回书籍</button>
    <header className="book-detail-header">
      {book.cover ? <img src={book.cover} alt=""/> : <div className="book-cover"><span>{book.title}</span></div>}
      <div><span className="eyebrow">{book.category || "微信读书"}</span><h1>{book.title}</h1><p>{book.author}</p><div className="book-detail-meta"><span>{book.highlightCount} 条划线</span><span>{book.thoughtCount} 条想法</span>{book.updatedAt&&<span>最近阅读 {book.updatedAt}</span>}</div>{book.deepLink&&<Button icon={<ExternalLink size={15}/>} onClick={() => api.openBook(book.id)}>在微信读书中打开</Button>}</div>
    </header>
    <div className="section-heading"><h2>全部笔记</h2><span>{notes.length} 条</span></div>
    {notes.length ? <section className="notes-list">{notes.map(note => <NoteCard key={`${note.type}-${note.id}`} note={note}/>)}</section> : <div className="empty-state">这本书还没有可导出的划线或想法</div>}
  </>;
}
