import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { SearchDialog } from "./SearchDialog";
import { api } from "../../api/tauri";
import { useAppStore } from "../../stores/app";
import type { Book, GlobalSearchPage, GlobalSearchResult, SearchEntityType } from "../../types/domain";

function result(overrides: Partial<GlobalSearchResult> & { id: string; type: SearchEntityType }): GlobalSearchResult {
  return {
    bookId: "b1",
    title: "书一",
    subtitle: "第一章",
    snippet: "正文",
    score: 1,
    updatedAt: "2026-01-01",
    ...overrides,
  } as GlobalSearchResult;
}

const books: Book[] = [
  { id: "b1", title: "书一", author: "作者甲", category: "社科", cover: "", highlightCount: 3, thoughtCount: 1, progress: 0, updatedAt: "", readingStatus: "reading" },
  { id: "b2", title: "书二", author: "作者乙", category: "历史", cover: "", highlightCount: 1, thoughtCount: 0, progress: 0, updatedAt: "", readingStatus: "reading" },
];

function page(results: GlobalSearchResult[], hasMore = false): GlobalSearchPage {
  return { results, hasMore };
}

describe("SearchDialog", () => {
  beforeEach(() => {
    useAppStore.setState({ searchOpen: true, selectedBookId: undefined, selectedNoteId: undefined });
    vi.spyOn(api, "books").mockResolvedValue(books);
    vi.useRealTimers();
  });

  async function typeAndWait(user: ReturnType<typeof userEvent.setup>, text: string) {
    await user.type(screen.getByPlaceholderText(/搜索书名/), text);
    // debounce 是 200ms
    await waitFor(() => expect(vi.mocked(api.globalSearch)).toHaveBeenCalled(), { timeout: 2000 });
  }

  it("点击划线结果可以打开对应书籍并关闭搜索框", async () => {
    const user = userEvent.setup();
    vi.spyOn(api, "globalSearch").mockResolvedValue(page([result({ id: "h1", type: "highlight" })]));
    render(<SearchDialog />);
    await typeAndWait(user, "划线");

    await user.click(await screen.findByText("正文"));
    await waitFor(() => {
      const state = useAppStore.getState();
      expect(state.selectedBookId).toBe("b1");
      expect(state.selectedNoteId).toBe("h1");
      expect(state.searchOpen).toBe(false);
    });
  });

  it("点击书籍结果只打开书籍详情，不定位笔记", async () => {
    const user = userEvent.setup();
    vi.spyOn(api, "globalSearch").mockResolvedValue(page([result({ id: "b2", type: "book", bookId: "b2", title: "书二", subtitle: "作者乙 · 历史", snippet: "ISBN 978" })]));
    render(<SearchDialog />);
    await typeAndWait(user, "书二");

    await user.click(await screen.findByText("书二"));
    await waitFor(() => {
      const state = useAppStore.getState();
      expect(state.selectedBookId).toBe("b2");
      expect(state.selectedNoteId).toBeUndefined();
      expect(state.searchOpen).toBe(false);
    });
  });

  it("方向键与 Enter 可以选择搜索结果", async () => {
    const user = userEvent.setup();
    vi.spyOn(api, "globalSearch").mockResolvedValue(page([
      result({ id: "h1", type: "highlight", snippet: "第一条划线" }),
      result({ id: "h2", type: "highlight", snippet: "第二条划线" }),
    ]));
    render(<SearchDialog />);
    await typeAndWait(user, "划线");
    await screen.findByText("第一条划线");

    await user.keyboard("{ArrowDown}");
    await waitFor(() => expect(screen.getByText("第二条划线").closest("button")).toHaveAttribute("aria-selected", "true"));
    await user.keyboard("{Enter}");

    await waitFor(() => {
      const state = useAppStore.getState();
      expect(state.selectedBookId).toBe("b1");
      expect(state.selectedNoteId).toBe("h2");
    });
  });

  it("ArrowUp 从第一项回绕到最后一项", async () => {
    const user = userEvent.setup();
    vi.spyOn(api, "globalSearch").mockResolvedValue(page([
      result({ id: "h1", type: "highlight", snippet: "第一条划线" }),
      result({ id: "h2", type: "highlight", snippet: "第二条划线" }),
    ]));
    render(<SearchDialog />);
    await typeAndWait(user, "划线");
    await screen.findByText("第一条划线");

    await user.keyboard("{ArrowUp}");
    await waitFor(() => expect(screen.getByText("第二条划线").closest("button")).toHaveAttribute("aria-selected", "true"));
  });

  it("新查询后选中项重置为第一项", async () => {
    const user = userEvent.setup();
    vi.spyOn(api, "globalSearch").mockResolvedValue(page([
      result({ id: "h1", type: "highlight", snippet: "第一条划线" }),
      result({ id: "h2", type: "highlight", snippet: "第二条划线" }),
    ]));
    render(<SearchDialog />);
    await typeAndWait(user, "划线");
    await screen.findByText("第一条划线");
    await user.keyboard("{ArrowDown}");

    await typeAndWait(user, "划线二");
    await waitFor(() => expect(screen.getByText("第一条划线").closest("button")).toHaveAttribute("aria-selected", "true"));
  });

  it("旧请求的响应不会覆盖新请求的结果", async () => {
    const user = userEvent.setup();
    const resolvers: ((value: GlobalSearchPage) => void)[] = [];
    vi.spyOn(api, "globalSearch").mockImplementation(() => new Promise(resolve => { resolvers.push(resolve); }));
    render(<SearchDialog />);

    const input = screen.getByPlaceholderText(/搜索书名/);
    await user.type(input, "旧");
    await waitFor(() => expect(resolvers).toHaveLength(1), { timeout: 2000 });
    await user.clear(input);
    await user.type(input, "新");
    await waitFor(() => expect(resolvers).toHaveLength(2), { timeout: 2000 });

    // 旧请求最后才返回，不能覆盖新结果
    await act(async () => { resolvers[1](page([result({ id: "new", type: "highlight", snippet: "新结果" })])); });
    await act(async () => { resolvers[0](page([result({ id: "old", type: "highlight", snippet: "旧结果" })])); });

    expect(await screen.findByText("新结果")).toBeInTheDocument();
    expect(screen.queryByText("旧结果")).not.toBeInTheDocument();
  });

  it("类型 chips 会把所选类型传给后端", async () => {
    const user = userEvent.setup();
    const search = vi.spyOn(api, "globalSearch").mockResolvedValue(page([result({ id: "h1", type: "highlight" })]));
    render(<SearchDialog />);
    await typeAndWait(user, "内容");

    await user.click(screen.getByRole("button", { name: "划线" }));
    await waitFor(() => {
      const last = search.mock.calls[search.mock.calls.length - 1][0];
      expect(last.types).toEqual(["highlight"]);
    });

    await user.click(screen.getByRole("button", { name: "想法" }));
    await waitFor(() => {
      const last = search.mock.calls[search.mock.calls.length - 1][0];
      expect(last.types).toEqual(["thought"]);
    });

    await user.click(screen.getByRole("button", { name: "全部" }));
    await waitFor(() => {
      const last = search.mock.calls[search.mock.calls.length - 1][0];
      expect(last.types).toEqual([]);
    });
  });

  it("限定书籍后把书籍过滤传给后端", async () => {
    const user = userEvent.setup();
    const search = vi.spyOn(api, "globalSearch").mockResolvedValue(page([result({ id: "h1", type: "highlight" })]));
    render(<SearchDialog />);
    await typeAndWait(user, "内容");

    await user.type(screen.getByLabelText("限定书籍"), "书一");
    await waitFor(() => {
      const last = search.mock.calls[search.mock.calls.length - 1][0];
      expect(last.bookId).toBe("书一");
    });

    await user.click(screen.getByLabelText("清除书籍过滤"));
    await waitFor(() => {
      const last = search.mock.calls[search.mock.calls.length - 1][0];
      expect(last.bookId).toBeUndefined();
    });
  });

  it("加载更多会带上 offset 并追加结果", async () => {
    const user = userEvent.setup();
    const search = vi.spyOn(api, "globalSearch").mockResolvedValueOnce(page([result({ id: "h1", type: "highlight", snippet: "首批结果" })], true));
    render(<SearchDialog />);
    await typeAndWait(user, "内容");
    await screen.findByText("首批结果");

    search.mockResolvedValueOnce(page([result({ id: "h2", type: "highlight", snippet: "第二批结果" })]));
    await user.click(screen.getByRole("button", { name: /加载更多/ }));

    expect(await screen.findByText("第二批结果")).toBeInTheDocument();
    expect(screen.getByText("首批结果")).toBeInTheDocument();
    expect(search.mock.calls[search.mock.calls.length - 1][0].offset).toBe(1);
  });

  it("加载、空结果、错误是三个不同状态", async () => {
    const user = userEvent.setup();
    const search = vi.spyOn(api, "globalSearch");
    // 空结果
    search.mockResolvedValueOnce(page([]));
    render(<SearchDialog />);
    await typeAndWait(user, "无结果");
    expect(await screen.findByText("没有找到相关记录")).toBeInTheDocument();
    await waitFor(() => expect(screen.queryByText("正在搜索…")).not.toBeInTheDocument());

    // 数据库错误不能伪装成空结果
    search.mockRejectedValueOnce(new Error("database is locked"));
    const input = screen.getByPlaceholderText(/搜索书名/);
    await user.type(input, "会报错");
    expect(await screen.findByText(/搜索失败：/)).toBeInTheDocument();
    expect(screen.queryByText("没有找到相关记录")).not.toBeInTheDocument();
  });

  it("搜索框刚打开时不请求后端", async () => {
    const search = vi.spyOn(api, "globalSearch").mockResolvedValue(page([]));
    render(<SearchDialog />);
    await screen.findByText(/输入书名、章节、划线或想法/);
    await new Promise(resolve => setTimeout(resolve, 400));
    expect(search).not.toHaveBeenCalled();
  });
});
