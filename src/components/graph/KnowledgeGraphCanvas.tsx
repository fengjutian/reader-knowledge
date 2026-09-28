import Graph from "graphology";
import Sigma from "sigma";
import { useEffect, useRef } from "react";
import type { Book } from "../../types/domain";

type GraphNode = Book & { x: number; y: number };
type GraphEdge = { id: string; from: string; to: string; score: number };

export function KnowledgeGraphCanvas({ nodes, edges, focusId, selectedId, selectedEdgeId, onNodeClick, onNodeOpen, onEdgeClick }: {
  nodes: GraphNode[]; edges: GraphEdge[]; focusId?: string; selectedId?: string; selectedEdgeId?: string;
  onNodeClick: (id: string) => void; onNodeOpen: (id: string) => void; onEdgeClick: (id: string) => void;
}) {
  const containerRef = useRef<HTMLDivElement>(null);
  const handlers = useRef({ onNodeClick, onNodeOpen, onEdgeClick });
  handlers.current = { onNodeClick, onNodeOpen, onEdgeClick };
  useEffect(() => {
    const container = containerRef.current; if (!container || !nodes.length) return;
    const styles = getComputedStyle(container), accent = styles.getPropertyValue("--accent").trim() || "#9d3f32", muted = styles.getPropertyValue("--muted").trim() || "#9c9284", text = styles.getPropertyValue("--text").trim() || "#29251f";
    const graph = new Graph();
    nodes.forEach(node => graph.addNode(node.id, { x: node.x, y: node.y, label: node.title, size: node.id === focusId ? 18 : 11 + Math.min(5, Math.log2(node.highlightCount + node.thoughtCount + 1)), color: node.id === focusId ? accent : node.id === selectedId ? text : "#b85a4b", forceLabel: node.id === focusId || node.id === selectedId, zIndex: node.id === focusId ? 3 : 1 }));
    edges.forEach(edge => { if (graph.hasNode(edge.from) && graph.hasNode(edge.to)) graph.addUndirectedEdgeWithKey(edge.id, edge.from, edge.to, { size: 1 + Math.min(4, edge.score * 10), color: edge.id === selectedEdgeId ? accent : muted, zIndex: edge.id === selectedEdgeId ? 2 : 0 }); });
    const renderer = new Sigma(graph, container, { allowInvalidContainer: true, enableEdgeEvents: true, renderEdgeLabels: false, labelRenderedSizeThreshold: 8, labelDensity: 1.5, labelGridCellSize: 80, labelFont: '"Source Han Serif SC", "Songti SC", serif', labelColor: { color: text }, defaultEdgeColor: muted, defaultNodeColor: accent, zIndex: true, minCameraRatio: .35, maxCameraRatio: 3 });
    renderer.on("clickNode", ({ node }) => handlers.current.onNodeClick(node));
    renderer.on("doubleClickNode", ({ node, event }) => { event.preventSigmaDefault(); handlers.current.onNodeOpen(node); });
    renderer.on("clickEdge", ({ edge }) => handlers.current.onEdgeClick(edge));
    return () => renderer.kill();
  }, [nodes, edges, focusId, selectedId, selectedEdgeId]);
  return <div ref={containerRef} className="sigma-graph" aria-label="可缩放的书籍关系图谱"/>;
}
