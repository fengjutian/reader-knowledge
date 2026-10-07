import Graph from "graphology";
import Sigma from "sigma";
import EdgeCurveProgram from "@sigma/edge-curve";
import { useEffect, useRef } from "react";
import type { ConceptRelationKind, EntityKind, KnowledgeEntity, KnowledgeRelation } from "../../types/domain";

/** 概念图的节点坐标由 Sigma 布局给出，这里不预设位置。 */
const kindColors: Record<EntityKind, string> = { concept: "#6d5dfc", topic: "#3f8790", idea: "#a06b3b" };
const relationColors: Record<ConceptRelationKind, string> = {
  broader: "#8b6aa8", narrower: "#8b6aa8", related: "#71808d",
  supports: "#4f8a64", conflicts: "#b74335", causes: "#4f78a8", applies: "#b1784f",
};

export function ConceptGraphCanvas({ entities, relations, selectedId, selectedRelationId, fitRequest, onNodeClick, onNodeOpen, onEdgeClick }: {
  entities: KnowledgeEntity[]; relations: KnowledgeRelation[];
  selectedId?: string; selectedRelationId?: string; fitRequest: number;
  onNodeClick: (id: string) => void; onNodeOpen: (id: string) => void; onEdgeClick: (id: string) => void;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const rendererRef = useRef<Sigma | null>(null);
  const handlers = useRef({ onNodeClick, onNodeOpen, onEdgeClick });
  handlers.current = { onNodeClick, onNodeOpen, onEdgeClick };

  useEffect(() => {
    const container = containerRef.current;
    if (!container || !entities.length) return;
    const styles = getComputedStyle(container);
    const text = styles.getPropertyValue("--text").trim() || "#29251f";
    const muted = styles.getPropertyValue("--muted").trim() || "#9c9284";
    const selectedEdgeColor = "#d65f48";
    const graph = new Graph({ multi: true });
    const relationById = new Map(relations.map(item => [item.id, item]));

    entities.forEach(entity => {
      const color = kindColors[entity.kind] ?? muted;
      graph.addNode(entity.id, {
        label: entity.canonicalName,
        // 证据多的概念画得更大，让「主干概念」自然突出
        size: entity.id === selectedId ? 17 : 7 + Math.min(8, Math.log2(entity.evidenceCount + 1) * 3),
        color: entity.id === selectedId ? text : color,
        baseColor: color,
        selectedColor: text,
        forceLabel: entity.id === selectedId,
        zIndex: entity.id === selectedId ? 3 : 1,
      });
    });

    relations.forEach((relation, index) => {
      if (!graph.hasNode(relation.fromEntityId) || !graph.hasNode(relation.toEntityId)) return;
      const size = 1.5 + Math.min(3.5, relation.confidence * 6);
      const baseColor = relationColors[relation.relation] ?? muted;
      const selected = relation.id === selectedRelationId;
      graph.addUndirectedEdgeWithKey(relation.id, relation.fromEntityId, relation.toEntityId, {
        type: "curved",
        curvature: (index % 2 === 0 ? 1 : -1) * (.12 + (index % 3) * .04),
        size: selected ? Math.max(6, size + 2.5) : size,
        baseSize: size,
        baseColor,
        color: selected ? selectedEdgeColor : baseColor,
        zIndex: selected ? 8 : 0,
      });
    });

    const renderer = new Sigma(graph, container, {
      allowInvalidContainer: true,
      enableEdgeEvents: true,
      renderEdgeLabels: false,
      edgeProgramClasses: { curved: EdgeCurveProgram },
      labelRenderedSizeThreshold: 9,
      labelDensity: 1.5,
      labelGridCellSize: 80,
      labelFont: '"Source Han Serif SC", "Songti SC", serif',
      labelColor: { color: text },
      defaultEdgeColor: muted,
      defaultNodeColor: muted,
      zIndex: true,
      minCameraRatio: .35,
      maxCameraRatio: 3,
    });
    rendererRef.current = renderer;

    const tooltip = document.createElement("div");
    tooltip.className = "graph-edge-tooltip";
    container.appendChild(tooltip);
    const nodeTooltip = document.createElement("div");
    nodeTooltip.className = "graph-node-tooltip";
    container.appendChild(nodeTooltip);

    renderer.on("clickNode", ({ node }) => handlers.current.onNodeClick(node));
    // 双击节点 = 展开邻居
    renderer.on("doubleClickNode", ({ node, event }) => { event.preventSigmaDefault(); handlers.current.onNodeOpen(node); });
    renderer.on("enterNode", ({ node }) => {
      const item = entities.find(value => value.id === node);
      const attributes = graph.getNodeAttributes(node);
      const point = renderer.graphToViewport({ x: attributes.x, y: attributes.y });
      nodeTooltip.textContent = item ? `${item.canonicalName}\n${item.description || "（无描述）"} · ${item.evidenceCount} 条证据` : "";
      nodeTooltip.style.transform = `translate(${point.x}px, ${point.y}px)`;
      nodeTooltip.classList.add("visible");
    });
    renderer.on("leaveNode", () => nodeTooltip.classList.remove("visible"));
    renderer.on("clickEdge", ({ edge }) => handlers.current.onEdgeClick(edge));
    renderer.on("enterEdge", ({ edge }) => {
      container.classList.add("is-edge-hovered");
      graph.mergeEdgeAttributes(edge, { color: selectedEdgeColor, size: Math.max(6, graph.getEdgeAttribute(edge, "baseSize") + 3), zIndex: 10 });
      const [source, target] = graph.extremities(edge);
      const left = graph.getNodeAttributes(source);
      const right = graph.getNodeAttributes(target);
      const curvature = Number(graph.getEdgeAttribute(edge, "curvature")) || 0;
      const dx = right.x - left.x;
      const dy = right.y - left.y;
      const point = renderer.graphToViewport({ x: (left.x + right.x) / 2 - dy * curvature * .5, y: (left.y + right.y) / 2 + dx * curvature * .5 });
      const relation = relationById.get(edge);
      tooltip.textContent = relation ? `${relation.summary || relation.relation} · ${Math.round(relation.confidence * 100)}%` : "点击查看关系";
      tooltip.style.transform = `translate(${point.x}px, ${point.y}px)`;
      tooltip.classList.add("visible");
    });
    renderer.on("leaveEdge", ({ edge }) => {
      container.classList.remove("is-edge-hovered");
      tooltip.classList.remove("visible");
      const selected = edge === selectedRelationId;
      graph.mergeEdgeAttributes(edge, {
        color: selected ? selectedEdgeColor : graph.getEdgeAttribute(edge, "baseColor"),
        size: selected ? Math.max(6, graph.getEdgeAttribute(edge, "baseSize") + 2.5) : graph.getEdgeAttribute(edge, "baseSize"),
        zIndex: selected ? 8 : 0,
      });
    });
    return () => { rendererRef.current = null; tooltip.remove(); nodeTooltip.remove(); renderer.kill(); };
    // selectedId / selectedRelationId 刻意不参与：选中态只改颜色，走下面的增量 effect，
    // 不应该为了改个颜色重建整张图。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [entities, relations]);

  useEffect(() => {
    const renderer = rendererRef.current;
    if (!renderer) return;
    const graph = renderer.getGraph();
    graph.forEachNode(node => graph.mergeNodeAttributes(node, { color: node === selectedId ? graph.getNodeAttribute(node, "selectedColor") : graph.getNodeAttribute(node, "baseColor"), forceLabel: node === selectedId }));
    graph.forEachEdge(edge => {
      const selected = edge === selectedRelationId;
      graph.mergeEdgeAttributes(edge, {
        color: selected ? "#d65f48" : graph.getEdgeAttribute(edge, "baseColor"),
        size: selected ? Math.max(6, graph.getEdgeAttribute(edge, "baseSize") + 2.5) : graph.getEdgeAttribute(edge, "baseSize"),
        zIndex: selected ? 8 : 0,
      });
    });
    renderer.refresh();
  }, [selectedId, selectedRelationId]);
  useEffect(() => { if (fitRequest > 0) void rendererRef.current?.getCamera().animatedReset({ duration: 300 }); }, [fitRequest]);

  return <div ref={containerRef} className="sigma-graph" aria-label="可缩放的概念网络图谱"/>;
}
