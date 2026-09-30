import ForceGraph3D, { type ForceGraphMethods, type GraphData, type LinkObject, type NodeObject } from "react-force-graph-3d";
import { useEffect, useMemo, useRef, useState } from "react";
import type { Book } from "../../types/domain";

type GraphNode = Book & { x: number; y: number };
type GraphEdge = { id: string; from: string; to: string; score: number; relation: string; semanticRelation?: string };
type Node3D = GraphNode & { id: string; relationCount: number };
type Link3D = GraphEdge & { source: string; target: string };
const semanticColors: Record<string, string> = { agreement: "#4f8a64", conflict: "#b74335", complementary: "#4f78a8", causal: "#8b6aa8", application: "#3f8790", same_concept: "#a06b3b", uncertain: "#a39b90" };

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
  return <div ref={hostRef} className="force-graph-3d">
    <ForceGraph3D<Node3D, Link3D>
      ref={graphRef}
      graphData={data}
      width={size.width}
      height={size.height}
      backgroundColor="#fbfaf6"
      showNavInfo={false}
      forceEngine="ngraph"
      warmupTicks={40}
      cooldownTime={5000}
      nodeLabel={node => `${node.title}${node.author ? `<br/>${node.author}` : ""}<br/>${node.relationCount} 个关系`}
      nodeVal={node => node.id === focusId ? 8 : node.id === selectedId ? 6 : node.relationCount ? 2.4 + Math.min(4, Math.log2(node.relationCount + 1)) : .7}
      nodeColor={node => node.id === focusId ? "#8f3025" : node.id === selectedId ? "#29251f" : node.relationCount ? "#bd5b49" : "#c9c2b7"}
      nodeOpacity={.9}
      nodeResolution={8}
      linkColor={link => link.semanticRelation ? semanticColors[link.semanticRelation] ?? "#9c9284" : link.relation.includes("作者") ? "#71808d" : "#a74335"}
      linkWidth={link => .25 + Math.min(1.8, link.score * 3)}
      linkOpacity={.32}
      linkResolution={3}
      onNodeClick={(node: NodeObject<Node3D>, event) => { const id = String(node.id); if (event.detail > 1) onNodeOpen(id); else onNodeClick(id); }}
      onLinkClick={(link: LinkObject<Node3D, Link3D>) => onEdgeClick(link.id)}
      enableNodeDrag
      enableNavigationControls
    />
  </div>;
}
