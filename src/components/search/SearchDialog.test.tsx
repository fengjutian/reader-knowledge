import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SearchDialog } from "./SearchDialog";
import { api } from "../../api/tauri";
import { useAppStore } from "../../stores/app";
import type { SearchResult } from "../../types/domain";

const results: SearchResult[] = [
  { id: "n1", bookId: "b1", bookTitle: "书一", chapter: "第一章", content: "第一条划线", type: "highlight", createdAt: "2026-01-01", score: 1 },
  { id: "n2", bookId: "b2", bookTitle: "书二", chapter: "第二章", content: "第二条划线", type: "highlight", createdAt: "2026-01-02", score: 1 },
  { id: "n3", bookId: "b3", bookTitle: "书三", chapter: "第三章", content: "第三条想法", type: "thought", createdAt: "2026-01-03", score: 1 },
] as unknown as SearchResult[];

describe("SearchDialog", () => {
  beforeEach(() => {
    useAppStore.setState({ searchOpen: true, selectedBookId: undefined, selectedNoteId: undefined });
  });

  it("点击搜索结果可以打开对应书籍并关闭搜索框", async () => {
    const user = userEvent.setup();
    vi.spyOn(api, "search").mockResolvedValue(results);
    render(<SearchDialog />);
    await user.type(screen.getByPlaceholderText("搜索你的阅读记录"), "划线");

    const item = await screen.findByText("第二条划线");
    await user.click(item);

    await waitFor(() => {
      const state = useAppStore.getState();
      expect(state.selectedBookId).toBe("b2");
      expect(state.selectedNoteId).toBe("n2");
      expect(state.searchOpen).toBe(false);
    });
  });

  it("方向键与 Enter 可以选择搜索结果", async () => {
    const user = userEvent.setup();
    vi.spyOn(api, "search").mockResolvedValue(results);
    render(<SearchDialog />);
    await user.type(screen.getByPlaceholderText("搜索你的阅读记录"), "划线");
    await screen.findByText("第一条划线");

    await user.keyboard("{ArrowDown}");
    await waitFor(() => expect(screen.getByText("第二条划线").closest("button")).toHaveAttribute("aria-selected", "true"));
    await user.keyboard("{Enter}");

    await waitFor(() => {
      const state = useAppStore.getState();
      expect(state.selectedBookId).toBe("b2");
      expect(state.selectedNoteId).toBe("n2");
    });
  });

  it("ArrowUp 从第一项回绕到最后一项", async () => {
    const user = userEvent.setup();
    vi.spyOn(api, "search").mockResolvedValue(results);
    render(<SearchDialog />);
    await user.type(screen.getByPlaceholderText("搜索你的阅读记录"), "划线");
    await screen.findByText("第一条划线");

    await user.keyboard("{ArrowUp}");
    await waitFor(() => expect(screen.getByText("第三条想法").closest("button")).toHaveAttribute("aria-selected", "true"));
  });

  it("新查询后选中项重置为第一项", async () => {
    const user = userEvent.setup();
    vi.spyOn(api, "search").mockResolvedValue(results);
    render(<SearchDialog />);
    const input = screen.getByPlaceholderText("搜索你的阅读记录");
    await user.type(input, "划线");
    await screen.findByText("第一条划线");
    await user.keyboard("{ArrowDown}{ArrowDown}");

    await user.clear(input);
    await user.type(input, "想法");
    await waitFor(() => expect(screen.getByText("第一条划线").closest("button")).toHaveAttribute("aria-selected", "true"));
  });

  it("旧请求的响应不会覆盖新请求的结果", async () => {
    const user = userEvent.setup();
    const staleOnly: SearchResult[] = [{ ...results[0], id: "stale", content: "过期的搜索结果" }] as SearchResult[];
    const resolvers: ((value: SearchResult[]) => void)[] = [];
    vi.spyOn(api, "search").mockImplementation(() => new Promise<SearchResult[]>(resolve => resolvers.push(resolve)));
    render(<SearchDialog />);
    const input = screen.getByPlaceholderText("搜索你的阅读记录");

    await user.type(input, "旧");
    await waitFor(() => expect(resolvers.length).toBe(1));
    await user.clear(input);
    await user.type(input, "新");
    await waitFor(() => expect(resolvers.length).toBe(2));

    // 先返回旧请求的结果，再返回新请求的结果。
    await act(async () => { resolvers[0](staleOnly); });
    await act(async () => { resolvers[1](results); });

    // 新请求结果胜出，旧请求的独有内容不得残留。
    await waitFor(() => expect(screen.getByText("第三条想法")).toBeInTheDocument());
    expect(screen.queryByText("过期的搜索结果")).not.toBeInTheDocument();
  });

  it("搜索失败显示错误而不是伪装成没有结果", async () => {
    const user = userEvent.setup();
    vi.spyOn(api, "search").mockRejectedValue(new Error("后端不可用"));
    render(<SearchDialog />);
    await user.type(screen.getByPlaceholderText("搜索你的阅读记录"), "划线");

    expect(await screen.findByText(/搜索失败：后端不可用/)).toBeInTheDocument();
    expect(screen.queryByText("没有找到相关记录")).not.toBeInTheDocument();
  });
});
