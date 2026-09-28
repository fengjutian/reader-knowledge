import { ArrowUp, BookOpen, Search, Sparkles, X } from "lucide-react";
import { Fragment, useEffect, useMemo, useState } from "react";
import { api } from "../api/tauri";
import { PageHeader } from "../components/ui/PageHeader";
import { useAppStore } from "../stores/app";
import type { AiAnswer, AiMode, Book } from "../types/domain";

const modeLabels: Record<AiMode, string> = { ask: "全库提问", summary: "单书总结", compare: "跨书分析" };
export function AI() {
  const [question, setQuestion] = useState("");
  const [mode, setMode] = useState<AiMode>("ask");
  const [books, setBooks] = useState<Book[]>([]);
  const [bookIds, setBookIds] = useState<string[]>([]);
  const [bookQuery, setBookQuery] = useState("");
  const [answer, setAnswer] = useState<AiAnswer>();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const openBook = useAppStore(state => state.openBook);
  useEffect(() => { api.books().then(setBooks).catch(() => setBooks([])); }, []);
  const selectedBooks = useMemo(() => bookIds.map(id => books.find(book => book.id === id)).filter((book): book is Book => Boolean(book)), [bookIds, books]);
  const visibleBooks = useMemo(() => {
    const query = bookQuery.trim().toLocaleLowerCase();
    return books
      .filter(book => book.highlightCount + book.thoughtCount > 0)
      .filter(book => !query || `${book.title} ${book.author}`.toLocaleLowerCase().includes(query))
      .sort((a, b) => (b.highlightCount + b.thoughtCount) - (a.highlightCount + a.thoughtCount))
      .slice(0, 60);
  }, [bookQuery, books]);
  function changeMode(next: AiMode) {
    setMode(next); setAnswer(undefined); setError(""); setBookIds([]); setBookQuery("");
    setQuestion(next === "summary" ? "请总结这本书的核心观点，并区分原文与我的想法。" : next === "compare" ? "请比较这些书对同一主题的观点、共识与分歧。" : "");
  }
  function toggleBook(id: string) {
    setBookIds(current => {
      if (mode === "summary") return [id];
      if (current.includes(id)) return current.filter(value => value !== id);
      if (current.length >= 12) { setError("跨书分析最多选择 12 本；更多书建议拆成多个主题分别分析"); return current; }
      setError("");
      return [...current, id];
    });
  }
  async function ask(event: React.FormEvent) {
    event.preventDefault(); if (!question.trim()) return;
    if (mode === "summary" && bookIds.length !== 1) { setError("请选择一本书进行总结"); return; }
    if (mode === "compare" && bookIds.length < 2) { setError("请至少选择两本书进行跨书分析"); return; }
    setLoading(true); setError(""); setAnswer(undefined);
    try { setAnswer(await api.ask({ question, mode, bookIds })); }
    catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); }
    finally { setLoading(false); }
  }
  return <><PageHeader title="AI 阅读助手" subtitle="仅使用本地检索出的阅读笔记回答，并附带可定位的来源。"/>
    <div className="ai-wrap">
      <div className="ai-modes">{(Object.keys(modeLabels) as AiMode[]).map(value => <button key={value} className={mode === value ? "active" : ""} onClick={() => changeMode(value)}>{modeLabels[value]}</button>)}</div>
      {(mode === "summary" || mode === "compare") && <section className="ai-book-picker">
        <div className="ai-book-picker__head"><div><strong>{mode === "summary" ? "选择一本书" : "选择 2–12 本书"}</strong><span>每本至少提取 5 条代表性笔记，按笔记数量排序</span></div><em>{bookIds.length} 本已选</em></div>
        {selectedBooks.length > 0 && <div className="ai-book-picker__selected">{selectedBooks.map(book => <button key={book.id} onClick={() => toggleBook(book.id)} title="移除"><span>{book.title}</span><X size={13}/></button>)}</div>}
        <label className="ai-book-search"><Search size={16}/><input value={bookQuery} onChange={event => setBookQuery(event.target.value)} placeholder="搜索书名或作者"/></label>
        <div className="ai-book-options">{visibleBooks.map(book => <label key={book.id} className={bookIds.includes(book.id) ? "selected" : ""}><input type={mode === "summary" ? "radio" : "checkbox"} checked={bookIds.includes(book.id)} onChange={() => toggleBook(book.id)}/><span><strong>{book.title}</strong><small>{book.author || "未知作者"} · {book.highlightCount + book.thoughtCount} 条笔记</small></span></label>)}</div>
        {visibleBooks.length === 0 && <p className="ai-book-picker__none">没有找到有笔记的书</p>}
      </section>}
      {!answer && !loading && !error && <section className="ai-empty"><span><Sparkles size={25}/></span><h2>{modeLabels[mode]}</h2><p>系统会检索相关划线与想法，只把有限上下文发送给已配置模型。</p></section>}
      {loading && <div className="ai-thinking"><Sparkles/><div><strong>正在检索证据并组织回答</strong><span>{mode === "ask" ? "从本地知识库筛选最相关的 20 条笔记" : mode === "compare" ? `正在分析 ${bookIds.length} 本书，每本至少提取 5 条代表性笔记` : "正在提取这本书的代表性笔记"}</span></div></div>}
      {error && <div className="ai-error"><strong>无法生成回答</strong><p>{error}</p></div>}
      {answer && <section className="answer"><div className="answer__question">{question}</div><div className="answer__body"><Sparkles size={18}/><p>{answer.content.split(/(\[\d+\])/g).map((part, index) => { const match = part.match(/^\[(\d+)\]$/); const citation = match && answer.citations.find(item => item.index === Number(match[1])); return citation ? <button className="answer__source-link" key={`${part}-${index}`} onClick={() => openBook(citation.note.bookId, citation.note.id)}>{part}</button> : <Fragment key={`${part}-${index}`}>{part}</Fragment>; })}</p></div><div className="answer__sources-head"><h3>引用的笔记</h3><span>检索 {answer.sourcesConsidered} 条 · 引用 {answer.citations.length} 条</span></div>{answer.citations.map(citation => <button className="citation" key={`${citation.index}-${citation.note.id}`} onClick={() => openBook(citation.note.bookId, citation.note.id)}><span>{citation.index}</span><div><strong><BookOpen size={14}/>《{citation.note.bookTitle}》 · {citation.note.chapter}</strong><p>{citation.note.content}</p></div></button>)}</section>}
      <form className="ask-box" onSubmit={ask}><textarea value={question} onChange={event => setQuestion(event.target.value)} placeholder="问问你的阅读知识库…"/><button disabled={!question.trim() || loading}><ArrowUp size={18}/></button><small>回答严格基于引用笔记；点击引用可定位原始笔记</small></form>
    </div>
  </>;
}
