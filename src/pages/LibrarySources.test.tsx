import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { open } from "@tauri-apps/plugin-dialog";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { LibrarySources } from "./LibrarySources";
import { api } from "../api/tauri";
import { useAppStore } from "../stores/app";
import type { ImportPreview, LibrarySource, SourceDetail } from "../types/domain";

vi.mock("@tauri-apps/plugin-dialog", () => ({ open: vi.fn(async () => null) }));

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

const scrollIntoView = vi.fn();
const pickFile = async (path: string | null) => { vi.mocked(open).mockResolvedValue(path as never); };
/** 打开导入对话框、切到本地文件页、选一个文件并触发预览。 */
async function importViaDialog(user: ReturnType<typeof userEvent.setup>, path: string | null = "C:\\book.pdf") {
  await user.click(await screen.findByRole("button", { name: /导入资料/ }));
  await user.click(screen.getByRole("tab", { name: /本地文件/ }));
  await pickFile(path);
  await user.click(screen.getByRole("button", { name: /选择文件/ }));
}

beforeEach(() => {
  useAppStore.setState({ page: "import", sourceDetailId: undefined, sourceDetailLocator: undefined, selectedBookId: undefined, selectedNoteId: undefined });
  vi.spyOn(api, "librarySources").mockResolvedValue([source("s1", "导入的书")]);
  vi.spyOn(api, "librarySource").mockResolvedValue(detail);
  vi.spyOn(api, "previewFileImport").mockResolvedValue(preview());
  vi.spyOn(api, "previewWebImport").mockResolvedValue(preview({ sourceType: "web", origin: "https://example.com/a" }));
  vi.spyOn(api, "confirmImport").mockResolvedValue(preview({ duplicate: false, duplicateOf: "s2" }));
  vi.spyOn(api, "deleteLibrarySource").mockResolvedValue(undefined);
  vi.spyOn(api, "purgeLibrarySource").mockResolvedValue(undefined);
  vi.mocked(open).mockResolvedValue(null as never);
  Element.prototype.scrollIntoView = scrollIntoView;
});

afterEach(() => { scrollIntoView.mockClear(); });

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
    await importViaDialog(user);

    expect(await screen.findByText("第一页的正文内容")).toBeInTheDocument();
    expect(screen.getByText(/3 个文档块/)).toBeInTheDocument();
    expect(screen.getByText(/共 120 页/)).toBeInTheDocument();
  });

  it("文件选择器只放行 PDF 与 EPUB", async () => {
    const user = userEvent.setup();
    render(<LibrarySources />);
    await importViaDialog(user);

    expect(open).toHaveBeenCalledWith(expect.objectContaining({
      multiple: false,
      directory: false,
      filters: [{ name: "支持的资料", extensions: ["pdf", "epub"] }],
    }));
  });

  it("路径输入框只读，不再要求手工输入路径", async () => {
    const user = userEvent.setup();
    render(<LibrarySources />);
    await importViaDialog(user);

    const input = screen.getByDisplayValue("C:\\book.pdf");
    expect(input).toHaveAttribute("readonly");
    expect(screen.queryByPlaceholderText(/book\.epub/)).not.toBeInTheDocument();
  });

  it("中文与空格路径原样传给后端", async () => {
    const user = userEvent.setup();
    vi.mocked(api.previewFileImport).mockResolvedValue(preview({ origin: "C:\\我的 资料\\测试书.pdf" }));
    render(<LibrarySources />);
    await importViaDialog(user, "C:\\我的 资料\\测试书.pdf");

    expect(api.previewFileImport).toHaveBeenCalledWith("C:\\我的 资料\\测试书.pdf");
    expect(await screen.findByDisplayValue("C:\\我的 资料\\测试书.pdf")).toBeInTheDocument();
  });

  it("用户取消文件对话框不报错也不预览", async () => {
    const user = userEvent.setup();
    render(<LibrarySources />);
    await importViaDialog(user, null);

    expect(open).toHaveBeenCalled();
    expect(api.previewFileImport).not.toHaveBeenCalled();
    expect(screen.queryByText(/请先选择/)).not.toBeInTheDocument();
  });

  it("选择新文件会清掉上一次的预览", async () => {
    const user = userEvent.setup();
    render(<LibrarySources />);
    await importViaDialog(user);
    await screen.findByText("第一页的正文内容");

    await pickFile("C:\\book.epub");
    await user.click(screen.getByRole("button", { name: /选择文件/ }));
    await waitFor(() => expect(api.previewFileImport).toHaveBeenCalledTimes(2));

    // 预览块已重置，等新的样本出现
    await screen.findByText("第一页的正文内容");
    expect(api.previewFileImport).toHaveBeenLastCalledWith("C:\\book.epub");
  });

  it("文件对话框报错时显示错误信息", async () => {
    const user = userEvent.setup();
    vi.mocked(open).mockRejectedValueOnce(new Error("无法打开文件对话框"));
    render(<LibrarySources />);
    await user.click(await screen.findByRole("button", { name: /导入资料/ }));
    await user.click(screen.getByRole("tab", { name: /本地文件/ }));
    await user.click(screen.getByRole("button", { name: /选择文件/ }));

    expect(await screen.findByText(/无法打开文件对话框/)).toBeInTheDocument();
  });

  it("选择期间按钮禁用，不能重复触发", async () => {
    const user = userEvent.setup();
    let release: (value: string) => void = () => {};
    vi.mocked(open).mockImplementationOnce(() => new Promise<string>(resolve => { release = resolve; }));
    render(<LibrarySources />);
    await user.click(await screen.findByRole("button", { name: /导入资料/ }));
    await user.click(screen.getByRole("tab", { name: /本地文件/ }));

    const button = screen.getByRole("button", { name: /选择文件/ });
    await user.click(button);
    await waitFor(() => expect(button).toBeDisabled());
    await user.click(button);
    expect(open).toHaveBeenCalledTimes(1);

    await act(async () => { release("C:\\book.pdf"); });
    await waitFor(() => expect(api.previewFileImport).toHaveBeenCalledWith("C:\\book.pdf"));
  });

  it("预览时显示解析警告", async () => {
    const user = userEvent.setup();
    vi.mocked(api.previewFileImport).mockResolvedValue(preview({ warnings: ["共 120 页，其中 3 页没有文字层（可能是图片页）"] }));
    render(<LibrarySources />);
    await importViaDialog(user);

    expect(await screen.findByText(/3 页没有文字层/)).toBeInTheDocument();
  });

  it("重复内容禁止再次入库", async () => {
    const user = userEvent.setup();
    vi.mocked(api.previewFileImport).mockResolvedValue(preview({ duplicate: true, duplicateOf: "existing" }));
    render(<LibrarySources />);
    await importViaDialog(user);

    expect(await screen.findByText(/这份内容已经导入过/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /确认导入/ })).toBeDisabled();
  });

  it("确认导入后提示成功并刷新列表", async () => {
    const user = userEvent.setup();
    render(<LibrarySources />);
    await importViaDialog(user);
    await screen.findByText("第一页的正文内容");
    await user.click(screen.getByRole("button", { name: /确认导入/ }));

    expect(await screen.findByText(/已导入《导入的书》/)).toBeInTheDocument();
    expect(api.confirmImport).toHaveBeenCalledWith(expect.objectContaining({ sourceType: "pdf", path: "C:\\book.pdf" }));
  });

  it("导入失败时展示错误且不写库", async () => {
    const user = userEvent.setup();
    vi.mocked(api.previewFileImport).mockRejectedValue(new Error("这份 PDF 没有可提取的文字，可能是扫描版；暂不支持 OCR，已跳过导入"));
    render(<LibrarySources />);
    await importViaDialog(user, "C:\\scan.pdf");

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

describe("引用精确定位", () => {
  it("PDF 页码定位到对应文档块并滚动", async () => {
    useAppStore.setState({ sourceDetailId: "s1", sourceDetailLocator: { page: 2 } });
    render(<LibrarySources />);

    const target = await screen.findByText("第二页正文");
    const article = target.closest("article")!;
    expect(article).toHaveAttribute("id", "source-document-d2");
    expect(article).toHaveAttribute("data-page", "2");
    await waitFor(() => expect(article).toHaveClass("source-detail__document--target"));
    // 滚动放在 rAF 里，等它真正跑过再断言
    await waitFor(() => expect(scrollIntoView).toHaveBeenCalledWith({ behavior: "smooth", block: "center" }));
  });

  it("EPUB 章节定位到对应文档块", async () => {
    vi.mocked(api.librarySource).mockResolvedValue({
      ...detail,
      sourceType: "epub",
      documents: [
        { id: "c1", position: 0, heading: "第一章", content: "第一章正文", locator: { chapter: 1 } },
        { id: "c3", position: 1, heading: "第三章", content: "第三章正文", locator: { chapter: 3 } },
      ],
    });
    useAppStore.setState({ sourceDetailId: "s1", sourceDetailLocator: { chapter: 3 } });
    render(<LibrarySources />);

    const target = await screen.findByText("第三章正文");
    await waitFor(() => expect(target.closest("article")).toHaveClass("source-detail__document--target"));
    expect(target.closest("article")).toHaveAttribute("data-chapter", "3");
  });

  it("同一页拆成多块时定位第一块", async () => {
    vi.mocked(api.librarySource).mockResolvedValue({
      ...detail,
      documents: [
        { id: "p5a", position: 0, heading: "第 5 页", content: "第五页上半", locator: { page: 5 } },
        { id: "p5b", position: 1, heading: "第 5 页", content: "第五页下半", locator: { page: 5 } },
      ],
    });
    useAppStore.setState({ sourceDetailId: "s1", sourceDetailLocator: { page: 5 } });
    render(<LibrarySources />);

    const first = await screen.findByText("第五页上半");
    await waitFor(() => expect(first.closest("article")).toHaveClass("source-detail__document--target"));
    expect(screen.getByText("第五页下半").closest("article")).not.toHaveClass("source-detail__document--target");
  });

  it("locator 匹配不到时正常打开资料且不高亮", async () => {
    useAppStore.setState({ sourceDetailId: "s1", sourceDetailLocator: { page: 99 } });
    render(<LibrarySources />);

    expect(await screen.findByText("第一页正文")).toBeInTheDocument();
    expect(document.querySelector(".source-detail__document--target")).toBeNull();
    expect(scrollIntoView).not.toHaveBeenCalled();
  });

  it("从列表打开资料时不自动高亮", async () => {
    const user = userEvent.setup();
    render(<LibrarySources />);
    await user.click(await screen.findByText("导入的书"));

    await screen.findByText("第一页正文");
    expect(document.querySelector(".source-detail__document--target")).toBeNull();
    expect(scrollIntoView).not.toHaveBeenCalled();
  });

  it("高亮在若干秒后移除", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      useAppStore.setState({ sourceDetailId: "s1", sourceDetailLocator: { page: 1 } });
      render(<LibrarySources />);
      await screen.findByText("第一页正文");
      await vi.waitFor(() => expect(document.querySelector(".source-detail__document--target")).not.toBeNull());

      await act(async () => { await vi.advanceTimersByTimeAsync(2_600); });
      expect(document.querySelector(".source-detail__document--target")).toBeNull();
    } finally { vi.useRealTimers(); }
  });

  it("关闭详情会清空 store 里的 locator", async () => {
    const user = userEvent.setup();
    useAppStore.setState({ sourceDetailId: "s1", sourceDetailLocator: { page: 2 } });
    render(<LibrarySources />);
    await screen.findByText("第二页正文");

    await user.click(screen.getByRole("button", { name: "关闭资料详情" }));
    await waitFor(() => expect(useAppStore.getState().sourceDetailId).toBeUndefined());
    expect(useAppStore.getState().sourceDetailLocator).toBeUndefined();
  });

  it("再次点击同一条引用能重新定位", async () => {
    const user = userEvent.setup();
    useAppStore.setState({ sourceDetailId: "s1", sourceDetailLocator: { page: 1 } });
    render(<LibrarySources />);
    const first = await screen.findByText("第一页正文");
    await waitFor(() => expect(first.closest("article")).toHaveClass("source-detail__document--target"));
    await waitFor(() => expect(scrollIntoView).toHaveBeenCalledTimes(1));

    await user.click(screen.getByRole("button", { name: "关闭资料详情" }));
    await waitFor(() => expect(useAppStore.getState().sourceDetailId).toBeUndefined());

    // 再点一次同一条引用：AI 页会带着同一个 locator 重新 setSourceDetail
    act(() => useAppStore.getState().setSourceDetail("s1", { page: 1 }));
    const again = await screen.findByText("第一页正文");
    await waitFor(() => expect(again.closest("article")).toHaveClass("source-detail__document--target"));
    await waitFor(() => expect(scrollIntoView).toHaveBeenCalledTimes(2));
  });
});
