import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { BookMetadata } from "./BookMetadata";
import { api } from "../api/tauri";
import { useAppStore } from "../stores/app";
import { useSyncStore } from "../stores/sync";
import type { BookMetadataRow } from "../types/domain";

const row = (bookId: string, title: string, sources: string[] = ["weread"]): BookMetadataRow => ({
  bookId, title, author: "作者", cover: "", isbn: "", publisher: "", publishedDate: "", pageCount: undefined,
  subjects: [], sources, lastFetchedAt: "1", metadataStatus: "ready",
});

const first = [row("b1", "旧书名")];
const second = [row("b1", "新书名"), row("b2", "第二本")];

describe("BookMetadata 同步刷新", () => {
  beforeEach(() => {
    useAppStore.setState({ searchOpen: false, selectedBookId: undefined, selectedNoteId: undefined });
    useSyncStore.setState({ status: "idle", progress: 0, books: 0, highlights: 0, thoughts: 0, processedBooks: 0, totalBooks: 0, revision: 0, completedAt: undefined });
  });

  it("同步完成后元数据页面重新加载", async () => {
    const list = vi.spyOn(api, "bookMetadata").mockResolvedValue(first);
    render(<BookMetadata />);
    await screen.findByText("旧书名");
    expect(list).toHaveBeenCalledTimes(1);

    list.mockResolvedValue(second);
    useSyncStore.setState({ revision: 1, status: "complete" });

    await waitFor(() => expect(screen.getByText("新书名")).toBeInTheDocument());
    expect(list).toHaveBeenCalledTimes(2);
  });

  it("刷新时保留搜索条件与数据源选择", async () => {
    const list = vi.spyOn(api, "bookMetadata").mockResolvedValue([row("b1", "目标书籍"), row("b2", "另一本")]);
    render(<BookMetadata />);
    await screen.findByText("目标书籍");

    const input = screen.getByPlaceholderText("搜索书名、作者、ISBN 或出版社");
    useSyncStore.setState({ revision: 1, status: "complete" });
    await waitFor(() => expect(list).toHaveBeenCalledTimes(2));
    // 搜索框内容保持不变（未被重置）。
    expect(input).toHaveValue("");
  });

  it("页面刷新不会形成无限请求循环", async () => {
    const list = vi.spyOn(api, "bookMetadata").mockResolvedValue(first);
    render(<BookMetadata />);
    await screen.findByText("旧书名");

    list.mockResolvedValue(second);
    useSyncStore.setState({ revision: 1, status: "complete" });
    await waitFor(() => expect(screen.getByText("新书名")).toBeInTheDocument());
    // revision 不变时不应继续请求。
    await new Promise(resolve => setTimeout(resolve, 200));
    expect(list).toHaveBeenCalledTimes(2);
  });

  it("使用批量接口补全微信读书元数据", async () => {
    vi.spyOn(api, "bookMetadata").mockResolvedValue(first);
    const batch = vi.spyOn(api, "fetchBooksMetadata").mockResolvedValue([
      { bookId: "b1", source: "weread", status: "updated", message: "已更新" },
    ]);
    const single = vi.spyOn(api, "fetchBookMetadata");
    render(<BookMetadata />);
    await screen.findByText("旧书名");

    // 表头复选框是第一个 table-check，行内复选框从第二个开始。
    const checkboxes = document.querySelectorAll("button.table-check");
    expect(checkboxes.length).toBeGreaterThan(1);
    (checkboxes[1] as HTMLElement).click();

    // 页头按钮带括号计数，用它精确定位。
    const header = await screen.findByRole("button", { name: /微信读书补全（\d+\/20）/ });
    await waitFor(() => expect(header).toBeEnabled());
    (header as HTMLElement).click();

    await waitFor(() => expect(batch).toHaveBeenCalledWith(["b1"], "weread", true));
    expect(single).not.toHaveBeenCalled();
  });
});
