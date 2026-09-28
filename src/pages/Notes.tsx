import { Highlighter, Lightbulb, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../api/tauri";
import { NoteCard } from "../components/notes/NoteCard";
import { Button } from "../components/ui/Button";
import { PageHeader } from "../components/ui/PageHeader";
import { useSyncStore } from "../stores/sync";
import type { Note, NoteType } from "../types/domain";

export function Notes({ type }: { type: NoteType }) {
  const [notes, setNotes] = useState<Note[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const sync = useSyncStore();
  const thought = type === "thought";
  const syncing = sync.status === "reading" || sync.status === "processing";

  useEffect(() => {
    setLoading(true);
    setError("");
    api.notes(type).then(setNotes).catch(reason => setError(reason instanceof Error ? reason.message : String(reason))).finally(() => setLoading(false));
  }, [type, sync.status, sync.progress]);

  return <>
    <PageHeader title={thought ? "想法" : "划线"} subtitle={thought ? "回到阅读时闪现的念头。" : "重读那些曾经打动你的句子。"} />
    {loading && <div className="notes-loading">正在整理你的阅读痕迹…</div>}
    {!loading && error && <section className="notes-empty"><div className="notes-empty__icon">{thought ? <Lightbulb /> : <Highlighter />}</div><h2>暂时无法读取内容</h2><p>{error}</p></section>}
    {!loading && !error && notes.length === 0 && <section className="notes-empty">
      <div className="notes-empty__icon">{thought ? <Lightbulb /> : <Highlighter />}</div>
      <span className="notes-empty__eyebrow">READING ARCHIVE</span>
      <h2>{thought ? "还没有留下想法" : "还没有收藏划线"}</h2>
      <p>{thought ? "同步微信读书后，你写下的感受与批注会安静地汇聚在这里。" : "同步微信读书后，那些让你停下来反复阅读的句子会出现在这里。"}</p>
      <Button icon={<RefreshCw size={15} className={syncing ? "spin" : ""} />} onClick={sync.run} disabled={syncing}>{syncing ? "正在同步…" : "同步微信读书"}</Button>
      {sync.status === "failed" && <small>{sync.message}</small>}
    </section>}
    {!loading && !error && notes.length > 0 && <section className="notes-list">{notes.map(note => <NoteCard key={note.id} note={note} />)}</section>}
  </>;
}
