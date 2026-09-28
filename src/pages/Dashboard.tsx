import { useEffect, useState } from "react";
import { ArrowUpRight, BookOpen, Highlighter, Lightbulb, RefreshCw } from "lucide-react";
import { motion } from "motion/react";
import { api } from "../api/tauri";
import { Button } from "../components/ui/Button";
import { PageHeader } from "../components/ui/PageHeader";
import { useSyncStore } from "../stores/sync";
import type { DashboardStats } from "../types/domain";

export function Dashboard() {
  const [stats, setStats] = useState<DashboardStats>({books:0,highlights:0,thoughts:0}); const sync = useSyncStore(); useEffect(() => { api.dashboard().then(setStats) }, [sync.status, sync.progress]);
  const live = sync.status === "processing" || sync.status === "complete";
  const cards = [{ label:"书籍", value:live?sync.books:stats.books, icon:BookOpen },{label:"划线",value:live?sync.highlights:stats.highlights,icon:Highlighter},{label:"想法",value:live?sync.thoughts:stats.thoughts,icon:Lightbulb}];
  return <><PageHeader title="我的阅读" subtitle="把读过的内容，变成可以继续生长的知识。" actions={<Button icon={<RefreshCw size={16} className={sync.status === "reading" || sync.status === "processing" ? "spin" : ""}/>} onClick={sync.run} disabled={sync.status === "reading" || sync.status === "processing"}>同步微信读书</Button>}/>
    <section className="stats-grid">{cards.map((c,i)=><motion.article className="stat-card" key={c.label} initial={{opacity:0,y:10}} animate={{opacity:1,y:0}} transition={{delay:i*.06}}><div className="stat-card__icon"><c.icon size={18}/></div><strong>{c.value.toLocaleString()}</strong><span>{c.label}</span><ArrowUpRight size={16}/></motion.article>)}</section>
    <section className="dashboard-grid"><article className="panel sync-panel"><div className="panel-title"><div><span className="eyebrow">同步状态</span><h2>{sync.status === "complete" ? "所有内容已是最新" : sync.message || "知识库已准备就绪"}</h2></div><span className={`status-dot status-dot--${sync.status}`}/></div><div className={`progress ${sync.status === "reading" ? "progress--active" : ""}`}><motion.div animate={{width:sync.status === "reading" ? "8%" : `${sync.progress}%`}}/></div><div className="sync-numbers"><span><strong>{live ? sync.books : stats.books}</strong> 本书</span><span><strong>{live ? sync.highlights : stats.highlights}</strong> 条划线</span><span><strong>{live ? sync.thoughts : stats.thoughts}</strong> 条想法</span></div><p>最近同步 {stats.lastSyncedAt || "尚未同步"}</p></article></section>
  </>;
}

