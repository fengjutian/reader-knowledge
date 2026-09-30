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
  const chartRef = useRef<echarts.ECharts | undefined>(undefined);
  const callbacksRef = useRef({ onNodeClick, onNodeOpen, onEdgeClick });
  callbacksRef.current = { onNodeClick, onNodeOpen, onEdgeClick };
  const data = useMemo(() => {
    const degree = new Map<string, number>();
    const adjacentIds = new Set<string>();
    edges.forEach(edge => {
      degree.set(edge.from, (degree.get(edge.from) ?? 0) + 1);
      degree.set(edge.to, (degree.get(edge.to) ?? 0) + 1);
      if (selectedId && (edge.from === selectedId || edge.to === selectedId)) {
        adjacentIds.add(edge.from);
        adjacentIds.add(edge.to);
      }
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
          const group = hash(node.category || node.author || node.title) % colors.length;
          const groupAngle = Math.PI * 2 * group / colors.length;
          const angle = groupAngle + ((seed % 1000) / 1000 - .5) * .72;
          const radius = 80 + ((seed >>> 8) % 180);
          x = Math.cos(angle) * radius;
          y = Math.sin(angle) * radius;
        } else {
          const angle = Math.PI * 2 * isolatedIndex / Math.max(1, isolatedCount);
          const radius = 760 + (isolatedIndex % 17) * 4;
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
          symbolSize: node.id === selectedId ? 9 : selectedId && adjacentIds.has(node.id) ? 5 : relationCount ? 4 : 2,
          itemStyle: {
            color: node.id === focusId ? "#ef493c" : node.id === selectedId ? "#27231f" : relationCount ? colors[hash(node.category || node.author || node.title) % colors.length] : "#d9d5ce",
            opacity: selectedId ? (node.id === selectedId || adjacentIds.has(node.id) ? 1 : .055) : relationCount ? .94 : .2,
          },
          label: node.id === selectedId ? { show: true, color: "#302a26", fontSize: 12 } : undefined,
          book: node,
          relationCount,
        };
      }),
      edges: edges.map(edge => {
        const adjacent = !!selectedId && (edge.from === selectedId || edge.to === selectedId);
        return {
          id: edge.id,
          source: edge.from,
          target: edge.to,
          value: Math.max(.25, edge.score),
          relation: edge.relation,
          lineStyle: selectedId
            ? { color: adjacent ? "#9b5044" : "#d8d5cf", width: adjacent ? 1.6 : .35, opacity: adjacent ? .9 : .025 }
            : { color: "#c9c6c0", width: .7, opacity: .32 },
        };
      }),
    };
  }, [nodes, edges, focusId, selectedId]);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const chart = echarts.init(host, undefined, { renderer: "canvas" });
    chartRef.current = chart;
    const observer = new ResizeObserver(() => chart.resize());
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
        name: "书籍关系",
        layout: "forceAtlas2",
        data: data.nodes,
        edges: data.edges,
        left: "7%",
        top: "7%",
        right: "7%",
        bottom: "7%",
        roam: true,
        focusNodeAdjacency: true,
        focusNodeAdjacencyOn: "mouseover",
        label: { show: false },
        forceAtlas2: {
          GPU: true,
          steps: 2,
          maxSteps: 420,
          repulsionByDegree: true,
          linLogMode: true,
          strongGravityMode: false,
          gravity: .65,
          scaling: 2.2,
          edgeWeightInfluence: .8,
          edgeWeight: [1, 4],
          nodeWeight: [1, 4],
          jitterTolerence: .16,
          preventOverlap: true,
        },
        lineStyle: { color: "#c9c6c0", width: .7, opacity: .32 },
        emphasis: {
          label: { show: true, color: "#302a26", fontSize: 12, backgroundColor: "rgba(255,252,247,.92)", padding: [5, 7], borderRadius: 5 },
          itemStyle: { color: "#ef493c", opacity: 1 },
          lineStyle: { color: "#8f8980", width: 1.2, opacity: .82 },
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
  </div>;
}
