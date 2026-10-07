import * as Dialog from "@radix-ui/react-dialog";
import { Search, X, Highlighter, Lightbulb, BookOpen, Loader2, ChevronDown } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "../../api/tauri";
import { useAppStore } from "../../stores/app";
import type { Book, GlobalSearchResult, SearchEntityType } from "../../types/domain";

const PAGE_SIZE = 30;
const DEBOUNCE_MS = 200;

type TypeFilter = "all" | SearchEntityType;
const typeOptions: { value: TypeFilter; label: string }[] = [
  { value: "all", label: "全部" },
  { value: "book", label: "书籍" },
  { value: "highlight", label: "划线" },
  { value: "thought", label: "想法" },
];

function readableError(error: unknown) {
  const message = error instanceof Error ? error.message : String(error);
  return message.includes("429") ? "请求过于频繁，请稍后再试。" : message.length > 160 ? `${message.slice(0, 160)}…` : message;
}

function resultIcon(type: SearchEntityType) {
  if (type === "book") return <BookOpen size={16}/>;
  if (type === "thought") return <Lightbulb size={16}/>;
  return <Highlighter size={16}/>;
}

export function SearchDialog() {
  const { searchOpen, setSearchOpen, openBook } = useAppStore();
  const [query, setQuery] = useState("");
  const [type, setType] = useState<TypeFilter>("all");
  const [bookId, setBookId] = useState("");
  const [books, setBooks] = useState<Book[]>([]);
  const [results, setResults] = useState<GlobalSearchResult[]>([]);
  const [activeIndex, setActiveIndex] = useState(0);
  const [loading, setLoading] = useState(false);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState("");
  const [hasMore, setHasMore] = useState(false);
  // 请求序号：只有最新一次请求的结果才允许写入 state。
  const requestIdRef = useRef(0);

  useEffect(() => {
    if (!searchOpen) return;
    api.books().then(setBooks).catch(() => setBooks([]));
  }, [searchOpen]);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") { event.preventDefault(); setSearchOpen(true); }
    };
    addEventListener("keydown", onKey);
    return () => removeEventListener("keydown", onKey);
  }, [setSearchOpen]);

  const bookFilterActive = bookId.trim().length > 0;
  // 书籍列表可能上千本，只提供匹配的前 60 本，避免渲染巨大下拉。
  const bookOptions = useMemo(() => {
    const query = bookId.trim().toLocaleLowerCase();
    return books
      .filter(book => !query || `${book.title} ${book.author}`.toLocaleLowerCase().includes(query))
      .slice(0, 60);
  }, [bookId, books]);

  useEffect(() => {
    const term = query.trim();
    // 空查询不请求：搜索框刚打开时不打后端。
    if (!term && !bookFilterActive) { setResults([]); setError(""); setLoading(false); setHasMore(false); return; }
    const requestId = requestIdRef.current + 1;
    requestIdRef.current = requestId;
    setLoading(true); setError("");
    const timer = window.setTimeout(() => {
      api.globalSearch({ query: term, types: type === "all" ? [] : [type], bookId: bookFilterActive ? bookId.trim() : undefined, limit: PAGE_SIZE, offset: 0 })
        .then(page => {
          if (requestIdRef.current !== requestId) return;
          setResults(page.results);
          setHasMore(page.hasMore);
          setActiveIndex(0);
        })
        .catch(reason => {
          if (requestIdRef.current !== requestId) return;
          // 失败必须显示错误，不能伪装成"没有结果"。
          setResults([]);
          setHasMore(false);
          setError(readableError(reason));
        })
        .finally(() => { if (requestIdRef.current === requestId) setLoading(false); });
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [query, type, bookId, bookFilterActive]);

  const open = useCallback((result: GlobalSearchResult) => {
    // 书籍结果没有笔记可定位，只打开书籍详情。
    openBook(result.bookId, result.type === "book" ? undefined : result.id);
    setSearchOpen(false);
    setQuery("");
    setResults([]);
    setActiveIndex(0);
  }, [openBook, setSearchOpen]);

  function loadMore() {
    if (loadingMore || !hasMore) return;
    const requestId = requestIdRef.current;
    setLoadingMore(true);
    api.globalSearch({ query: query.trim(), types: type === "all" ? [] : [type], bookId: bookFilterActive ? bookId.trim() : undefined, limit: PAGE_SIZE, offset: results.length })
      .then(page => {
        // 翻页期间用户又改了条件，整页结果作废。
        if (requestIdRef.current !== requestId) return;
        setResults(current => [...current, ...page.results]);
        setHasMore(page.hasMore);
      })
      .catch(reason => { if (requestIdRef.current === requestId) setError(readableError(reason)); })
      .finally(() => { if (requestIdRef.current === requestId) setLoadingMore(false); });
  }

  function onKeyDown(event: React.KeyboardEvent) {
    if (event.key === "ArrowDown") { event.preventDefault(); setActiveIndex(current => results.length ? (current + 1) % results.length : 0); }
    else if (event.key === "ArrowUp") { event.preventDefault(); setActiveIndex(current => results.length ? (current - 1 + results.length) % results.length : 0); }
    else if (event.key === "Enter") { event.preventDefault(); const result = results[activeIndex]; if (result) open(result); }
    else if (event.key === "Escape") { setSearchOpen(false); }
  }

  function resetFilters() {
    setQuery(""); setType("all"); setBookId(""); setResults([]); setActiveIndex(0); setError(""); setHasMore(false);
  }

  const showEmpty = (!!query.trim() || bookFilterActive) && !loading && !error && results.length === 0;
  const showHint = !query.trim() && !bookFilterActive;

  return <Dialog.Root open={searchOpen} onOpenChange={setSearchOpen}><AnimatePresence>{searchOpen && <Dialog.Portal forceMount><Dialog.Overlay asChild><motion.div className="dialog-overlay" initial={{opacity:0}} animate={{opacity:1}} exit={{opacity:0}}/></Dialog.Overlay><Dialog.Content asChild><motion.div className="search-dialog" initial={{opacity:0,y:-18,scale:.98}} animate={{opacity:1,y:0,scale:1}} exit={{opacity:0,y:-10}} onKeyDown={onKeyDown}><Dialog.Title className="sr-only">搜索阅读记录</Dialog.Title><div className="search-dialog__input"><Search size={20}/><input autoFocus placeholder="搜索书名、作者、ISBN、划线或想法" role="combobox" aria-expanded={results.length > 0} aria-controls="search-results" aria-activedescendant={results[activeIndex] ? `search-result-${results[activeIndex].id}` : undefined} value={query} onChange={e => setQuery(e.target.value)}/><Dialog.Close className="icon-button"><X size={18}/></Dialog.Close></div><div className="search-dialog__filters"><div className="search-dialog__chips" role="group" aria-label="搜索类型">{typeOptions.map(option => <button type="button" key={option.value} className={`search-chip${type === option.value ? " is-active" : ""}`} aria-pressed={type === option.value} onClick={() => setType(option.value)}>{option.label}</button>)}</div><div className="search-dialog__book"><BookOpen size={15}/><input list="search-book-options" placeholder="限定书籍（可留空）" aria-label="限定书籍" value={bookId} onChange={e => setBookId(e.target.value)}/><datalist id="search-book-options">{bookOptions.map(book => <option key={book.id} value={book.title}/>)}</datalist>{bookFilterActive && <button type="button" className="search-dialog__book-clear" aria-label="清除书籍过滤" onClick={() => setBookId("")}><X size={14}/></button>}<ChevronDown size={14}/></div></div><div className="search-dialog__body" id="search-results" role="listbox">{showHint && <div className="search-hint">输入书名、章节、划线或想法<br/><kbd>↑</kbd> <kbd>↓</kbd> 选择 · <kbd>Enter</kbd> 打开</div>}{loading && <div className="search-hint">正在搜索…</div>}{error && <div className="search-hint search-hint--error">搜索失败：{error}</div>}{showEmpty && <div className="search-hint">没有找到相关记录<button type="button" className="search-hint__reset" onClick={resetFilters}>清除筛选条件</button></div>}{results.map((result, index) => <button type="button" role="option" aria-selected={index === activeIndex} id={`search-result-${result.id}`} className={`search-result${index === activeIndex ? " is-active" : ""}`} key={`${result.type}-${result.id}`} onMouseEnter={() => setActiveIndex(index)} onClick={() => open(result)}>{resultIcon(result.type)}<div><strong>{result.type === "book" ? result.title : result.snippet}</strong><small>《{result.title}》{result.subtitle && ` · ${result.subtitle}`}</small></div></button>)}{hasMore && <button type="button" className="search-dialog__more" onClick={loadMore} disabled={loadingMore}>{loadingMore ? <><Loader2 size={14} className="search-dialog__spinner"/>加载中…</> : "加载更多"}</button>}</div></motion.div></Dialog.Content></Dialog.Portal>}</AnimatePresence></Dialog.Root>;
}
