import React, { useState, memo } from "react";
import { invoke } from "@tauri-apps/api/core";

interface ProfilerPanelProps {
  projectPath: string;
}

interface ProfilerResult {
  tool: string;
  output_file?: string | null;
  summary: string;
  success?: boolean;
}

interface BundleFile {
  name: string;
  size_bytes: number;
  percent: number;
  kind: string;
}

type CpuSubTab = "profiler" | "bundle";

function formatSize(bytes: number): string {
  if (bytes >= 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(2) + " MB";
  return (bytes / 1024).toFixed(1) + " KB";
}

const KIND_COLORS: Record<string, string> = {
  js: "#3b82f6",
  css: "#a78bfa",
  wasm: "#22c55e",
  asset: "#6b7280",
  sourcemap: "#374151",
};

const panelStyle: React.CSSProperties = {
  display: "flex",
  flexDirection: "column",
  height: "100%",
  background: "var(--bg-primary)",
  color: "var(--text-primary)",
  fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
  fontSize: 13,
  overflow: "hidden",
};

const tabBarStyle: React.CSSProperties = {
  display: "flex",
  borderBottom: "1px solid var(--border-color)",
  flexShrink: 0,
};

function tabStyle(active: boolean): React.CSSProperties {
  return {
    padding: "6px 14px",
    cursor: "pointer",
    background: active ? "var(--bg-hover)" : "transparent",
    color: active ? "var(--text-primary)" : "var(--text-muted)",
    border: "none",
    borderBottom: active ? "2px solid #89b4fa" : "2px solid transparent",
    fontSize: 12,
    fontFamily: "inherit",
  };
}

const btnStyle: React.CSSProperties = {
  background: "var(--bg-hover)",
  border: "1px solid #45475a",
  color: "var(--text-primary)",
  borderRadius: 4,
  padding: "5px 12px",
  cursor: "pointer",
  fontSize: 12,
  fontFamily: "inherit",
  marginTop: 8,
};

function Spinner() {
  return (
    <span
      aria-label="Loading"
      style={{
        display: "inline-block",
        width: 14,
        height: 14,
        border: "2px solid #45475a",
        borderTop: "2px solid #89b4fa",
        borderRadius: "50%",
        animation: "spin 0.8s linear infinite",
        marginLeft: 8,
        verticalAlign: "middle",
      }}
    />
  );
}

function CpuProfilerTab({ projectPath }: { projectPath: string }) {
  const hasProject = projectPath.trim().length > 0;
  const [cpuSubTab, setCpuSubTab] = useState<CpuSubTab>("profiler");
  const [cpuRunning, setCpuRunning] = useState(false);
  const [cpuResult, setCpuResult] = useState<ProfilerResult | null>(null);
  const [cpuError, setCpuError] = useState<string | null>(null);
  const [bundleRunning, setBundleRunning] = useState(false);
  const [bundleResult, setBundleResult] = useState<BundleFile[] | null>(null);
  const [bundleError, setBundleError] = useState<string | null>(null);
  const showCpuInstallHint =
    cpuResult?.tool === "none" && cpuResult.summary.toLowerCase().includes("flamegraph");

  const handleRunCpu = async () => {
    if (!hasProject) {
      setCpuError("Open a project folder before running the CPU profiler.");
      return;
    }
    setCpuRunning(true);
    setCpuError(null);
    setCpuResult(null);
    try {
      const result = await invoke<ProfilerResult>("run_cpu_profiler", { projectPath });
      setCpuResult(result);
    } catch (e) {
      setCpuError(String(e));
    } finally {
      setCpuRunning(false);
    }
  };

  const handleReveal = async (filePath: string) => {
    try {
      await invoke("reveal_in_explorer", { path: filePath });
    } catch { /* ignore */ }
  };

  const handleAnalyzeBundle = async () => {
    if (!hasProject) {
      setBundleError("Open a project folder before analyzing the bundle.");
      return;
    }
    setBundleRunning(true);
    setBundleError(null);
    setBundleResult(null);
    try {
      const result = await invoke<BundleFile[]>("analyze_bundle", { projectPath });
      setBundleResult(result);
    } catch (e) {
      setBundleError(String(e));
    } finally {
      setBundleRunning(false);
    }
  };

  const totalBundleSize = bundleResult?.reduce((sum, file) => sum + file.size_bytes, 0) ?? 0;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <div style={tabBarStyle}>
        <button style={tabStyle(cpuSubTab === "profiler")} onClick={() => setCpuSubTab("profiler")}>
          CPU Profiler
        </button>
        <button style={tabStyle(cpuSubTab === "bundle")} onClick={() => setCpuSubTab("bundle")}>
          Bundle Analysis
        </button>
      </div>

      <div style={{ flex: 1, overflowY: "auto", padding: 12 }}>
        {cpuSubTab === "profiler" && (
          <div>
            <div style={{ marginBottom: 4, color: "var(--text-muted)", fontSize: 11 }}>
              Run a CPU profiler on your project.
            </div>
            {!hasProject && (
              <div style={{ marginBottom: 8, color: "var(--text-muted)", fontSize: 11 }}>
                Open a project folder to enable profiling.
              </div>
            )}
            <button style={btnStyle} onClick={handleRunCpu} disabled={cpuRunning || !hasProject}>
              {cpuRunning ? <>Running...{<Spinner />}</> : "Run CPU Profiler"}
            </button>

            {cpuError && (
              <div style={{ color: "#f38ba8", marginTop: 8, fontSize: 12 }}>{cpuError}</div>
            )}

            {cpuResult && (
              <div style={{ marginTop: 12 }}>
                <div style={{ marginBottom: 4, fontSize: 11, color: "var(--text-muted)" }}>
                  Tool: <span style={{ color: "var(--accent-hover)" }}>{cpuResult.tool}</span>
                </div>
                <div style={{ marginBottom: 4, fontSize: 11 }}>
                  Output:{" "}
                  {cpuResult.output_file ? (
                    <span
                      style={{ color: "var(--accent-hover)", cursor: "pointer", textDecoration: "underline" }}
                      onClick={() => handleReveal(cpuResult.output_file!)}
                      title="Reveal in Explorer"
                    >
                      {cpuResult.output_file}
                    </span>
                  ) : (
                    <span style={{ color: "var(--text-muted)" }}>none</span>
                  )}
                </div>
                {cpuResult.output_file?.endsWith(".svg") && (
                  <div style={{ marginTop: 8, border: "1px solid var(--border-color)", borderRadius: 4, overflow: "auto", maxHeight: 400 }}>
                    <object
                      type="image/svg+xml"
                      data={`asset://${cpuResult.output_file}`}
                      style={{ width: "100%", minHeight: 200 }}
                    >
                      <img src={`asset://${cpuResult.output_file}`} alt="CPU flamegraph" style={{ maxWidth: "100%" }} />
                    </object>
                  </div>
                )}
                <div style={{ marginTop: 8, background: "var(--bg-primary)", border: "1px solid var(--border-color)", borderRadius: 4, padding: 10, fontSize: 12, whiteSpace: "pre-wrap" }}>
                  {cpuResult.summary}
                </div>
                {showCpuInstallHint && (
                  <div style={{ marginTop: 8 }}>
                    <a
                      href="https://github.com/flamegraph-rs/flamegraph"
                      target="_blank"
                      rel="noopener noreferrer"
                      style={{ color: "var(--accent-hover)", fontSize: 11 }}
                    >
                      Install cargo-flamegraph
                    </a>
                  </div>
                )}
              </div>
            )}
          </div>
        )}

        {cpuSubTab === "bundle" && (
          <div>
            {bundleResult && (
              <div style={{ marginBottom: 8, fontSize: 12, color: "#a6e3a1" }}>
                Total bundle size: <strong>{formatSize(totalBundleSize)}</strong>
              </div>
            )}
            {!hasProject && (
              <div style={{ marginBottom: 8, color: "var(--text-muted)", fontSize: 11 }}>
                Open a project folder to analyze bundle output.
              </div>
            )}
            <button style={btnStyle} onClick={handleAnalyzeBundle} disabled={bundleRunning || !hasProject}>
              {bundleRunning ? <>Analyzing...{<Spinner />}</> : "Analyze Bundle"}
            </button>

            {bundleError && (
              <div style={{ color: "#f38ba8", marginTop: 8, fontSize: 12 }}>{bundleError}</div>
            )}

            {bundleResult && bundleResult.length > 0 && (
              <div style={{ marginTop: 12 }}>
                {bundleResult.slice(0, 20).map((file, i) => (
                  <div key={`${file.name}-${i}`} style={{ marginBottom: 6 }}>
                    <div style={{ display: "flex", justifyContent: "space-between", fontSize: 11, marginBottom: 2 }}>
                      <span style={{ color: "var(--text-primary)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", maxWidth: "70%" }}>
                        {file.name}
                      </span>
                      <span style={{ color: "var(--text-muted)", flexShrink: 0 }}>
                        {formatSize(file.size_bytes)} ({file.percent.toFixed(1)}%)
                      </span>
                    </div>
                    <div style={{ height: 14, background: "var(--bg-primary)", borderRadius: 3, overflow: "hidden" }}>
                      <div
                        style={{
                          height: "100%",
                          width: `${Math.max(file.percent, 0.5)}%`,
                          background: KIND_COLORS[file.kind] ?? "#6b7280",
                          borderRadius: 3,
                          transition: "width 0.3s ease",
                          display: "flex",
                          alignItems: "center",
                          paddingLeft: 4,
                          fontSize: 9,
                          color: "var(--bg-primary)",
                          fontWeight: 700,
                          whiteSpace: "nowrap",
                          overflow: "hidden",
                        }}
                      >
                        {file.kind}
                      </div>
                    </div>
                  </div>
                ))}
              </div>
            )}
            {bundleResult && bundleResult.length === 0 && (
              <div style={{ marginTop: 12, color: "var(--text-muted)", fontSize: 12 }}>
                No bundle files were found in the build output.
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function MemoryProfilerTab({ projectPath }: { projectPath: string }) {
  const hasProject = projectPath.trim().length > 0;
  const [memRunning, setMemRunning] = useState(false);
  const [memResult, setMemResult] = useState<ProfilerResult | null>(null);
  const [memError, setMemError] = useState<string | null>(null);

  const handleRunMemory = async () => {
    if (!hasProject) {
      setMemError("Open a project folder before running the memory profiler.");
      return;
    }
    setMemRunning(true);
    setMemError(null);
    setMemResult(null);
    try {
      const result = await invoke<ProfilerResult>("run_memory_profiler", { projectPath });
      setMemResult(result);
    } catch (e) {
      setMemError(String(e));
    } finally {
      setMemRunning(false);
    }
  };

  const handleReveal = async (filePath: string) => {
    try {
      await invoke("reveal_in_explorer", { path: filePath });
    } catch { /* ignore */ }
  };

  const isToolNotFound = memError?.toLowerCase().includes("not found") || memError?.toLowerCase().includes("command not found");

  return (
    <div style={{ padding: 12 }}>
      <div style={{ marginBottom: 4, color: "var(--text-muted)", fontSize: 11 }}>
        Run a memory profiler (e.g. heaptrack, valgrind) on your project.
      </div>
      {!hasProject && (
        <div style={{ marginBottom: 8, color: "var(--text-muted)", fontSize: 11 }}>
          Open a project folder to enable memory profiling.
        </div>
      )}
      <button style={btnStyle} onClick={handleRunMemory} disabled={memRunning || !hasProject}>
        {memRunning ? <>Running...{<Spinner />}</> : "Run Memory Profiler"}
      </button>

      {memError && (
        <div style={{ color: "#f38ba8", marginTop: 8, fontSize: 12 }}>
          {memError}
          {isToolNotFound && (
            <div style={{ marginTop: 6 }}>
              <a
                href="https://github.com/KDE/heaptrack"
                target="_blank"
                rel="noopener noreferrer"
                style={{ color: "var(--accent-hover)", fontSize: 11 }}
              >
                Install heaptrack
              </a>
            </div>
          )}
        </div>
      )}

      {memResult && (
        <div style={{ marginTop: 12 }}>
          <div style={{ marginBottom: 4, fontSize: 11, color: "var(--text-muted)" }}>
            Tool: <span style={{ color: "var(--accent-hover)" }}>{memResult.tool}</span>
          </div>
          <div style={{ marginBottom: 4, fontSize: 11 }}>
            Output:{" "}
            {memResult.output_file ? (
              <span
                style={{ color: "var(--accent-hover)", cursor: "pointer", textDecoration: "underline" }}
                onClick={() => handleReveal(memResult.output_file!)}
                title="Reveal in Explorer"
              >
                {memResult.output_file}
              </span>
            ) : (
              <span style={{ color: "var(--text-muted)" }}>none</span>
            )}
          </div>
          <div style={{ marginTop: 8, background: "var(--bg-primary)", border: "1px solid var(--border-color)", borderRadius: 4, padding: 10, fontSize: 12, whiteSpace: "pre-wrap" }}>
            {memResult.summary}
          </div>
        </div>
      )}
    </div>
  );
}

const ProfilerPanel = memo(function ProfilerPanel({ projectPath }: ProfilerPanelProps) {
  const [mainTab, setMainTab] = useState<"cpu" | "memory">("cpu");

  return (
    <div style={panelStyle}>
      {/* Inject spin keyframe once */}
      <style>{`@keyframes spin { to { transform: rotate(360deg); } }`}</style>
      <div style={{ padding: "8px 12px", borderBottom: "1px solid var(--border-color)", fontWeight: 700, fontSize: 12, color: "var(--accent-hover)", flexShrink: 0 }}>
        PROFILER
      </div>
      <div style={tabBarStyle}>
        <button style={tabStyle(mainTab === "cpu")} onClick={() => setMainTab("cpu")}>CPU</button>
        <button style={tabStyle(mainTab === "memory")} onClick={() => setMainTab("memory")}>Memory</button>
      </div>

      <div style={{ flex: 1, overflowY: "auto", display: "flex", flexDirection: "column" }}>
        {mainTab === "cpu" && <CpuProfilerTab projectPath={projectPath} />}
        {mainTab === "memory" && <MemoryProfilerTab projectPath={projectPath} />}
      </div>
    </div>
  );
});

export default ProfilerPanel;
