import { LayoutDashboard, Library, NotebookText, Search, Sparkles, Settings, BookOpen, Share2, Database, Compass, Languages } from "lucide-react";
import { useAppStore, type Page } from "../../stores/app";

const items: { id: Page; label: string; icon: typeof Library }[] = [
  { id: "dashboard", label: "概览", icon: LayoutDashboard }, { id: "books", label: "书籍", icon: Library },
  { id: "discover", label: "好书推荐", icon: Compass },
  { id: "metadata", label: "书籍元数据", icon: Database },
  { id: "notes", label: "笔记", icon: NotebookText },
  { id: "ai", label: "AI 助手", icon: Sparkles },
  { id: "graph", label: "知识图谱", icon: Share2 },
  { id: "glossary", label: "名词库", icon: Languages },
  { id: "database", label: "数据库", icon: Database },
];

export function Sidebar() {
  const { page, setPage, setSearchOpen } = useAppStore();
  return <aside className="sidebar">
    <div className="brand"><span className="brand__mark"><BookOpen size={18}/></span><span>wereader</span></div>
    <button className="quick-search" onClick={() => setSearchOpen(true)}><Search size={15}/><span>搜索知识库</span><kbd>⌘ K</kbd></button>
    <nav className="nav">{items.map(item => <button key={item.id} className={page === item.id ? "active" : ""} onClick={() => setPage(item.id)}><item.icon size={17}/>{item.label}</button>)}</nav>
    <div className="sidebar__footer">
      <button onClick={() => setPage("settings")} className={page === "settings" ? "active" : ""}><Settings size={17}/>设置</button>
    </div>
  </aside>;
}

