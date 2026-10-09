import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AI } from "./AI";
import { api } from "../api/tauri";
import { useAppStore } from "../stores/app";
import type { AiAnswer, AiStreamEvent } from "../types/domain";

// 与 AI.sourceCitation.test.tsx 相同的 Tauri 桩。
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

function lastRequestId(): string {
  const last = [...invoked].reverse().find(item => item.command === "ask_ai_stream");
  return String(last?.args?.requestId ?? "");
}

function emit(event: AiStreamEvent) {
  act(() => { [...handlers].forEach(handler => handler({ payload: event })); });
}

async function askWith(content: string) {
  const user = userEvent.setup();
  render(<AI />);
  await user.type(screen.getByPlaceholderText("问问你的阅读知识库…"), "这本书讲了什么");
  await user.click(screen.getByLabelText("发送"));
  await waitFor(() => expect(lastRequestId()).not.toBe(""));
  const requestId = lastRequestId();
  emit({ requestId, type: "started" });
  emit({
    requestId,
    type: "completed",
    answer: { content, citations: [], glossaryCitations: [], sourcesConsidered: 0 } satisfies AiAnswer,
  });
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

describe("AI 回答的 URL 消毒", () => {
  it("javascript: 链接不会变成可执行的 href", async () => {
    // 真实缺陷：urlTransform 曾是恒等函数，绕过了 react-markdown 的默认消毒，
    // AI 回答里的 javascript: 协议会原样进入 <a href>，在 Tauri WebView 内可执行。
    await askWith("点我 [点我](javascript:alert(1)) 再看 [数据](JaVaScRiPt:alert(2))");
    await waitFor(() => expect(screen.getByText("点我", { exact: false })).toBeInTheDocument());
    const anchors = Array.from(document.querySelectorAll("a"));
    const dangerous = anchors.filter(a => {
      const href = a.getAttribute("href") ?? "";
      return href.toLowerCase().includes("javascript:");
    });
    expect(dangerous, `不应存在 javascript: 链接，实际 href=${JSON.stringify(anchors.map(a => a.getAttribute("href")))}`).toHaveLength(0);
  });

  it("data: 与 vbscript: 同样被拦截", async () => {
    await askWith("[甲](data:text/html;base64,PHNjcmlwdD4x) [乙](vbscript:msgbox)");
    await waitFor(() => expect(screen.getByLabelText("发送")).toBeInTheDocument());
    const hrefs = Array.from(document.querySelectorAll("a")).map(a => (a.getAttribute("href") ?? "").toLowerCase());
    // 危险协议要么不生成链接，要么 href 被清空，绝不能原样出现。
    expect(hrefs.some(href => href.startsWith("data:"))).toBe(false);
    expect(hrefs.some(href => href.startsWith("vbscript:"))).toBe(false);
  });

  it("正常外链仍然可用", async () => {
    await askWith("见 [官网](https://example.com/article)");
    await waitFor(() => expect(screen.getByText("官网")).toBeInTheDocument());
    const link = document.querySelector('a[href="https://example.com/article"]');
    expect(link).not.toBeNull();
    expect(link?.getAttribute("rel")).toContain("noreferrer");
  });

  it("citation: 与 glossary: 内部引用协议仍然放行", async () => {
    await askWith("见 [[1]](citation:1) 与 [[W1]](glossary:1)");
    // 等回答区域出现（没有任何 citation 命中时退回纯文本，但协议仍不该被当成外链）。
    await waitFor(() => expect(screen.getByLabelText("发送")).toBeInTheDocument());
    const anchors = Array.from(document.querySelectorAll(".answer a"));
    // 允许 citation:/glossary: 这类内部协议，绝不能出现 javascript:。
    for (const anchor of anchors) {
      const href = (anchor.getAttribute("href") ?? "").toLowerCase();
      expect(href === "" || href.startsWith("citation:") || href.startsWith("glossary:") || href.startsWith("http")).toBe(true);
    }
  });
});