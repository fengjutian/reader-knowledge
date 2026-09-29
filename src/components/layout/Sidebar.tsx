import { LayoutDashboard, Library, Highlighter, Lightbulb, Search, Sparkles, Settings, Moon, Sun, BookOpen, Share2 } from "lucide-react";
import { useAppStore, type Page } from "../../stores/app";

const items: { id: Page; label: string; icon: typeof Library }[] = [
  { id: "dashboard", label: "概览", icon: LayoutDashboard }, { id: "books", label: "书籍", icon: Library },
  { id: "highlights", label: "划线", icon: Highlighter }, { id: "thoughts", label: "想法", icon: Lightbulb },
  { id: "search", label: "搜索", icon: Search },
  { id: "ai", label: "AI 助手", icon: Sparkles },
  { id: "graph", label: "知识图谱", icon: Share2 },
];

export function Sidebar() {
  const { page, setPage, theme, toggleTheme, setSearchOpen } = useAppStore();
  return <aside className="sidebar">
    <div className="brand"><span className="brand__mark"><BookOpen size={18}/></span><span>wereader-knowledge</span></div>
    <button className="quick-search" onClick={() => setSearchOpen(true)}><Search size={15}/><span>搜索知识库</span><kbd>⌘ K</kbd></button>
    <nav className="nav">{items.map(item => <button key={item.id} className={page === item.id ? "active" : ""} onClick={() => setPage(item.id)}><item.icon size={17}/>{item.label}</button>)}</nav>
    <div className="sidebar__footer">
      <button onClick={() => setPage("settings")} className={page === "settings" ? "active" : ""}><Settings size={17}/>设置</button>
      <button onClick={toggleTheme}>{theme === "light" ? <Moon size={17}/> : <Sun size={17}/>}切换{theme === "light" ? "深色" : "浅色"}</button>
    </div>
  </aside>;
}

