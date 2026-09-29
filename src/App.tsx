import { lazy, Suspense, useEffect, useLayoutEffect, useRef, type ReactNode } from "react";
import { motion } from "motion/react";
import { AppLayout } from "./components/layout/AppLayout";
import { SearchDialog } from "./components/search/SearchDialog";
import { useAppStore, type Page } from "./stores/app";
import { Dashboard } from "./pages/Dashboard";
import { Books } from "./pages/Books";
import { BookDetail } from "./pages/BookDetail";
import { Notes } from "./pages/Notes";
import { SearchPage } from "./pages/Search";
import { AI } from "./pages/AI";
import { Settings } from "./pages/Settings";
const KnowledgeGraph = lazy(() => import("./pages/KnowledgeGraph").then(module => ({ default: module.KnowledgeGraph })));
const pages: Record<Page, ReactNode> = {dashboard:<Dashboard/>,books:<Books/>,highlights:<Notes type="highlight"/>,thoughts:<Notes type="thought"/>,search:<SearchPage/>,ai:<AI/>,graph:<Suspense fallback={<div className="notes-loading">正在打开图谱…</div>}><KnowledgeGraph/></Suspense>,settings:<Settings/>};
export default function App(){
  const {page,theme,fontFamily,selectedBookId}=useAppStore();
  const visitedPages=useRef(new Set<Page>()).current;
  const scrollPositions=useRef<Partial<Record<Page,number>>>({});
  visitedPages.add(page);
  useEffect(()=>{document.documentElement.dataset.theme=theme},[theme]);
  useEffect(()=>{document.documentElement.dataset.font=fontFamily},[fontFamily]);
  useLayoutEffect(()=>{
    const frame=requestAnimationFrame(()=>window.scrollTo({top:scrollPositions.current[page]??0}));
    return ()=>{cancelAnimationFrame(frame);scrollPositions.current[page]=window.scrollY};
  },[page]);
  return <AppLayout>{(Object.entries(pages) as [Page,ReactNode][]).map(([id,content])=>visitedPages.has(id)&&<motion.div className={`page page--module page--${id}`} key={id} hidden={page!==id} initial={{opacity:0,y:3}} animate={{opacity:1,y:0}} transition={{duration:.12}}>{content}</motion.div>)}<SearchDialog/>{selectedBookId&&<BookDetail/>}</AppLayout>
}

