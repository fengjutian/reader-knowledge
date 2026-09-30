import { lazy, Suspense, useEffect, useLayoutEffect, useRef, type ReactNode } from "react";
import { motion } from "motion/react";
import { AppLayout } from "./components/layout/AppLayout";
import { SearchDialog } from "./components/search/SearchDialog";
import { useAppStore, type Page } from "./stores/app";

const Dashboard=lazy(()=>import("./pages/Dashboard").then(m=>({default:m.Dashboard})));
const Books=lazy(()=>import("./pages/Books").then(m=>({default:m.Books})));
const Discover=lazy(()=>import("./pages/Discover").then(m=>({default:m.Discover})));
const BookDetail=lazy(()=>import("./pages/BookDetail").then(m=>({default:m.BookDetail})));
const Notes=lazy(()=>import("./pages/Notes").then(m=>({default:m.Notes})));
const AI=lazy(()=>import("./pages/AI").then(m=>({default:m.AI})));
const Settings=lazy(()=>import("./pages/Settings").then(m=>({default:m.Settings})));
const DatabasePage=lazy(()=>import("./pages/Database").then(m=>({default:m.DatabasePage})));
const BookMetadata=lazy(()=>import("./pages/BookMetadata").then(m=>({default:m.BookMetadata})));
const KnowledgeGraph=lazy(()=>import("./pages/KnowledgeGraph").then(m=>({default:m.KnowledgeGraph})));
const Glossary=lazy(()=>import("./pages/Glossary").then(m=>({default:m.Glossary})));

const pages:Record<Page,ReactNode>={dashboard:<Dashboard/>,books:<Books/>,discover:<Discover/>,metadata:<BookMetadata/>,notes:<Notes/>,ai:<AI/>,graph:<KnowledgeGraph/>,glossary:<Glossary/>,database:<DatabasePage/>,settings:<Settings/>};
export default function App(){
 const {page,theme,fontFamily,selectedBookId}=useAppStore(); const visitedPages=useRef(new Set<Page>()).current,scrollPositions=useRef<Partial<Record<Page,number>>>({}); visitedPages.add(page);
 useLayoutEffect(()=>{document.documentElement.dataset.theme=theme;delete document.documentElement.dataset.bootTheme},[theme]);
 useEffect(()=>{document.documentElement.dataset.font=fontFamily},[fontFamily]);
 useLayoutEffect(()=>{const frame=requestAnimationFrame(()=>window.scrollTo({top:scrollPositions.current[page]??0}));return()=>{cancelAnimationFrame(frame);scrollPositions.current[page]=window.scrollY}},[page]);
 return <AppLayout>{(Object.entries(pages) as [Page,ReactNode][]).map(([id,content])=>visitedPages.has(id)&&<motion.div className={`page page--module page--${id}`} key={id} hidden={page!==id} initial={{opacity:0,y:3}} animate={{opacity:1,y:0}} transition={{duration:.12}}><Suspense fallback={<div className="notes-loading">正在打开模块…</div>}>{content}</Suspense></motion.div>)}<SearchDialog/>{selectedBookId&&<Suspense fallback={null}><BookDetail/></Suspense>}</AppLayout>;
}
