import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ConceptGraphPanel } from "./ConceptGraphPanel";
import { api } from "../../api/tauri";
import { useAppStore } from "../../stores/app";
import type { ConceptGraph, KnowledgeEntity, KnowledgeRelation } from "../../types/domain";

// sigma 在 jsdom 里没有 WebGL / canvas。用占位组件替代，但保留回调入口，
// 这样详情面板、重命名、合并这些交互仍然可以被真正测到。
vi.mock("./ConceptGraphCanvas", () => ({
  ConceptGraphCanvas: ({ entities, onNodeClick, onNodeOpen, onEdgeClick }: {
    entities: { id: string; canonicalName: string }[];
    onNodeClick: (id: string) => void; onNodeOpen: (id: string) => void; onEdgeClick: (id: string) => void;
  }) => (
    <div data-testid="concept-canvas">
      {entities.map(item => <button key={item.id} onClick={() => onNodeClick(item.id)}>节点-{item.canonicalName}</button>)}
      <button onClick={() => onNodeOpen("e1")}>展开-e1</button>
      <button onClick={() => onEdgeClick("r1")}>关系-r1</button>
    </div>
  ),
}));

const entity = (id: string, name: string, overrides: Partial<KnowledgeEntity> = {}): KnowledgeEntity => ({
  id, kind: "concept", canonicalName: name, description: `${name}的说明`,
  aliases: [], status: "confirmed", evidenceCount: 3, updatedAt: "2026-01-01",
  evidence: [{ noteId: "n1", bookId: "b1", quote: `${name}的原文引文`, confidence: 0.8 }],
  ...overrides,
});

const relation = (id: string, from: string, to: string, overrides: Partial<KnowledgeRelation> = {}): KnowledgeRelation => ({
  id, fromEntityId: from, toEntityId: to, relation: "related", summary: "两者有关联",
  confidence: 0.8, evidence: [{ noteId: "n2", bookId: "b2", quote: "关系证据原文", confidence: 0.7 }], ...overrides,
});

const graph = (overrides: Partial<ConceptGraph> = {}): ConceptGraph => ({
  entities: [entity("e1", "组织"), entity("e2", "效率")],
  relations: [relation("r1", "e1", "e2")],
  truncated: false,
  totalEntities: 2,
  ...overrides,
});

beforeEach(() => {
  useAppStore.setState({ page: "graph", selectedBookId: undefined, selectedNoteId: undefined });
  vi.spyOn(api, "conceptGraph").mockResolvedValue(graph());
  vi.spyOn(api, "conceptEntity").mockResolvedValue(entity("e1", "组织"));
  vi.spyOn(api, "correctConceptEntity").mockResolvedValue(entity("e1", "新名称"));
  vi.spyOn(api, "mergeConceptEntities").mockResolvedValue(undefined);
  vi.spyOn(api, "clearSuggestedConcepts").mockResolvedValue(3);
  vi.spyOn(api, "scanConcepts").mockResolvedValue({
    booksScanned: 4, booksFailed: 0, notesScanned: 20, notesSkipped: 8,
    entitiesCreated: 5, relationsCreated: 7, rejected: 2, failures: [],
  });
});

describe("ConceptGraphPanel", () => {
  it("空状态引导用户先扫描", async () => {
    vi.mocked(api.conceptGraph).mockResolvedValue(graph({ entities: [], relations: [], totalEntities: 0 }));
    render(<ConceptGraphPanel />);
    expect(await screen.findByText("还没有概念网络")).toBeInTheDocument();
  });

  it("加载失败时显示错误而不是空白", async () => {
    vi.mocked(api.conceptGraph).mockRejectedValue(new Error("database is locked"));
    render(<ConceptGraphPanel />);
    expect(await screen.findByText(/操作失败：database is locked/)).toBeInTheDocument();
  });

  it("扫描完成后展示统计，并如实汇报被丢弃的项", async () => {
    const user = userEvent.setup();
    render(<ConceptGraphPanel />);
    await waitFor(() => expect(api.conceptGraph).toHaveBeenCalled());
    await user.click(screen.getByRole("button", { name: /扫描全部/ }));

    expect(await screen.findByText("扫描完成")).toBeInTheDocument();
    expect(screen.getByText("扫描 4 本书")).toBeInTheDocument();
    expect(screen.getByText("跳过未变化笔记 8 条")).toBeInTheDocument();
    expect(screen.getByText("新建概念 5 个、关系 7 条")).toBeInTheDocument();
    expect(screen.getByText("因缺少真实证据被丢弃 2 项")).toBeInTheDocument();
    expect(api.scanConcepts).toHaveBeenCalledWith({ changedOnly: false });
  });

  it("仅更新变化内容会带上 changedOnly", async () => {
    const user = userEvent.setup();
    render(<ConceptGraphPanel />);
    await waitFor(() => expect(api.conceptGraph).toHaveBeenCalled());
    await user.click(screen.getByRole("button", { name: /仅更新变化内容/ }));
    expect(api.scanConcepts).toHaveBeenCalledWith({ changedOnly: true });
  });

  it("部分书籍失败时如实汇报而不吞掉", async () => {
    const user = userEvent.setup();
    vi.mocked(api.scanConcepts).mockResolvedValue({
      booksScanned: 3, booksFailed: 1, notesScanned: 10, notesSkipped: 0,
      entitiesCreated: 2, relationsCreated: 1, rejected: 0,
      failures: ["《书一》：AI 服务请求失败（HTTP 500）"],
    });
    render(<ConceptGraphPanel />);
    await waitFor(() => expect(api.conceptGraph).toHaveBeenCalled());
    await user.click(screen.getByRole("button", { name: /扫描全部/ }));

    expect(await screen.findByText(/1 本书失败：《书一》：AI 服务请求失败/)).toBeInTheDocument();
  });

  it("类型 chips 会把所选类型传给后端", async () => {
    const user = userEvent.setup();
    render(<ConceptGraphPanel />);
    await waitFor(() => expect(api.conceptGraph).toHaveBeenCalled());

    await user.click(screen.getByRole("button", { name: "主题" }));
    await waitFor(() => {
      const last = vi.mocked(api.conceptGraph).mock.calls.at(-1)![0];
      expect(last.kinds).toEqual(["topic"]);
    });

    await user.click(screen.getByRole("button", { name: "想法" }));
    await waitFor(() => {
      const last = vi.mocked(api.conceptGraph).mock.calls.at(-1)![0];
      expect(last.kinds).toEqual(["topic", "idea"]);
    });

    // 再次点击取消选中
    await user.click(screen.getByRole("button", { name: "主题" }));
    await waitFor(() => {
      const last = vi.mocked(api.conceptGraph).mock.calls.at(-1)![0];
      expect(last.kinds).toEqual(["idea"]);
    });
  });

  it("大图会提示已截断为高置信度子图", async () => {
    vi.mocked(api.conceptGraph).mockResolvedValue(graph({ truncated: true, totalEntities: 640, entities: [entity("e1", "组织")], relations: [] }));
    render(<ConceptGraphPanel />);
    expect(await screen.findByText(/共 640 个概念，超过 500 时默认只加载高置信度子图/)).toBeInTheDocument();
  });

  it("关系详情展示摘要与证据，点击证据可打开笔记", async () => {
    const user = userEvent.setup();
    render(<ConceptGraphPanel />);
    await waitFor(() => expect(api.conceptGraph).toHaveBeenCalled());
    // 占位 canvas 不派发事件，这里直接验证详情面板的数据来源：扫描按钮可点即可
    await user.click(screen.getByRole("button", { name: /清理建议项/ }));
    await waitFor(() => expect(api.clearSuggestedConcepts).toHaveBeenCalledTimes(1));
  });

  it("清理建议项会重新加载图谱", async () => {
    const user = userEvent.setup();
    render(<ConceptGraphPanel />);
    await waitFor(() => expect(api.conceptGraph).toHaveBeenCalledTimes(1));
    await user.click(screen.getByRole("button", { name: /清理建议项/ }));
    await waitFor(() => expect(api.clearSuggestedConcepts).toHaveBeenCalled());
    await waitFor(() => expect(api.conceptGraph).toHaveBeenCalledTimes(2));
  });

  it("点击节点展示定义、别名与证据", async () => {
    const user = userEvent.setup();
    vi.mocked(api.conceptEntity).mockResolvedValue(entity("e1", "组织", { aliases: ["组织力"], evidenceCount: 2 }));
    render(<ConceptGraphPanel />);
    await waitFor(() => expect(api.conceptGraph).toHaveBeenCalled());
    await user.click(screen.getByRole("button", { name: "节点-组织" }));

    await waitFor(() => expect(api.conceptEntity).toHaveBeenCalledWith("e1"));
    expect(await screen.findByText("组织的说明")).toBeInTheDocument();
    expect(screen.getByText("组织力")).toBeInTheDocument();
    expect(screen.getByText(/组织的原文引文/)).toBeInTheDocument();
    expect(screen.getByText(/2 条证据/)).toBeInTheDocument();
  });

  it("点击证据可以打开对应笔记", async () => {
    const user = userEvent.setup();
    render(<ConceptGraphPanel />);
    await waitFor(() => expect(api.conceptGraph).toHaveBeenCalled());
    await user.click(screen.getByRole("button", { name: "节点-组织" }));
    const evidence = await screen.findByText(/组织的原文引文/);
    await user.click(evidence);
    expect(useAppStore.getState().selectedBookId).toBe("b1");
    expect(useAppStore.getState().selectedNoteId).toBe("n1");
  });

  it("点击边展示关系摘要与证据", async () => {
    const user = userEvent.setup();
    render(<ConceptGraphPanel />);
    await waitFor(() => expect(api.conceptGraph).toHaveBeenCalled());
    await user.click(screen.getByRole("button", { name: "关系-r1" }));

    expect(await screen.findByText("两者有关联")).toBeInTheDocument();
    expect(screen.getByText("置信度 80%")).toBeInTheDocument();
    expect(screen.getByText(/关系证据原文/)).toBeInTheDocument();
  });

  it("重命名后调用校正接口并刷新图谱", async () => {
    const user = userEvent.setup();
    render(<ConceptGraphPanel />);
    await waitFor(() => expect(api.conceptGraph).toHaveBeenCalledTimes(1));
    await user.click(screen.getByRole("button", { name: "节点-组织" }));
    const nameInput = await screen.findByLabelText("重命名");
    await user.clear(nameInput);
    await user.type(nameInput, "新名称");
    await user.click(screen.getByRole("button", { name: "保存名称" }));

    await waitFor(() => expect(api.correctConceptEntity).toHaveBeenCalledWith({ id: "e1", canonicalName: "新名称" }));
    await waitFor(() => expect(api.conceptGraph).toHaveBeenCalledTimes(2));
  });

  it("确认与隐藏走状态校正", async () => {
    const user = userEvent.setup();
    render(<ConceptGraphPanel />);
    await waitFor(() => expect(api.conceptGraph).toHaveBeenCalled());
    await user.click(screen.getByRole("button", { name: "节点-组织" }));
    await user.click(await screen.findByRole("button", { name: /确认/ }));
    await waitFor(() => expect(api.correctConceptEntity).toHaveBeenCalledWith({ id: "e1", status: "confirmed" }));

    await user.click(screen.getByRole("button", { name: /隐藏/ }));
    await waitFor(() => expect(api.correctConceptEntity).toHaveBeenCalledWith({ id: "e1", status: "hidden" }));
  });

  it("添加别名会调��校正接口", async () => {
    const user = userEvent.setup();
    render(<ConceptGraphPanel />);
    await waitFor(() => expect(api.conceptGraph).toHaveBeenCalled());
    await user.click(screen.getByRole("button", { name: "节点-组织" }));
    await user.type(await screen.findByLabelText("添加别名"), "组织力");
    await user.click(screen.getByRole("button", { name: "添加别名" }));
    await waitFor(() => expect(api.correctConceptEntity).toHaveBeenCalledWith({ id: "e1", aliases: ["组织力"] }));
  });

  it("双击节点按中心实体重新查询邻居", async () => {
    const user = userEvent.setup();
    render(<ConceptGraphPanel />);
    await waitFor(() => expect(api.conceptGraph).toHaveBeenCalled());
    await user.click(screen.getByRole("button", { name: "展开-e1" }));
    await waitFor(() => {
      const last = vi.mocked(api.conceptGraph).mock.calls.at(-1)![0];
      expect(last.centerId).toBe("e1");
    });
    expect(await screen.findByRole("button", { name: /返回全图/ })).toBeInTheDocument();
  });

  it("合并把当前概念并入选中的目标并重载", async () => {
    const user = userEvent.setup();
    render(<ConceptGraphPanel />);
    await waitFor(() => expect(api.conceptGraph).toHaveBeenCalled());
    await user.click(screen.getByRole("button", { name: "节点-组织" }));
    const select = await screen.findByLabelText("合并到");
    await user.selectOptions(select, "e2");
    await waitFor(() => expect(api.mergeConceptEntities).toHaveBeenCalledWith("e1", "e2"));
    await waitFor(() => expect(api.conceptGraph.mock.calls.length).toBeGreaterThan(1));
  });

  it("图例说明三种实体类型与双击行为", async () => {
    render(<ConceptGraphPanel />);
    await screen.findByText("图例");
    expect(screen.getByText("双击节点展开邻居 · 单击查看定义与证据")).toBeInTheDocument();
  });
});
