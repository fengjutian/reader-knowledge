import { useEffect } from "react";
import { AnimatePresence, motion } from "motion/react";
import { AppLayout } from "./components/layout/AppLayout";
import { SearchDialog } from "./components/search/SearchDialog";
import { useAppStore } from "./stores/app";
import { Dashboard } from "./pages/Dashboard";
import { Books } from "./pages/Books";
import { Notes } from "./pages/Notes";
import { SearchPage } from "./pages/Search";
import { Settings } from "./pages/Settings";
const pages={dashboard:<Dashboard/>,books:<Books/>,highlights:<Notes type="highlight"/>,thoughts:<Notes type="thought"/>,search:<SearchPage/>,settings:<Settings/>};
export default function App(){const {page,theme}=useAppStore();useEffect(()=>{document.documentElement.dataset.theme=theme},[theme]);return <AppLayout><AnimatePresence mode="wait"><motion.div className="page" key={page} initial={{opacity:0,y:5}} animate={{opacity:1,y:0}} exit={{opacity:0,y:-3}} transition={{duration:.18}}>{pages[page]}</motion.div></AnimatePresence><SearchDialog/></AppLayout>}

