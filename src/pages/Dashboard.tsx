import { useEffect, useMemo, useState } from "react";
import { ResponsiveBar } from "@nivo/bar";
import { ResponsiveCalendar } from "@nivo/calendar";
import { ResponsiveLine } from "@nivo/line";
import { ArrowDownRight, ArrowUpRight, BookOpen, Clock3, Headphones, Highlighter, Lightbulb, RefreshCw } from "lucide-react";
import { motion } from "motion/react";
import { api } from "../api/tauri";
import { Button } from "../components/ui/Button";
import { PageHeader } from "../components/ui/PageHeader";
import { useSyncStore } from "../stores/sync";
import type { DashboardStats, ReadingPeriod, ReadingStats } from "../types/domain";

type DashboardModule = "knowledge" | "reading";
type ReadingModule = "trend" | "preference" | "ranking";
const periods: { value: ReadingPeriod; label: string }[] = [{ value: "weekly", label: "本周" }, { value: "monthly", label: "本月" }, { value: "annually", label: "本年" }, { value: "overall", label: "历年" }];
const readingModules: { value: ReadingModule; label: string }[] = [{ value: "trend", label: "阅读趋势" }, { value: "preference", label: "阅读偏好" }, { value: "ranking", label: "阅读排行" }];

function duration(seconds = 0) {
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes} 分钟`;
  const hours = Math.floor(minutes / 60);
  return `${hours} 小时${minutes % 60 ? ` ${minutes % 60} 分` : ""}`;
}
function dateLabel(timestamp: string, period: ReadingPeriod) {
  const date = new Date(Number(timestamp) * 1000);
  if (period === "overall") return `${date.getFullYear()}年`;
  return period === "annually" ? `${date.getMonth() + 1}月` : `${date.getMonth() + 1}/${date.getDate()}`;
}
function isoDate(timestamp: string) {
  const date = new Date(Number(timestamp) * 1000);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}
const chartTheme = {
  text: { fill: "var(--secondary)", fontSize: 11 }, axis: { ticks: { text: { fill: "var(--muted)" } }, domain: { line: { stroke: "var(--border)" } } },
  grid: { line: { stroke: "var(--border)", strokeOpacity: 0.55 } }, tooltip: { container: { background: "var(--surface)", color: "var(--text)", border: "1px solid var(--border)", borderRadius: 8, boxShadow: "0 8px 24px #0002" } },
};

export function Dashboard() {
  const [module, setModule] = useState<DashboardModule>("knowledge");
  const [readingModule, setReadingModule] = useState<ReadingModule>("trend");
  const [stats, setStats] = useState<DashboardStats>({ books: 0, highlights: 0, thoughts: 0 });
  const [period, setPeriod] = useState<ReadingPeriod>("monthly");
  const [reading, setReading] = useState<ReadingStats | null>(null);
  const [readingState, setReadingState] = useState<"idle" | "loading" | "ready" | "error">("idle");
  const [readingError, setReadingError] = useState("");
  const sync = useSyncStore();

  useEffect(() => { api.dashboard().then(setStats); }, [sync.status]);
  useEffect(() => {
    if (module !== "reading") return;
    let active = true; setReadingState("loading");
    api.readingStats(period).then(value => { if (active) { setReading(value); setReadingState("ready"); } }).catch(error => { if (active) { setReadingError(String(error)); setReadingState("error"); } });
    return () => { active = false; };
  }, [module, period, sync.status]);

  const syncing = sync.status === "reading" || sync.status === "processing";
  const live = sync.status === "processing" || sync.status === "complete";
  const cards = [{ label: "书籍", value: live ? sync.books : stats.books, icon: BookOpen }, { label: "划线", value: live ? sync.highlights : stats.highlights, icon: Highlighter }, { label: "想法", value: live ? sync.thoughts : stats.thoughts, icon: Lightbulb }];
  const trend = useMemo(() => [{ id: "阅读时长", data: Object.entries(reading?.readTimes ?? {}).sort(([a], [b]) => Number(a) - Number(b)).map(([time, seconds]) => ({ x: dateLabel(time, period), y: Math.round(seconds / 60) })) }], [reading, period]);
  const categories = useMemo(() => (reading?.preferCategory ?? []).filter(item => item.readingTime > 0).slice(0, 8).map(item => ({ category: item.categoryTitle, minutes: Math.round(item.readingTime / 60), books: item.readingCount })), [reading]);
  const timePreference = useMemo(() => (reading?.preferTime ?? []).map((seconds, index) => ({ hour: `${(index + 6) % 24}时`, minutes: Math.round(seconds / 60) })), [reading]);
  const calendar = useMemo(() => Object.entries(reading?.dailyReadTimes ?? {}).map(([time, seconds]) => ({ day: isoDate(time), value: Math.round(seconds / 60) })), [reading]);
  const calendarYear = new Date((reading?.baseTime || Math.floor(Date.now() / 1000)) * 1000).getFullYear();
  const compare = reading?.compare;

  return <>
    <PageHeader title="我的阅读" subtitle="把读过的内容，变成可以继续生长的知识。" actions={<Button icon={<RefreshCw size={16} className={syncing ? "spin" : ""}/>} onClick={sync.run} disabled={syncing}>同步微信读书</Button>}/>
    <nav className="dashboard-tabs" aria-label="概览模块"><button className={module === "knowledge" ? "active" : ""} onClick={() => setModule("knowledge")}><BookOpen/>知识概览</button><button className={module === "reading" ? "active" : ""} onClick={() => setModule("reading")}><Clock3/>阅读统计</button></nav>

    {module === "knowledge" && <motion.div className="dashboard-module" initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }}>
      <section className="stats-grid">{cards.map((card, index) => <motion.article className="stat-card" key={card.label} initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: index * .06 }}><div className="stat-card__icon"><card.icon size={18}/></div><strong>{card.value.toLocaleString()}</strong><span>{card.label}</span><ArrowUpRight size={16}/></motion.article>)}</section>
      <section className="dashboard-grid"><article className="panel sync-panel"><div className="panel-title"><div><span className="eyebrow">同步状态</span><h2>{sync.status === "complete" ? "所有内容已是最新" : sync.message || "知识库已准备就绪"}</h2></div><span className={`status-dot status-dot--${sync.status}`}/></div>{syncing && <div className="sync-progress-wrap"><div className="sync-progress-label"><span>{sync.message || "正在同步"}</span><strong>{Math.round(sync.progress)}%</strong></div><div className={`progress ${sync.status === "reading" ? "progress--active" : ""}`}><motion.div animate={{ width: sync.status === "reading" ? "8%" : `${sync.progress}%` }}/></div></div>}<div className="sync-numbers"><span><strong>{live ? sync.books : stats.books}</strong> 本书</span><span><strong>{live ? sync.highlights : stats.highlights}</strong> 条划线</span><span><strong>{live ? sync.thoughts : stats.thoughts}</strong> 条想法</span></div><p>最近同步 {stats.lastSyncedAt || "尚未同步"}</p></article></section>
    </motion.div>}

    {module === "reading" && <motion.section className="reading-dashboard" initial={{ opacity: 0, y: 4 }} animate={{ opacity: 1, y: 0 }}>
      <nav className="reading-module-tabs">{readingModules.map(item => <button key={item.value} className={readingModule === item.value ? "active" : ""} onClick={() => setReadingModule(item.value)}>{item.label}</button>)}<div className="period-tabs" aria-label="统计周期">{periods.map(item => <button key={item.value} className={period === item.value ? "active" : ""} onClick={() => setPeriod(item.value)}>{item.label}</button>)}</div></nav>
      {readingState === "loading" && <div className="reading-state"><RefreshCw className="spin"/>正在读取阅读统计…</div>}
      {readingState === "error" && <div className="reading-state reading-state--error"><strong>阅读数据暂时无法获取</strong><span>{readingError}</span></div>}
      {readingState === "ready" && reading && <>
        <div className="reading-kpis"><div><Clock3/><span>总阅读时长</span><strong>{duration(reading.totalReadTime)}</strong></div><div><BookOpen/><span>阅读天数</span><strong>{reading.readDays ?? 0} 天</strong></div><div><Clock3/><span>自然日均</span><strong>{duration(reading.dayAverageReadTime)}</strong></div><div className={compare != null && compare < 0 ? "is-down" : ""}>{compare != null && compare < 0 ? <ArrowDownRight/> : <ArrowUpRight/>}<span>较上周期</span><strong>{compare == null ? "暂无对比" : `${compare >= 0 ? "+" : ""}${Math.round(compare * 100)}%`}</strong></div></div>

        {readingModule === "trend" && <div className="reading-module-content"><article className="panel chart-card chart-card--wide"><div className="chart-heading"><div><span>{period === "overall" ? "历年趋势" : "阅读时长"}</span><h3>{period === "overall" ? "每一年的阅读足迹" : "每天留给阅读的时间"}</h3></div><small>单位：分钟</small></div><div className="chart-body">{trend[0].data.length ? <ResponsiveLine data={trend} theme={chartTheme} margin={{ top: 16, right: 22, bottom: 42, left: 48 }} colors={["#496d31"]} curve="monotoneX" enableArea areaOpacity={0.13} pointSize={7} pointColor="var(--surface)" pointBorderWidth={2} pointBorderColor="#496d31" enableGridX={false} axisLeft={{ tickSize: 0, tickPadding: 9 }} axisBottom={{ tickSize: 0, tickPadding: 12 }} enableSlices="x" useMesh /> : <div className="chart-empty">这个周期还没有阅读记录</div>}</div></article>{period === "annually" && <article className="panel calendar-card"><div className="chart-heading"><div><span>阅读日历</span><h3>{calendarYear} 年的阅读足迹</h3></div><small>{reading.readDays ?? 0} 个阅读日</small></div><div className="calendar-body">{calendar.length ? <ResponsiveCalendar data={calendar} from={`${calendarYear}-01-01`} to={`${calendarYear}-12-31`} theme={chartTheme} emptyColor="var(--surface-hover)" colors={["#dcebd5", "#a9c18f", "#6f9459", "#315f43", "#173f38"]} margin={{ top: 24, right: 20, bottom: 20, left: 26 }} yearSpacing={36} monthBorderColor="var(--surface)" dayBorderWidth={2} dayBorderColor="var(--surface)" /> : <div className="chart-empty">暂无每日阅读明细</div>}</div></article>}</div>}

        {readingModule === "preference" && <div className="reading-charts"><article className="panel chart-card"><div className="chart-heading"><div><span>分类偏好</span><h3>{reading.preferCategoryWord || "常读的内容"}</h3></div></div><div className="chart-body">{categories.length ? <ResponsiveBar data={categories} keys={["minutes"]} indexBy="category" theme={chartTheme} layout="horizontal" margin={{ top: 8, right: 18, bottom: 36, left: 72 }} padding={0.4} colors={["#b48a5a"]} borderRadius={3} enableGridY={false} axisLeft={{ tickSize: 0, tickPadding: 9 }} axisBottom={{ tickSize: 0, tickPadding: 8 }} labelSkipWidth={42} label={datum => `${datum.value} 分`} tooltip={({ indexValue, value, data }) => <div className="chart-tooltip"><strong>{indexValue}</strong><span>{value} 分钟 · {String(data.books)} 本</span></div>} /> : <div className="chart-empty">分类数据积累后将在这里显示</div>}</div></article><article className="panel chart-card"><div className="chart-heading"><div><span>阅读时段</span><h3>{reading.preferTimeWord || "一天中的阅读习惯"}</h3></div></div><div className="chart-body">{timePreference.some(item => item.minutes > 0) ? <ResponsiveBar data={timePreference} keys={["minutes"]} indexBy="hour" theme={chartTheme} margin={{ top: 8, right: 12, bottom: 40, left: 44 }} padding={0.28} colors={["#527c69"]} borderRadius={2} enableLabel={false} enableGridX={false} axisLeft={{ tickSize: 0, tickPadding: 8 }} axisBottom={{ tickSize: 0, tickPadding: 8, tickValues: ["6时", "9时", "12时", "15时", "18时", "21时", "0时", "3时"] }} /> : <div className="chart-empty">暂无阅读时段数据</div>}</div></article></div>}

        {readingModule === "ranking" && <div className="ranking-layout"><article className="panel reading-ranking"><div className="chart-heading"><div><span>本周期</span><h3>读得最多的书</h3></div><small>阅读时长</small></div>{(reading.readLongest ?? []).length ? <ol>{reading.readLongest!.slice(0, 8).map((item, index) => { const content = item.book ?? item.albumInfo; return <li key={item.book?.bookId ?? item.albumInfo?.albumId ?? index}><span className="ranking-number">{index + 1}</span>{content?.cover ? <img src={content.cover} alt=""/> : <div className="ranking-cover"><BookOpen/></div>}<div><strong>{content?.title || "未知内容"}</strong><span>{content?.author || item.tags?.join(" · ") || "微信读书"}</span></div><b>{duration(item.readTime)}</b></li>; })}</ol> : <div className="chart-empty">本周期暂无阅读排行</div>}</article><aside className="ranking-side"><article className="panel reading-summary"><div className="chart-heading"><div><span>统计摘要</span><h3>阅读成果</h3></div></div>{(reading.readStat ?? []).length ? <div className="summary-grid">{reading.readStat!.map(item => <div key={item.stat}><strong>{item.counts}</strong><span>{item.stat}</span></div>)}</div> : <div className="chart-empty chart-empty--small">暂无摘要</div>}</article>{reading.readRate != null && <article className="panel read-method"><Headphones/><div><span>文字阅读占比</span><strong>{reading.readRate}%</strong><small>阅读 {duration(reading.wrReadTime)} · 听书 {duration(reading.wrListenTime)}</small></div></article>}</aside></div>}
      </>}
    </motion.section>}
  </>;
}
