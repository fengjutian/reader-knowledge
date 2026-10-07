import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AI } from "./AI";
import { api } from "../api/tauri";
import { useAppStore } from "../stores/app";
import type { AiAnswer, AiStreamEvent } from "../types/domain";

// 测试环境没有 Tauri 运行时，必须让 adapter 认为自己跑在 Tauri 里才会走流式分支。
const handlers = new Set<(event: { payload: AiStreamEvent }) => void>();
const invoked: { command: string; args?: Record<string, unknown> }[] = [];
const unlistenCount = { value: 0 };

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (command: string, args?: Record<string, unknown>) => {
    invoked.push({ command, args });
    return undefined;
  }),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_event: string, handler: (event: { payload: AiStreamEvent }) => void) => {
    handlers.add(handler);
    return () => {
      handlers.delete(handler);
      unlistenCount.value += 1;
    };
  }),
  emit: vi.fn(async () => undefined),
}));

const answer: AiAnswer = {
  content: "这是完整回答",
  citations: [{ index: 1, note: { id: "n1", type: "highlight", bookId: "b1", bookTitle: "书一", chapter: "第一章", content: "划线正文", createdAt: "2026-01-01" } }],
  glossaryCitations: [],
  sourcesConsidered: 5,
};

/** 推送一条后端事件，走 act 让 React 处理状态更新。 */
function emit(event: AiStreamEvent) {
  act(() => { [...handlers].forEach(handler => handler({ payload: event })); });
}

/** 取出最近一次 ask_ai_stream 使用的 requestId。 */
function lastRequestId(): string {
  const last = [...invoked].reverse().find(item => item.command === "ask_ai_stream");
  return String(last?.args?.requestId ?? "");
}

function cancelledRequestIds(): string[] {
  return invoked.filter(item => item.command === "cancel_ai_stream").map(item => String(item.args?.requestId ?? ""));
}

describe("AI 流式回答", () => {
  beforeEach(() => {
    handlers.clear();
    invoked.length = 0;
    unlistenCount.value = 0;
    localStorage.clear();
    // 没有 __TAURI_INTERNALS__ 时 adapter 会直接拒绝，必须先伪装成 Tauri 运行时。
    (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
    vi.spyOn(api, "books").mockResolvedValue([]);
  });

  async function ask(text: string) {
    const user = userEvent.setup();
    const view = render(<AI />);
    await user.type(screen.getByPlaceholderText("问问你的阅读知识库…"), text);
    await user.click(screen.getByLabelText("发送"));
    await waitFor(() => expect(lastRequestId()).not.toBe(""));
    return { user, unmount: view.unmount };
  }

  it("逐块显示模型输出并把发送按钮换成停止按钮", async () => {
    await ask("流式测试");
    const requestId = lastRequestId();
    emit({ requestId, type: "started" });
    emit({ requestId, type: "delta", content: "第一段" });
    emit({ requestId, type: "delta", content: "，第二段" });

    expect(await screen.findByText("第一段，第二段")).toBeInTheDocument();
    expect(screen.getByLabelText("停止生成")).toBeInTheDocument();

    emit({ requestId, type: "completed", answer });
    await waitFor(() => expect(screen.queryByLabelText("停止生成")).not.toBeInTheDocument());
    expect(await screen.findByText("这是完整回答")).toBeInTheDocument();
  });

  it("旧请求的迟到事件不会串进当前回答", async () => {
    await ask("串流测试");
    const requestId = lastRequestId();
    emit({ requestId, type: "delta", content: "本会话内容" });
    emit({ requestId: "stale-request", type: "delta", content: "上个会话的尾巴" });
    expect(await screen.findByText("本会话内容")).toBeInTheDocument();
    expect(screen.queryByText("上个会话的尾巴")).not.toBeInTheDocument();

    // 迟到的 completed 也不能把上个会话的回答写进历史。
    emit({ requestId: "stale-request", type: "completed", answer: { ...answer, content: "上个会话的回答" } });
    expect(screen.queryByText("上个会话的回答")).not.toBeInTheDocument();
    expect(document.querySelectorAll(".ai-history__item")).toHaveLength(0);
    expect(screen.getByLabelText("停止生成")).toBeInTheDocument();
  });

  it("点击停止后通知后端取消，并且不再追加内容", async () => {
    const { user } = await ask("停止测试");
    const requestId = lastRequestId();
    emit({ requestId, type: "delta", content: "已生成的部分" });
    await user.click(screen.getByLabelText("停止生成"));

    expect(await screen.findByText("已停止生成，以上内容未完成。")).toBeInTheDocument();
    expect(cancelledRequestIds()).toEqual([requestId]);

    emit({ requestId, type: "delta", content: "不该再出现" });
    expect(screen.queryByText("已生成的部分不该再出现")).not.toBeInTheDocument();
  });

  it("引用导入资料时跳到资料页并展示页码定位", async () => {
    const { user } = await ask("资料引用");
    const requestId = lastRequestId();
    emit({ requestId, type: "completed", answer: {
      ...answer,
      content: "根据导入资料[1] 的说法。",
      citations: [],
      sourceCitations: [{ index: 1, sourceId: "s1", sourceType: "pdf", title: "导入的书", locator: { page: 12 }, quote: "原文引文片段" }],
    } });

    expect(await screen.findByText("引用的导入资料")).toBeInTheDocument();
    expect(screen.getByText("第 12 页")).toBeInTheDocument();

    await user.click(screen.getByText("导入的书"));
    await waitFor(() => {
      const state = useAppStore.getState();
      expect(state.sourceDetailId).toBe("s1");
      expect(state.page).toBe("import");
    });
  });

  it("重排降级提示展示为非阻断消息，正文照常显示", async () => {
    await ask("降级提示");
    const requestId = lastRequestId();
    emit({ requestId, type: "delta", content: "这是正文" });
    emit({ requestId, type: "completed", answer: { ...answer, content: "这是正文", rerankNote: "重排服务超时，本次沿用本地排序" } });

    expect(await screen.findByText("重排服务超时，本次沿用本地排序")).toBeInTheDocument();
    expect(screen.getByText("这是正文")).toBeInTheDocument();
    // 提示不能变成错误，回答仍然是正常入库的一条 turn
    await waitFor(() => expect(document.querySelectorAll(".ai-history__item")).toHaveLength(1));
    expect(screen.queryByText(/搜索失败|生成失败/)).not.toBeInTheDocument();
  });

  it("完成后只写入一次历史", async () => {
    await ask("历史测试");
    const requestId = lastRequestId();
    emit({ requestId, type: "delta", content: "草稿" });
    emit({ requestId, type: "completed", answer });
    await waitFor(() => expect(document.querySelectorAll(".ai-history__item")).toHaveLength(1));

    // 后端重复推送同一个 completed 也不能再写一条。
    emit({ requestId, type: "completed", answer });
    await waitFor(() => expect(document.querySelectorAll(".ai-history__item")).toHaveLength(1));

    const saved = JSON.parse(localStorage.getItem("readflow-ai-history") || "[]");
    expect(saved).toHaveLength(1);
    expect(saved[0].turns).toHaveLength(1);
  });

  it("失败后展示错误、保留部分内容并可以重试", async () => {
    const { user } = await ask("失败测试");
    const requestId = lastRequestId();
    emit({ requestId, type: "delta", content: "写到一半" });
    emit({ requestId, type: "failed", message: "AI 服务请求失败（HTTP 500）" });

    expect(await screen.findByText("AI 服务请求失败（HTTP 500）")).toBeInTheDocument();
    expect(screen.getByText("写到一半")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /重试/ }));
    await waitFor(() => expect(screen.getByLabelText("停止生成")).toBeInTheDocument());
    // 重试沿用原问题，但生成新的 request ID。
    const retryId = lastRequestId();
    expect(retryId).not.toBe(requestId);
    emit({ requestId: retryId, type: "delta", content: "第二次尝试" });
    expect(await screen.findByText("第二次尝试")).toBeInTheDocument();
  });

  it("切换会话时取消当前请求并解除监听", async () => {
    const { user } = await ask("切换测试");
    const requestId = lastRequestId();
    emit({ requestId, type: "delta", content: "生成中" });

    await user.click(screen.getByText("新对话"));
    expect(cancelledRequestIds()).toEqual([requestId]);
    expect(unlistenCount.value).toBeGreaterThan(0);
    expect(handlers.size).toBe(0);
  });

  it("组件卸载时解除监听", async () => {
    const { unmount } = await ask("卸载测试");
    expect(handlers.size).toBe(1);
    unmount();
    expect(handlers.size).toBe(0);
    expect(unlistenCount.value).toBeGreaterThan(0);
  });
});
