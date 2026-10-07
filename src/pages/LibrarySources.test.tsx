import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { LibrarySources } from "./LibrarySources";
import { api } from "../api/tauri";
import { useAppStore } from "../stores/app";
import type { ImportPreview, LibrarySource, SourceDetail } from "../types/domain";

const source = (id: string, title: string, overrides: Partial<LibrarySource> = {}): LibrarySource => ({
  id, sourceType: "pdf", title, author: "作者", origin: "C:\\book.pdf",
  pageCount: 120, documentCount: 42, importedAt: "1700000000", deleted: false, ...overrides,
});

const preview = (overrides: Partial<ImportPreview> = {}): ImportPreview => ({
  sourceType: "pdf", title: "导入的书", author: "作者", origin: "C:\\book.pdf",
  pageCount: 120, documentCount: 3, warnings: [], duplicate: false,
  sample: [
    { id: "", position: 0, heading: "第 1 页", content: "第一页的正文内容", locator: { page: 1 } },
    { id: "", position: 1, heading: "第 2 页", content: "第二页的正文内容", locator: { page: 2 } },
  ],
  ...overrides,
});

const detail: SourceDetail = {
  ...source("s1", "导入的书"),
  documents: [
    { id: "d1", position: 0, heading: "第 1 页", content: "第一页正文", locator: { page: 1 } },
    { id: "d2", position: 1, heading: "第 2 页", content: "第二页正文", locator: { page: 2 } },
  ],
};

beforeEach(() => {
  useAppStore.setState({ page: "import", sourceDetailId: undefined, selectedBookId: undefined, selectedNoteId: undefined });
  vi.spyOn(api, "librarySources").mockResolvedValue([source("s1", "导入的书")]);
  vi.spyOn(api, "librarySource").mockResolvedValue(detail);
  vi.spyOn(api, "previewFileImport").mockResolvedValue(preview());
  vi.spyOn(api, "previewWebImport").mockResolvedValue(preview({ sourceType: "web", origin: "https://example.com/a" }));
  vi.spyOn(api, "confirmImport").mockResolvedValue(preview({ duplicate: false, duplicateOf: "s2" }));
  vi.spyOn(api, "deleteLibrarySource").mockResolvedValue(undefined);
  vi.spyOn(api, "purgeLibrarySource").mockResolvedValue(undefined);
});

describe("LibrarySources", () => {
  it("空状态引导先导入", async () => {
    vi.mocked(api.librarySources).mockResolvedValue([]);
    render(<LibrarySources />);
    expect(await screen.findByText("还没有导入任何资料")).toBeInTheDocument();
  });

  it("列表展示类型、作者、页数与文档块数", async () => {
    render(<LibrarySources />);
    const item = await screen.findByText("导入的书");
    expect(item).toBeInTheDocument();
    expect(screen.getByText(/PDF · 作者 · 120 页 · 42 个文档块/)).toBeInTheDocument();
  });

  it("加载失败时显示错误", async () => {
    vi.mocked(api.librarySources).mockRejectedValue(new Error("database is locked"));
    render(<LibrarySources />);
    expect(await screen.findByText(/操作失败：database is locked/)).toBeInTheDocument();
  });

  it("移除是软删除，只调接口并刷新", async () => {
    const user = userEvent.setup();
    render(<LibrarySources />);
    await screen.findByText("导入的书");
    await user.click(screen.getByRole("button", { name: /移除/ }));

    await waitFor(() => expect(api.deleteLibrarySource).toHaveBeenCalledWith("s1"));
    await waitFor(() => expect(api.librarySources).toHaveBeenCalledTimes(2));
    expect(api.purgeLibrarySource).not.toHaveBeenCalled();
  });

  it("彻底删除需要二次确认，取消时不删除", async () => {
    const user = userEvent.setup();
    render(<LibrarySources />);
    await screen.findByText("导入的书");
    await user.click(screen.getByRole("button", { name: /彻底删除/ }));

    const confirm = await screen.findByRole("alertdialog");
    expect(within(confirm).getByText(/彻底删除《导入的书》？/)).toBeInTheDocument();

    await user.click(within(confirm).getByRole("button", { name: "取消" }));
    await waitFor(() => expect(screen.queryByRole("alertdialog")).not.toBeInTheDocument());
    expect(api.purgeLibrarySource).not.toHaveBeenCalled();
  });

  it("确认后彻底删除", async () => {
    const user = userEvent.setup();
    render(<LibrarySources />);
    await screen.findByText("导入的书");
    await user.click(screen.getByRole("button", { name: /彻底删除/ }));
    const confirm = await screen.findByRole("alertdialog");
    await user.click(within(confirm).getByRole("button", { name: "确认删除" }));

    await waitFor(() => expect(api.purgeLibrarySource).toHaveBeenCalledWith("s1"));
  });

  it("点击资料打开详情并显示页码定位", async () => {
    const user = userEvent.setup();
    render(<LibrarySources />);
    await user.click(await screen.findByText("导入的书"));

    expect(await screen.findByText("第一页正文")).toBeInTheDocument();
    // 标题与页码标签同时存在，用 locator 精确断言页码那一个
    expect(screen.getAllByText("第 1 页").length).toBeGreaterThan(0);
    expect(useAppStore.getState().sourceDetailId).toBe("s1");
  });

  it("从全局搜索进来的资料会直接展开详情", async () => {
    render(<LibrarySources />);
    useAppStore.setState({ sourceDetailId: "s1" });
    expect(await screen.findByText("第二页正文")).toBeInTheDocument();
  });

  it("导入对话框预览后展示正文样本", async () => {
    const user = userEvent.setup();
    render(<LibrarySources />);
    await user.click(await screen.findByRole("button", { name: /导入资料/ }));

    await user.click(screen.getByRole("tab", { name: /本地文件/ }));
    await user.type(screen.getByPlaceholderText(/book\.epub/), "C:\\book.pdf");
    await user.click(screen.getByRole("button", { name: "预览" }));

    expect(await screen.findByText("第一页的正文内容")).toBeInTheDocument();
    expect(screen.getByText(/3 个文档块/)).toBeInTheDocument();
    expect(screen.getByText(/共 120 页/)).toBeInTheDocument();
  });

  it("预览时显示解析警告", async () => {
    const user = userEvent.setup();
    vi.mocked(api.previewFileImport).mockResolvedValue(preview({ warnings: ["共 120 页，其中 3 页没有文字层（可能是图片页）"] }));
    render(<LibrarySources />);
    await user.click(await screen.findByRole("button", { name: /导入资料/ }));
    await user.click(screen.getByRole("tab", { name: /本地文件/ }));
    await user.type(screen.getByPlaceholderText(/book\.epub/), "C:\\book.pdf");
    await user.click(screen.getByRole("button", { name: "预览" }));

    expect(await screen.findByText(/3 页没有文字层/)).toBeInTheDocument();
  });

  it("重复内容禁止再次入库", async () => {
    const user = userEvent.setup();
    vi.mocked(api.previewFileImport).mockResolvedValue(preview({ duplicate: true, duplicateOf: "existing" }));
    render(<LibrarySources />);
    await user.click(await screen.findByRole("button", { name: /导入资料/ }));
    await user.click(screen.getByRole("tab", { name: /本地文件/ }));
    await user.type(screen.getByPlaceholderText(/book\.epub/), "C:\\book.pdf");
    await user.click(screen.getByRole("button", { name: "预览" }));

    expect(await screen.findByText(/这份内容已经导入过/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /确认导入/ })).toBeDisabled();
  });

  it("确认导入后提示成功并刷新列表", async () => {
    const user = userEvent.setup();
    render(<LibrarySources />);
    await user.click(await screen.findByRole("button", { name: /导入资料/ }));
    await user.click(screen.getByRole("tab", { name: /本地文件/ }));
    await user.type(screen.getByPlaceholderText(/book\.epub/), "C:\\book.pdf");
    await user.click(screen.getByRole("button", { name: "预览" }));
    await screen.findByText("第一页的正文内容");
    await user.click(screen.getByRole("button", { name: /确认导入/ }));

    expect(await screen.findByText(/已导入《导入的书》/)).toBeInTheDocument();
    expect(api.confirmImport).toHaveBeenCalledWith(expect.objectContaining({ sourceType: "pdf", path: "C:\\book.pdf" }));
  });

  it("导入失败时展示错误且不写库", async () => {
    const user = userEvent.setup();
    vi.mocked(api.previewFileImport).mockRejectedValue(new Error("这份 PDF 没有可提取的文字，可能是扫描版；暂不支持 OCR，已跳过导入"));
    render(<LibrarySources />);
    await user.click(await screen.findByRole("button", { name: /导入资料/ }));
    await user.click(screen.getByRole("tab", { name: /本地文件/ }));
    await user.type(screen.getByPlaceholderText(/book\.epub/), "C:\\scan.pdf");
    await user.click(screen.getByRole("button", { name: "预览" }));

    // 对话框内与页面级错误区都会显示，用 getAllByText 避免依赖具体容器
    expect((await screen.findAllByText(/暂不支持 OCR/)).length).toBeGreaterThan(0);
    expect(api.confirmImport).not.toHaveBeenCalled();
  });

  it("网页导入走 previewWebImport", async () => {
    const user = userEvent.setup();
    render(<LibrarySources />);
    await user.click(await screen.findByRole("button", { name: /导入资料/ }));
    await user.type(screen.getByPlaceholderText(/example\.com/), "https://example.com/a");
    await user.click(screen.getByRole("button", { name: "预览" }));

    await waitFor(() => expect(api.previewWebImport).toHaveBeenCalledWith("https://example.com/a"));
  });
});
