import ForceGraph3D, { type ForceGraphMethods, type GraphData, type LinkObject, type NodeObject } from "react-force-graph-3d";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Book } from "../../types/domain";

type GraphNode = Book & { x: number; y: number };
type GraphEdge = { id: string; from: string; to: string; score: number; relation: string; semanticRelation?: string };
type Node3D = GraphNode & { id: string; z?: number; fx?: number; fy?: number; fz?: number; relationCount: number };
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

function seededUnit(value: string, salt: number) {
  let hash = salt | 0;
  for (let index = 0; index < value.length; index += 1) hash = Math.imul(hash ^ value.charCodeAt(index), 16777619);
  return (Math.abs(hash) % 100000) / 100000;
}

function spherePoint(index: number, total: number, radius: number) {
  const y = 1 - ((index + .5) / Math.max(1, total)) * 2;
  const ring = Math.sqrt(Math.max(0, 1 - y * y));
  const angle = Math.PI * (3 - Math.sqrt(5)) * index;
  return { x: Math.cos(angle) * ring * radius, y: y * radius, z: Math.sin(angle) * ring * radius };
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
  const autoFitTimerRef = useRef<number | undefined>(undefined);
  const userInteractedRef = useRef(false);
  const [size, setSize] = useState({ width: 1, height: 1 });
  const [hoveredId, setHoveredId] = useState<string>();
  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const update = () => setSize({ width: Math.max(1, host.clientWidth), height: Math.max(1, host.clientHeight) });
    update();
    const observer = new ResizeObserver(update);
    observer.observe(host);
    const stopAutoFit = () => {
      userInteractedRef.current = true;
      if (autoFitTimerRef.current !== undefined) window.clearInterval(autoFitTimerRef.current);
      autoFitTimerRef.current = undefined;
    };
    host.addEventListener("wheel", stopAutoFit, { passive: true });
    host.addEventListener("pointerdown", stopAutoFit, { passive: true });
    return () => {
      observer.disconnect();
      host.removeEventListener("wheel", stopAutoFit);
      host.removeEventListener("pointerdown", stopAutoFit);
    };
  }, []);
  useEffect(() => { if (fitRequest > 0) graphRef.current?.zoomToFit(500, 70); }, [fitRequest]);
  const data = useMemo<GraphData<Node3D, Link3D>>(() => {
    const degree = new Map<string, number>();
    edges.forEach(edge => { degree.set(edge.from, (degree.get(edge.from) ?? 0) + 1); degree.set(edge.to, (degree.get(edge.to) ?? 0) + 1); });
    const isolatedTotal = nodes.reduce((total, node) => total + (degree.has(node.id) ? 0 : 1), 0);
    let isolatedIndex = 0;
    return {
      nodes: nodes.map(node => {
        const relationCount = degree.get(node.id) ?? 0;
        if (!relationCount) {
          const shell = spherePoint(isolatedIndex++, isolatedTotal, 620 + seededUnit(node.id, 17) * 24);
          return { ...node, id: node.id, relationCount, ...shell, fx: shell.x, fy: shell.y, fz: shell.z };
        }
        const y = seededUnit(node.id, 31) * 2 - 1;
        const angle = seededUnit(node.id, 73) * Math.PI * 2;
        const radius = 70 + seededUnit(node.id, 109) * 150;
        const ring = Math.sqrt(Math.max(0, 1 - y * y));
        return { ...node, id: node.id, relationCount, x: Math.cos(angle) * ring * radius, y: y * radius, z: Math.sin(angle) * ring * radius };
      }),
      links: edges.map(edge => ({ ...edge, source: edge.from, target: edge.to })),
    };
  }, [nodes, edges]);
  useEffect(() => {
    if (!data.nodes.length) return;
    userInteractedRef.current = false;
    const startRotation = window.setTimeout(() => {
      const controls = graphRef.current?.controls() as { autoRotate?: boolean; autoRotateSpeed?: number } | undefined;
      if (controls) { controls.autoRotate = true; controls.autoRotateSpeed = .38; }
    }, 400);
    let fitCount = 0;
    autoFitTimerRef.current = window.setInterval(() => {
      if (userInteractedRef.current) return;
      graphRef.current?.zoomToFit(450, 90);
      fitCount += 1;
      if (fitCount >= 15 && autoFitTimerRef.current !== undefined) {
        window.clearInterval(autoFitTimerRef.current);
        autoFitTimerRef.current = undefined;
      }
    }, 2000);
    return () => {
      window.clearTimeout(startRotation);
      if (autoFitTimerRef.current !== undefined) window.clearInterval(autoFitTimerRef.current);
      autoFitTimerRef.current = undefined;
    };
  }, [data]);
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
        timestep: 20,
        springLength: 70,
        springCoefficient: .0008,
        gravity: -1.8,
        dragCoefficient: .02,
      }}
      warmupTicks={120}
      cooldownTime={30000}
      nodeLabel={node => `<div class="graph-3d-tooltip"><strong>${node.title}</strong>${node.author ? `<span>${node.author}</span>` : ""}<small>${node.category || "未分类"} · ${node.relationCount} 个关系</small></div>`}
      nodeVal={.12}
      nodeColor={node => node.id === focusId ? "#ff493d" : node.id === selectedId ? "#28231f" : node.relationCount ? categoryColor(node) : "#bdb6aa"}
      nodeOpacity={.86}
      nodeResolution={7}
      linkColor={link => {
        const source = endpointId(link.source);
        const target = endpointId(link.target);
        return hoveredId && (source === hoveredId || target === hoveredId) ? "#aaa49b" : "#d4d0c9";
      }}
      linkWidth={link => {
        const source = endpointId(link.source);
        const target = endpointId(link.target);
        return hoveredId && (source === hoveredId || target === hoveredId) ? 1.1 : .11;
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
        return hoveredId && (source === hoveredId || target === hoveredId) ? .035 : .016 + Math.min(.012, link.score * .02);
      }}
      linkDirectionalParticleWidth={link => {
        const source = endpointId(link.source);
        const target = endpointId(link.target);
        return hoveredId && (source === hoveredId || target === hoveredId) ? 2.2 : 1.15;
      }}
      linkDirectionalParticleColor={link => link.semanticRelation ? semanticColors[link.semanticRelation] ?? "#9c9284" : link.relation.includes("作者") ? "#587184" : "#b84b38"}
      onEngineStop={() => { if (!userInteractedRef.current) graphRef.current?.zoomToFit(800, 90); }}
      onNodeHover={node => setHoveredId(node ? String(node.id) : undefined)}
      onNodeClick={(node: NodeObject<Node3D>, event) => { const id = String(node.id); if (event.detail > 1) onNodeOpen(id); else onNodeClick(id); }}
      onLinkClick={(link: LinkObject<Node3D, Link3D>) => onEdgeClick(link.id)}
      enableNodeDrag
      enableNavigationControls
    />
    <div className="graph-3d-guide"><strong>全库 3D 星图</strong><span>自动巡航 · 拖拽旋转 · 悬停聚焦关系</span></div>
  </div>;
}
