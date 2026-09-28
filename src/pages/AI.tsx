import { ArrowUp, BookOpen, Sparkles } from "lucide-react";
import { useState } from "react";
import { api } from "../api/tauri";
import { PageHeader } from "../components/ui/PageHeader";
import type { AiAnswer } from "../types/domain";

export function AI() {
  const [question,setQuestion]=useState("");
  const [answer,setAnswer]=useState<AiAnswer>();
  const [loading,setLoading]=useState(false);
  const [error,setError]=useState("");
  async function ask(event:React.FormEvent){event.preventDefault();if(!question.trim())return;setLoading(true);setError("");setAnswer(undefined);try{setAnswer(await api.ask(question))}catch(reason){setError(typeof reason==="string"?reason:reason instanceof Error?reason.message:"AI 请求失败")}finally{setLoading(false)}}
  return <><PageHeader title="AI 阅读助手" subtitle="回答只使用从本地知识库检索出的笔记，并附带来源。"/><div className="ai-wrap">{!answer&&!loading&&!error&&<section className="ai-empty"><span><Sparkles size={25}/></span><h2>从自己的阅读记录中寻找答案</h2><p>系统会先检索相关划线和想法，再把有限上下文发送给已配置模型。</p></section>}{loading&&<div className="ai-thinking"><Sparkles/><div><strong>正在检索并分析笔记</strong><span>只发送与问题相关的 Top‑20 内容</span></div></div>}{error&&<div className="ai-error"><strong>无法生成回答</strong><p>{error}</p></div>}{answer&&<section className="answer"><div className="answer__question">{question}</div><div className="answer__body"><Sparkles size={18}/><p>{answer.content}</p></div><h3>引用的笔记</h3>{answer.citations.map(c=><article className="citation" key={`${c.index}-${c.note.id}`}><span>{c.index}</span><div><strong><BookOpen size={14}/>《{c.note.bookTitle}》 · {c.note.chapter}</strong><p>{c.note.content}</p></div></article>)}</section>}<form className="ask-box" onSubmit={ask}><textarea value={question} onChange={event=>setQuestion(event.target.value)} placeholder="问问你的阅读知识库…"/><button disabled={!question.trim()||loading}><ArrowUp size={18}/></button><small>回答严格基于引用笔记，请结合原文核实</small></form></div></>;
}
