import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AI } from "./AI";
import { api } from "../api/tauri";
import { useAppStore } from "../stores/app";
import type { AiAnswer, AiStreamEvent } from "../types/domain";

// 与 AI.stream.test.tsx 同样的 Tauri 桩：组件自己生成 requestId，
// 只能从 invoke 的入参里把它捞回来。
const handlers = new Set<(event: { payload: AiStreamEvent }) => void>();
const invoked: { command: string; args?: Record<string, unknown> }[] = [];

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(async (command: string, args?: Record<string, unknown>) => {
    invoked.push({ command, args });
    return undefined;
  }),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_event: string, handler: (event: { payload: AiStreamEvent }) => void) => {
    handlers.add(handler);
    return () => { handlers.delete(handler); };
  }),
  emit: vi.fn(async () => undefined),
}));

const answerWith = (sourceCitations: NonNullable<AiAnswer["sourceCitations"]>): AiAnswer => ({
  content: "答案正文 [1]",
  citations: [],
  sourceCitations,
  glossaryCitations: [],
  sourcesConsidered: 3,
});

function lastRequestId(): string {
  const last = [...invoked].reverse().find(item => item.command === "ask_ai_stream");
  return String(last?.args?.requestId ?? "");
}

function emit(event: AiStreamEvent) {
  act(() => { [...handlers].forEach(handler => handler({ payload: event })); });
}

/** 提问并把流式回答收尾，返回可继续操作的 userEvent。 */
async function askWith(answer: AiAnswer) {
  const user = userEvent.setup();
  render(<AI />);
  await user.type(screen.getByPlaceholderText("问问你的阅读知识库…"), "这本书讲了什么");
  await user.click(screen.getByLabelText("发送"));
  await waitFor(() => expect(lastRequestId()).not.toBe(""));
  const requestId = lastRequestId();
  emit({ requestId, type: "started" });
  emit({ requestId, type: "completed", answer });
  return user;
}

beforeEach(() => {
  handlers.clear();
  invoked.length = 0;
  localStorage.clear();
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  vi.spyOn(api, "books").mockResolvedValue([]);
  useAppStore.setState({ page: "ai", sourceDetailId: undefined, sourceDetailLocator: undefined });
});

describe("AI 引用跳转导入资料", () => {
  it("点击 PDF 引用保存页码 locator 并切到导入页", async () => {
    const user = await askWith(answerWith([
      { index: 1, sourceId: "s1", title: "导入的 PDF", locator: { page: 12 }, quote: "第十二页的原话" },
    ]));

    await waitFor(() => expect(screen.getByText("导入的 PDF")).toBeInTheDocument());
    await user.click(screen.getByText("导入的 PDF"));

    expect(useAppStore.getState().sourceDetailId).toBe("s1");
    expect(useAppStore.getState().sourceDetailLocator).toEqual({ page: 12 });
    expect(useAppStore.getState().page).toBe("import");
  });

  it("点击 EPUB 引用保存章节 locator", async () => {
    const user = await askWith(answerWith([
      { index: 1, sourceId: "e1", title: "导入的 EPUB", locator: { chapter: 3 }, quote: "第三章的原话" },
    ]));

    await waitFor(() => expect(screen.getByText("导入的 EPUB")).toBeInTheDocument());
    await user.click(screen.getByText("导入的 EPUB"));

    expect(useAppStore.getState().sourceDetailId).toBe("e1");
    expect(useAppStore.getState().sourceDetailLocator).toEqual({ chapter: 3 });
    expect(useAppStore.getState().page).toBe("import");
  });

  it("正文里的引用角标也能带上 locator", async () => {
    const user = await askWith(answerWith([
      { index: 1, sourceId: "s1", title: "导入的 PDF", locator: { page: 7 }, quote: "第七页的原话" },
    ]));

    await waitFor(() => expect(screen.getByText("导入的 PDF")).toBeInTheDocument());
    // markdown 里 `[1]` 被改写成 `[[1]](citation:1)`，所以角标的可访问名是 `[1]`
    await user.click(screen.getByRole("button", { name: /\[1\]/ }));

    expect(useAppStore.getState().sourceDetailLocator).toEqual({ page: 7 });
  });

  it("重复点同一条引用也能重新带上 locator", async () => {
    const locator = { page: 12 };
    const user = await askWith(answerWith([
      { index: 1, sourceId: "s1", title: "导入的 PDF", locator, quote: "第十二页的原话" },
    ]));

    await waitFor(() => expect(screen.getByText("导入的 PDF")).toBeInTheDocument());
    const link = screen.getByText("导入的 PDF");
    await user.click(link);
    const first = useAppStore.getState().sourceDetailLocator;
    await user.click(link);

    // 每次点击都复制一份 locator：值相同但对象身份不同，详情面板才会重新定位
    expect(useAppStore.getState().sourceDetailLocator).toEqual({ page: 12 });
    expect(useAppStore.getState().sourceDetailLocator).not.toBe(first);
  });
});
