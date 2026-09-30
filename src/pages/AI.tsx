import * as Dialog from "@radix-ui/react-dialog";
import { ArrowUp, BookOpen, ChevronRight, MessageSquare, PanelLeftClose, PanelLeftOpen, Plus, Search, Sparkles, Trash2, X } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { api } from "../api/tauri";
import { useAppStore } from "../stores/app";
import type { AiAnswer, AiMode, AiTurn, Book } from "../types/domain";

const modeLabels: Record<AiMode, string> = { ask: "全库提问", summary: "单书总结", compare: "跨书分析" };
const historyKey = "readflow-ai-history";

interface AiConversation {
  id: string;
  title: string;
  turns: AiTurn[];
  mode: AiMode;
  bookIds: string[];
  createdAt: number;
}

function loadHistory(): AiConversation[] {
  try {
    const saved = JSON.parse(localStorage.getItem(historyKey) || "[]") as (AiConversation & { question?: string; answer?: AiAnswer })[];
    return saved.map(item => item.turns ? item : { id: item.id, title: item.question || "历史对话", turns: item.question && item.answer ? [{ question: item.question, answer: item.answer }] : [], mode: item.mode, bookIds: item.bookIds, createdAt: item.createdAt });
  }
  catch { return []; }
}

function MarkdownAnswer({ answer, openBook }: { answer: AiAnswer; openBook: (bookId: string, noteId?: string) => void }) {
  const markdown = answer.content.replace(/(?<!\\)\[(\d+)\]/g, "[[$1]](citation:$1)");
  return <ReactMarkdown
    remarkPlugins={[remarkGfm]}
    urlTransform={url => url.startsWith("citation:") ? url : url}
    components={{
      a: ({ href, children }) => {
        if (href?.startsWith("citation:")) {
          const index = Number(href.slice("citation:".length));
          const citation = answer.citations.find(item => item.index === index);
          return citation ? <button type="button" className="answer__source-link" onClick={() => openBook(citation.note.bookId, citation.note.id)}>{children}</button> : <>{children}</>;
        }
        return <a href={href} target="_blank" rel="noreferrer">{children}</a>;
      },
    }}
  >{markdown}</ReactMarkdown>;
}

export function AI() {
  const initialDraft = useRef(useAppStore.getState().aiDraft).current;
  const aiDraft = useAppStore(state => state.aiDraft);
  const [question, setQuestion] = useState(initialDraft?.question ?? "");
  const [mode, setMode] = useState<AiMode>(initialDraft?.mode ?? "ask");
  const [books, setBooks] = useState<Book[]>([]);
  const [bookIds, setBookIds] = useState<string[]>(initialDraft?.bookIds ?? []);
  const [draftBookIds, setDraftBookIds] = useState<string[]>([]);
  const [bookPickerOpen, setBookPickerOpen] = useState(false);
  const [selectionError, setSelectionError] = useState("");
  const [bookQuery, setBookQuery] = useState("");
  const [turns, setTurns] = useState<AiTurn[]>([]);
  const [history, setHistory] = useState<AiConversation[]>(loadHistory);
  const [activeConversationId, setActiveConversationId] = useState<string>();
  const [historyCollapsed, setHistoryCollapsed] = useState(true);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const openBook = useAppStore(state => state.openBook);
  useEffect(() => {
    if (!aiDraft) return;
    setQuestion(aiDraft.question);
    setMode(aiDraft.mode);
    setBookIds(aiDraft.bookIds);
    setTurns([]);
    setActiveConversationId(undefined);
    setError("");
    useAppStore.getState().clearAiDraft();
  }, [aiDraft]);
  useEffect(() => { localStorage.setItem(historyKey, JSON.stringify(history)); }, [history]);
  useEffect(() => { api.books().then(setBooks).catch(() => setBooks([])); }, []);
  useEffect(() => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    textarea.style.height = "0";
    textarea.style.height = `${Math.min(textarea.scrollHeight, 160)}px`;
  }, [question]);
  useEffect(() => { requestAnimationFrame(() => scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight, behavior: "smooth" })); }, [turns, loading]);
  const selectedBooks = useMemo(() => bookIds.map(id => books.find(book => book.id === id)).filter((book): book is Book => Boolean(book)), [bookIds, books]);
  const visibleBooks = useMemo(() => {
    const query = bookQuery.trim().toLocaleLowerCase();
    return books
      .filter(book => book.highlightCount + book.thoughtCount > 0)
      .filter(book => !query || `${book.title} ${book.author}`.toLocaleLowerCase().includes(query))
      .sort((a, b) => (b.highlightCount + b.thoughtCount) - (a.highlightCount + a.thoughtCount))
      .slice(0, 100);
  }, [bookQuery, books]);
  function changeMode(next: AiMode) {
    setMode(next); setTurns([]); setActiveConversationId(undefined); setError(""); setBookIds([]); setBookQuery("");
    setQuestion(next === "summary" ? "请总结这本书的核心观点，并区分原文与我的想法。" : next === "compare" ? "请比较这些书对同一主题的观点、共识与分歧。" : "");
  }
  function newConversation() {
    setQuestion(""); setTurns([]); setActiveConversationId(undefined); setError(""); setBookIds([]); setBookQuery("");
  }
  function openConversation(conversation: AiConversation) {
    setActiveConversationId(conversation.id); setQuestion(""); setTurns(conversation.turns);
    setMode(conversation.mode); setBookIds(conversation.bookIds); setError("");
  }
  function deleteConversation(id: string) {
    setHistory(current => current.filter(item => item.id !== id));
    if (activeConversationId === id) newConversation();
  }
  function openBookPicker() {
    setDraftBookIds(bookIds);
    setBookQuery("");
    setSelectionError("");
    setBookPickerOpen(true);
  }
  function toggleDraftBook(id: string) {
    setDraftBookIds(current => {
      if (mode === "summary") return [id];
      if (current.includes(id)) return current.filter(value => value !== id);
      if (current.length >= 100) { setSelectionError("跨书分析最多选择 100 本书"); return current; }
      setSelectionError("");
      return [...current, id];
    });
  }
  function confirmBookSelection() {
    if (mode === "summary" && draftBookIds.length !== 1) { setSelectionError("请选择一本书"); return; }
    if (mode === "compare" && draftBookIds.length < 2) { setSelectionError("请至少选择两本书"); return; }
    setBookIds(draftBookIds);
    setError("");
    setBookPickerOpen(false);
  }
  async function ask(event: React.FormEvent) {
    event.preventDefault(); if (!question.trim()) return;
    if (mode === "summary" && bookIds.length !== 1) { setError("请选择一本书进行总结"); return; }
    if (mode === "compare" && bookIds.length < 2) { setError("请至少选择两本书进行跨书分析"); return; }
    const submitted = question.trim();
    setLoading(true); setError(""); setQuestion("");
    try {
      const nextAnswer = await api.ask({ question: submitted, mode, bookIds, history: turns.map(turn => ({ question: turn.question, answer: turn.answer.content })) });
      const nextTurns = [...turns, { question: submitted, answer: nextAnswer }].slice(-50);
      const id = activeConversationId || crypto.randomUUID();
      const conversation: AiConversation = { id, title: turns[0]?.question || submitted, turns: nextTurns, mode, bookIds: [...bookIds], createdAt: Date.now() };
      setTurns(nextTurns); setActiveConversationId(id);
      setHistory(current => [conversation, ...current.filter(item => item.id !== id)].slice(0, 30));
    }
    catch (reason) { setError(reason instanceof Error ? reason.message : String(reason)); }
    finally { setLoading(false); }
  }
  return <div className="ai-page">
    <div className={`ai-layout${historyCollapsed ? " ai-layout--history-collapsed" : ""}`}>
      <aside className={`ai-history${historyCollapsed ? " is-collapsed" : ""}`}>
        <button className="ai-history__collapse" onClick={() => setHistoryCollapsed(value => !value)} aria-label={historyCollapsed ? "展开最近对话" : "折叠最近对话"} title={historyCollapsed ? "展开最近对话" : "折叠最近对话"}>{historyCollapsed ? <PanelLeftOpen size={17}/> : <PanelLeftClose size={17}/>}</button>
        <div className="ai-history__content">
        <button className="ai-history__new" onClick={newConversation}><Plus size={16}/>新对话</button>
        <div className="ai-history__title">最近对话</div>
        <div className="ai-history__list">
          {history.map(conversation => <div key={conversation.id} className={`ai-history__item ${activeConversationId === conversation.id ? "active" : ""}`}>
            <button className="ai-history__open" onClick={() => openConversation(conversation)} title={conversation.title}><MessageSquare size={14}/><span>{conversation.title}</span></button>
            <button className="ai-history__delete" onClick={() => deleteConversation(conversation.id)} aria-label="删除对话"><Trash2 size={14}/></button>
          </div>)}
          {history.length === 0 && <p className="ai-history__empty">提问后，对话会保存在这里</p>}
        </div>
        </div>
      </aside>
      <div className="ai-wrap">
      <div className="ai-controls">
      <div className="ai-modes">{(Object.keys(modeLabels) as AiMode[]).map(value => <button key={value} className={mode === value ? "active" : ""} onClick={() => changeMode(value)}>{modeLabels[value]}</button>)}</div>
      {(mode === "summary" || mode === "compare") && <section className="ai-book-picker">
        <button type="button" className="ai-book-picker__trigger" onClick={openBookPicker}><div><strong>{mode === "summary" ? "选择一本书" : "选择分析书籍"}</strong><span>{bookIds.length ? `已选择 ${bookIds.length} 本` : mode === "summary" ? "从有笔记的书籍中选择" : "请选择 2–100 本书"}</span></div><ChevronRight size={18}/></button>
        {selectedBooks.length > 0 && <div className="ai-book-picker__selected">{selectedBooks.map(book => <span key={book.id} title={book.title}>{book.title}</span>)}</div>}
      </section>}
      <Dialog.Root open={bookPickerOpen} onOpenChange={setBookPickerOpen}><Dialog.Portal><Dialog.Overlay className="dialog-overlay"/><Dialog.Content className="book-picker-dialog"><div className="book-picker-dialog__head"><div><Dialog.Title>{mode === "summary" ? "选择一本书" : "选择分析书籍"}</Dialog.Title><Dialog.Description>{mode === "summary" ? "仅显示有笔记的书" : "选择 2–100 本书进行跨书分析"}</Dialog.Description></div><Dialog.Close className="icon-button" aria-label="关闭"><X size={19}/></Dialog.Close></div><label className="ai-book-search"><Search size={16}/><input autoFocus value={bookQuery} onChange={event => setBookQuery(event.target.value)} placeholder="搜索书名或作者"/></label><div className="ai-book-options">{visibleBooks.map(book => <label key={book.id} className={draftBookIds.includes(book.id) ? "selected" : ""}><input type={mode === "summary" ? "radio" : "checkbox"} checked={draftBookIds.includes(book.id)} onChange={() => toggleDraftBook(book.id)}/><span><strong>{book.title}</strong><small>{book.author || "未知作者"} · {book.highlightCount + book.thoughtCount} 条笔记</small></span></label>)}</div>{visibleBooks.length === 0 && <p className="ai-book-picker__none">没有找到有笔记的书</p>}<div className="book-picker-dialog__footer"><span className={selectionError ? "book-picker-dialog__error" : ""}>{selectionError || `已选择 ${draftBookIds.length} 本`}</span><div><Dialog.Close className="book-picker-dialog__cancel">取消</Dialog.Close><button type="button" className="book-picker-dialog__confirm" onClick={confirmBookSelection}>完成</button></div></div></Dialog.Content></Dialog.Portal></Dialog.Root>
      </div>
      <div className="ai-content" ref={scrollRef}>
      {turns.length === 0 && !loading && !error && <section className="ai-empty"><span><Sparkles size={25}/></span><h2>{modeLabels[mode]}</h2><p>系统会检索相关划线与想法，只把有限上下文发送给已配置模型。</p></section>}
      {error && <div className="ai-error"><strong>无法生成回答</strong><p>{error}</p></div>}
      {turns.length > 0 && <div className="ai-thread">{turns.map((turn, turnIndex) => <section className="answer" key={`${turn.question}-${turnIndex}`}><div className="answer__question">{turn.question}</div><div className="answer__body"><Sparkles size={18}/><div className="answer__markdown"><MarkdownAnswer answer={turn.answer} openBook={openBook}/></div></div><div className="answer__sources-head"><h3>引用的笔记</h3><span>检索 {turn.answer.sourcesConsidered} 条 · 引用 {turn.answer.citations.length} 条</span></div>{turn.answer.citations.map(citation => <button className="citation" key={`${turnIndex}-${citation.index}-${citation.note.id}`} onClick={() => openBook(citation.note.bookId, citation.note.id)}><span>{citation.index}</span><div><strong><BookOpen size={14}/>《{citation.note.bookTitle}》 · {citation.note.chapter}</strong><p>{citation.note.content}</p></div></button>)}</section>)}</div>}
      {loading && <div className="ai-thinking"><Sparkles/><div><strong>正在检索证据并组织回答</strong><span>{mode === "ask" ? "从本地知识库筛选相关笔记，并覆盖更多书籍" : mode === "compare" ? `正在分析 ${bookIds.length} 本书，保证每本书都有证据进入上下文` : "正在提取这本书的代表性笔记"}</span></div></div>}
      </div>
      <form className="ask-box" onSubmit={ask}>
        <div className="ask-box__composer">
          <textarea ref={textareaRef} rows={1} value={question} onChange={event => setQuestion(event.target.value)} onKeyDown={event => { if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); event.currentTarget.form?.requestSubmit(); } }} placeholder="问问你的阅读知识库…"/>
          <button type="submit" aria-label="发送" disabled={!question.trim() || loading}><ArrowUp size={18}/></button>
        </div>
      </form>
      </div>
    </div>
  </div>;
}
