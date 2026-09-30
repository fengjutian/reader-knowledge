import * as Dialog from "@radix-ui/react-dialog";
import { ExternalLink, Eye, Plus, Search, Trash2, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api } from "../api/tauri";
import { PageHeader } from "../components/ui/PageHeader";
import { DataTable, type DataTableColumn } from "../components/ui/DataTable";
import { Button } from "../components/ui/Button";
import type { GlossaryTerm, WikipediaCandidate } from "../types/domain";

const empty = (): GlossaryTerm => ({ id: 0, term: "", canonicalName: "", aliases: [], definition: "", source: "manual", sourceTitle: "", sourceUrl: "", wikipediaSnapshot: "", status: "confirmed", updatedAt: 0 });

export function Glossary() {
  const [items, setItems] = useState<GlossaryTerm[]>([]);
  const [editing, setEditing] = useState<GlossaryTerm>(empty());
  const [query, setQuery] = useState("");
  const [candidates, setCandidates] = useState<WikipediaCandidate[]>([]);
  const [drawerOpen, setDrawerOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const termInputRef = useRef<HTMLInputElement>(null);

  const load = () => api.glossaryTerms(query).then(setItems).catch(reason => setError(String(reason)));
  useEffect(() => { void load(); }, [query]);

  function create() { setEditing(empty()); setCandidates([]); setError(""); setDrawerOpen(true); requestAnimationFrame(() => termInputRef.current?.focus()); }
  function view(item: GlossaryTerm) { setEditing(item); setCandidates([]); setError(""); setDrawerOpen(true); }
  async function wiki() { if (!editing.term.trim()) return; setBusy(true); setError(""); try { setCandidates(await api.searchWikipedia(editing.term.trim())); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } }
  function choose(item: WikipediaCandidate) { const definition = item.excerpt.trim() || item.description.trim(); setEditing(value => ({ ...value, canonicalName: item.title, definition, source: "wikipedia", sourceTitle: item.title, sourceUrl: item.url, wikipediaSnapshot: definition, status: "confirmed" })); setCandidates([]); }
  async function save() { setBusy(true); setError(""); try { const savedTerm = editing.term.trim(); await api.saveGlossaryTerm({ ...editing, term: savedTerm, canonicalName: editing.canonicalName || savedTerm }); setItems(await api.glossaryTerms(query)); setDrawerOpen(false); setCandidates([]); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } }
  async function remove(id: number) { setBusy(true); setError(""); try { await api.deleteGlossaryTerm(id); setDrawerOpen(false); setEditing(empty()); await load(); } catch (reason) { setError(String(reason)); } finally { setBusy(false); } }

  const columns: DataTableColumn<GlossaryTerm>[] = [
    { key: "term", header: "名词", render: item => <div className="glossary-term-cell"><strong>{item.term}</strong><small>{item.definition}</small></div> },
    { key: "canonicalName", header: "标准名称", width: 180, render: item => item.canonicalName || "—" },
    { key: "source", header: "来源", width: 110, render: item => item.source === "wikipedia" ? "维基百科" : "人工编辑" },
    { key: "status", header: "状态", width: 90, render: item => <span className="glossary-status">{item.status === "confirmed" ? "已确认" : "待确认"}</span> },
    { key: "actions", header: "操作", width: 90, align: "center", render: item => <Button variant="secondary" className="glossary-view" icon={<Eye size={14}/>} onClick={() => view(item)}>查看</Button> },
  ];

  return <>
    <PageHeader title="名词库" subtitle="维基百科负责初始化，人工编辑内容优先生效；解释只作为 AI 背景。" actions={<Button icon={<Plus size={15}/>} onClick={create}>新建名词</Button>}/>
    <section className="glossary-list glossary-list--full">
      <label className="field field--search"><Search size={16}/><input value={query} onChange={event => setQuery(event.target.value)} placeholder="搜索名词或解释"/></label>
      <DataTable columns={columns} rows={items} rowKey={item => item.id} empty="暂无名词"/>
    </section>
    <Dialog.Root open={drawerOpen} onOpenChange={setDrawerOpen}><Dialog.Portal>
      <Dialog.Overlay className="glossary-drawer__overlay"/>
      <Dialog.Content className="glossary-drawer">
        <header className="glossary-drawer__head"><div><Dialog.Title>{editing.id > 0 ? "查看与编辑名词" : "新建名词"}</Dialog.Title><Dialog.Description>{editing.id > 0 ? "修改后保存将立即用于 AI 问答" : "填写名词后可优先从维基百科获取解释"}</Dialog.Description></div><Dialog.Close className="icon-button" aria-label="关闭"><X size={19}/></Dialog.Close></header>
        <div className="glossary-editor">
          <label>名词<div className="glossary-term-input"><input ref={termInputRef} value={editing.term} onChange={event => setEditing({ ...editing, term: event.target.value })} placeholder="输入需要解释的名词"/><button type="button" disabled={busy || !editing.term.trim()} onClick={() => void wiki()}><Search size={14}/>{busy ? "正在搜索…" : "优先从维基百科获取"}</button></div></label>
          <label>标准名称<input value={editing.canonicalName} onChange={event => setEditing({ ...editing, canonicalName: event.target.value })}/></label>
          <label>别名<input value={editing.aliases.join("、")} onChange={event => setEditing({ ...editing, aliases: event.target.value.split(/[、,，]/).map(value => value.trim()).filter(Boolean) })}/></label>
          {editing.sourceUrl && <div className="glossary-wiki"><button type="button" onClick={() => void api.openExternalUrl(editing.sourceUrl)}><ExternalLink size={14}/>查看来源</button></div>}
          {candidates.length > 0 && <div className="glossary-candidates">{candidates.map(item => <button key={item.title} onClick={() => choose(item)}><strong>{item.title}</strong><span>{item.description || item.excerpt}</span></button>)}</div>}
          <label>解释<textarea rows={12} value={editing.definition} onChange={event => setEditing({ ...editing, definition: event.target.value, source: "manual" })}/></label>
          {error && <p className="glossary-error">{error}</p>}
        </div>
        <footer className="glossary-drawer__footer">{editing.id > 0 && <button className="danger" disabled={busy} onClick={() => void remove(editing.id)}><Trash2 size={14}/>删除</button>}<button className="button button--primary" disabled={busy || !editing.term.trim() || !editing.definition.trim()} onClick={() => void save()}>保存并确认</button></footer>
      </Dialog.Content>
    </Dialog.Portal></Dialog.Root>
  </>;
}
