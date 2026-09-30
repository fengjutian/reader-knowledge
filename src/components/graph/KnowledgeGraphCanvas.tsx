import Graph from "graphology";
import Sigma from "sigma";
import EdgeCurveProgram from "@sigma/edge-curve";
import { useEffect, useRef } from "react";
import type { Book } from "../../types/domain";

type GraphNode = Book & { x: number; y: number };
type GraphEdge = { id: string; from: string; to: string; score: number; relation: string; semanticRelation?: string };
const semanticColors: Record<string, string> = { agreement: "#4f8a64", conflict: "#b74335", complementary: "#4f78a8", causal: "#8b6aa8", application: "#3f8790", same_concept: "#a06b3b", uncertain: "#a39b90" };

export function KnowledgeGraphCanvas({ nodes, edges, focusId, selectedId, selectedEdgeId, fitRequest, onNodeClick, onNodeOpen, onEdgeClick }: {
  nodes: GraphNode[]; edges: GraphEdge[]; focusId?: string; selectedId?: string; selectedEdgeId?: string;
  fitRequest: number;
  onNodeClick: (id: string) => void; onNodeOpen: (id: string) => void; onEdgeClick: (id: string) => void;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const rendererRef = useRef<Sigma | null>(null);
  const handlers = useRef({ onNodeClick, onNodeOpen, onEdgeClick });
  handlers.current = { onNodeClick, onNodeOpen, onEdgeClick };
  useEffect(() => {
    const container = containerRef.current; if (!container || !nodes.length) return;
    const styles = getComputedStyle(container), accent = styles.getPropertyValue("--accent").trim() || "#9d3f32", muted = styles.getPropertyValue("--muted").trim() || "#9c9284", text = styles.getPropertyValue("--text").trim() || "#29251f", selectedEdgeColor = "#d65f48";
    const graph = new Graph({ multi: true }), edgeById = new Map(edges.map(edge => [edge.id, edge]));
    nodes.forEach(node => graph.addNode(node.id, { x: node.x, y: node.y, label: node.title, size: node.id === focusId ? 18 : 11 + Math.min(5, Math.log2(node.highlightCount + node.thoughtCount + 1)), color: node.id === focusId ? accent : node.id === selectedId ? text : "#b85a4b", forceLabel: node.id === focusId || node.id === selectedId, zIndex: node.id === focusId ? 3 : 1 }));
    edges.forEach((edge, index) => { if (graph.hasNode(edge.from) && graph.hasNode(edge.to)) { const size = 2.25 + Math.min(3.5, edge.score * 8), baseColor = edge.semanticRelation ? semanticColors[edge.semanticRelation] ?? muted : edge.relation.includes("作者") ? "#71808d" : edge.score >= .28 ? accent : edge.score >= .12 ? "#b1784f" : muted, curvature = (index % 2 === 0 ? 1 : -1) * (.14 + index % 3 * .035), selected = edge.id === selectedEdgeId; graph.addUndirectedEdgeWithKey(edge.id, edge.from, edge.to, { type: "curved", curvature, size: selected ? Math.max(6, size + 2.5) : size, baseSize: size, baseColor, color: selected ? selectedEdgeColor : baseColor, zIndex: selected ? 8 : 0 }); } });
    const renderer = new Sigma(graph, container, { allowInvalidContainer: true, enableEdgeEvents: true, renderEdgeLabels: false, edgeProgramClasses: { curved: EdgeCurveProgram }, labelRenderedSizeThreshold: 8, labelDensity: 1.5, labelGridCellSize: 80, labelFont: '"Source Han Serif SC", "Songti SC", serif', labelColor: { color: text }, defaultEdgeColor: muted, defaultNodeColor: accent, zIndex: true, minCameraRatio: .35, maxCameraRatio: 3 });
    rendererRef.current = renderer;
    const tooltip = document.createElement("div"); tooltip.className = "graph-edge-tooltip"; container.appendChild(tooltip);
    const nodeTooltip = document.createElement("div"); nodeTooltip.className = "graph-node-tooltip"; container.appendChild(nodeTooltip);
    renderer.on("clickNode", ({ node }) => handlers.current.onNodeClick(node));
    renderer.on("doubleClickNode", ({ node, event }) => { event.preventSigmaDefault(); handlers.current.onNodeOpen(node); });
    renderer.on("enterNode", ({ node }) => { const item = nodes.find(value => value.id === node), relation = edges.find(edge => edge.from === node || edge.to === node), attributes = graph.getNodeAttributes(node), point = renderer.graphToViewport({ x: attributes.x, y: attributes.y }); nodeTooltip.textContent = `${item?.title ?? ""}\n${item?.author || "未知作者"} · ${(item?.highlightCount ?? 0) + (item?.thoughtCount ?? 0)} 条笔记${relation ? `\n${relation.semanticRelation ?? relation.relation} · ${Math.round(relation.score * 100)}%` : ""}`; nodeTooltip.style.transform = `translate(${point.x}px, ${point.y}px)`; nodeTooltip.classList.add("visible"); });
    renderer.on("leaveNode", () => nodeTooltip.classList.remove("visible"));
    renderer.on("clickEdge", ({ edge }) => handlers.current.onEdgeClick(edge));
    renderer.on("enterEdge", ({ edge }) => { container.classList.add("is-edge-hovered"); graph.mergeEdgeAttributes(edge, { color: selectedEdgeColor, size: Math.max(6, graph.getEdgeAttribute(edge, "baseSize") + 3), zIndex: 10 }); const [source, target] = graph.extremities(edge), left = graph.getNodeAttributes(source), right = graph.getNodeAttributes(target), curvature = Number(graph.getEdgeAttribute(edge, "curvature")) || 0, dx = right.x - left.x, dy = right.y - left.y, point = renderer.graphToViewport({ x: (left.x + right.x) / 2 - dy * curvature * .5, y: (left.y + right.y) / 2 + dx * curvature * .5 }), item = edgeById.get(edge); tooltip.textContent = item ? `点击查看：${item.relation} · ${Math.round(item.score * 100)}%` : "点击查看关系"; tooltip.style.transform = `translate(${point.x}px, ${point.y}px)`; tooltip.classList.add("visible"); });
    renderer.on("leaveEdge", ({ edge }) => { container.classList.remove("is-edge-hovered"); tooltip.classList.remove("visible"); const selected = edge === selectedEdgeId; graph.mergeEdgeAttributes(edge, { color: selected ? selectedEdgeColor : graph.getEdgeAttribute(edge, "baseColor"), size: selected ? Math.max(6, graph.getEdgeAttribute(edge, "baseSize") + 2.5) : graph.getEdgeAttribute(edge, "baseSize"), zIndex: selected ? 8 : 0 }); });
    return () => { rendererRef.current = null; tooltip.remove(); nodeTooltip.remove(); renderer.kill(); };
  }, [nodes, edges, focusId, selectedId, selectedEdgeId]);
  useEffect(() => { if (fitRequest > 0) void rendererRef.current?.getCamera().animatedReset({ duration: 300 }); }, [fitRequest]);
  return <div ref={containerRef} className="sigma-graph" aria-label="可缩放的书籍关系图谱"/>;
}
