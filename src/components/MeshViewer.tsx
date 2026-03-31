import React, { useEffect, useRef, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

interface Vec3 { x: number; y: number; z: number; }
interface MeshViewerProps { filePath: string; }

interface ParsedMesh {
  vertices: Vec3[];
  faces: number[][];
}

function parseObj(content: string): ParsedMesh {
  const vertices: Vec3[] = [];
  const faces: number[][] = [];

  for (const line of content.split("\n")) {
    const parts = line.trim().split(/\s+/);
    if (parts[0] === "v") {
      vertices.push({
        x: parseFloat(parts[1]) || 0,
        y: parseFloat(parts[2]) || 0,
        z: parseFloat(parts[3]) || 0,
      });
    } else if (parts[0] === "f") {
      // Face indices: 1-based, support v/vt/vn notation
      const indices = parts.slice(1).map((p) => {
        const idx = parseInt(p.split("/")[0]);
        return idx > 0 ? idx - 1 : vertices.length + idx;
      });
      if (indices.length >= 3) {
        // Fan triangulation
        for (let i = 1; i < indices.length - 1; i++) {
          faces.push([indices[0], indices[i], indices[i + 1]]);
        }
      }
    }
  }

  return { vertices, faces };
}

function rotateX(v: Vec3, angle: number): Vec3 {
  const c = Math.cos(angle);
  const s = Math.sin(angle);
  return { x: v.x, y: v.y * c - v.z * s, z: v.y * s + v.z * c };
}

function rotateY(v: Vec3, angle: number): Vec3 {
  const c = Math.cos(angle);
  const s = Math.sin(angle);
  return { x: v.x * c + v.z * s, y: v.y, z: -v.x * s + v.z * c };
}

function project(v: Vec3, cx: number, cy: number, dist: number, scale: number): [number, number] {
  const d = v.z + dist;
  if (d <= 0) return [cx, cy];
  const sx = cx + (v.x / d) * scale;
  const sy = cy - (v.y / d) * scale;
  return [sx, sy];
}

export default function MeshViewer({ filePath }: MeshViewerProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const animRef = useRef<number>(0);
  const meshRef = useRef<ParsedMesh | null>(null);
  const angleRef = useRef({ x: 0.3, y: 0 });
  const autoRotateRef = useRef(true);
  const dragRef = useRef<{ active: boolean; lastX: number; lastY: number }>({ active: false, lastX: 0, lastY: 0 });

  const [status, setStatus] = useState<string>("Loading...");
  const [meshInfo, setMeshInfo] = useState<string>("");
  const ext = filePath.split(".").pop()?.toLowerCase();

  useEffect(() => {
    if (ext === "fbx") {
      setStatus("FBX preview not supported — convert to .obj first");
      return;
    }

    invoke<string>("read_file_content", { path: filePath })
      .then((content) => {
        if (ext === "obj") {
          const mesh = parseObj(content);
          meshRef.current = mesh;
          setMeshInfo(`${mesh.vertices.length} vertices, ${mesh.faces.length} faces`);
          setStatus("ok");
        } else if (ext === "gltf") {
          try {
            const gltf = JSON.parse(content);
            const meshCount = gltf?.meshes?.length ?? 0;
            setStatus(`GLTF loaded (${meshCount} mesh${meshCount !== 1 ? "es" : ""})`);
            // Try to extract vertices from first mesh if bufferViews are inline
            // For most GLTF, binary buffers aren't inline so we just show info
            setMeshInfo(`Primitives: ${gltf?.meshes?.[0]?.primitives?.length ?? 0}`);
          } catch {
            setStatus("Failed to parse GLTF JSON.");
          }
        }
      })
      .catch((e) => setStatus("Error: " + String(e)));
  }, [filePath, ext]);

  // Normalize mesh to fit in a -1..1 box
  const normalizedVertices = useCallback((): Vec3[] => {
    const mesh = meshRef.current;
    if (!mesh || mesh.vertices.length === 0) return [];

    let minX = Infinity, maxX = -Infinity;
    let minY = Infinity, maxY = -Infinity;
    let minZ = Infinity, maxZ = -Infinity;

    for (const v of mesh.vertices) {
      if (v.x < minX) minX = v.x;
      if (v.x > maxX) maxX = v.x;
      if (v.y < minY) minY = v.y;
      if (v.y > maxY) maxY = v.y;
      if (v.z < minZ) minZ = v.z;
      if (v.z > maxZ) maxZ = v.z;
    }

    const cx = (minX + maxX) / 2;
    const cy = (minY + maxY) / 2;
    const cz = (minZ + maxZ) / 2;
    const scale = Math.max(maxX - minX, maxY - minY, maxZ - minZ) || 1;

    return mesh.vertices.map((v) => ({
      x: (v.x - cx) / scale * 2,
      y: (v.y - cy) / scale * 2,
      z: (v.z - cz) / scale * 2,
    }));
  }, []);

  // Render loop
  useEffect(() => {
    if (status !== "ok") return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const draw = () => {
      const w = canvas.width;
      const h = canvas.height;
      ctx.fillStyle = "#0d0d1a";
      ctx.fillRect(0, 0, w, h);

      const verts = normalizedVertices();
      if (!verts.length || !meshRef.current) {
        animRef.current = requestAnimationFrame(draw);
        return;
      }

      const { x: ax, y: ay } = angleRef.current;
      if (autoRotateRef.current) {
        angleRef.current.y += 0.008;
      }

      const dist = 4;
      const scale = Math.min(w, h) * 0.38;
      const cx = w / 2;
      const cy = h / 2;

      const transformed = verts.map((v) => {
        const r1 = rotateX(v, ax);
        const r2 = rotateY(r1, ay);
        return r2;
      });

      ctx.strokeStyle = "#00e5ff";
      ctx.lineWidth = 0.5;
      ctx.globalAlpha = 0.7;

      for (const face of meshRef.current.faces) {
        if (face.length < 3) continue;
        const [ax2, ay2] = project(transformed[face[0]], cx, cy, dist, scale);
        ctx.beginPath();
        ctx.moveTo(ax2, ay2);
        for (let i = 1; i < face.length; i++) {
          const [px, py] = project(transformed[face[i]], cx, cy, dist, scale);
          ctx.lineTo(px, py);
        }
        ctx.closePath();
        ctx.stroke();
      }

      ctx.globalAlpha = 1;
      animRef.current = requestAnimationFrame(draw);
    };

    animRef.current = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(animRef.current);
  }, [status, normalizedVertices]);

  // Mouse drag
  const onMouseDown = (e: React.MouseEvent) => {
    dragRef.current = { active: true, lastX: e.clientX, lastY: e.clientY };
    autoRotateRef.current = false;
  };
  const onMouseMove = (e: React.MouseEvent) => {
    if (!dragRef.current.active) return;
    const dx = e.clientX - dragRef.current.lastX;
    const dy = e.clientY - dragRef.current.lastY;
    angleRef.current.y += dx * 0.01;
    angleRef.current.x += dy * 0.01;
    dragRef.current.lastX = e.clientX;
    dragRef.current.lastY = e.clientY;
  };
  const onMouseUp = () => {
    dragRef.current.active = false;
  };

  if (ext === "fbx") {
    return (
      <div style={{ height: "100%", display: "flex", alignItems: "center", justifyContent: "center", background: "#0d0d1a", color: "#6c7086", fontFamily: "monospace", fontSize: 13 }}>
        FBX preview not supported — convert to .obj first
      </div>
    );
  }

  if (ext === "gltf" && status !== "ok") {
    return (
      <div style={{ height: "100%", display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", background: "#0d0d1a", color: "#a6adc8", fontFamily: "monospace", gap: 8 }}>
        <div style={{ fontSize: 32 }}>🗺️</div>
        <div style={{ fontSize: 13, color: "#89b4fa" }}>{status}</div>
        {meshInfo && <div style={{ fontSize: 11, color: "#6c7086" }}>{meshInfo}</div>}
        <div style={{ fontSize: 11, color: "#45475a", marginTop: 8 }}>{filePath.split("/").pop()}</div>
      </div>
    );
  }

  if (status !== "ok") {
    return (
      <div style={{ height: "100%", display: "flex", alignItems: "center", justifyContent: "center", background: "#0d0d1a", color: "#6c7086", fontFamily: "monospace", fontSize: 13 }}>
        {status}
      </div>
    );
  }

  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column", background: "#0d0d1a" }}>
      <div style={{ display: "flex", alignItems: "center", padding: "4px 10px", borderBottom: "1px solid #1a1a2e", flexShrink: 0 }}>
        <span style={{ color: "#00e5ff", fontFamily: "monospace", fontSize: 11, flex: 1 }}>
          {filePath.split("/").pop()} — Wireframe Preview
        </span>
        <span style={{ color: "#45475a", fontSize: 10, fontFamily: "monospace" }}>{meshInfo}</span>
        <button
          onClick={() => { autoRotateRef.current = !autoRotateRef.current; }}
          style={{ background: "transparent", border: "1px solid #1a1a2e", borderRadius: 3, color: "#6c7086", padding: "2px 8px", fontSize: 10, cursor: "pointer", marginLeft: 8 }}
        >
          Auto-rotate
        </button>
      </div>
      <canvas
        ref={canvasRef}
        width={600}
        height={400}
        style={{ flex: 1, width: "100%", cursor: "grab" }}
        onMouseDown={onMouseDown}
        onMouseMove={onMouseMove}
        onMouseUp={onMouseUp}
        onMouseLeave={onMouseUp}
      />
    </div>
  );
}
