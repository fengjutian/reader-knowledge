import ForceGraph3D, { type ForceGraphMethods, type GraphData, type LinkObject, type NodeObject } from "react-force-graph-3d";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Book } from "../../types/domain";

type GraphNode = Book & { x: number; y: number };
type GraphEdge = { id: string; from: string; to: string; score: number; relation: string; semanticRelation?: string };
type Node3D = GraphNode & { id: string; relationCount: number };
type Link3D = GraphEdge & { source: string; target: string };
const semanticColors: Record<string, string> = { agreement: "#4f8a64", conflict: "#b74335", complementary: "#4f78a8", causal: "#8b6aa8", application: "#3f8790", same_concept: "#a06b3b", uncertain: "#a39b90" };
const clusterColors = ["#a84435", "#4d7185", "#66805d", "#a8783f", "#77638c", "#397b78", "#9a5d72", "#6f7450"];

function categoryColor(node: Node3D) {
  const key = node.category || node.author || node.title;
  let hash = 0;
  for (let index = 0; index < key.length; index += 1) hash = ((hash << 5) - hash + key.charCodeAt(index)) | 0;
  return clusterColors[Math.abs(hash) % clusterColors.length];
}

function endpointId(value: unknown) {
  return typeof value === "object" && value && "id" in value ? String(value.id) : String(value);
}

export function KnowledgeGraph3D({ nodes, edges, focusId, selectedId, fitRequest, onNodeClick, onNodeOpen, onEdgeClick }: {
  nodes: GraphNode[];
  edges: GraphEdge[];
  focusId?: string;
  selectedId?: string;
  fitRequest: number;
  onNodeClick: (id: string) => void;
  onNodeOpen: (id: string) => void;
  onEdgeClick: (id: string) => void;
}) {
  const hostRef = useRef<HTMLDivElement>(null);
  const graphRef = useRef<ForceGraphMethods<Node3D, Link3D> | undefined>(undefined);
  const [size, setSize] = useState({ width: 1, height: 1 });
  const [hoveredId, setHoveredId] = useState<string>();
  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const update = () => setSize({ width: Math.max(1, host.clientWidth), height: Math.max(1, host.clientHeight) });
    update();
    const observer = new ResizeObserver(update);
    observer.observe(host);
    return () => observer.disconnect();
  }, []);
  useEffect(() => { if (fitRequest > 0) graphRef.current?.zoomToFit(500, 70); }, [fitRequest]);
  const data = useMemo<GraphData<Node3D, Link3D>>(() => {
    const degree = new Map<string, number>();
    edges.forEach(edge => { degree.set(edge.from, (degree.get(edge.from) ?? 0) + 1); degree.set(edge.to, (degree.get(edge.to) ?? 0) + 1); });
    return {
      nodes: nodes.map(node => ({ ...node, id: node.id, relationCount: degree.get(node.id) ?? 0 })),
      links: edges.map(edge => ({ ...edge, source: edge.from, target: edge.to })),
    };
  }, [nodes, edges]);
  const neighbours = useMemo(() => {
    const map = new Map<string, Set<string>>();
    edges.forEach(edge => {
      if (!map.has(edge.from)) map.set(edge.from, new Set());
      if (!map.has(edge.to)) map.set(edge.to, new Set());
      map.get(edge.from)!.add(edge.to);
      map.get(edge.to)!.add(edge.from);
    });
    return map;
  }, [edges]);
  const isHighlighted = (id: string) => !hoveredId || id === hoveredId || neighbours.get(hoveredId)?.has(id);
  return <div ref={hostRef} className="force-graph-3d">
    <ForceGraph3D<Node3D, Link3D>
      ref={graphRef}
      graphData={data}
      width={size.width}
      height={size.height}
      backgroundColor="#f7f3eb"
      showNavInfo={false}
      forceEngine="ngraph"
      ngraphPhysics={{
        timestep: 1.8,
        springLength: 100,
        springCoefficient: .0008,
        gravity: -1.4,
        dragCoefficient: .02,
      }}
      warmupTicks={80}
      cooldownTime={7000}
      nodeLabel={node => `<div class="graph-3d-tooltip"><strong>${node.title}</strong>${node.author ? `<span>${node.author}</span>` : ""}<small>${node.category || "未分类"} · ${node.relationCount} 个关系</small></div>`}
      nodeVal={node => node.id === focusId ? 5.5 : node.id === selectedId ? 4.5 : node.relationCount ? 1.15 + Math.min(2.8, Math.log2(node.relationCount + 1) * .55) : .16}
      nodeColor={node => node.id === focusId ? "#8f3025" : node.id === selectedId ? "#28231f" : !isHighlighted(node.id) ? "#d8d3ca" : node.relationCount ? categoryColor(node) : "#c9c3b9"}
      nodeOpacity={.84}
      nodeResolution={10}
      linkColor={link => {
        const source = endpointId(link.source);
        const target = endpointId(link.target);
        if (hoveredId && source !== hoveredId && target !== hoveredId) return "#d8d3ca";
        return link.semanticRelation ? semanticColors[link.semanticRelation] ?? "#9c9284" : link.relation.includes("作者") ? "#587184" : link.score >= .28 ? "#a84435" : "#b99a78";
      }}
      linkWidth={link => {
        const source = endpointId(link.source);
        const target = endpointId(link.target);
        return hoveredId && (source === hoveredId || target === hoveredId) ? 1.5 : .12 + Math.min(.65, link.score * 1.15);
      }}
      linkOpacity={.18}
      linkResolution={2}
      linkDirectionalParticles={link => {
        const source = endpointId(link.source);
        const target = endpointId(link.target);
        if (hoveredId && (source === hoveredId || target === hoveredId)) return 3;
        return link.score >= .28 || link.relation.includes("作者") ? 1 : 0;
      }}
      linkDirectionalParticleSpeed={link => {
        const source = endpointId(link.source);
        const target = endpointId(link.target);
        return hoveredId && (source === hoveredId || target === hoveredId) ? .018 : .007 + Math.min(.006, link.score * .012);
      }}
      linkDirectionalParticleWidth={link => {
        const source = endpointId(link.source);
        const target = endpointId(link.target);
        return hoveredId && (source === hoveredId || target === hoveredId) ? 2.2 : 1.15;
      }}
      linkDirectionalParticleColor={link => link.semanticRelation ? semanticColors[link.semanticRelation] ?? "#9c9284" : link.relation.includes("作者") ? "#587184" : "#b84b38"}
      onNodeHover={node => setHoveredId(node ? String(node.id) : undefined)}
      onNodeClick={(node: NodeObject<Node3D>, event) => { const id = String(node.id); if (event.detail > 1) onNodeOpen(id); else onNodeClick(id); }}
      onLinkClick={(link: LinkObject<Node3D, Link3D>) => onEdgeClick(link.id)}
      enableNodeDrag
      enableNavigationControls
    />
    <div className="graph-3d-guide"><strong>全库 3D 星图</strong><span>拖拽旋转 · 滚轮缩放 · 悬停聚焦关系</span></div>
  </div>;
}
