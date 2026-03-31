import React, { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface DebugPanelProps {
  projectPath: string;
}

type DebugTab = "variables" | "callstack" | "breakpoints" | "watch";

interface DebugVariable {
  name: string;
  value: string;
  type: string;
  children?: DebugVariable[];
}

interface StackFrame {
  id: number;
  name: string;
  file: string;
  line: number;
}

interface Breakpoint {
  id: number;
  file: string;
  line: number;
  enabled: boolean;
  condition?: string;
}

interface WatchExpression {
  id: number;
  expression: string;
  value: string | null;
}

interface DebugPausedPayload {
  frames: StackFrame[];
  variables: DebugVariable[];
  reason: string;
}

interface DebugOutputPayload {
  text: string;
  category: string;
}

interface DebugBreakpointPayload {
  breakpoints: Breakpoint[];
}

export default function DebugPanel({ projectPath }: DebugPanelProps) {
  const [activeTab, setActiveTab] = useState<DebugTab>("variables");
  const [running, setRunning] = useState(false);
  const [paused, setPaused] = useState(false);
  const [frames, setFrames] = useState<StackFrame[]>([]);
  const [variables, setVariables] = useState<DebugVariable[]>([]);
  const [breakpoints, setBreakpoints] = useState<Breakpoint[]>([]);
  const [watchExpressions, setWatchExpressions] = useState<WatchExpression[]>([]);
  const [newWatchExpr, setNewWatchExpr] = useState("");
  const [output, setOutput] = useState<string[]>([]);
  const [expandedVars, setExpandedVars] = useState<Set<string>>(new Set());
  const [activeFrameId, setActiveFrameId] = useState<number | null>(null);

  // Tauri event listeners
  useEffect(() => {
    const unlistenPromises = [
      listen<DebugPausedPayload>("debug-paused", (e) => {
        setPaused(true);
        setRunning(true);
        setFrames(e.payload.frames);
        setVariables(e.payload.variables);
        if (e.payload.frames.length > 0) setActiveFrameId(e.payload.frames[0].id);
      }),
      listen<void>("debug-continued", () => {
        setPaused(false);
      }),
      listen<DebugOutputPayload>("debug-output", (e) => {
        setOutput((prev) => [...prev.slice(-500), `[${e.payload.category}] ${e.payload.text}`]);
      }),
      listen<void>("debug-stopped", () => {
        setRunning(false);
        setPaused(false);
        setFrames([]);
        setVariables([]);
      }),
      listen<DebugBreakpointPayload>("debug-breakpoints-changed", (e) => {
        setBreakpoints(e.payload.breakpoints);
      }),
    ];

    return () => {
      unlistenPromises.forEach((p) => p.then((fn) => fn()));
    };
  }, []);

  const handleLaunch = async () => {
    setRunning(true);
    setPaused(false);
    setOutput([]);
    try {
      await invoke("dap_launch", { projectPath });
    } catch {
      setRunning(false);
    }
  };

  const handleStop = async () => {
    try { await invoke("dap_stop"); } catch { /* ignore */ }
    setRunning(false);
    setPaused(false);
    setFrames([]);
    setVariables([]);
  };

  const handleContinue = async () => {
    try { await invoke("dap_continue"); } catch { /* ignore */ }
    setPaused(false);
  };

  const handlePause = async () => {
    try { await invoke("dap_pause"); } catch { /* ignore */ }
  };

  const handleStepOver = async () => {
    try { await invoke("dap_step_over"); } catch { /* ignore */ }
  };

  const handleStepInto = async () => {
    try { await invoke("dap_step_into"); } catch { /* ignore */ }
  };

  const handleStepOut = async () => {
    try { await invoke("dap_step_out"); } catch { /* ignore */ }
  };

  const handleToggleBreakpoint = async (bp: Breakpoint) => {
    const updated = { ...bp, enabled: !bp.enabled };
    setBreakpoints((prev) => prev.map((b) => (b.id === bp.id ? updated : b)));
    try {
      await invoke("dap_toggle_breakpoint", { id: bp.id, enabled: updated.enabled });
    } catch { /* ignore */ }
  };

  const handleRemoveBreakpoint = async (id: number) => {
    setBreakpoints((prev) => prev.filter((b) => b.id !== id));
    try { await invoke("dap_remove_breakpoint", { id }); } catch { /* ignore */ }
  };

  const handleAddWatch = useCallback(async () => {
    if (!newWatchExpr.trim()) return;
    const expr = newWatchExpr.trim();
    const id = Date.now();
    let value: string | null = null;
    try {
      value = await invoke<string>("dap_evaluate", { expression: expr });
    } catch { /* ignore */ }
    setWatchExpressions((prev) => [...prev, { id, expression: expr, value }]);
    setNewWatchExpr("");
  }, [newWatchExpr]);

  const handleRemoveWatch = (id: number) => {
    setWatchExpressions((prev) => prev.filter((w) => w.id !== id));
  };

  const toggleExpandVar = (path: string) => {
    setExpandedVars((prev) => {
      const next = new Set(prev);
      next.has(path) ? next.delete(path) : next.add(path);
      return next;
    });
  };

  const renderVariable = (v: DebugVariable, depth = 0, pathPrefix = ""): React.ReactNode => {
    const path = `${pathPrefix}/${v.name}`;
    const hasChildren = v.children && v.children.length > 0;
    const expanded = expandedVars.has(path);
    return (
      <div key={path}>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            padding: `2px 8px 2px ${8 + depth * 12}px`,
            cursor: hasChildren ? "pointer" : "default",
            fontSize: 12,
          }}
          onClick={() => hasChildren && toggleExpandVar(path)}
        >
          {hasChildren && (
            <span style={{ marginRight: 4, fontSize: 9, opacity: 0.6 }}>{expanded ? "▼" : "▶"}</span>
          )}
          {!hasChildren && <span style={{ marginRight: 12 }} />}
          <span style={{ color: "var(--accent-hover)", marginRight: 6 }}>{v.name}</span>
          <span style={{ color: "var(--text-muted)", marginRight: 6, fontSize: 10 }}>{v.type}</span>
          <span style={{ color: "#a6e3a1" }}>{v.value}</span>
        </div>
        {expanded && hasChildren && v.children!.map((child) => renderVariable(child, depth + 1, path))}
      </div>
    );
  };

  const controlBtn = (label: string, title: string, onClick: () => void, disabled: boolean, color = "var(--accent-hover)"): React.ReactNode => (
    <button
      title={title}
      aria-label={title}
      onClick={onClick}
      disabled={disabled}
      style={{
        background: disabled ? "var(--bg-hover)" : color,
        color: disabled ? "var(--text-muted)" : "var(--bg-primary)",
        border: "none",
        borderRadius: 4,
        padding: "3px 8px",
        cursor: disabled ? "not-allowed" : "pointer",
        fontSize: 13,
        fontFamily: "inherit",
        fontWeight: 600,
      }}
    >
      {label}
    </button>
  );

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", fontFamily: "'JetBrains Mono', 'Fira Code', monospace", fontSize: 13, color: "var(--text-primary)", background: "var(--bg-primary)" }}>
      {/* Controls */}
      <div style={{ display: "flex", alignItems: "center", gap: 4, padding: "6px 8px", borderBottom: "1px solid var(--border-color)", flexWrap: "wrap" }}>
        {!running
          ? controlBtn("▶ Run", "Launch debugger", handleLaunch, false, "#3fb950")
          : controlBtn("■ Stop", "Stop debugger", handleStop, false, "#f38ba8")}
        {controlBtn(paused ? "▶ Continue" : "⏸ Pause", paused ? "Continue" : "Pause", paused ? handleContinue : handlePause, !running)}
        {controlBtn("⤵ Over", "Step Over", handleStepOver, !paused)}
        {controlBtn("⤷ Into", "Step Into", handleStepInto, !paused)}
        {controlBtn("⤴ Out", "Step Out", handleStepOut, !paused)}
      </div>

      {/* Tabs */}
      <div role="tablist" style={{ display: "flex", borderBottom: "1px solid var(--border-color)", flexShrink: 0 }}>
        {(["variables", "callstack", "breakpoints", "watch"] as DebugTab[]).map((tab) => (
          <button
            key={tab}
            role="tab"
            aria-selected={activeTab === tab}
            aria-controls={`debug-panel-${tab}`}
            id={`debug-tab-${tab}`}
            onClick={() => setActiveTab(tab)}
            style={{
              background: activeTab === tab ? "var(--bg-hover)" : "transparent",
              color: activeTab === tab ? "var(--text-primary)" : "var(--text-muted)",
              border: "none",
              borderBottom: activeTab === tab ? "2px solid #89b4fa" : "2px solid transparent",
              padding: "5px 10px",
              cursor: "pointer",
              fontSize: 11,
              fontFamily: "inherit",
              textTransform: "capitalize",
            }}
          >
            {tab === "callstack" ? "Call Stack" : tab.charAt(0).toUpperCase() + tab.slice(1)}
          </button>
        ))}
      </div>

      {/* Tab content */}
      <div role="tabpanel" id={`debug-panel-${activeTab}`} aria-labelledby={`debug-tab-${activeTab}`} style={{ flex: 1, overflowY: "auto" }}>
        {activeTab === "variables" && (
          <div>
            {variables.length === 0 && (
              <div style={{ padding: 12, color: "var(--text-muted)", fontSize: 12 }}>
                {paused ? "No variables" : "Not paused"}
              </div>
            )}
            {variables.map((v) => renderVariable(v))}
          </div>
        )}

        {activeTab === "callstack" && (
          <div>
            {frames.length === 0 && (
              <div style={{ padding: 12, color: "var(--text-muted)", fontSize: 12 }}>
                {paused ? "No frames" : "Not paused"}
              </div>
            )}
            {frames.map((frame) => (
              <div
                key={frame.id}
                onClick={() => setActiveFrameId(frame.id)}
                style={{
                  padding: "5px 10px",
                  cursor: "pointer",
                  background: activeFrameId === frame.id ? "var(--bg-hover)" : "transparent",
                  borderBottom: "1px solid #181825",
                }}
              >
                <div style={{ color: "var(--text-primary)" }}>{frame.name}</div>
                <div style={{ color: "var(--text-muted)", fontSize: 11 }}>{frame.file}:{frame.line}</div>
              </div>
            ))}
          </div>
        )}

        {activeTab === "breakpoints" && (
          <div>
            {breakpoints.length === 0 && (
              <div style={{ padding: 12, color: "var(--text-muted)", fontSize: 12 }}>No breakpoints set</div>
            )}
            {breakpoints.map((bp) => (
              <div key={bp.id} style={{ display: "flex", alignItems: "center", padding: "4px 8px", gap: 6, borderBottom: "1px solid #181825" }}>
                <input
                  type="checkbox"
                  checked={bp.enabled}
                  onChange={() => handleToggleBreakpoint(bp)}
                  style={{ cursor: "pointer" }}
                />
                <span style={{ flex: 1, fontSize: 12, color: bp.enabled ? "var(--text-primary)" : "var(--text-muted)" }}>
                  {bp.file.split("/").pop()}:{bp.line}
                  {bp.condition && <span style={{ color: "#fab387", marginLeft: 4 }}>({bp.condition})</span>}
                </span>
                <button
                  onClick={() => handleRemoveBreakpoint(bp.id)}
                  style={{ background: "transparent", border: "none", color: "#f38ba8", cursor: "pointer", fontSize: 14, padding: "0 2px" }}
                >×</button>
              </div>
            ))}
          </div>
        )}

        {activeTab === "watch" && (
          <div>
            <div style={{ padding: "6px 8px", borderBottom: "1px solid var(--border-color)", display: "flex", gap: 4 }}>
              <input
                type="text"
                placeholder="Expression…"
                value={newWatchExpr}
                onChange={(e) => setNewWatchExpr(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter") handleAddWatch(); }}
                style={{
                  flex: 1,
                  background: "var(--bg-primary)",
                  border: "1px solid var(--border-color)",
                  borderRadius: 4,
                  color: "var(--text-primary)",
                  padding: "3px 6px",
                  fontSize: 12,
                  fontFamily: "inherit",
                  outline: "none",
                }}
              />
              <button
                onClick={handleAddWatch}
                style={{ background: "var(--accent-hover)", color: "var(--bg-primary)", border: "none", borderRadius: 4, padding: "3px 8px", cursor: "pointer", fontSize: 12, fontFamily: "inherit" }}
              >+</button>
            </div>
            {watchExpressions.map((w) => (
              <div key={w.id} style={{ display: "flex", alignItems: "center", padding: "4px 8px", gap: 6, borderBottom: "1px solid #181825" }}>
                <span style={{ flex: 1, fontSize: 12 }}>
                  <span style={{ color: "var(--accent-hover)" }}>{w.expression}</span>
                  <span style={{ color: "var(--text-muted)" }}> = </span>
                  <span style={{ color: "#a6e3a1" }}>{w.value ?? "—"}</span>
                </span>
                <button
                  onClick={() => handleRemoveWatch(w.id)}
                  style={{ background: "transparent", border: "none", color: "#f38ba8", cursor: "pointer", fontSize: 14, padding: "0 2px" }}
                >×</button>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Debug output */}
      {output.length > 0 && (
        <div style={{ borderTop: "1px solid var(--border-color)", maxHeight: 120, overflowY: "auto", background: "var(--bg-primary)", padding: "4px 8px", fontSize: 11, fontFamily: "inherit", color: "var(--text-secondary)" }}>
          {output.map((line, i) => <div key={i}>{line}</div>)}
        </div>
      )}
    </div>
  );
}
