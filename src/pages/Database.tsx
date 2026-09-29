import { useEffect, useMemo, useState } from "react";
import { BookOpen, Database as DatabaseIcon, HardDrive, Highlighter, Lightbulb, RefreshCw, Search } from "lucide-react";
import { api } from "../api/tauri";
import { Button } from "../components/ui/Button";
import { PageHeader } from "../components/ui/PageHeader";
import type { DatabaseOverview, DatabaseRows } from "../types/domain";

const browsable = [
  { id: "books", label: "书籍", icon: BookOpen },
  { id: "highlights", label: "划线", icon: Highlighter },
  { id: "thoughts", label: "想法", icon: Lightbulb },
  { id: "sync_sessions", label: "同步记录", icon: RefreshCw },
] as const;
const formatBytes = (bytes: number) => bytes < 1024 * 1024 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`;

export function DatabasePage() {
  const [overview, setOverview] = useState<DatabaseOverview | null>(null);
  const [data, setData] = useState<DatabaseRows>({ total: 0, rows: [] });
  const [table, setTable] = useState("books");
  const [query, setQuery] = useState("");
  const [page, setPage] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const limit = 40;
  const loadOverview = () => api.databaseOverview().then(setOverview);
  useEffect(() => { loadOverview().catch(reason => setError(String(reason))); }, []);
  useEffect(() => {
    let active = true;
    setLoading(true);
    const timer = window.setTimeout(() => api.databaseRows(table, query, limit, page * limit)
      .then(value => { if (active) { setData(value); setError(""); } })
      .catch(reason => { if (active) setError(String(reason)); })
      .finally(() => { if (active) setLoading(false); }), 180);
    return () => { active = false; window.clearTimeout(timer); };
  }, [table, query, page]);
  useEffect(() => setPage(0), [table, query]);
  const maxCategory = useMemo(() => Math.max(1, ...(overview?.categories.map(item => item.count) ?? [])), [overview]);
  const activeLabel = browsable.find(item => item.id === table)?.label ?? "数据";
  const refresh = () => {
    setLoading(true);
    Promise.all([loadOverview(), api.databaseRows(table, query, limit, page * limit).then(setData)])
      .catch(reason => setError(String(reason))).finally(() => setLoading(false));
  };

  return <>
    <PageHeader title="数据库" subtitle="查看保存在本机 SQLite 中的微信读书数据。" actions={<Button variant="secondary" icon={<RefreshCw size={15}/>} onClick={refresh} disabled={loading}>刷新</Button>}/>
    {error && <div className="database-error">读取数据库失败：{error}</div>}
    <section className="database-summary">
      <article><span><DatabaseIcon size={18}/></span><div><strong>{overview?.tables.reduce((sum, item) => sum + item.rows, 0).toLocaleString() ?? "—"}</strong><small>数据记录</small></div></article>
      <article><span><HardDrive size={18}/></span><div><strong>{overview ? formatBytes(overview.sizeBytes) : "—"}</strong><small>数据库大小</small></div></article>
      <article><span><RefreshCw size={18}/></span><div><strong className="database-sync-time">{overview?.lastSyncedAt || "尚未同步"}</strong><small>最近同步</small></div></article>
    </section>
    <div className="database-insights">
      <section className="database-panel"><header><div><span className="eyebrow">SQLite tables</span><h2>数据表概览</h2></div><DatabaseIcon size={18}/></header><div className="database-table-stats">{overview?.tables.map(item => <div key={item.name}><span>{item.label}<code>{item.name}</code></span><strong>{item.rows.toLocaleString()}</strong></div>)}</div></section>
      <section className="database-panel"><header><div><span className="eyebrow">Distribution</span><h2>书籍分类</h2></div></header><div className="database-bars">{overview?.categories.length ? overview.categories.map(item => <div key={item.label}><div><span>{item.label}</span><strong>{item.count}</strong></div><i><b style={{ width: `${Math.max(4, item.count / maxCategory * 100)}%` }}/></i></div>) : <p>同步书籍后显示分类分布</p>}</div></section>
    </div>
    <section className="database-browser">
      <div className="database-browser__head"><div><span className="eyebrow">Data browser</span><h2>记录浏览</h2></div><label><Search size={15}/><input value={query} onChange={event => setQuery(event.target.value)} placeholder={`搜索${activeLabel}…`}/></label></div>
      <nav>{browsable.map(item => <button key={item.id} className={table === item.id ? "active" : ""} onClick={() => setTable(item.id)}><item.icon size={15}/>{item.label}<span>{overview?.tables.find(stat => stat.name === item.id)?.rows ?? 0}</span></button>)}</nav>
      <div className="database-records">
        <div className="database-records__header"><span>内容</span><span>来源 / 分类</span><span>章节 / 详情</span><span>时间</span></div>
        {loading && <div className="database-empty">正在读取数据…</div>}
        {!loading && !data.rows.length && <div className="database-empty">没有找到记录</div>}
        {!loading && data.rows.map(row => <div className="database-record" key={row.id}><strong title={row.primary}>{row.primary}</strong><span title={row.secondary}>{row.secondary || "—"}</span><span title={row.detail}>{row.detail || "—"}</span><time>{row.createdAt || "—"}</time></div>)}
      </div>
      <footer><span>共 {data.total.toLocaleString()} 条，第 {data.total ? page + 1 : 0} / {Math.ceil(data.total / limit)} 页</span><div><button disabled={page === 0 || loading} onClick={() => setPage(value => value - 1)}>上一页</button><button disabled={(page + 1) * limit >= data.total || loading} onClick={() => setPage(value => value + 1)}>下一页</button></div></footer>
    </section>
  </>;
}
