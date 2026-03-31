import React, { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

interface HunkStagingPanelProps {
  repoPath: string;
  filePath: string;
}

interface Hunk {
  header: string;
  lines: string[];
}

const panelStyle: React.CSSProperties = {
  display: "flex",
  flexDirection: "column",
  height: "100%",
  background: "#1e1e2e",
  color: "#cdd6f4",
  fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
  fontSize: 12,
};

const btnStyle: React.CSSProperties = {
  background: "#313244",
  border: "1px solid #45475a",
  color: "#cdd6f4",
  borderRadius: 4,
  padding: "3px 8px",
  cursor: "pointer",
  fontSize: 11,
  fontFamily: "inherit",
};

const primaryBtnStyle: React.CSSProperties = {
  ...btnStyle,
  background: "#89b4fa",
  color: "#1e1e2e",
  border: "none",
};

function lineColor(line: string): string {
  if (line.startsWith("+")) return "#a6e3a1";
  if (line.startsWith("-")) return "#f38ba8";
  return "#6c7086";
}

function lineBackground(line: string): string {
  if (line.startsWith("+")) return "rgba(166,227,161,0.08)";
  if (line.startsWith("-")) return "rgba(243,139,168,0.08)";
  return "transparent";
}

function HunkCard({
  hunk,
  actionLabel,
  onAction,
}: {
  hunk: Hunk;
  actionLabel: string;
  onAction: () => void;
}) {
  const [collapsed, setCollapsed] = useState(false);

  return (
    <div style={{ border: "1px solid #313244", borderRadius: 4, marginBottom: 8, overflow: "hidden" }}>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "4px 8px",
          background: "#181825",
          cursor: "pointer",
          userSelect: "none",
        }}
        onClick={() => setCollapsed((p) => !p)}
      >
        <span style={{ fontSize: 9, opacity: 0.6 }}>{collapsed ? "▶" : "▼"}</span>
        <span style={{ flex: 1, color: "#89b4fa", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {hunk.header}
        </span>
        <button
          style={actionLabel === "Stage" ? primaryBtnStyle : btnStyle}
          onClick={(e) => { e.stopPropagation(); onAction(); }}
        >
          {actionLabel}
        </button>
      </div>
      {!collapsed && (
        <div style={{ overflowX: "auto" }}>
          {hunk.lines.map((line, i) => (
            <div
              key={i}
              style={{
                padding: "1px 8px",
                background: lineBackground(line),
                color: lineColor(line),
                whiteSpace: "pre",
                fontFamily: "inherit",
                fontSize: 11,
              }}
            >
              {line}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

export default function HunkStagingPanel({ repoPath, filePath }: HunkStagingPanelProps) {
  const [unstagedHunks, setUnstagedHunks] = useState<Hunk[]>([]);
  const [stagedHunks, setStagedHunks] = useState<Hunk[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fetchHunks = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const [unstaged, staged] = await Promise.all([
        invoke<Hunk[]>("git_diff_hunks", { repo_path: repoPath, file_path: filePath, staged: false }),
        invoke<Hunk[]>("git_diff_hunks", { repo_path: repoPath, file_path: filePath, staged: true }),
      ]);
      setUnstagedHunks(unstaged ?? []);
      setStagedHunks(staged ?? []);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [repoPath, filePath]);

  useEffect(() => {
    fetchHunks();
  }, [fetchHunks]);

  const stageHunk = async (hunk: Hunk) => {
    try {
      await invoke("git_stage_hunk", {
        repo_path: repoPath,
        file_path: filePath,
        hunk_header: hunk.header,
        hunk_lines: hunk.lines,
      });
      await fetchHunks();
    } catch (e) {
      setError(String(e));
    }
  };

  const unstageHunk = async (hunk: Hunk) => {
    try {
      await invoke("git_unstage_hunk", {
        repo_path: repoPath,
        file_path: filePath,
        hunk_header: hunk.header,
        hunk_lines: hunk.lines,
      });
      await fetchHunks();
    } catch (e) {
      setError(String(e));
    }
  };

  const stageAll = async () => {
    try {
      await invoke("git_stage_file", { repo_path: repoPath, file_path: filePath });
      await fetchHunks();
    } catch (e) {
      setError(String(e));
    }
  };

  const unstageAll = async () => {
    try {
      await invoke("git_unstage_file", { repo_path: repoPath, file_path: filePath });
      await fetchHunks();
    } catch (e) {
      setError(String(e));
    }
  };

  const shortPath = filePath.split("/").pop() ?? filePath;

  return (
    <div style={panelStyle}>
      {/* Header */}
      <div style={{ padding: "6px 10px", borderBottom: "1px solid #313244", flexShrink: 0 }}>
        <div style={{ fontWeight: 700, fontSize: 11, color: "#89b4fa", marginBottom: 4 }}>
          HUNK STAGING — {shortPath}
        </div>
        <div style={{ display: "flex", gap: 6 }}>
          <button style={primaryBtnStyle} onClick={stageAll} disabled={loading}>Stage All</button>
          <button style={btnStyle} onClick={unstageAll} disabled={loading}>Unstage All</button>
          <button style={btnStyle} onClick={fetchHunks} disabled={loading}>Refresh</button>
        </div>
      </div>

      {error && (
        <div style={{ padding: 8, color: "#f38ba8", fontSize: 11, borderBottom: "1px solid #313244" }}>
          {error}
        </div>
      )}

      {loading && (
        <div style={{ padding: 12, color: "#6c7086", fontSize: 11 }}>Loading…</div>
      )}

      {!loading && (
        <div style={{ flex: 1, overflowY: "auto", display: "flex", gap: 0 }}>
          {/* Unstaged */}
          <div style={{ flex: 1, padding: 10, borderRight: "1px solid #313244", overflowY: "auto" }}>
            <div style={{ fontSize: 10, fontWeight: 700, color: "#6c7086", textTransform: "uppercase", marginBottom: 8, letterSpacing: "0.05em" }}>
              Unstaged ({unstagedHunks.length})
            </div>
            {unstagedHunks.length === 0 && (
              <div style={{ color: "#6c7086", fontSize: 11 }}>No unstaged changes</div>
            )}
            {unstagedHunks.map((hunk, i) => (
              <HunkCard key={i} hunk={hunk} actionLabel="Stage" onAction={() => stageHunk(hunk)} />
            ))}
          </div>

          {/* Staged */}
          <div style={{ flex: 1, padding: 10, overflowY: "auto" }}>
            <div style={{ fontSize: 10, fontWeight: 700, color: "#6c7086", textTransform: "uppercase", marginBottom: 8, letterSpacing: "0.05em" }}>
              Staged ({stagedHunks.length})
            </div>
            {stagedHunks.length === 0 && (
              <div style={{ color: "#6c7086", fontSize: 11 }}>No staged changes</div>
            )}
            {stagedHunks.map((hunk, i) => (
              <HunkCard key={i} hunk={hunk} actionLabel="Unstage" onAction={() => unstageHunk(hunk)} />
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
