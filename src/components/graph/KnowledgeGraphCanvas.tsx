import Graph from "graphology";
import Sigma from "sigma";
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
    const styles = getComputedStyle(container), accent = styles.getPropertyValue("--accent").trim() || "#9d3f32", muted = styles.getPropertyValue("--muted").trim() || "#9c9284", text = styles.getPropertyValue("--text").trim() || "#29251f";
    const graph = new Graph(), edgeById = new Map(edges.map(edge => [edge.id, edge]));
    nodes.forEach(node => graph.addNode(node.id, { x: node.x, y: node.y, label: node.title, size: node.id === focusId ? 18 : 11 + Math.min(5, Math.log2(node.highlightCount + node.thoughtCount + 1)), color: node.id === focusId ? accent : node.id === selectedId ? text : "#b85a4b", forceLabel: node.id === focusId || node.id === selectedId, zIndex: node.id === focusId ? 3 : 1 }));
    edges.forEach(edge => { if (graph.hasNode(edge.from) && graph.hasNode(edge.to)) { const size = 1.5 + Math.min(4, edge.score * 10), baseColor = edge.semanticRelation ? semanticColors[edge.semanticRelation] ?? muted : edge.relation === "同一作者" ? "#71808d" : edge.score >= .28 ? accent : edge.score >= .12 ? "#b1784f" : muted; graph.addUndirectedEdgeWithKey(edge.id, edge.from, edge.to, { size, baseSize: size, baseColor, color: edge.id === selectedEdgeId ? accent : baseColor, zIndex: edge.id === selectedEdgeId ? 2 : 0 }); } });
    const renderer = new Sigma(graph, container, { allowInvalidContainer: true, enableEdgeEvents: true, renderEdgeLabels: false, labelRenderedSizeThreshold: 8, labelDensity: 1.5, labelGridCellSize: 80, labelFont: '"Source Han Serif SC", "Songti SC", serif', labelColor: { color: text }, defaultEdgeColor: muted, defaultNodeColor: accent, zIndex: true, minCameraRatio: .35, maxCameraRatio: 3 });
    rendererRef.current = renderer;
    const tooltip = document.createElement("div"); tooltip.className = "graph-edge-tooltip"; container.appendChild(tooltip);
    const nodeTooltip = document.createElement("div"); nodeTooltip.className = "graph-node-tooltip"; container.appendChild(nodeTooltip);
    renderer.on("clickNode", ({ node }) => handlers.current.onNodeClick(node));
    renderer.on("doubleClickNode", ({ node, event }) => { event.preventSigmaDefault(); handlers.current.onNodeOpen(node); });
    renderer.on("enterNode", ({ node }) => { const item = nodes.find(value => value.id === node), relation = edges.find(edge => edge.from === node || edge.to === node), point = renderer.graphToViewport(graph.getNodeAttributes(node)); nodeTooltip.textContent = `${item?.title ?? ""}\n${item?.author || "未知作者"} · ${(item?.highlightCount ?? 0) + (item?.thoughtCount ?? 0)} 条笔记${relation ? `\n${relation.semanticRelation ?? relation.relation} · ${Math.round(relation.score * 100)}%` : ""}`; nodeTooltip.style.transform = `translate(${point.x}px, ${point.y}px)`; nodeTooltip.classList.add("visible"); });
    renderer.on("leaveNode", () => nodeTooltip.classList.remove("visible"));
    renderer.on("clickEdge", ({ edge }) => handlers.current.onEdgeClick(edge));
    renderer.on("enterEdge", ({ edge }) => { container.classList.add("is-edge-hovered"); graph.mergeEdgeAttributes(edge, { color: accent, size: Math.max(4, graph.getEdgeAttribute(edge, "baseSize") + 2), zIndex: 4 }); const [source, target] = graph.extremities(edge), left = graph.getNodeAttributes(source), right = graph.getNodeAttributes(target), point = renderer.graphToViewport({ x: (left.x + right.x) / 2, y: (left.y + right.y) / 2 }), item = edgeById.get(edge); tooltip.textContent = item ? `${item.relation} · ${Math.round(item.score * 100)}%` : "查看关系"; tooltip.style.transform = `translate(${point.x}px, ${point.y}px)`; tooltip.classList.add("visible"); });
    renderer.on("leaveEdge", ({ edge }) => { container.classList.remove("is-edge-hovered"); tooltip.classList.remove("visible"); graph.mergeEdgeAttributes(edge, { color: edge === selectedEdgeId ? accent : graph.getEdgeAttribute(edge, "baseColor"), size: graph.getEdgeAttribute(edge, "baseSize"), zIndex: edge === selectedEdgeId ? 2 : 0 }); });
    return () => { rendererRef.current = null; tooltip.remove(); nodeTooltip.remove(); renderer.kill(); };
  }, [nodes, edges, focusId, selectedId, selectedEdgeId]);
  useEffect(() => { if (fitRequest > 0) void rendererRef.current?.getCamera().animatedReset({ duration: 300 }); }, [fitRequest]);
  return <div ref={containerRef} className="sigma-graph" aria-label="可缩放的书籍关系图谱"/>;
}
