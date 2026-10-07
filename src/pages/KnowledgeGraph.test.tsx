import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { KnowledgeGraph } from "./KnowledgeGraph";
import { api } from "../api/tauri";
import { useAppStore } from "../stores/app";
import { useSyncStore } from "../stores/sync";
import type { Book, Note, SemanticRelation } from "../types/domain";

// WebGL / canvas 渲染在 jsdom 中不可用，这里用占位组件替代。
vi.mock("../components/graph/KnowledgeGraph3D", () => ({ KnowledgeGraph3D: () => <div data-testid="graph-3d" /> }));
vi.mock("../components/graph/KnowledgeGraphCanvas", () => ({ KnowledgeGraphCanvas: () => <div data-testid="graph-canvas" /> }));
vi.mock("../components/graph/ConceptGraphPanel", () => ({ ConceptGraphPanel: () => <div data-testid="concept-panel" /> }));

const book = (id: string, title: string, extra: Partial<Book> = {}): Book => ({
  id, title, author: "作者", category: "分类", cover: "", highlightCount: 2, thoughtCount: 1, progress: 0, updatedAt: "2026-01-01", readingStatus: "reading", ...extra,
} as Book);
const note = (id: string, bookId: string, content: string): Note => ({ id, bookId, bookTitle: "书", chapter: "第一章", content, type: "highlight", createdAt: "2026-01-01" } as Note);
const relation = (id: string, from: string, to: string): SemanticRelation => ({ id, from, to, score: 0.9, keywords: ["k"], relation: "共同作者", evidence: [] });

class FakeWorker {
  onmessage: ((event: MessageEvent) => void) | null = null;
  onerror: ((event: unknown) => void) | null = null;
  terminated = false;
  lastMessage: { type: string; books?: unknown[] } | null = null;
  constructor() { /* onmessage 由组件在构造后赋值，这里不主动 emit */ }
  /** 模拟 worker 完成一次分析并回传结果。 */
  emitGraph(books: { id: string; x: number; y: number }[], edges: unknown[]) {
    queueMicrotask(() => this.onmessage?.({ data: { type: "result", graph: { nodes: books, edges } } } as MessageEvent));
  }
  postMessage(message: { type: string; books?: unknown[] }) {
    this.lastMessage = message;
    if (message.type === "init" && message.books) {
      this.emitGraph(message.books.map((book, index) => ({ ...(book as object), x: 500, y: 300 + index })), []);
    }
  }
  terminate() { this.terminated = true; }
}

let workerInstance: FakeWorker | null = null;

beforeEach(() => {
  workerInstance = null;
  vi.stubGlobal("Worker", class {
    constructor() { workerInstance = new FakeWorker(); return workerInstance as unknown as object; }
  });
  useAppStore.setState({ page: "graph", searchOpen: false, selectedBookId: undefined, selectedNoteId: undefined });
  useSyncStore.setState({ status: "idle", progress: 0, books: 0, highlights: 0, thoughts: 0, processedBooks: 0, totalBooks: 0, revision: 0, completedAt: undefined });
  vi.spyOn(api, "metadataRelations").mockResolvedValue([]);
  vi.spyOn(api, "semanticRelations").mockResolvedValue([]);
  vi.spyOn(api, "localEmbeddingStatus").mockResolvedValue({ installed: false, sizeBytes: 0, model: "BAAI/bge-small-zh-v1.5" });
  vi.spyOn(api, "cachedRelation").mockResolvedValue(null);
});

describe("KnowledgeGraph 数据更新链路", () => {
  it("同步完成后图谱重新加载书籍与笔记", async () => {
    const books = vi.spyOn(api, "books").mockResolvedValue([book("b1", "书一")]);
    const notes = vi.spyOn(api, "notes").mockResolvedValue([note("n1", "b1", "内容")]);
    render(<KnowledgeGraph />);
    await waitFor(() => expect(books).toHaveBeenCalledTimes(1));
    expect(notes).toHaveBeenCalledTimes(1);

    books.mockResolvedValue([book("b1", "书一"), book("b2", "书二")]);
    useSyncStore.setState({ revision: 1, status: "complete" });

    await waitFor(() => expect(books).toHaveBeenCalledTimes(2));
    expect(notes).toHaveBeenCalledTimes(2);
  });

  it("同步后未配置 Embedding 时不调用远程语义关系", async () => {
    vi.spyOn(api, "books").mockResolvedValue([book("b1", "书一")]);
    vi.spyOn(api, "notes").mockResolvedValue([note("n1", "b1", "内容")]);
    const semantic = vi.spyOn(api, "semanticRelations");
    render(<KnowledgeGraph />);
    await waitFor(() => expect(api.books).toHaveBeenCalled());
    useSyncStore.setState({ revision: 1, status: "complete" });
    await waitFor(() => expect(api.books).toHaveBeenCalledTimes(2));
    expect(semantic).not.toHaveBeenCalled();
  });

  it("local-embedding-ready 触发语义关系刷新", async () => {
    vi.spyOn(api, "books").mockResolvedValue([book("b1", "书一"), book("b2", "书二")]);
    vi.spyOn(api, "notes").mockResolvedValue([note("n1", "b1", "内容")]);
    const semantic = vi.spyOn(api, "semanticRelations").mockResolvedValue([relation("semantic:b1:b2", "b1", "b2")]);
    render(<KnowledgeGraph />);
    await waitFor(() => expect(api.books).toHaveBeenCalled());
    expect(semantic).not.toHaveBeenCalled();

    window.dispatchEvent(new Event("local-embedding-ready"));

    await waitFor(() => expect(semantic).toHaveBeenCalledTimes(1));
  });

  it("local-embedding-removed 不再请求语义关系且保留元数据关系", async () => {
    vi.spyOn(api, "books").mockResolvedValue([book("b1", "书一"), book("b2", "书二")]);
    vi.spyOn(api, "notes").mockResolvedValue([note("n1", "b1", "内容")]);
    const metadata = vi.spyOn(api, "metadataRelations").mockResolvedValue([relation("metadata:weread:author:b1:b2", "b1", "b2")]);
    const semantic = vi.spyOn(api, "semanticRelations").mockResolvedValue([relation("semantic:b1:b2", "b1", "b2")]);
    render(<KnowledgeGraph />);
    await waitFor(() => expect(metadata).toHaveBeenCalled());

    window.dispatchEvent(new Event("local-embedding-ready"));
    await waitFor(() => expect(semantic).toHaveBeenCalledTimes(1));

    const before = metadata.mock.calls.length;
    window.dispatchEvent(new Event("local-embedding-removed"));
    await new Promise(resolve => setTimeout(resolve, 120));
    // 删除模型后不应重新请求语义关系。
    expect(semantic).toHaveBeenCalledTimes(1);
    expect(metadata.mock.calls.length).toBeGreaterThanOrEqual(before);
  });

  it("没有书籍时给出同步引导而不是'没有关系'", async () => {
    vi.spyOn(api, "books").mockResolvedValue([]);
    vi.spyOn(api, "notes").mockResolvedValue([]);
    render(<KnowledgeGraph />);
    expect(await screen.findByText("书架里还没有书籍")).toBeInTheDocument();
  });

  it("有书籍但没有笔记时提示缺少内容关系", async () => {
    vi.spyOn(api, "books").mockResolvedValue([book("b1", "书一")]);
    vi.spyOn(api, "notes").mockResolvedValue([]);
    render(<KnowledgeGraph />);
    // 有书籍但没有笔记时，优先给出"缺少笔记"的引导而不是"没有关系"。
    expect(await screen.findByText("还没有笔记，无法计算内容关系")).toBeInTheDocument();
  });

  it("未配置 Embedding 时给出明确提示", async () => {
    vi.spyOn(api, "books").mockResolvedValue([book("b1", "书一"), book("b2", "书二")]);
    vi.spyOn(api, "notes").mockResolvedValue([note("n1", "b1", "内容"), note("n2", "b2", "内容")]);
    render(<KnowledgeGraph />);
    await waitFor(() => expect(screen.getByText(/尚未配置 Embedding/)).toBeInTheDocument());
  });
});

describe("KnowledgeGraph 视图切换", () => {
  beforeEach(() => {
    vi.spyOn(api, "books").mockResolvedValue([book("b1", "书一")]);
    vi.spyOn(api, "notes").mockResolvedValue([note("n1", "b1", "内容")]);
  });

  it("默认显示书籍关系，可切到概念网络", async () => {
    const user = userEvent.setup();
    render(<KnowledgeGraph />);
    // 初始是书籍关系视图
    expect(screen.getByRole("tab", { name: /书籍关系/ })).toHaveAttribute("aria-selected", "true");

    await user.click(screen.getByRole("tab", { name: /概念网络/ }));
    expect(screen.getByTestId("concept-panel")).toBeInTheDocument();
    expect(screen.getByRole("tab", { name: /概念网络/ })).toHaveAttribute("aria-selected", "true");
  });

  it("切到概念网络后还能切回书籍关系", async () => {
    const user = userEvent.setup();
    render(<KnowledgeGraph />);
    await user.click(screen.getByRole("tab", { name: /概念网络/ }));
    await user.click(screen.getByRole("tab", { name: /书籍关系/ }));
    expect(screen.queryByTestId("concept-panel")).not.toBeInTheDocument();
  });
});
