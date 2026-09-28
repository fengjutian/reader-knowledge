import { Check, Eye, EyeOff, KeyRound, ShieldCheck, Sparkles } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../api/tauri";
import { Button } from "../components/ui/Button";
import { PageHeader } from "../components/ui/PageHeader";

type Status = "idle" | "testing" | "success" | "error";
type SettingsTab = "weread" | "ai" | "privacy";
const defaults: Record<string, { endpoint: string; model: string }> = {
  DeepSeek: { endpoint: "https://api.deepseek.com/chat/completions", model: "deepseek-chat" },
  MiniMax: { endpoint: "https://api.minimaxi.com/v1/text/chatcompletion_v2", model: "MiniMax-M3" },
};
const errorText = (error: unknown, fallback: string) => {
  const message = typeof error === "string" ? error : error instanceof Error ? error.message : fallback;
  return message.includes("No matching entry found in secure storage") ? "未找到已保存的密钥，请重新输入并测试连接。" : message;
};

function SecretField({ label, kind }: { label: string; kind: string }) {
  const [value, setValue] = useState(""); const [show, setShow] = useState(false);
  const [saved, setSaved] = useState(false);
  const [status, setStatus] = useState<Status>("idle"); const [message, setMessage] = useState("");
  useEffect(() => { api.hasSecret(kind).then(setSaved).catch(() => setSaved(false)); }, [kind]);
  async function test() { setStatus("testing"); setMessage(""); try { if (value) await api.saveSecret(kind, value); await api.testConnection(kind, value || undefined); setSaved(true); setValue(""); setStatus("success"); } catch (error) { setStatus("error"); setMessage(errorText(error, "连接失败")); } }
  return <div className="setting-field"><label>{label}{saved && <span className="secret-saved"><Check size={12} />已保存</span>}</label><div className="secret-input"><input type={show ? "text" : "password"} placeholder={saved ? "••••••••••••（留空则继续使用）" : "输入 API Key"} value={value} onChange={e => { setValue(e.target.value); setStatus("idle"); }} /><button type="button" aria-label={show ? "隐藏密钥" : "显示密钥"} onClick={() => setShow(!show)}>{show ? <EyeOff size={16} /> : <Eye size={16} />}</button></div><Button variant="secondary" onClick={test} disabled={(!value && !saved) || status === "testing"}>{status === "success" ? <><Check size={15} />连接成功</> : status === "testing" ? "正在连接…" : saved && !value ? "测试已保存的 Key" : "保存并测试连接"}</Button>{status === "error" && <small className="field-error">{message}</small>}</div>;
}

function AiSettings() {
  const [provider, setProvider] = useState("DeepSeek"); const [endpoint, setEndpoint] = useState(defaults.DeepSeek.endpoint); const [model, setModel] = useState(defaults.DeepSeek.model);
  const [apiKey, setApiKey] = useState(""); const [show, setShow] = useState(false); const [status, setStatus] = useState<Status>("idle"); const [message, setMessage] = useState("");
  useEffect(() => { api.aiSettings().then(value => { if (value) { setProvider(value.provider); setEndpoint(value.endpoint); setModel(value.model); } }).catch(() => undefined); }, []);
  function changeProvider(next: string) { setProvider(next); setEndpoint(defaults[next]?.endpoint ?? ""); setModel(defaults[next]?.model ?? ""); setStatus("idle"); setMessage(""); }
  async function saveAndTest() { setStatus("testing"); setMessage(""); try { await api.saveAiSettings({ provider, endpoint, model }, apiKey); await api.testAi(apiKey); setStatus("success"); } catch (error) { setStatus("error"); setMessage(errorText(error, "AI 连接失败")); } }
  return <div className="setting-field"><label>Provider</label><select value={provider} onChange={e => changeProvider(e.target.value)}><option>DeepSeek</option><option>MiniMax</option><option>自定义</option></select><label>Chat Completions Endpoint</label><input className="settings-input" value={endpoint} onChange={e => setEndpoint(e.target.value)} placeholder="粘贴官方完整 HTTPS Endpoint" /><label>Model</label><input className="settings-input" value={model} onChange={e => setModel(e.target.value)} placeholder="填写官方模型 ID" />{provider === "MiniMax" && <small className="field-hint">已使用 MiniMax 国内 API 与最新 MiniMax-M3 模型。</small>}<label>API Key</label><div className="secret-input"><input type={show ? "text" : "password"} value={apiKey} onChange={e => { setApiKey(e.target.value); setStatus("idle"); }} placeholder="输入 API Key" /><button type="button" aria-label={show ? "隐藏密钥" : "显示密钥"} onClick={() => setShow(!show)}>{show ? <EyeOff size={16} /> : <Eye size={16} />}</button></div><Button variant="secondary" onClick={saveAndTest} disabled={!endpoint || !model || !apiKey || status === "testing"}>{status === "success" ? <><Check size={15} />AI 连接成功</> : status === "testing" ? "正在测试…" : "保存并测试 AI"}</Button>{status === "error" && <small className="field-error">{message}</small>}</div>;
}

const tabs = [{ id: "weread" as const, label: "微信读书", icon: KeyRound }, { id: "ai" as const, label: "AI 服务", icon: Sparkles }, { id: "privacy" as const, label: "数据与隐私", icon: ShieldCheck }];
export function Settings() {
  const [tab, setTab] = useState<SettingsTab>("weread");
  return <><PageHeader title="设置" subtitle="连接微信读书与 AI 服务。" /><div className="settings"><div className="settings-tabs" role="tablist" aria-label="设置模块">{tabs.map(({ id, label, icon: Icon }) => <button key={id} type="button" role="tab" aria-selected={tab === id} className={tab === id ? "active" : ""} onClick={() => setTab(id)}><Icon size={16} />{label}</button>)}</div>{tab === "weread" && <section className="settings-card" role="tabpanel"><div><h2>微信读书</h2><p>密钥会安全保存在系统凭据库中。</p></div><SecretField label="Agent Gateway API Key" kind="weread" /></section>}{tab === "ai" && <section className="settings-card" role="tabpanel"><div><h2>AI Provider</h2><p>选择供应商后会自动填入官方端点与推荐模型，也可以手动修改。</p></div><AiSettings /></section>}{tab === "privacy" && <section className="settings-card" role="tabpanel"><div><h2>数据与隐私</h2><p>阅读记录保存在本机。AI 请求只发送问题和检索出的 Top‑20 笔记，API Key 不写入数据库或日志。</p></div><div className="privacy-badge"><Check size={16} />本地优先</div></section>}</div></>;
}
