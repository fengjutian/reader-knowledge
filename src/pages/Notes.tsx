import { useEffect, useState } from "react";
import { NoteCard } from "../components/notes/NoteCard";
import { PageHeader } from "../components/ui/PageHeader";
import { api } from "../api/tauri";
import type { Note, NoteType } from "../types/domain";
export function Notes({ type }: { type: NoteType }) { const [notes,setNotes]=useState<Note[]>([]); useEffect(()=>{api.notes(type).then(setNotes)},[type]); const thought=type==="thought"; return <><PageHeader title={thought?"想法":"划线"} subtitle={thought?"回到阅读时闪现的念头。":"重读那些曾经打动你的句子。"}/><section className="notes-list">{notes.map(note=><NoteCard key={note.id} note={note}/>)}</section></> }

