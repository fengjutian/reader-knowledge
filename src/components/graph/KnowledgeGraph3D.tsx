import * as echarts from "echarts";
import "echarts-gl";
import { LocateFixed, Minus, Plus } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
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

export function KnowledgeGraph3D({ nodes, edges, focusId, selectedId, showIsolated, fitRequest, onNodeClick, onNodeOpen, onEdgeClick }: {
  nodes: GraphNode[];
  edges: GraphEdge[];
  focusId?: string;
  selectedId?: string;
  showIsolated: boolean;
  fitRequest: number;
  onNodeClick: (id: string) => void;
  onNodeOpen: (id: string) => void;
  onEdgeClick: (id: string) => void;
}) {
  const hostRef = useRef<HTMLDivElement>(null);
  const chartRef = useRef<echarts.ECharts | undefined>(undefined);
  const [zoom, setZoom] = useState(1);
  const callbacksRef = useRef({ onNodeClick, onNodeOpen, onEdgeClick });
  callbacksRef.current = { onNodeClick, onNodeOpen, onEdgeClick };
  const data = useMemo(() => {
    const fullDegree = new Map<string, number>();
    edges.forEach(edge => {
      fullDegree.set(edge.from, (fullDegree.get(edge.from) ?? 0) + 1);
      fullDegree.set(edge.to, (fullDegree.get(edge.to) ?? 0) + 1);
    });
    const skeletonDegree = new Map<string, number>();
    const sortedEdges = [...edges].sort((left, right) => right.score - left.score);
    const visibleEdges = selectedId
      ? sortedEdges.filter(edge => edge.from === selectedId || edge.to === selectedId)
      : sortedEdges.filter(edge => {
        if ((skeletonDegree.get(edge.from) ?? 0) >= 3 || (skeletonDegree.get(edge.to) ?? 0) >= 3) return false;
        skeletonDegree.set(edge.from, (skeletonDegree.get(edge.from) ?? 0) + 1);
        skeletonDegree.set(edge.to, (skeletonDegree.get(edge.to) ?? 0) + 1);
        return true;
      }).slice(0, 900);
    const degree = new Map<string, number>();
    const adjacentIds = new Set<string>();
    visibleEdges.forEach(edge => {
      degree.set(edge.from, (degree.get(edge.from) ?? 0) + 1);
      degree.set(edge.to, (degree.get(edge.to) ?? 0) + 1);
      if (selectedId && (edge.from === selectedId || edge.to === selectedId)) {
        adjacentIds.add(edge.from);
        adjacentIds.add(edge.to);
      }
    });
    const hubIds = new Set([...fullDegree.entries()].sort((left, right) => right[1] - left[1]).slice(0, 18).map(([id]) => id));
    const isolatedCount = nodes.reduce((sum, node) => sum + (degree.has(node.id) ? 0 : 1), 0);
    let isolatedIndex = 0;
    const mappedNodes = nodes.map(node => {
        const relationCount = degree.get(node.id) ?? 0;
        const totalRelationCount = fullDegree.get(node.id) ?? 0;
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
        if (selectedId && relationCount) {
          if (node.id === selectedId) {
            x = 0;
            y = 0;
          } else {
            const angle = (seed % 6283) / 1000;
            x = Math.cos(angle) * 220;
            y = Math.sin(angle) * 220;
          }
        }
        return {
          id: node.id,
          name: node.title,
          value: relationCount,
          x,
          y,
          fixed: node.id === selectedId,
          symbolSize: node.id === selectedId ? 14 : selectedId && adjacentIds.has(node.id) ? 9 : relationCount ? 7 : 3,
          itemStyle: {
            color: node.id === focusId ? "#ef493c" : node.id === selectedId ? "#27231f" : relationCount ? colors[hash(node.category || node.author || node.title) % colors.length] : "#d9d5ce",
            opacity: selectedId ? (node.id === selectedId || adjacentIds.has(node.id) ? 1 : .055) : relationCount ? .94 : .2,
          },
          label: node.id === selectedId || (!selectedId && hubIds.has(node.id))
            ? { show: true, color: "#49413b", fontSize: node.id === selectedId ? 12 : 10 }
            : undefined,
          book: node,
          relationCount: totalRelationCount,
        };
      });
    return {
      nodes: mappedNodes.filter(node => node.value > 0),
      isolatedNodes: mappedNodes.filter(node => node.value === 0),
      edges: visibleEdges.map(edge => {
        const adjacent = !!selectedId && (edge.from === selectedId || edge.to === selectedId);
        return {
          id: edge.id,
          source: edge.from,
          target: edge.to,
          value: Math.max(.25, edge.score),
          relation: edge.relation,
          lineStyle: selectedId
            ? { color: adjacent ? "#9b5044" : "#d8d5cf", width: adjacent ? 1.6 : .35, opacity: adjacent ? .9 : .025 }
            : { color: "#c9c6c0", width: .6, opacity: .22 },
        };
      }),
    };
  }, [nodes, edges, focusId, selectedId]);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const chart = echarts.init(host, undefined, { renderer: "canvas", devicePixelRatio: Math.min(window.devicePixelRatio, 1.25) });
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
      series: selectedId ? [{
        type: "graph",
        name: "相邻关系",
        layout: "force",
        data: data.nodes,
        links: data.edges,
        left: "10%",
        top: "10%",
        right: "10%",
        bottom: "10%",
        roam: true,
        draggable: true,
        label: {
          show: true,
          position: "right",
          distance: 6,
          color: "#514943",
          fontSize: 11,
          formatter: "{b}",
        },
        force: {
          initLayout: "circular",
          repulsion: 520,
          gravity: .12,
          edgeLength: [115, 190],
          friction: .72,
          layoutAnimation: true,
        },
        lineStyle: { color: "#ad6a5d", width: 1.5, opacity: .78, curveness: .16 },
        edgeLabel: { show: false },
        emphasis: {
          focus: "adjacency",
          itemStyle: { color: "#ef493c", opacity: 1 },
          lineStyle: { color: "#8d4034", width: 2.2, opacity: 1, curveness: .2 },
        },
      }] : [{
        type: "graph",
        name: "未关联书籍",
        layout: "none",
        data: showIsolated ? data.isolatedNodes : [],
        links: [],
        left: "2%",
        top: "2%",
        right: "2%",
        bottom: "2%",
        roam: false,
        label: { show: false },
        lineStyle: { opacity: 0 },
        emphasis: { itemStyle: { color: "#aaa49b", opacity: .75 } },
      }, {
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
        focusNodeAdjacency: false,
        label: { show: false },
        forceAtlas2: {
          GPU: true,
          steps: 4,
          maxSteps: 140,
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
        lineStyle: { color: "#c9c6c0", width: .6, opacity: .22 },
        emphasis: {
          label: { show: true, color: "#302a26", fontSize: 12, backgroundColor: "rgba(255,252,247,.92)", padding: [5, 7], borderRadius: 5 },
          itemStyle: { color: "#ef493c", opacity: 1 },
          lineStyle: { color: "#8f8980", width: 1.2, opacity: .82 },
        },
      }],
    } as never, true);
  }, [data, selectedId, showIsolated]);

  useEffect(() => {
    if (fitRequest <= 0) return;
    chartRef.current?.dispatchAction({ type: "restore" });
  }, [fitRequest]);

  useEffect(() => setZoom(1), [selectedId]);

  function changeZoom(nextValue: number) {
    const chart = chartRef.current;
    if (!chart) return;
    const next = Math.max(.35, Math.min(4, Number(nextValue.toFixed(2))));
    setZoom(next);
    if (selectedId) chart.setOption({ series: [{ zoom: next }] } as never);
    else chart.dispatchAction({ type: "graphGLRoam", seriesIndex: 1, zoom: next });
  }

  function resetView() {
    const chart = chartRef.current;
    if (!chart) return;
    setZoom(1);
    if (selectedId) chart.setOption({ series: [{ zoom: 1, center: undefined }] } as never);
    else chart.dispatchAction({ type: "graphGLRoam", seriesIndex: 1, zoom: 1, offset: [0, 0] });
  }

  return <div className="force-graph-3d force-graph-echarts">
    <div ref={hostRef} className="force-graph-echarts__canvas" />
    <div className="graph-zoom-controls" aria-label="图谱缩放控制">
      <button type="button" onClick={() => changeZoom(zoom / 1.25)} title="缩小" aria-label="缩小"><Minus size={15}/></button>
      <span>{Math.round(zoom * 100)}%</span>
      <button type="button" onClick={() => changeZoom(zoom * 1.25)} title="放大" aria-label="放大"><Plus size={15}/></button>
      <button type="button" onClick={resetView} title="复位视图" aria-label="复位视图"><LocateFixed size={15}/></button>
    </div>
  </div>;
}
