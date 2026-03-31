import React, { useRef, useEffect, useState, useCallback, memo } from "react";

interface GlslPreviewPanelProps {
  fragmentShader?: string;
  vertexShader?: string;
}

const DEFAULT_VERTEX_SHADER = `
attribute vec2 position;
void main() {
  gl_Position = vec4(position, 0.0, 1.0);
}
`;

const DEFAULT_FRAGMENT_SHADER = `
precision mediump float;
uniform float iTime;
uniform vec2 iResolution;
void main() {
  vec2 uv = gl_FragCoord.xy / iResolution.xy;
  gl_FragColor = vec4(uv, 0.5 + 0.5 * sin(iTime), 1.0);
}
`;

const GLSL_EXTENSIONS = new Set(["glsl", "frag", "vert", "gdshader"]);

function getExt(path: string): string {
  return path.split(".").pop()?.toLowerCase() ?? "";
}

const GlslPreviewPanel = memo(function GlslPreviewPanel({ fragmentShader, vertexShader }: GlslPreviewPanelProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const glRef = useRef<WebGLRenderingContext | null>(null);
  const programRef = useRef<WebGLProgram | null>(null);
  const animFrameRef = useRef<number | null>(null);
  const startTimeRef = useRef<number>(performance.now());
  const mouseRef = useRef<[number, number, number, number]>([0, 0, 0, 0]);

  const [playing, setPlaying] = useState(true);
  const [compileErrors, setCompileErrors] = useState<string[]>([]);
  const [currentFrag, setCurrentFrag] = useState(fragmentShader ?? DEFAULT_FRAGMENT_SHADER);
  const [currentVert, setCurrentVert] = useState(vertexShader ?? DEFAULT_VERTEX_SHADER);

  // Update shaders when props change
  useEffect(() => {
    if (fragmentShader) setCurrentFrag(fragmentShader);
  }, [fragmentShader]);

  useEffect(() => {
    if (vertexShader) setCurrentVert(vertexShader);
  }, [vertexShader]);

  const buildProgram = useCallback((gl: WebGLRenderingContext, vert: string, frag: string): WebGLProgram | null => {
    const errors: string[] = [];

    const vs = (() => {
      const shader = gl.createShader(gl.VERTEX_SHADER);
      if (!shader) return null;
      gl.shaderSource(shader, vert);
      gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
        errors.push("Vertex: " + (gl.getShaderInfoLog(shader) ?? "error"));
        gl.deleteShader(shader);
        return null;
      }
      return shader;
    })();

    const fs = (() => {
      const shader = gl.createShader(gl.FRAGMENT_SHADER);
      if (!shader) return null;
      gl.shaderSource(shader, frag);
      gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) {
        errors.push("Fragment: " + (gl.getShaderInfoLog(shader) ?? "error"));
        gl.deleteShader(shader);
        return null;
      }
      return shader;
    })();

    setCompileErrors(errors);
    if (!vs || !fs) return null;

    const program = gl.createProgram();
    if (!program) return null;
    gl.attachShader(program, vs);
    gl.attachShader(program, fs);
    gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) {
      errors.push("Link: " + (gl.getProgramInfoLog(program) ?? "error"));
      setCompileErrors(errors);
      gl.deleteProgram(program);
      return null;
    }
    setCompileErrors([]);
    return program;
  }, []);

  // Initialize WebGL and quad geometry
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const gl = canvas.getContext("webgl");
    if (!gl) return;
    glRef.current = gl;

    // Quad vertices covering -1..1
    const vertices = new Float32Array([-1, -1, 1, -1, -1, 1, 1, 1]);
    const buffer = gl.createBuffer();
    gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, vertices, gl.STATIC_DRAW);

    return () => {
      if (animFrameRef.current !== null) cancelAnimationFrame(animFrameRef.current);
    };
  }, []);

  // Recompile when shaders change
  useEffect(() => {
    const gl = glRef.current;
    if (!gl) return;

    if (programRef.current) {
      gl.deleteProgram(programRef.current);
      programRef.current = null;
    }

    const program = buildProgram(gl, currentVert, currentFrag);
    programRef.current = program;
  }, [currentFrag, currentVert, buildProgram]);

  // Render loop
  useEffect(() => {
    const gl = glRef.current;
    const canvas = canvasRef.current;
    if (!gl || !canvas) return;

    const render = () => {
      const program = programRef.current;
      if (!program) {
        animFrameRef.current = requestAnimationFrame(render);
        return;
      }

      gl.viewport(0, 0, canvas.width, canvas.height);
      gl.clearColor(0, 0, 0, 1);
      gl.clear(gl.COLOR_BUFFER_BIT);

      gl.useProgram(program);

      // Setup quad
      const posLoc = gl.getAttribLocation(program, "position");
      gl.enableVertexAttribArray(posLoc);
      gl.vertexAttribPointer(posLoc, 2, gl.FLOAT, false, 0, 0);

      // Uniforms
      const timeLoc = gl.getUniformLocation(program, "iTime");
      const resLoc = gl.getUniformLocation(program, "iResolution");
      const mouseLoc = gl.getUniformLocation(program, "iMouse");

      const elapsed = playing ? (performance.now() - startTimeRef.current) / 1000 : 0;
      if (timeLoc) gl.uniform1f(timeLoc, elapsed);
      if (resLoc) gl.uniform2f(resLoc, canvas.width, canvas.height);
      if (mouseLoc) gl.uniform4fv(mouseLoc, mouseRef.current);

      gl.drawArrays(gl.TRIANGLE_STRIP, 0, 4);

      animFrameRef.current = requestAnimationFrame(render);
    };

    if (animFrameRef.current !== null) cancelAnimationFrame(animFrameRef.current);
    animFrameRef.current = requestAnimationFrame(render);

    return () => {
      if (animFrameRef.current !== null) cancelAnimationFrame(animFrameRef.current);
    };
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [playing]);

  // Pause/resume: adjust startTime
  const handlePlayPause = useCallback(() => {
    setPlaying((prev) => {
      if (!prev) {
        // Resuming: reset start time so iTime continues from where paused
        startTimeRef.current = performance.now();
      }
      return !prev;
    });
  }, []);

  // Mouse tracking
  const handleMouseMove = useCallback((e: React.MouseEvent<HTMLCanvasElement>) => {
    const rect = canvasRef.current?.getBoundingClientRect();
    if (!rect) return;
    const x = e.clientX - rect.left;
    const y = rect.height - (e.clientY - rect.top);
    mouseRef.current = [x, y, mouseRef.current[2], mouseRef.current[3]];
  }, []);

  const handleMouseDown = useCallback((e: React.MouseEvent<HTMLCanvasElement>) => {
    const rect = canvasRef.current?.getBoundingClientRect();
    if (!rect) return;
    const x = e.clientX - rect.left;
    const y = rect.height - (e.clientY - rect.top);
    mouseRef.current = [x, y, x, y];
  }, []);

  const handleMouseUp = useCallback(() => {
    mouseRef.current = [mouseRef.current[0], mouseRef.current[1], 0, 0];
  }, []);

  // Listen for saved GLSL files
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<{ path: string; content: string }>).detail;
      if (!detail?.path) return;
      const ext = getExt(detail.path);
      if (!GLSL_EXTENSIONS.has(ext)) return;
      if (ext === "vert") {
        setCurrentVert(detail.content ?? DEFAULT_VERTEX_SHADER);
      } else {
        setCurrentFrag(detail.content ?? DEFAULT_FRAGMENT_SHADER);
      }
    };
    window.addEventListener("editor-file-saved", handler);
    return () => window.removeEventListener("editor-file-saved", handler);
  }, []);

  // Listen for open-glsl-preview event
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<{ path: string; content?: string }>).detail;
      if (!detail?.path) return;
      const ext = getExt(detail.path);
      if (!GLSL_EXTENSIONS.has(ext)) return;
      if (detail.content) {
        if (ext === "vert") setCurrentVert(detail.content);
        else setCurrentFrag(detail.content);
      }
    };
    window.addEventListener("open-glsl-preview", handler);
    return () => window.removeEventListener("open-glsl-preview", handler);
  }, []);

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", background: "var(--bg-primary)", color: "var(--text-primary)", fontFamily: "'JetBrains Mono', 'Fira Code', monospace" }}>
      <div style={{ padding: "6px 10px", borderBottom: "1px solid var(--border-color)", display: "flex", alignItems: "center", gap: 8, flexShrink: 0 }}>
        <span style={{ fontWeight: 700, fontSize: 11, color: "var(--accent-hover)" }}>GLSL PREVIEW</span>
        <button
          onClick={handlePlayPause}
          style={{
            background: "var(--bg-hover)",
            border: "1px solid #45475a",
            color: "var(--text-primary)",
            borderRadius: 4,
            padding: "2px 8px",
            cursor: "pointer",
            fontSize: 11,
            fontFamily: "inherit",
          }}
          aria-label={playing ? "Pause animation" : "Play animation"}
        >
          {playing ? "⏸ Pause" : "▶ Play"}
        </button>
      </div>

      <canvas
        ref={canvasRef}
        width={600}
        height={400}
        onMouseMove={handleMouseMove}
        onMouseDown={handleMouseDown}
        onMouseUp={handleMouseUp}
        style={{ flex: 1, width: "100%", display: "block", cursor: "crosshair", minHeight: 0 }}
      />

      {compileErrors.length > 0 && (
        <div
          style={{
            background: "#2d1a1a",
            borderTop: "1px solid #f38ba8",
            padding: "8px 12px",
            flexShrink: 0,
            maxHeight: 150,
            overflowY: "auto",
          }}
        >
          <div style={{ fontSize: 10, fontWeight: 700, color: "#f38ba8", marginBottom: 4 }}>COMPILE ERRORS</div>
          {compileErrors.map((err, i) => {
            // Highlight line numbers in the error
            const lines = err.split("\n").map((line, j) => (
              <div key={j} style={{ color: line.match(/:\d+:/) ? "#fab387" : "#f38ba8", fontSize: 11 }}>
                {line}
              </div>
            ));
            return <div key={i} style={{ marginBottom: 4 }}>{lines}</div>;
          })}
        </div>
      )}
    </div>
  );
});

export default GlslPreviewPanel;
