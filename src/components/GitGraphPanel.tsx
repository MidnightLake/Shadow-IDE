import { useEffect, useState, useCallback, memo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import HunkStagingPanel from "./HunkStagingPanel";

interface CommitNode {
  hash: string;
  parents: string[];
  author: string;
  timestamp: number;
  subject: string;
  refs: string[];
}

interface GitGraphPanelProps {
  rootPath: string;
}

// Simple hash function to derive a color from a string
function hashColor(str: string): string {
  let h = 0;
  for (let i = 0; i < str.length; i++) {
    h = ((h << 5) - h + str.charCodeAt(i)) | 0;
  }
  const hue = Math.abs(h) % 360;
  return `hsl(${hue}, 70%, 60%)`;
}

// Assign columns to commits for lane-based layout
function assignColumns(commits: CommitNode[]): Map<string, number> {
  const colMap = new Map<string, number>();
  const activeLanes: (string | null)[] = [];

  for (const commit of commits) {
    // Find existing lane for this commit
    let col = activeLanes.indexOf(commit.hash);
    if (col === -1) {
      // Find a free lane
      col = activeLanes.indexOf(null);
      if (col === -1) {
        col = activeLanes.length;
        activeLanes.push(null);
      }
    }
    colMap.set(commit.hash, col);

    // Replace this commit's slot with its first parent (continue the lane)
    if (commit.parents.length > 0) {
      activeLanes[col] = commit.parents[0];
      // Add extra parents to new lanes
      for (let i = 1; i < commit.parents.length; i++) {
        const freeIdx = activeLanes.indexOf(null);
        if (freeIdx !== -1) {
          activeLanes[freeIdx] = commit.parents[i];
        } else {
          activeLanes.push(commit.parents[i]);
        }
      }
    } else {
      activeLanes[col] = null;
    }
  }

  return colMap;
}

interface GitStatusFile {
  path: string;
  status: string;
}

const GitGraphPanel = memo(function GitGraphPanel({ rootPath }: GitGraphPanelProps) {
  const [commits, setCommits] = useState<CommitNode[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [changedFiles, setChangedFiles] = useState<GitStatusFile[]>([]);
  const [selectedFile, setSelectedFile] = useState<string | null>(null);

  const loadGraph = useCallback(async () => {
    if (!rootPath) return;
    setLoading(true);
    setError(null);
    try {
      const data = await invoke<CommitNode[]>("git_branch_graph", {
        root: rootPath,
        maxCommits: 100,
      });
      setCommits(data);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [rootPath]);

  const loadChangedFiles = useCallback(async () => {
    if (!rootPath) return;
    try {
      const files = await invoke<GitStatusFile[]>("git_status", { path: rootPath });
      setChangedFiles(files ?? []);
    } catch { /* ignore */ }
  }, [rootPath]);

  useEffect(() => {
    loadGraph();
    loadChangedFiles();
  }, [loadGraph, loadChangedFiles]);

  useEffect(() => {
    const unsub = listen("git-repo-changed", () => { loadGraph(); loadChangedFiles(); });
    return () => { unsub.then((fn) => fn()); };
  }, [loadGraph, loadChangedFiles]);

  const colMap = assignColumns(commits);
  const maxCol = Math.max(...Array.from(colMap.values()), 0);
  const svgWidth = Math.max(400, (maxCol + 1) * 24 + 300);
  const svgHeight = commits.length * 32 + 16;

  const formatDate = (ts: number) => {
    if (!ts) return "";
    return new Date(ts * 1000).toLocaleDateString();
  };

  return (
    <div
      style={{
        background: "var(--bg-primary)",
        color: "var(--text-primary)",
        height: "100%",
        overflow: "auto",
        padding: "8px",
        fontFamily: "monospace",
        fontSize: "12px",
        display: "flex",
        flexDirection: "column",
      }}
    >
      <div style={{ display: "flex", alignItems: "center", gap: "8px", marginBottom: "8px" }}>
        <span style={{ fontWeight: "bold", color: "var(--accent-hover)" }}>Git Graph</span>
        <button
          onClick={loadGraph}
          disabled={loading}
          style={{
            background: "var(--bg-hover)",
            border: "1px solid var(--border-color)",
            color: "var(--text-primary)",
            borderRadius: "4px",
            padding: "2px 8px",
            cursor: "pointer",
            fontSize: "11px",
          }}
        >
          {loading ? "Loading..." : "Refresh"}
        </button>
      </div>

      {error && (
        <div style={{ color: "#f38ba8", padding: "8px", fontSize: "11px" }}>
          Error: {error}
        </div>
      )}

      {!loading && commits.length === 0 && !error && (
        <div style={{ color: "var(--text-muted)", padding: "8px" }}>No commits found.</div>
      )}

      {/* Changed Files */}
      {changedFiles.length > 0 && (
        <div style={{ marginBottom: 8, border: "1px solid var(--border-color)", borderRadius: 4, overflow: "hidden" }}>
          <div style={{ padding: "4px 8px", background: "var(--bg-primary)", fontSize: 10, color: "var(--text-muted)", fontWeight: 700, textTransform: "uppercase" }}>
            Changed Files ({changedFiles.length})
          </div>
          {changedFiles.map((f) => (
            <div
              key={f.path}
              onClick={() => setSelectedFile((prev) => prev === f.path ? null : f.path)}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 6,
                padding: "3px 8px",
                cursor: "pointer",
                background: selectedFile === f.path ? "var(--bg-hover)" : "transparent",
                borderBottom: "1px solid #181825",
              }}
            >
              <span style={{ fontSize: 9, color: f.status === "M" ? "#fab387" : f.status === "A" ? "#a6e3a1" : "#f38ba8" }}>
                {f.status}
              </span>
              <span style={{ fontSize: 11, color: "var(--text-primary)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                {f.path.split("/").pop() ?? f.path}
              </span>
            </div>
          ))}
        </div>
      )}

      {/* Hunk Staging Panel for selected file */}
      {selectedFile && (
        <div style={{ marginBottom: 8, border: "1px solid var(--border-color)", borderRadius: 4, overflow: "hidden", minHeight: 200, maxHeight: 400 }}>
          <HunkStagingPanel repoPath={rootPath} filePath={selectedFile} />
        </div>
      )}

      {commits.length > 0 && (
        <svg
          width={svgWidth}
          height={svgHeight}
          style={{ display: "block", minWidth: "100%" }}
        >
          {/* Draw edges (parent lines) */}
          {commits.map((commit, row) => {
            const col = colMap.get(commit.hash) ?? 0;
            const cx = col * 24 + 12;
            const cy = row * 32 + 16;
            return commit.parents.map((parentHash) => {
              const parentRow = commits.findIndex((c) => c.hash === parentHash);
              if (parentRow === -1) return null;
              const parentCol = colMap.get(parentHash) ?? col;
              const px = parentCol * 24 + 12;
              const py = parentRow * 32 + 16;
              const color = hashColor(parentHash.slice(0, 8));
              return (
                <line
                  key={`${commit.hash}-${parentHash}`}
                  x1={cx}
                  y1={cy}
                  x2={px}
                  y2={py}
                  stroke={color}
                  strokeWidth={1.5}
                  strokeOpacity={0.7}
                />
              );
            });
          })}

          {/* Draw commit circles and labels */}
          {commits.map((commit, row) => {
            const col = colMap.get(commit.hash) ?? 0;
            const cx = col * 24 + 12;
            const cy = row * 32 + 16;
            const color = hashColor(commit.refs[0] ?? commit.hash.slice(0, 8));
            const shortHash = commit.hash.slice(0, 7);
            const textX = (maxCol + 1) * 24 + 8;
            const tooltip = `${commit.hash}\n${commit.author}\n${formatDate(commit.timestamp)}\n${commit.subject}`;

            return (
              <g key={commit.hash}>
                <circle cx={cx} cy={cy} r={8} fill={color} stroke="var(--bg-primary)" strokeWidth={2}>
                  <title>{tooltip}</title>
                </circle>

                {/* Branch/tag labels */}
                {commit.refs.filter((r) => r && !r.startsWith("HEAD")).map((ref, ri) => {
                  const refX = col * 24 + 24;
                  const refY = cy - 10 + ri * 14;
                  const isTag = ref.includes("tag:");
                  return (
                    <g key={ref}>
                      <rect
                        x={refX}
                        y={refY - 9}
                        width={ref.length * 6 + 8}
                        height={14}
                        rx={3}
                        fill={isTag ? "#f9e2af" : "var(--accent-hover)"}
                        opacity={0.85}
                      />
                      <text
                        x={refX + 4}
                        y={refY + 1}
                        fontSize={9}
                        fill="var(--bg-primary)"
                        fontFamily="monospace"
                      >
                        {ref.replace("tag: ", "")}
                      </text>
                    </g>
                  );
                })}

                {/* Hash + subject */}
                <text x={textX} y={cy + 4} fontSize={11} fill="var(--text-secondary)" fontFamily="monospace">
                  {shortHash}{" "}
                  <tspan fill="var(--text-primary)">
                    {commit.subject.length > 60
                      ? commit.subject.slice(0, 60) + "..."
                      : commit.subject}
                  </tspan>
                  <title>{tooltip}</title>
                </text>
              </g>
            );
          })}
        </svg>
      )}
    </div>
  );
});

export default GitGraphPanel;
