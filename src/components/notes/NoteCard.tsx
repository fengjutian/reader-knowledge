import { Copy, Highlighter, Lightbulb } from "lucide-react";
import type { Note } from "../../types/domain";
export function NoteCard({ note }: { note: Note }) {
  const thought = note.type === "thought";
  return <article className={`note-card ${thought ? "note-card--thought" : ""}`}>
    <div className="note-card__meta"><span className="note-kind">{thought ? <Lightbulb size={14}/> : <Highlighter size={14}/>} {thought ? "我的想法" : "原文划线"}</span><button className="icon-button" title="复制" onClick={() => navigator.clipboard?.writeText(note.content)}><Copy size={15}/></button></div>
    <blockquote>{note.content}</blockquote>
    <footer><strong>《{note.bookTitle}》</strong><span>{note.chapter}</span><time>{note.createdAt}</time></footer>
  </article>;
}

