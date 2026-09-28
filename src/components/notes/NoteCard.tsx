import { Copy, Highlighter, Lightbulb } from "lucide-react";
import type { ReactNode } from "react";
import type { Note } from "../../types/domain";

function highlighted(text: string, query?: string): ReactNode {
  const rawTerms = query?.trim().split(/\s+/).filter(Boolean) ?? [];
  const terms = [...rawTerms];
  for (const raw of rawTerms) {
    const chars = [...raw.replace(/[^\p{L}\p{N}]/gu, "")];
    if (chars.length > 2) for (let index = 0; index < chars.length - 1; index++) terms.push(chars.slice(index, index + 2).join(""));
  }
  terms.sort((a, b) => b.length - a.length);
  if (!terms.length) return text;
  const escaped = terms.map(term => term.replace(/[.*+?^${}()|[\]\\]/g, "\\$&"));
  const matcher = new RegExp(`(${escaped.join("|")})`, "gi");
  return text.split(matcher).map((part, index) =>
    terms.some(term => part.toLocaleLowerCase() === term.toLocaleLowerCase())
      ? <mark key={`${part}-${index}`}>{part}</mark>
      : part
  );
}

export function NoteCard({ note, query, focused = false }: { note: Note; query?: string; focused?: boolean }) {
  const thought = note.type === "thought";
  return <article id={`note-${note.id}`} className={`note-card ${thought ? "note-card--thought" : ""} ${focused ? "note-card--focused" : ""}`}>
    <div className="note-card__meta"><span className="note-kind">{thought ? <Lightbulb size={14}/> : <Highlighter size={14}/>} {thought ? "我的想法" : "原文划线"}</span><button className="icon-button" title="复制" onClick={() => navigator.clipboard?.writeText(note.content)}><Copy size={15}/></button></div>
    <blockquote>{highlighted(note.content, query)}</blockquote>
    <footer><strong>《{note.bookTitle}》</strong><span>{note.chapter}</span><time>{note.createdAt}</time></footer>
  </article>;
}

