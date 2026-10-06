import * as Dialog from "@radix-ui/react-dialog";
import { Search, X, Highlighter, Lightbulb } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../../api/tauri";
import { useAppStore } from "../../stores/app";
import type { SearchResult } from "../../types/domain";

function readableError(error: unknown) {
  const message = error instanceof Error ? error.message : String(error);
  return message.includes("429") ? "请求过于频繁，请稍后再试。" : message.length > 160 ? `${message.slice(0, 160)}…` : message;
}

export function SearchDialog() {
  const { searchOpen, setSearchOpen, openBook } = useAppStore();
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<SearchResult[]>([]);
  const [activeIndex, setActiveIndex] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  // 请求序号：只有最新一次请求的结果才允许写入 state。
  const requestIdRef = useRef(0);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") { event.preventDefault(); setSearchOpen(true); }
    };
    addEventListener("keydown", onKey);
    return () => removeEventListener("keydown", onKey);
  }, [setSearchOpen]);

  useEffect(() => {
    const term = query.trim();
    if (!term) { setResults([]); setError(""); setLoading(false); return; }
    const requestId = requestIdRef.current + 1;
    requestIdRef.current = requestId;
    setLoading(true); setError("");
    const timer = window.setTimeout(() => {
      api.search(term)
        .then(value => {
          if (requestIdRef.current !== requestId) return;
          setResults(value);
          setActiveIndex(0);
        })
        .catch(reason => {
          if (requestIdRef.current !== requestId) return;
          // 失败必须显示错误，不能伪装成"没有结果"。
          setResults([]);
          setError(readableError(reason));
        })
        .finally(() => { if (requestIdRef.current === requestId) setLoading(false); });
    }, 160);
    return () => window.clearTimeout(timer);
  }, [query]);

  const open = useCallback((result: SearchResult) => {
    openBook(result.bookId, result.id);
    setSearchOpen(false);
    setQuery("");
    setResults([]);
    setActiveIndex(0);
  }, [openBook, setSearchOpen]);

  function onKeyDown(event: React.KeyboardEvent) {
    if (event.key === "ArrowDown") { event.preventDefault(); setActiveIndex(current => results.length ? (current + 1) % results.length : 0); }
    else if (event.key === "ArrowUp") { event.preventDefault(); setActiveIndex(current => results.length ? (current - 1 + results.length) % results.length : 0); }
    else if (event.key === "Enter") { event.preventDefault(); const result = results[activeIndex]; if (result) open(result); }
    else if (event.key === "Escape") { setSearchOpen(false); }
  }

  const showEmpty = !!query.trim() && !loading && !error && results.length === 0;

  return <Dialog.Root open={searchOpen} onOpenChange={setSearchOpen}><AnimatePresence>{searchOpen && <Dialog.Portal forceMount><Dialog.Overlay asChild><motion.div className="dialog-overlay" initial={{opacity:0}} animate={{opacity:1}} exit={{opacity:0}}/></Dialog.Overlay><Dialog.Content asChild><motion.div className="search-dialog" initial={{opacity:0,y:-18,scale:.98}} animate={{opacity:1,y:0,scale:1}} exit={{opacity:0,y:-10}} onKeyDown={onKeyDown}><Dialog.Title className="sr-only">搜索阅读记录</Dialog.Title><div className="search-dialog__input"><Search size={20}/><input autoFocus placeholder="搜索你的阅读记录" role="combobox" aria-expanded={results.length > 0} aria-controls="search-results" aria-activedescendant={results[activeIndex] ? `search-result-${results[activeIndex].id}` : undefined} value={query} onChange={e => setQuery(e.target.value)}/><Dialog.Close className="icon-button"><X size={18}/></Dialog.Close></div><div className="search-dialog__body" id="search-results" role="listbox">{!query && <div className="search-hint">输入书名、章节、划线或想法<br/><kbd>↑</kbd> <kbd>↓</kbd> 选择 · <kbd>Enter</kbd> 打开</div>}{loading && <div className="search-hint">正在搜索…</div>}{error && <div className="search-hint search-hint--error">搜索失败：{error}</div>}{showEmpty && <div className="search-hint">没有找到相关记录</div>}{results.map((result, index) => <button type="button" role="option" aria-selected={index === activeIndex} id={`search-result-${result.id}`} className={`search-result${index === activeIndex ? " is-active" : ""}`} key={result.id} onMouseEnter={() => setActiveIndex(index)} onClick={() => open(result)}>{result.type === "thought" ? <Lightbulb size={16}/> : <Highlighter size={16}/>}<div><strong>{result.content}</strong><small>《{result.bookTitle}》 · {result.chapter}</small></div></button>)}</div></motion.div></Dialog.Content></Dialog.Portal>}</AnimatePresence></Dialog.Root>;
}
