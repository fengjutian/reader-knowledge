import { BookOpen, ChevronRight, Compass, ExternalLink, LoaderCircle, RefreshCw, Star, Users, X } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { api } from "../api/tauri";
import { Button } from "../components/ui/Button";
import { PageHeader } from "../components/ui/PageHeader";
import type { RecommendedBook } from "../types/domain";

let recommendationsCache: RecommendedBook[] | null = null;

function numberOf(value: unknown) {
  return typeof value === "number" && Number.isFinite(value) ? value : 0;
}

function ratingOf(book: RecommendedBook) {
  const rawRating = numberOf(book.newRating);
  if (!rawRating) return "暂无评分";
  const rating = rawRating > 10 ? rawRating / 10 : rawRating;
  return rating.toFixed(1);
}

export function Discover() {
  const [books, setBooks] = useState<RecommendedBook[]>(recommendationsCache ?? []);
  const [loading, setLoading] = useState(!recommendationsCache);
  const [loadingMore, setLoadingMore] = useState(false);
  const [error, setError] = useState("");
  const [selectedBook, setSelectedBook] = useState<RecommendedBook | null>(null);

  const load = useCallback(async (append = false) => {
    append ? setLoadingMore(true) : setLoading(true);
    setError("");
    try {
      const maxIdx = append && books.length ? numberOf(books[books.length - 1].searchIdx) : 0;
      const result = await api.recommendations(12, maxIdx);
      const next = Array.isArray(result.books) ? result.books : [];
      const merged = append
        ? [...books, ...next.filter(item => !books.some(book => book.bookId === item.bookId))]
        : next;
      recommendationsCache = merged;
      setBooks(merged);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setLoading(false);
      setLoadingMore(false);
    }
  }, [books]);

  useEffect(() => { if (!recommendationsCache) void load(); }, []); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(() => {
    if (!selectedBook) return;
    const closeOnEscape = (event: KeyboardEvent) => { if (event.key === "Escape") setSelectedBook(null); };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [selectedBook]);

  const openBook = (book: RecommendedBook) => {
    if (book.bookId) void api.openBook(book.bookId);
  };

  return <>
    <PageHeader title="好书推荐" subtitle="从你的阅读偏好出发，发现下一本值得读的书。" actions={
      <Button variant="secondary" icon={<RefreshCw size={15}/>} disabled={loading} onClick={() => void load()}>换一批</Button>
    }/>
    {loading ? <div className="discover-state"><LoaderCircle className="spin"/><strong>正在寻找适合你的书…</strong><span>根据微信读书阅读记录生成推荐</span></div>
      : error ? <div className="discover-state discover-state--error"><Compass/><strong>推荐数据获取失败</strong><span>{error}</span><Button onClick={() => void load()}>重新加载</Button></div>
      : books.length === 0 ? <div className="discover-state"><BookOpen/><strong>暂时没有找到合适的推荐</strong><span>稍后再来看看，或先在微信读书中读几本书。</span></div>
      : <div className="discover-scroll">
        <section className="recommend-grid">
          {books.map((book, index) => <article className="recommend-card recommend-card--clickable" key={`${book.bookId}-${index}`} role="button" tabIndex={0} onClick={() => setSelectedBook(book)} onKeyDown={event => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); setSelectedBook(book); } }}>
            <div className="recommend-cover"><span>{book.title}</span>{book.cover && <img src={book.cover} alt={`${book.title}封面`} loading="lazy" referrerPolicy="no-referrer"/>}<i>{String(index + 1).padStart(2, "0")}</i></div>
            <div className="recommend-copy">
              <div className="recommend-meta"><span>{book.category || "为你推荐"}</span>{book.newRatingDetail?.title && <em>{book.newRatingDetail.title}</em>}</div>
              <h2>{book.title}</h2><p className="recommend-author">{book.author || "作者未署名"}</p>
              <p className="recommend-reason">{book.reason || book.intro || "与你的阅读偏好相契合"}</p>
              <footer><span><Star size={13}/>{ratingOf(book)}</span>{numberOf(book.readingCount) > 0 && <span><Users size={13}/>{numberOf(book.readingCount).toLocaleString()} 人在读</span>}{book.bookId && <ChevronRight size={16}/>}</footer>
            </div>
          </article>)}
        </section>
        <div className="recommend-more"><Button variant="secondary" icon={loadingMore ? <LoaderCircle className="spin" size={15}/> : <Compass size={15}/>} disabled={loadingMore} onClick={() => void load(true)}>{loadingMore ? "正在推荐…" : "继续发现"}</Button></div>
      </div>}
    {selectedBook && <div className="recommend-drawer-layer" role="presentation">
      <button className="recommend-drawer-backdrop" aria-label="关闭书籍详情" onClick={() => setSelectedBook(null)}/>
      <aside className="recommend-drawer" role="dialog" aria-modal="true" aria-labelledby="recommend-drawer-title">
        <header><span>书籍详情</span><button aria-label="关闭" onClick={() => setSelectedBook(null)}><X size={18}/></button></header>
        <div className="recommend-drawer__body">
          <div className="recommend-drawer__book">
            <div className="recommend-drawer__cover"><span>{selectedBook.title}</span>{selectedBook.cover && <img src={selectedBook.cover} alt={`${selectedBook.title}封面`} referrerPolicy="no-referrer"/>}</div>
            <div><div className="recommend-meta"><span>{selectedBook.category || "为你推荐"}</span>{selectedBook.newRatingDetail?.title && <em>{selectedBook.newRatingDetail.title}</em>}</div><h2 id="recommend-drawer-title">{selectedBook.title}</h2><p>{selectedBook.author || "作者未署名"}</p></div>
          </div>
          <div className="recommend-drawer__stats"><span><Star size={15}/><b>{ratingOf(selectedBook)}</b><small>{numberOf(selectedBook.newRatingCount) > 0 ? `${numberOf(selectedBook.newRatingCount).toLocaleString()} 人评分` : "微信读书评分"}</small></span><span><Users size={15}/><b>{numberOf(selectedBook.readingCount).toLocaleString()}</b><small>人在读</small></span></div>
          {selectedBook.reason && <section><label>推荐理由</label><p>{selectedBook.reason}</p></section>}
          <section><label>内容简介</label><p>{selectedBook.intro || "这本书暂时没有简介。"}</p></section>
        </div>
        <footer><Button disabled={!selectedBook.bookId} icon={<ExternalLink size={15}/>} onClick={() => openBook(selectedBook)}>打开微信读书</Button></footer>
      </aside>
    </div>}
  </>;
}
