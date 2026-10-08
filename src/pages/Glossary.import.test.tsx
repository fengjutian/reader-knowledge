import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { Glossary } from "./Glossary";
import { api } from "../api/tauri";
import type { GlossaryImportJob, GlossaryTerm } from "../types/domain";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => undefined) }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(), emit: vi.fn() }));

const wikipediaTerm = (overrides: Partial<GlossaryTerm> = {}): GlossaryTerm => ({
  id: 1,
  term: "人工智能",
  canonicalName: "人工智能",
  aliases: ["AI"],
  definition: "人工智能是模拟人类智能的科学与技术。",
  source: "wikipedia",
  sourceTitle: "人工智能",
  sourceUrl: "https://zh.wikipedia.org/wiki/%E4%BA%BA%E5%B7%A5%E6%99%BA%E8%83%BD",
  wikipediaSnapshot: "人工智能是模拟人类智能的科学与技术。",
  status: "pending",
  updatedAt: 1_700_000_000,
  externalPageId: 1001,
  sourceRevisionId: 2001,
  sourceDumpVersion: "2026-09-01",
  sourceUpdatedAt: 1_700_000_000,
  sourceSyncedAt: 1_700_000_100,
  licenseCode: "CC BY-SA 4.0",
  manuallyEdited: false,
  sourceContentHash: "abc",
  publishedBatchId: "job-1",
  ...overrides,
});

const job = (overrides: Partial<GlossaryImportJob> = {}): GlossaryImportJob => ({
  id: "job-1",
  sourceType: "wikipedia",
  dumpVersion: "2026-09-01",
  sourceUrl: "https://dumps.wikimedia.org/zhwiki/latest/zhwiki-latest-pages-articles-multistream.xml.bz2",
  mode: "summary",
  status: "ready_to_publish",
  totalBytes: 1024,
  downloadedBytes: 1024,
  scannedCount: 1000,
  acceptedCount: 800,
  redirectCount: 120,
  filteredCount: 80,
  insertedCount: 0,
  updatedCount: 0,
  skippedCount: 0,
  conflictCount: 0,
  errorCount: 0,
  currentFile: "",
  bytesPerSecond: 0,
  errorMessage: "",
  autoPublish: false,
  startedAt: 1_700_000_000,
  finishedAt: 0,
  createdAt: 1_700_000_000,
  updatedAt: 1_700_000_000,
  ...overrides,
});

let terms: GlossaryTerm[] = [];
let jobs: GlossaryImportJob[] = [];
let listSpy: ReturnType<typeof vi.spyOn>;
let importsSpy: ReturnType<typeof vi.spyOn>;

beforeEach(() => {
  terms = [];
  jobs = [];
  listSpy = vi.spyOn(api, "glossaryTerms").mockImplementation(async () => [...terms]);
  importsSpy = vi.spyOn(api, "glossaryImports").mockImplementation(async () => [...jobs]);
  vi.spyOn(api, "setGlossaryTermStatus").mockImplementation(async () => undefined);
  vi.spyOn(api, "bulkGlossaryTermStatus").mockImplementation(async () => 0);
  vi.spyOn(api, "createGlossaryImport").mockImplementation(async () => job({ status: "pending" }));
  vi.spyOn(api, "publishGlossaryImport").mockImplementation(async () => ({ inserted: 1, updated: 0, skipped: 0, conflicts: 0, aliasesInserted: 0, sourceMissing: 0 }));
  vi.spyOn(api, "openExternalUrl").mockImplementation(async () => undefined);
});

describe("名词库的维基导入与来源展示", () => {
  it("把来源与状态筛选传给后端", async () => {
    render(<Glossary />);
    await screen.findByText("暂无名词");
    await userEvent.selectOptions(screen.getByLabelText("按来源筛选"), "wikipedia");
    await waitFor(() => expect(listSpy).toHaveBeenCalledWith("", "wikipedia", ""));
    await userEvent.selectOptions(screen.getByLabelText("按状态筛选"), "pending");
    await waitFor(() => expect(listSpy).toHaveBeenCalledWith("", "wikipedia", "pending"));
  });

  it("待确认条目可以直接确认", async () => {
    terms = [wikipediaTerm()];
    const spy = vi.spyOn(api, "setGlossaryTermStatus").mockImplementation(async () => undefined);
    render(<Glossary />);
    await userEvent.click(await screen.findByTitle("确认"));
    await waitFor(() => expect(spy).toHaveBeenCalledWith(1, "confirmed"));
  });

  it("可以一键确认当前筛选出的待确认条目", async () => {
    terms = [wikipediaTerm(), wikipediaTerm({ id: 2, term: "机器学习", canonicalName: "机器学习" })];
    const spy = vi.spyOn(api, "bulkGlossaryTermStatus").mockImplementation(async () => 2);
    render(<Glossary />);
    await userEvent.click(await screen.findByText("确认当前 2 条待确认"));
    await waitFor(() => expect(spy).toHaveBeenCalledWith([1, 2], "confirmed"));
  });

  it("详情页展示来源链接与 CC BY-SA 4.0", async () => {
    terms = [wikipediaTerm()];
    render(<Glossary />);
    await userEvent.click(await screen.findByText("查看"));
    const drawer = await screen.findByRole("dialog");
    // 正文段落和许可证字段都会出现 CC BY-SA 4.0，用 getAllByText 断言两处都在
    expect(within(drawer).getAllByText(/CC BY-SA 4\.0/).length).toBeGreaterThanOrEqual(2);
    // 来源可追溯信息：page id / revision / dump 版本
    expect(within(drawer).getByText("1001")).toBeTruthy();
    expect(within(drawer).getByText("2001")).toBeTruthy();
    expect(within(drawer).getByText("2026-09-01")).toBeTruthy();
    expect(within(drawer).getByText("查看原文与贡献历史")).toBeTruthy();
  });

  it("人工编辑过的条目提示不会被同步覆盖并可对比维基快照", async () => {
    terms = [wikipediaTerm({ manuallyEdited: true, definition: "我自己改过的解释", wikipediaSnapshot: "维基原始摘要内容。" })];
    render(<Glossary />);
    await userEvent.click(await screen.findByText("查看"));
    const drawer = await screen.findByRole("dialog");
    expect(within(drawer).getByText(/后续维基同步只更新来源快照/)).toBeTruthy();
    await userEvent.click(within(drawer).getByText("对比最新维基摘要与当前展示内容"));
    expect(within(drawer).getByText(/维基原始摘要内容/)).toBeTruthy();
  });

  it("导入面板可以创建小样本任务", async () => {
    const spy = vi.spyOn(api, "createGlossaryImport").mockImplementation(async () => job({ status: "pending" }));
    render(<Glossary />);
    await userEvent.click(await screen.findByText("维基导入"));
    const limit = screen.getByLabelText("最多处理");
    await userEvent.clear(limit);
    await userEvent.type(limit, "1000");
    await userEvent.click(screen.getByText("开始导入"));
    await waitFor(() => expect(spy).toHaveBeenCalledWith(expect.objectContaining({ mode: "summary", maxItems: 1000 })));
  });

  it("就绪任务提供发布按钮", async () => {
    jobs = [job()];
    const spy = vi.spyOn(api, "publishGlossaryImport").mockImplementation(async () => ({ inserted: 1, updated: 0, skipped: 0, conflicts: 0, aliasesInserted: 0, sourceMissing: 0 }));
    render(<Glossary />);
    await userEvent.click(await screen.findByText("维基导入"));
    await userEvent.click(await screen.findByText("发布本批"));
    await waitFor(() => expect(spy).toHaveBeenCalledWith("job-1"));
  });

  it("运行中的任务才轮询刷新", async () => {
    jobs = [job({ status: "parsing" })];
    render(<Glossary />);
    await screen.findByText("维基导入");
    await userEvent.click(screen.getByText("维基导入"));
    await screen.findByText("解析中");
    const before = importsSpy.mock.calls.length;
    await new Promise(resolve => setTimeout(resolve, 1800));
    expect(importsSpy.mock.calls.length).toBeGreaterThan(before);
  });

  it("失败任务展示错误信息", async () => {
    jobs = [job({ status: "failed", errorMessage: "MD5 校验失败：期望 abc，实际 def" })];
    render(<Glossary />);
    await userEvent.click(await screen.findByText("维基导入"));
    expect(await screen.findByText(/MD5 校验失败/)).toBeTruthy();
  });
});
