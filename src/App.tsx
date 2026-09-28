import { useEffect } from "react";
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
const pages={dashboard:<Dashboard/>,books:<Books/>,bookDetail:<BookDetail/>,highlights:<Notes type="highlight"/>,thoughts:<Notes type="thought"/>,search:<SearchPage/>,ai:<AI/>,settings:<Settings/>};
export default function App(){const {page,theme}=useAppStore();useEffect(()=>{document.documentElement.dataset.theme=theme},[theme]);return <AppLayout><motion.div className="page" key={page} initial={{opacity:0,y:3}} animate={{opacity:1,y:0}} transition={{duration:.12}}>{pages[page]}</motion.div><SearchDialog/></AppLayout>}

