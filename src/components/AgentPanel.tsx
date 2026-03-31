import { useEffect, useState, useCallback, memo } from "react";
import { listen, emit } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";

type AgentStatus = "running" | "done" | "error";

interface ToolCallEntry {
  name: string;
  argsSummary: string;
  success?: boolean;
}

interface AgentTask {
  taskId: string;
  step: number;
  total: number;
  description: string;
  status: AgentStatus;
  startTime: number;
  endTime?: number;
  toolCalls: ToolCallEntry[];
  expanded: boolean;
}

interface AgentProgressEvent {
  taskId: string;
  step: number;
  total: number;
  description: string;
  status: AgentStatus;
  tool?: string;
  args?: string;
  toolSuccess?: boolean;
  agentId?: string;
}

// Multi-agent card state
interface MultiAgentCard {
  id: string;
  label: string;
  accentColor: string;
  status: "idle" | "running" | "paused" | "done" | "cancelled";
  progress: number;
  toolLog: string[];
  paused: boolean;
}

// Agent template type
interface AgentTemplate {
  id: string;
  name: string;
  description: string;
  category: string;
  steps: string[];
  prompt: string;
}

interface AgentPanelProps {
  visible?: boolean;
}

const MAX_COMPLETED = 10;

const HARDCODED_TEMPLATES: AgentTemplate[] = [
  {
    id: "full-stack-feature",
    name: "Full-Stack Feature",
    category: "development",
    description: "Implement a full-stack feature with backend API and frontend UI.",
    steps: ["Analyze requirements", "Create backend route", "Add database schema", "Build frontend component", "Write tests"],
    prompt: "Implement a full-stack feature: ",
  },
  {
    id: "code-review",
    name: "Code Review",
    category: "review",
    description: "Review the codebase for bugs, performance issues, and code quality.",
    steps: ["Scan files", "Identify issues", "Suggest improvements", "Generate report"],
    prompt: "Review the code in this project for issues and improvements.",
  },
  {
    id: "refactor",
    name: "Refactor Module",
    category: "refactoring",
    description: "Refactor a module to improve structure and readability.",
    steps: ["Analyze module", "Extract functions", "Rename symbols", "Update imports", "Run tests"],
    prompt: "Refactor the following module: ",
  },
  {
    id: "test-generation",
    name: "Generate Tests",
    category: "testing",
    description: "Generate comprehensive test coverage for a file or module.",
    steps: ["Analyze source", "Identify test cases", "Write unit tests", "Write integration tests"],
    prompt: "Generate tests for: ",
  },
  {
    id: "bug-fix",
    name: "Bug Fix",
    category: "debugging",
    description: "Diagnose and fix a reported bug.",
    steps: ["Reproduce issue", "Identify root cause", "Apply fix", "Verify fix", "Add regression test"],
    prompt: "Fix the following bug: ",
  },
  {
    id: "documentation",
    name: "Add Documentation",
    category: "docs",
    description: "Add comprehensive documentation and comments to a file.",
    steps: ["Read source", "Write module docs", "Document functions", "Add examples"],
    prompt: "Add documentation to: ",
  },
];

const AgentPanel = memo(function AgentPanel({ visible = true }: AgentPanelProps) {
  const [tasks, setTasks] = useState<Map<string, AgentTask>>(new Map());
  const [completedTasks, setCompletedTasks] = useState<AgentTask[]>([]);

  // Multi-agent state
  const [multiAgents, setMultiAgents] = useState<MultiAgentCard[]>([
    { id: "backend", label: "Backend Agent", accentColor: "var(--accent-hover)", status: "idle", progress: 0, toolLog: [], paused: false },
    { id: "frontend", label: "Frontend Agent", accentColor: "#a6e3a1", status: "idle", progress: 0, toolLog: [], paused: false },
  ]);
  const [fullStackMode, setFullStackMode] = useState(false);
  const [agentPrompt, setAgentPrompt] = useState("");

  // Templates modal
  const [showTemplates, setShowTemplates] = useState(false);
  const [templates, setTemplates] = useState<AgentTemplate[]>(HARDCODED_TEMPLATES);
  const [expandedTemplate, setExpandedTemplate] = useState<string | null>(null);

  const updateTask = useCallback((e: AgentProgressEvent) => {
    // Route to multi-agent cards if agentId is present
    if (e.agentId) {
      setMultiAgents((prev) =>
        prev.map((card) => {
          if (card.id !== e.agentId) return card;
          const newLog = e.tool
            ? [...card.toolLog.slice(-19), `${e.tool}: ${e.args ?? ""}`]
            : card.toolLog;
          const progress = e.total > 0 ? Math.round((e.step / e.total) * 100) : card.progress;
          const status: MultiAgentCard["status"] =
            e.status === "done" ? "done"
              : e.status === "error" ? "idle"
                : "running";
          return { ...card, progress, toolLog: newLog, status };
        })
      );
      return;
    }

    setTasks((prev) => {
      const next = new Map(prev);
      const existing = next.get(e.taskId);

      if (e.status === "done" || e.status === "error") {
        const task: AgentTask = existing ?? {
          taskId: e.taskId, step: e.step, total: e.total,
          description: e.description, status: e.status,
          startTime: Date.now(), toolCalls: [], expanded: false,
        };
        task.step = e.step;
        task.status = e.status;
        task.endTime = Date.now();
        next.delete(e.taskId);
        setCompletedTasks((prev2) => [task, ...prev2].slice(0, MAX_COMPLETED));
        return next;
      }

      const updated: AgentTask = existing ?? {
        taskId: e.taskId, step: 0, total: e.total,
        description: e.description, status: "running",
        startTime: Date.now(), toolCalls: [], expanded: false,
      };

      updated.step = e.step;
      updated.total = e.total;
      updated.description = e.description;
      updated.status = e.status;

      if (e.tool) {
        updated.toolCalls = [
          ...updated.toolCalls,
          { name: e.tool, argsSummary: e.args ? e.args.slice(0, 80) : "", success: e.toolSuccess },
        ];
      }

      next.set(e.taskId, { ...updated });
      return next;
    });
  }, []);

  useEffect(() => {
    const unsub = listen<AgentProgressEvent>("agent-progress", (e) => {
      updateTask(e.payload);
    });
    return () => { unsub.then((fn) => fn()); };
  }, [updateTask]);

  // Load templates from backend (fallback to hardcoded)
  useEffect(() => {
    invoke<AgentTemplate[]>("list_agent_templates")
      .then((tpl) => { if (tpl && tpl.length > 0) setTemplates(tpl); })
      .catch(() => { /* use hardcoded */ });
  }, []);

  const toggleExpand = (taskId: string) => {
    setTasks((prev) => {
      const next = new Map(prev);
      const t = next.get(taskId);
      if (t) next.set(taskId, { ...t, expanded: !t.expanded });
      return next;
    });
  };

  const pauseTask = async (taskId: string) => { await emit("agent-pause", { taskId }); };
  const resumeTask = async (taskId: string) => { await emit("agent-resume", { taskId }); };
  const cancelTask = async (taskId: string) => { await emit("agent-cancel", { taskId }); };

  const pauseAgent = async (agentId: string) => {
    setMultiAgents((prev) => prev.map((c) => c.id === agentId ? { ...c, paused: true, status: "paused" } : c));
    await emit("agent-pause", { taskId: agentId });
  };
  const resumeAgent = async (agentId: string) => {
    setMultiAgents((prev) => prev.map((c) => c.id === agentId ? { ...c, paused: false, status: "running" } : c));
    await emit("agent-resume", { taskId: agentId });
  };
  const cancelAgent = async (agentId: string) => {
    setMultiAgents((prev) => prev.map((c) => c.id === agentId ? { ...c, status: "cancelled", progress: 0 } : c));
    await emit("agent-cancel", { taskId: agentId });
  };

  const runTwoAgents = async () => {
    const backendPrompt = fullStackMode
      ? `Backend: ${agentPrompt}`
      : agentPrompt;
    const frontendPrompt = fullStackMode
      ? `Frontend: ${agentPrompt}`
      : agentPrompt;

    setMultiAgents((prev) => prev.map((c) => ({ ...c, status: "running", progress: 0, toolLog: [] })));

    try {
      await invoke("run_agent", { taskId: "backend", prompt: backendPrompt, agentId: "backend" });
    } catch { /* stub */ }
    try {
      await invoke("run_agent", { taskId: "frontend", prompt: frontendPrompt, agentId: "frontend" });
    } catch { /* stub */ }
  };

  const useTemplate = (tpl: AgentTemplate) => {
    setAgentPrompt(tpl.prompt);
    setShowTemplates(false);
  };

  const formatDuration = (start: number, end?: number) => {
    const ms = (end ?? Date.now()) - start;
    if (ms < 1000) return `${ms}ms`;
    return `${(ms / 1000).toFixed(1)}s`;
  };

  const progressPct = (step: number, total: number) =>
    total > 0 ? Math.min(100, Math.round((step / total) * 100)) : 0;

  if (!visible) return null;

  const activeTasks = Array.from(tasks.values());

  const categoryColors: Record<string, string> = {
    development: "var(--accent-hover)",
    review: "#f9e2af",
    refactoring: "#a6e3a1",
    testing: "#fab387",
    debugging: "#f38ba8",
    docs: "var(--accent)",
  };

  return (
    <div style={{ background: "var(--bg-primary)", color: "var(--text-primary)", height: "100%", overflow: "auto", padding: "8px", fontFamily: "monospace", fontSize: "12px" }}>
      <div style={{ fontWeight: "bold", color: "var(--accent-hover)", marginBottom: "8px" }}>
        Agent Tasks
      </div>

      {/* Multi-agent section */}
      <div style={{ marginBottom: 12, background: "var(--bg-primary)", borderRadius: 6, padding: 8, border: "1px solid var(--border-color)" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 8 }}>
          <span style={{ fontWeight: "bold", color: "var(--accent)", fontSize: 11 }}>Multi-Agent</span>
          <label style={{ display: "flex", alignItems: "center", gap: 4, fontSize: 10, color: "var(--text-secondary)", marginLeft: "auto", cursor: "pointer" }}>
            <input
              type="checkbox"
              checked={fullStackMode}
              onChange={(e) => setFullStackMode(e.target.checked)}
              style={{ accentColor: "var(--accent-hover)" }}
            />
            Full-stack mode
          </label>
        </div>

        <div style={{ display: "flex", gap: 4, marginBottom: 6 }}>
          <input
            value={agentPrompt}
            onChange={(e) => setAgentPrompt(e.target.value)}
            placeholder="Describe the task..."
            style={{
              flex: 1, background: "var(--bg-hover)", border: "1px solid var(--border-color)", borderRadius: 4,
              color: "var(--text-primary)", padding: "4px 8px", fontSize: 11, outline: "none",
            }}
          />
          <button
            onClick={() => setShowTemplates(true)}
            style={{ background: "var(--bg-hover)", border: "1px solid var(--border-color)", borderRadius: 4, color: "var(--accent)", padding: "4px 8px", fontSize: 11, cursor: "pointer", flexShrink: 0 }}
          >
            Templates
          </button>
        </div>

        <button
          onClick={runTwoAgents}
          style={{ width: "100%", background: "var(--accent)", color: "var(--bg-primary)", border: "none", borderRadius: 4, padding: "5px 0", fontSize: 11, fontWeight: 700, cursor: "pointer", marginBottom: 8 }}
        >
          Run Two Agents
        </button>

        {/* Two agent cards side by side */}
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 6 }}>
          {multiAgents.map((card) => (
            <div key={card.id} style={{ background: "var(--bg-hover)", borderRadius: 5, padding: 6, border: `1px solid ${card.accentColor}33` }}>
              <div style={{ display: "flex", alignItems: "center", gap: 4, marginBottom: 4 }}>
                <span style={{ width: 6, height: 6, borderRadius: "50%", background: card.status === "running" ? card.accentColor : card.status === "done" ? "#a6e3a1" : "var(--text-muted)", flexShrink: 0 }} />
                <span style={{ color: card.accentColor, fontWeight: "bold", fontSize: 10, flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{card.label}</span>
              </div>
              {/* Progress bar */}
              <div style={{ background: "var(--bg-hover)", borderRadius: 2, height: 3, marginBottom: 4 }}>
                <div style={{ background: card.accentColor, borderRadius: 2, height: "100%", width: `${card.progress}%`, transition: "width 0.3s" }} />
              </div>
              <div style={{ fontSize: 9, color: "var(--text-muted)", marginBottom: 4, maxHeight: 40, overflowY: "auto" }}>
                {card.toolLog.slice(-3).map((log, i) => (
                  <div key={i} style={{ overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{log}</div>
                ))}
              </div>
              <div style={{ display: "flex", gap: 2 }}>
                <button onClick={() => pauseAgent(card.id)} disabled={card.status !== "running"} style={{ background: "transparent", border: "none", color: "#f9e2af", cursor: "pointer", fontSize: 10, opacity: card.status !== "running" ? 0.4 : 1 }}>⏸</button>
                <button onClick={() => resumeAgent(card.id)} disabled={card.status !== "paused"} style={{ background: "transparent", border: "none", color: card.accentColor, cursor: "pointer", fontSize: 10, opacity: card.status !== "paused" ? 0.4 : 1 }}>▶</button>
                <button onClick={() => cancelAgent(card.id)} disabled={card.status === "idle"} style={{ background: "transparent", border: "none", color: "#f38ba8", cursor: "pointer", fontSize: 10, opacity: card.status === "idle" ? 0.4 : 1 }}>✕</button>
              </div>
            </div>
          ))}
        </div>
      </div>

      {activeTasks.length === 0 && completedTasks.length === 0 && (
        <div style={{ color: "var(--text-muted)", padding: "8px" }}>No agent tasks running.</div>
      )}

      {/* Active tasks */}
      {activeTasks.map((task) => (
        <div key={task.taskId} style={{ background: "var(--bg-hover)", borderRadius: "6px", padding: "8px", marginBottom: "8px", border: "1px solid var(--border-color)" }}>
          <div style={{ display: "flex", alignItems: "center", gap: "6px", marginBottom: "4px" }}>
            <span style={{ width: "8px", height: "8px", borderRadius: "50%", background: "#fab387", animation: "pulse 1s infinite", flexShrink: 0 }} />
            <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", color: "var(--text-primary)" }}>{task.description}</span>
            <span style={{ color: "var(--text-muted)", fontSize: "10px" }}>{formatDuration(task.startTime)}</span>
          </div>
          <div style={{ background: "var(--bg-hover)", borderRadius: "3px", height: "4px", marginBottom: "6px" }}>
            <div style={{ background: "var(--accent-hover)", borderRadius: "3px", height: "100%", width: `${progressPct(task.step, task.total)}%`, transition: "width 0.3s ease" }} />
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: "4px", fontSize: "10px", color: "var(--text-muted)" }}>
            <span>Step {task.step}/{task.total}</span>
            <span style={{ marginLeft: "auto" }} />
            <button onClick={() => toggleExpand(task.taskId)} style={{ background: "transparent", border: "none", color: "var(--text-secondary)", cursor: "pointer", fontSize: "10px" }}>
              {task.expanded ? "Hide logs" : "Show logs"}
            </button>
            <button onClick={() => pauseTask(task.taskId)} style={{ background: "transparent", border: "none", color: "#f9e2af", cursor: "pointer", fontSize: "10px" }}>⏸</button>
            <button onClick={() => resumeTask(task.taskId)} style={{ background: "transparent", border: "none", color: "#a6e3a1", cursor: "pointer", fontSize: "10px" }}>▶</button>
            <button onClick={() => cancelTask(task.taskId)} style={{ background: "transparent", border: "none", color: "#f38ba8", cursor: "pointer", fontSize: "10px" }}>✕</button>
          </div>
          {task.expanded && task.toolCalls.length > 0 && (
            <div style={{ marginTop: "6px", borderTop: "1px solid #45475a", paddingTop: "6px" }}>
              {task.toolCalls.map((tc, i) => (
                <div key={i} style={{ display: "flex", alignItems: "center", gap: "6px", padding: "2px 0", fontSize: "11px" }}>
                  <span style={{ color: tc.success === false ? "#f38ba8" : "#a6e3a1" }}>
                    {tc.success === false ? "✗" : tc.success === true ? "✓" : "◐"}
                  </span>
                  <span style={{ color: "var(--accent-hover)" }}>{tc.name}</span>
                  <span style={{ color: "var(--text-muted)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{tc.argsSummary}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      ))}

      {/* Completed tasks */}
      {completedTasks.length > 0 && (
        <div style={{ marginTop: "12px" }}>
          <div style={{ color: "var(--text-muted)", fontSize: "11px", marginBottom: "6px" }}>Recent ({completedTasks.length})</div>
          {completedTasks.map((task, i) => (
            <div key={i} style={{ display: "flex", alignItems: "center", gap: "6px", padding: "4px 6px", background: "var(--bg-primary)", borderRadius: "4px", marginBottom: "2px", fontSize: "11px" }}>
              <span style={{ color: task.status === "error" ? "#f38ba8" : "#a6e3a1" }}>
                {task.status === "error" ? "✗" : "✓"}
              </span>
              <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", color: "var(--text-secondary)" }}>{task.description}</span>
              <span style={{ color: "var(--text-muted)", fontSize: "10px", flexShrink: 0 }}>{formatDuration(task.startTime, task.endTime)}</span>
            </div>
          ))}
        </div>
      )}

      {/* Templates Modal */}
      {showTemplates && (
        <div style={{
          position: "fixed", inset: 0, background: "rgba(0,0,0,0.7)", zIndex: 9999,
          display: "flex", alignItems: "center", justifyContent: "center",
        }}
          onClick={() => setShowTemplates(false)}
        >
          <div
            style={{ background: "var(--bg-primary)", border: "1px solid var(--border-color)", borderRadius: 8, width: 440, maxHeight: "70vh", overflow: "hidden", display: "flex", flexDirection: "column" }}
            onClick={(e) => e.stopPropagation()}
          >
            <div style={{ display: "flex", alignItems: "center", padding: "10px 14px", borderBottom: "1px solid var(--border-color)" }}>
              <span style={{ fontWeight: "bold", color: "var(--accent-hover)", flex: 1 }}>Agent Templates</span>
              <button onClick={() => setShowTemplates(false)} style={{ background: "transparent", border: "none", color: "var(--text-muted)", cursor: "pointer", fontSize: 16 }}>✕</button>
            </div>
            <div style={{ overflowY: "auto", flex: 1, padding: 8 }}>
              {templates.map((tpl) => (
                <div key={tpl.id} style={{ background: "var(--bg-primary)", borderRadius: 5, marginBottom: 6, border: "1px solid var(--border-color)", overflow: "hidden" }}>
                  <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "8px 10px" }}>
                    <span style={{ background: categoryColors[tpl.category] ?? "var(--text-muted)", color: "var(--bg-primary)", borderRadius: 3, padding: "1px 6px", fontSize: 9, fontWeight: 700, flexShrink: 0 }}>
                      {tpl.category}
                    </span>
                    <span style={{ fontWeight: "bold", color: "var(--text-primary)", flex: 1 }}>{tpl.name}</span>
                    <button
                      onClick={() => setExpandedTemplate(expandedTemplate === tpl.id ? null : tpl.id)}
                      style={{ background: "transparent", border: "none", color: "var(--text-muted)", cursor: "pointer", fontSize: 10 }}
                    >
                      {expandedTemplate === tpl.id ? "▲" : "▼"}
                    </button>
                    <button
                      onClick={() => useTemplate(tpl)}
                      style={{ background: "var(--accent-hover)", color: "var(--bg-primary)", border: "none", borderRadius: 4, padding: "3px 10px", fontSize: 10, fontWeight: 700, cursor: "pointer" }}
                    >
                      Use
                    </button>
                  </div>
                  <div style={{ padding: "0 10px 6px", fontSize: 11, color: "var(--text-secondary)" }}>{tpl.description}</div>
                  {expandedTemplate === tpl.id && (
                    <div style={{ padding: "6px 10px", borderTop: "1px solid var(--border-color)", background: "var(--bg-primary)" }}>
                      <div style={{ fontSize: 10, color: "var(--text-muted)", marginBottom: 4 }}>Steps:</div>
                      {tpl.steps.map((step, i) => (
                        <div key={i} style={{ display: "flex", gap: 6, fontSize: 11, color: "var(--text-secondary)", marginBottom: 2 }}>
                          <span style={{ color: "var(--accent-hover)", fontWeight: "bold" }}>{i + 1}.</span>
                          {step}
                        </div>
                      ))}
                    </div>
                  )}
                </div>
              ))}
            </div>
          </div>
        </div>
      )}
    </div>
  );
});

export default AgentPanel;
