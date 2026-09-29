import { AlertCircle, CheckCircle2, Info, X } from "lucide-react";
import { useEffect } from "react";

export type MessageKind = "info" | "success" | "error";
export interface MessageValue { text: string; kind: MessageKind }

export function Message({ value, onClose, duration = 4200 }: { value?: MessageValue; onClose: () => void; duration?: number }) {
  useEffect(() => {
    if (!value || duration <= 0) return;
    const timer = window.setTimeout(onClose, duration);
    return () => window.clearTimeout(timer);
  }, [value, duration, onClose]);
  if (!value) return null;
  const Icon = value.kind === "success" ? CheckCircle2 : value.kind === "error" ? AlertCircle : Info;
  return <div className={`app-message app-message--${value.kind}`} role={value.kind === "error" ? "alert" : "status"}><Icon size={18}/><span>{value.text}</span><button type="button" onClick={onClose} aria-label="关闭消息"><X size={15}/></button></div>;
}
