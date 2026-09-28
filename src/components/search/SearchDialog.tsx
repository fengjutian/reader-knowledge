import * as Dialog from "@radix-ui/react-dialog";
import { Search, X, Highlighter, Lightbulb } from "lucide-react";
import { AnimatePresence, motion } from "motion/react";
import { useEffect, useState } from "react";
import { api } from "../../api/tauri";
import { useAppStore } from "../../stores/app";
import type { SearchResult } from "../../types/domain";

export function SearchDialog() {
  const { searchOpen, setSearchOpen } = useAppStore(); const [query, setQuery] = useState(""); const [results, setResults] = useState<SearchResult[]>([]);
  useEffect(() => { const onKey = (event: KeyboardEvent) => { if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "k") { event.preventDefault(); setSearchOpen(true); } }; addEventListener("keydown", onKey); return () => removeEventListener("keydown", onKey); }, [setSearchOpen]);
  useEffect(() => { if (!query.trim()) return setResults([]); const timer = setTimeout(() => api.search(query).then(setResults), 160); return () => clearTimeout(timer); }, [query]);
  return <Dialog.Root open={searchOpen} onOpenChange={setSearchOpen}><AnimatePresence>{searchOpen && <Dialog.Portal forceMount><Dialog.Overlay asChild><motion.div className="dialog-overlay" initial={{opacity:0}} animate={{opacity:1}} exit={{opacity:0}}/></Dialog.Overlay><Dialog.Content asChild><motion.div className="search-dialog" initial={{opacity:0,y:-18,scale:.98}} animate={{opacity:1,y:0,scale:1}} exit={{opacity:0,y:-10}}><Dialog.Title className="sr-only">搜索阅读记录</Dialog.Title><div className="search-dialog__input"><Search size={20}/><input autoFocus placeholder="搜索你的阅读记录" value={query} onChange={e => setQuery(e.target.value)}/><Dialog.Close className="icon-button"><X size={18}/></Dialog.Close></div><div className="search-dialog__body">{!query && <div className="search-hint">输入书名、章节、划线或想法<br/><kbd>↑</kbd> <kbd>↓</kbd> 选择 · <kbd>Enter</kbd> 打开</div>}{query && !results.length && <div className="search-hint">没有找到相关记录</div>}{results.map(result => <div className="search-result" key={result.id}>{result.type === "thought" ? <Lightbulb size={16}/> : <Highlighter size={16}/>}<div><strong>{result.content}</strong><small>《{result.bookTitle}》 · {result.chapter}</small></div></div>)}</div></motion.div></Dialog.Content></Dialog.Portal>}</AnimatePresence></Dialog.Root>;
}

