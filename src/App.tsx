import { lazy, Suspense, useEffect } from "react";
import { motion } from "motion/react";
import { AppLayout } from "./components/layout/AppLayout";
import { SearchDialog } from "./components/search/SearchDialog";
import { useAppStore } from "./stores/app";
import { Dashboard } from "./pages/Dashboard";
import { Books } from "./pages/Books";
import { BookDetail } from "./pages/BookDetail";
import { Notes } from "./pages/Notes";
import { SearchPage } from "./pages/Search";
import { AI } from "./pages/AI";
import { Settings } from "./pages/Settings";
const KnowledgeGraph = lazy(() => import("./pages/KnowledgeGraph").then(module => ({ default: module.KnowledgeGraph })));
const pages={dashboard:<Dashboard/>,books:<Books/>,highlights:<Notes type="highlight"/>,thoughts:<Notes type="thought"/>,search:<SearchPage/>,ai:<AI/>,graph:<Suspense fallback={<div className="notes-loading">正在打开图谱…</div>}><KnowledgeGraph/></Suspense>,settings:<Settings/>};
export default function App(){const {page,theme,selectedBookId}=useAppStore();useEffect(()=>{document.documentElement.dataset.theme=theme},[theme]);return <AppLayout><motion.div className="page" key={page} initial={{opacity:0,y:3}} animate={{opacity:1,y:0}} transition={{duration:.12}}>{pages[page]}</motion.div><SearchDialog/>{selectedBookId&&<BookDetail/>}</AppLayout>}

