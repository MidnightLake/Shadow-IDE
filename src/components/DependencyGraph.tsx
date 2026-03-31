import React, { useEffect, useRef, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

interface DependencyGraphProps {
  projectPath: string;
}

interface GraphNode {
  id: string;
  label: string;
  type: "file" | "external" | "circular" | string;
  // Runtime position
  x: number;
  y: number;
  vx: number;
  vy: number;
}

interface GraphEdge {
  from: string;
  to: string;
}

interface GraphData {
  nodes: Array<{ id: string; label: string; type: string }>;
  edges: GraphEdge[];
}

const NODE_RADIUS = 22;
const REPULSION = 4000;
const ATTRACTION = 0.04;
const DAMPING = 0.85;
const ITERATIONS_PER_FRAME = 3;

function nodeColor(type: string): string {
  if (type === "file") return "var(--accent-hover)";
  if (type === "circular") return "#f38ba8";
  return "var(--text-secondary)";
}

function buildAdjacency(edges: GraphEdge[]): Map<string, Set<string>> {
  const adj = new Map<string, Set<string>>();
  for (const e of edges) {
    if (!adj.has(e.from)) adj.set(e.from, new Set());
    if (!adj.has(e.to)) adj.set(e.to, new Set());
    adj.get(e.from)!.add(e.to);
  }
  return adj;
}

export default function DependencyGraph({ projectPath }: DependencyGraphProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [nodes, setNodes] = useState<GraphNode[]>([]);
  const [edges, setEdges] = useState<GraphEdge[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const animFrameRef = useRef<number | null>(null);

  // Pan/zoom state
  const transformRef = useRef({ x: 0, y: 0, scale: 1 });
  const dragStateRef = useRef<{ panning: boolean; startX: number; startY: number; lastTx: number; lastTy: number } | null>(null);
  const nodeDragRef = useRef<{ nodeId: string; offsetX: number; offsetY: number } | null>(null);

  const loadData = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const data = await invoke<GraphData>("analyze_dependencies", { project_path: projectPath });
      if (!data || data.nodes.length === 0) {
        setError("No dependency data");
        setNodes([]);
        setEdges([]);
        return;
      }

      const canvas = canvasRef.current;
      const cx = canvas ? canvas.width / 2 : 400;
      const cy = canvas ? canvas.height / 2 : 300;
      const radius = Math.min(cx, cy) * 0.6;
      const count = data.nodes.length;

      const initialNodes: GraphNode[] = data.nodes.map((n, i) => ({
        ...n,
        x: cx + radius * Math.cos((2 * Math.PI * i) / count),
        y: cy + radius * Math.sin((2 * Math.PI * i) / count),
        vx: 0,
        vy: 0,
      }));
      setNodes(initialNodes);
      setEdges(data.edges);
    } catch (err) {
      setError(String(err));
    } finally {
      setLoading(false);
    }
  }, [projectPath]);

  useEffect(() => {
    loadData();
  }, [loadData]);

  // Force simulation
  useEffect(() => {
    if (nodes.length === 0) return;
    let localNodes = nodes.map((n) => ({ ...n }));
    let stopped = false;

    const step = () => {
      if (stopped) return;
      for (let iter = 0; iter < ITERATIONS_PER_FRAME; iter++) {
        // Repulsion between all nodes
        for (let i = 0; i < localNodes.length; i++) {
          for (let j = i + 1; j < localNodes.length; j++) {
            const dx = localNodes[j].x - localNodes[i].x;
            const dy = localNodes[j].y - localNodes[i].y;
            const dist2 = dx * dx + dy * dy + 1;
            const force = REPULSION / dist2;
            const nx = (dx / Math.sqrt(dist2)) * force;
            const ny = (dy / Math.sqrt(dist2)) * force;
            localNodes[i].vx -= nx;
            localNodes[i].vy -= ny;
            localNodes[j].vx += nx;
            localNodes[j].vy += ny;
          }
        }
        // Attraction along edges
        for (const e of edges) {
          const src = localNodes.find((n) => n.id === e.from);
          const dst = localNodes.find((n) => n.id === e.to);
          if (!src || !dst) continue;
          const dx = dst.x - src.x;
          const dy = dst.y - src.y;
          src.vx += dx * ATTRACTION;
          src.vy += dy * ATTRACTION;
          dst.vx -= dx * ATTRACTION;
          dst.vy -= dy * ATTRACTION;
        }
        // Apply + dampen
        for (const n of localNodes) {
          n.vx *= DAMPING;
          n.vy *= DAMPING;
          n.x += n.vx;
          n.y += n.vy;
        }
      }

      setNodes(localNodes.map((n) => ({ ...n })));
      animFrameRef.current = requestAnimationFrame(step);
    };

    animFrameRef.current = requestAnimationFrame(step);
    return () => {
      stopped = true;
      if (animFrameRef.current) cancelAnimationFrame(animFrameRef.current);
    };
  }, [edges, nodes.length]); // eslint-disable-line react-hooks/exhaustive-deps -- only restart when topology changes

  // Draw
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const { x: tx, y: ty, scale } = transformRef.current;

    ctx.clearRect(0, 0, canvas.width, canvas.height);
    ctx.save();
    ctx.translate(tx, ty);
    ctx.scale(scale, scale);

    const adjSelected = selectedId ? buildAdjacency(edges) : null;
    const highlightedIds = new Set<string>();
    if (selectedId && adjSelected) {
      highlightedIds.add(selectedId);
      adjSelected.get(selectedId)?.forEach((id) => highlightedIds.add(id));
      // Also incoming edges
      for (const e of edges) {
        if (e.to === selectedId) highlightedIds.add(e.from);
      }
    }

    // Draw edges
    for (const e of edges) {
      const src = nodes.find((n) => n.id === e.from);
      const dst = nodes.find((n) => n.id === e.to);
      if (!src || !dst) continue;
      const isHighlighted = selectedId ? highlightedIds.has(e.from) && highlightedIds.has(e.to) : false;
      ctx.beginPath();
      ctx.moveTo(src.x, src.y);
      ctx.lineTo(dst.x, dst.y);
      ctx.strokeStyle = isHighlighted ? "var(--accent-hover)" : (selectedId ? "#2a2a3e" : "#3b4048");
      ctx.lineWidth = isHighlighted ? 2 : 1;
      ctx.stroke();

      // Arrowhead
      const angle = Math.atan2(dst.y - src.y, dst.x - src.x);
      const ax = dst.x - Math.cos(angle) * (NODE_RADIUS + 4);
      const ay = dst.y - Math.sin(angle) * (NODE_RADIUS + 4);
      ctx.beginPath();
      ctx.moveTo(ax, ay);
      ctx.lineTo(ax - 8 * Math.cos(angle - 0.4), ay - 8 * Math.sin(angle - 0.4));
      ctx.lineTo(ax - 8 * Math.cos(angle + 0.4), ay - 8 * Math.sin(angle + 0.4));
      ctx.closePath();
      ctx.fillStyle = isHighlighted ? "var(--accent-hover)" : (selectedId ? "#2a2a3e" : "#3b4048");
      ctx.fill();
    }

    // Draw nodes
    for (const node of nodes) {
      const isSelected = node.id === selectedId;
      const isHighlighted = selectedId ? highlightedIds.has(node.id) : false;
      const alpha = selectedId && !isHighlighted ? 0.3 : 1;
      ctx.globalAlpha = alpha;
      ctx.beginPath();
      ctx.arc(node.x, node.y, NODE_RADIUS, 0, Math.PI * 2);
      ctx.fillStyle = nodeColor(node.type);
      ctx.fill();
      if (isSelected) {
        ctx.strokeStyle = "#ffffff";
        ctx.lineWidth = 2;
        ctx.stroke();
      }
      ctx.globalAlpha = 1;

      // Label
      ctx.fillStyle = alpha < 1 ? "#4a4a5e" : "var(--bg-primary)";
      ctx.font = "bold 10px JetBrains Mono, monospace";
      ctx.textAlign = "center";
      ctx.textBaseline = "middle";
      const label = node.label.length > 12 ? node.label.slice(0, 10) + "…" : node.label;
      ctx.fillText(label, node.x, node.y);
    }

    ctx.restore();
  }, [nodes, edges, selectedId]);

  // Mouse events
  const getNodeAtPos = useCallback((clientX: number, clientY: number): GraphNode | null => {
    const canvas = canvasRef.current;
    if (!canvas) return null;
    const rect = canvas.getBoundingClientRect();
    const { x: tx, y: ty, scale } = transformRef.current;
    const wx = (clientX - rect.left - tx) / scale;
    const wy = (clientY - rect.top - ty) / scale;
    for (const n of nodes) {
      const dx = n.x - wx;
      const dy = n.y - wy;
      if (dx * dx + dy * dy <= NODE_RADIUS * NODE_RADIUS) return n;
    }
    return null;
  }, [nodes]);

  const handleMouseDown = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const node = getNodeAtPos(e.clientX, e.clientY);
    if (node) {
      const canvas = canvasRef.current!;
      const rect = canvas.getBoundingClientRect();
      const { x: tx, y: ty, scale } = transformRef.current;
      const wx = (e.clientX - rect.left - tx) / scale;
      const wy = (e.clientY - rect.top - ty) / scale;
      nodeDragRef.current = { nodeId: node.id, offsetX: wx - node.x, offsetY: wy - node.y };
      setSelectedId((prev) => (prev === node.id ? null : node.id));
    } else {
      dragStateRef.current = { panning: true, startX: e.clientX, startY: e.clientY, lastTx: transformRef.current.x, lastTy: transformRef.current.y };
    }
  };

  const handleMouseMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    if (nodeDragRef.current) {
      const canvas = canvasRef.current!;
      const rect = canvas.getBoundingClientRect();
      const { x: tx, y: ty, scale } = transformRef.current;
      const wx = (e.clientX - rect.left - tx) / scale;
      const wy = (e.clientY - rect.top - ty) / scale;
      const { nodeId, offsetX, offsetY } = nodeDragRef.current;
      setNodes((prev) => prev.map((n) => n.id === nodeId ? { ...n, x: wx - offsetX, y: wy - offsetY, vx: 0, vy: 0 } : n));
    } else if (dragStateRef.current?.panning) {
      const ds = dragStateRef.current;
      transformRef.current.x = ds.lastTx + (e.clientX - ds.startX);
      transformRef.current.y = ds.lastTy + (e.clientY - ds.startY);
      // Trigger redraw via setNodes identity (shallow copy)
      setNodes((prev) => [...prev]);
    }
  };

  const handleMouseUp = () => {
    dragStateRef.current = null;
    nodeDragRef.current = null;
  };

  const handleWheel = (e: React.WheelEvent<HTMLCanvasElement>) => {
    e.preventDefault();
    const factor = e.deltaY > 0 ? 0.9 : 1.1;
    transformRef.current.scale = Math.max(0.1, Math.min(5, transformRef.current.scale * factor));
    setNodes((prev) => [...prev]);
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", background: "var(--bg-primary)", color: "var(--text-primary)", fontFamily: "'JetBrains Mono', 'Fira Code', monospace" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "6px 10px", borderBottom: "1px solid var(--border-color)", flexShrink: 0 }}>
        <span style={{ fontWeight: 700, fontSize: 12, color: "var(--accent-hover)" }}>Dependency Graph</span>
        <button onClick={loadData} style={{ background: "transparent", border: "1px solid var(--border-color)", color: "var(--text-primary)", borderRadius: 4, padding: "2px 8px", cursor: "pointer", fontSize: 11, fontFamily: "inherit" }}>
          Refresh
        </button>
        {/* Legend */}
        {["file", "external", "circular"].map((type) => (
          <span key={type} style={{ display: "flex", alignItems: "center", gap: 4, fontSize: 10, color: "var(--text-muted)" }}>
            <span style={{ display: "inline-block", width: 8, height: 8, borderRadius: "50%", background: nodeColor(type) }} />
            {type}
          </span>
        ))}
        <span style={{ fontSize: 10, color: "var(--text-muted)", marginLeft: "auto" }}>
          {loading ? "Loading…" : `${nodes.length} nodes, ${edges.length} edges`}
        </span>
      </div>

      <div style={{ flex: 1, position: "relative", overflow: "hidden" }}>
        {loading && (
          <div style={{ position: "absolute", inset: 0, display: "flex", alignItems: "center", justifyContent: "center", color: "var(--text-muted)" }}>
            Loading dependency data…
          </div>
        )}
        {!loading && error && (
          <div style={{ position: "absolute", inset: 0, display: "flex", alignItems: "center", justifyContent: "center", color: "var(--text-muted)" }}>
            {error}
          </div>
        )}
        <canvas
          ref={canvasRef}
          width={800}
          height={600}
          style={{ width: "100%", height: "100%", cursor: "grab", display: loading || error ? "none" : "block" }}
          onMouseDown={handleMouseDown}
          onMouseMove={handleMouseMove}
          onMouseUp={handleMouseUp}
          onMouseLeave={handleMouseUp}
          onWheel={handleWheel}
        />
      </div>
    </div>
  );
}
