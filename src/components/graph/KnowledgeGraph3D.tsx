import * as echarts from "echarts";
import "echarts-gl";
import { useEffect, useMemo, useRef } from "react";
import type { Book } from "../../types/domain";

type GraphNode = Book & { x: number; y: number };
type GraphEdge = { id: string; from: string; to: string; score: number; relation: string; semanticRelation?: string };
const colors = ["#a84435", "#4d7185", "#66805d", "#a8783f", "#77638c", "#397b78", "#9a5d72", "#6f7450"];

function hash(value: string) {
  let result = 0;
  for (let index = 0; index < value.length; index += 1) result = Math.imul(result ^ value.charCodeAt(index), 16777619);
  return Math.abs(result);
}

function escapeHtml(value: string) {
  return value.replace(/[&<>"']/g, character => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", "\"": "&quot;", "'": "&#39;" })[character]!);
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
  const chartRef = useRef<echarts.ECharts>();
  const callbacksRef = useRef({ onNodeClick, onNodeOpen, onEdgeClick });
  callbacksRef.current = { onNodeClick, onNodeOpen, onEdgeClick };
  const data = useMemo(() => {
    const degree = new Map<string, number>();
    edges.forEach(edge => {
      degree.set(edge.from, (degree.get(edge.from) ?? 0) + 1);
      degree.set(edge.to, (degree.get(edge.to) ?? 0) + 1);
    });
    const isolatedCount = nodes.reduce((sum, node) => sum + (degree.has(node.id) ? 0 : 1), 0);
    let isolatedIndex = 0;
    return {
      nodes: nodes.map(node => {
        const relationCount = degree.get(node.id) ?? 0;
        const seed = hash(node.id);
        let x: number;
        let y: number;
        if (relationCount) {
          const angle = (seed % 6283) / 1000;
          const radius = 35 + ((seed >>> 8) % 260);
          x = Math.cos(angle) * radius;
          y = Math.sin(angle) * radius;
        } else {
          const angle = Math.PI * 2 * isolatedIndex / Math.max(1, isolatedCount);
          const radius = 610 + (isolatedIndex % 11) * 5;
          isolatedIndex += 1;
          x = Math.cos(angle) * radius;
          y = Math.sin(angle) * radius;
        }
        return {
          id: node.id,
          name: node.title,
          value: relationCount,
          x,
          y,
          symbolSize: 3,
          itemStyle: {
            color: node.id === focusId ? "#ef493c" : node.id === selectedId ? "#27231f" : relationCount ? colors[hash(node.category || node.author || node.title) % colors.length] : "#d9d5ce",
            opacity: relationCount ? .92 : .42,
          },
          book: node,
          relationCount,
        };
      }),
      links: edges.map(edge => ({ id: edge.id, source: edge.from, target: edge.to, value: Math.max(.1, edge.score), relation: edge.relation })),
    };
  }, [nodes, edges, focusId, selectedId]);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const chart = echarts.init(host, undefined, { renderer: "canvas" });
    chartRef.current = chart;
    const resize = () => chart.resize();
    const observer = new ResizeObserver(resize);
    observer.observe(host);
    chart.on("click", params => {
      const value = params.data as { id?: string } | undefined;
      if (!value?.id) return;
      if (params.dataType === "edge") callbacksRef.current.onEdgeClick(value.id);
      else callbacksRef.current.onNodeClick(value.id);
    });
    chart.on("dblclick", params => {
      const value = params.data as { id?: string } | undefined;
      if (params.dataType !== "edge" && value?.id) callbacksRef.current.onNodeOpen(value.id);
    });
    return () => { observer.disconnect(); chart.dispose(); chartRef.current = undefined; };
  }, []);

  useEffect(() => {
    const chart = chartRef.current;
    if (!chart) return;
    chart.setOption({
      backgroundColor: "transparent",
      animation: true,
      animationDuration: 900,
      tooltip: {
        confine: true,
        formatter: (params: { dataType?: string; data?: { book?: Book; relationCount?: number; relation?: string } }) => {
          if (params.dataType === "edge") return escapeHtml(params.data?.relation || "书籍关系");
          const book = params.data?.book;
          if (!book) return "";
          return `<div class="graph-3d-tooltip"><strong>${escapeHtml(book.title)}</strong><span>${escapeHtml(book.author || "未知作者")}</span><small>${params.data?.relationCount ?? 0} 个关系</small></div>`;
        },
      },
      series: [{
        type: "graphGL",
        data: data.nodes,
        nodes: data.nodes,
        edges: data.links,
        links: data.links,
        roam: true,
        focusNodeAdjacency: true,
        lineStyle: { color: "rgba(175, 172, 166, .5)", width: .7, opacity: .5 },
        emphasis: { itemStyle: { color: "#ef493c", opacity: 1 }, lineStyle: { color: "#aaa49b", width: 1.5, opacity: .9 } },
        forceAtlas2: {
          steps: 8,
          stopThreshold: 6,
          jitterTolerence: 8,
          edgeWeight: 1.2,
          gravity: 1.4,
          edgeWeightInfluence: 1,
          scaling: 1.8,
          preventOverlap: true,
        },
      }],
    } as never, true);
  }, [data]);

  useEffect(() => {
    if (fitRequest <= 0) return;
    chartRef.current?.dispatchAction({ type: "restore" });
  }, [fitRequest]);

  return <div className="force-graph-3d force-graph-echarts">
    <div ref={hostRef} className="force-graph-echarts__canvas" />
    <div className="graph-3d-guide"><strong>全库关系图</strong><span>ECharts GL · 布局完成后自动静止</span></div>
  </div>;
}
