import { describe, expect, it } from "vitest";
import { aiStreamReducer, initialAiStreamState, isStreaming, type AiStreamState } from "./aiStream";
import type { AiAnswer, AiStreamEvent } from "../types/domain";

const answer: AiAnswer = {
  content: "完整回答",
  citations: [],
  glossaryCitations: [],
  sourcesConsidered: 3,
};

function streaming(requestId: string): AiStreamState {
  return aiStreamReducer(initialAiStreamState, { type: "start", requestId, question: "问题" });
}

function apply(state: AiStreamState, ...events: AiStreamEvent[]): AiStreamState {
  return events.reduce((current, event) => aiStreamReducer(current, { type: "event", event }), state);
}

describe("aiStreamReducer", () => {
  it("start 之后进入生成状态并清空上一次的内容", () => {
    const dirty = apply(streaming("a"), { requestId: "a", type: "delta", content: "旧内容" }, { requestId: "a", type: "failed", message: "炸了" });
    const next = aiStreamReducer(dirty, { type: "start", requestId: "b", question: "新问题" });
    expect(next).toMatchObject({ requestId: "b", status: "streaming", question: "新问题", draft: "", error: "" });
    expect(isStreaming(next)).toBe(true);
  });

  it("delta 按到达顺序追加", () => {
    const next = apply(
      streaming("a"),
      { requestId: "a", type: "started" },
      { requestId: "a", type: "delta", content: "第一段" },
      { requestId: "a", type: "delta", content: "第二段" },
      { requestId: "a", type: "delta", content: "。" },
    );
    expect(next.draft).toBe("第一段第二段。");
  });

  it("忽略其他请求的事件", () => {
    const next = apply(
      streaming("a"),
      { requestId: "a", type: "delta", content: "本会话" },
      { requestId: "b", type: "delta", content: "别串进来" },
      { requestId: "b", type: "completed", answer },
      { requestId: "b", type: "failed", message: "别串进来" },
    );
    expect(next.draft).toBe("本会话");
    expect(next.status).toBe("streaming");
    expect(next.answer).toBeNull();
  });

  it("停止之后不再追加增量", () => {
    const beforeStop = apply(streaming("a"), { requestId: "a", type: "delta", content: "已生成" });
    const stopped = aiStreamReducer(beforeStop, { type: "stop", requestId: "a" });
    expect(stopped.status).toBe("stopped");
    const after = apply(stopped, { requestId: "a", type: "delta", content: "不该出现" });
    expect(after.draft).toBe("已生成");
  });

  it("后端发来的 cancelled 也会停止追加", () => {
    const next = apply(
      streaming("a"),
      { requestId: "a", type: "delta", content: "半句" },
      { requestId: "a", type: "cancelled" },
      { requestId: "a", type: "delta", content: "不该出现" },
    );
    expect(next.status).toBe("stopped");
    expect(next.draft).toBe("半句");
  });

  it("completed 之后不会再被 delta 追加", () => {
    const done = apply(streaming("a"), { requestId: "a", type: "delta", content: "草稿" }, { requestId: "a", type: "completed", answer });
    expect(done.status).toBe("completed");
    expect(done.answer).toBe(answer);
    expect(done.draft).toBe("完整回答");
    const after = apply(done, { requestId: "a", type: "delta", content: "不该出现" });
    expect(after.draft).toBe("完整回答");
  });

  it("失败时保留已生成内容并给出错误", () => {
    const next = apply(
      streaming("a"),
      { requestId: "a", type: "delta", content: "写到一半" },
      { requestId: "a", type: "failed", message: "AI 服务请求失败（HTTP 500）" },
    );
    expect(next.status).toBe("error");
    expect(next.draft).toBe("写到一半");
    expect(next.error).toBe("AI 服务请求失败（HTTP 500）");
  });

  it("stop 对已经结束的请求无效", () => {
    const done = apply(streaming("a"), { requestId: "a", type: "completed", answer });
    expect(aiStreamReducer(done, { type: "stop", requestId: "a" })).toBe(done);
  });

  it("stop 只作用于当前请求", () => {
    const state = streaming("a");
    expect(aiStreamReducer(state, { type: "stop", requestId: "b" })).toBe(state);
  });

  it("started 上的回退说明会展示出来", () => {
    const next = apply(streaming("a"), { requestId: "a", type: "started", notice: "已回退到非流式" });
    expect(next.notice).toBe("已回退到非流式");
  });

  it("没有活动请求时任何事件都不会被接受", () => {
    const next = apply(initialAiStreamState, { requestId: "a", type: "delta", content: "孤儿事件" });
    expect(next).toBe(initialAiStreamState);
  });

  it("reset 清空全部流式状态", () => {
    const next = apply(streaming("a"), { requestId: "a", type: "delta", content: "x" });
    expect(aiStreamReducer(next, { type: "reset" })).toEqual(initialAiStreamState);
  });
});
