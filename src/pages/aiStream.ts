import type { AiAnswer, AiStreamEvent } from "../types/domain";

export type AiStreamStatus = "idle" | "streaming" | "stopped" | "error" | "completed";

export interface AiStreamState {
  /** 当前活动请求的标识；为空表示没有正在进行的流式请求。 */
  requestId: string | null;
  status: AiStreamStatus;
  question: string;
  /** 已收到的增量正文。失败或被停止时保留，用于「部分内容 + 重试」。 */
  draft: string;
  error: string;
  notice: string;
  answer: AiAnswer | null;
}

export const initialAiStreamState: AiStreamState = {
  requestId: null,
  status: "idle",
  question: "",
  draft: "",
  error: "",
  notice: "",
  answer: null,
};

export type AiStreamAction =
  | { type: "start"; requestId: string; question: string }
  | { type: "event"; event: AiStreamEvent }
  | { type: "stop"; requestId: string }
  | { type: "reset" };

/**
 * AI 流式回答的状态机。
 *
 * 抽成纯函数是为了能直接测「旧 requestId 被忽略」「停止后不再追加」这类
 * 时序问题，不必真的连后端流。
 */
export function aiStreamReducer(state: AiStreamState, action: AiStreamAction): AiStreamState {
  switch (action.type) {
    case "start":
      return {
        requestId: action.requestId,
        status: "streaming",
        question: action.question,
        draft: "",
        error: "",
        notice: "",
        answer: null,
      };

    case "stop":
      // 只有当前请求、且确实还在生成时，停止才生效。
      if (state.requestId !== action.requestId || state.status !== "streaming") return state;
      return { ...state, status: "stopped" };

    case "reset":
      return initialAiStreamState;

    case "event": {
      const { event } = action;
      // 迟到的旧请求事件不能污染当前会话。
      if (!state.requestId || event.requestId !== state.requestId) return state;
      switch (event.type) {
        case "started":
          if (state.status !== "streaming") return state;
          return state.notice === (event.notice ?? "") ? state : { ...state, notice: event.notice ?? "" };
        case "delta":
          if (state.status !== "streaming") return state;
          return { ...state, draft: state.draft + event.content };
        case "completed":
          return {
            ...state,
            status: "completed",
            draft: event.answer.content,
            answer: event.answer,
          };
        case "failed":
          // 保留已生成正文，方便用户看到断在哪里再重试。
          return { ...state, status: "error", error: event.message };
        case "cancelled":
          return { ...state, status: "stopped" };
      }
    }
  }
}

/** 流是否仍在进行中（用于把发送按钮切成停止按钮）。 */
export function isStreaming(state: AiStreamState): boolean {
  return state.status === "streaming";
}
