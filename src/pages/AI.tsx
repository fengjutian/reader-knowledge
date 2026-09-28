import { ArrowUp, BookOpen, Sparkles } from "lucide-react";
import { useEffect, useState } from "react";
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
  const [answer, setAnswer] = useState<AiAnswer>();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const openBook = useAppStore(state => state.openBook);
  useEffect(() => { api.books().then(setBooks).catch(() => setBooks([])); }, []);
  function changeMode(next: AiMode) {
    setMode(next); setAnswer(undefined); setError(""); setBookIds([]);
    setQuestion(next === "summary" ? "请总结这本书的核心观点，并区分原文与我的想法。" : next === "compare" ? "请比较这些书对同一主题的观点、共识与分歧。" : "");
  }
  function toggleBook(id: string) {
    setBookIds(current => mode === "summary" ? [id] : current.includes(id) ? current.filter(value => value !== id) : [...current, id]);
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
      {(mode === "summary" || mode === "compare") && <section className="ai-book-picker"><strong>{mode === "summary" ? "选择一本书" : "选择至少两本书"}</strong><div>{books.map(book => <label key={book.id}><input type={mode === "summary" ? "radio" : "checkbox"} checked={bookIds.includes(book.id)} onChange={() => toggleBook(book.id)}/><span>{book.title}</span></label>)}</div></section>}
      {!answer && !loading && !error && <section className="ai-empty"><span><Sparkles size={25}/></span><h2>{modeLabels[mode]}</h2><p>系统会检索相关划线与想法，只把有限上下文发送给已配置模型。</p></section>}
      {loading && <div className="ai-thinking"><Sparkles/><div><strong>正在检索并分析笔记</strong><span>只发送相关的 Top-20 内容</span></div></div>}
      {error && <div className="ai-error"><strong>无法生成回答</strong><p>{error}</p></div>}
      {answer && <section className="answer"><div className="answer__question">{question}</div><div className="answer__body"><Sparkles size={18}/><p>{answer.content}</p></div><h3>引用的笔记</h3>{answer.citations.map(citation => <button className="citation" key={`${citation.index}-${citation.note.id}`} onClick={() => openBook(citation.note.bookId, citation.note.id)}><span>{citation.index}</span><div><strong><BookOpen size={14}/>《{citation.note.bookTitle}》 · {citation.note.chapter}</strong><p>{citation.note.content}</p></div></button>)}</section>}
      <form className="ask-box" onSubmit={ask}><textarea value={question} onChange={event => setQuestion(event.target.value)} placeholder="问问你的阅读知识库…"/><button disabled={!question.trim() || loading}><ArrowUp size={18}/></button><small>回答严格基于引用笔记；点击引用可定位原始笔记</small></form>
    </div>
  </>;
}
