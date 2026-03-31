import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

interface PrPanelProps {
  repoPath: string;
}

interface PullRequest {
  number: number;
  title: string;
  author: string;
  draft: boolean;
  labels: string[];
  body: string;
  url: string;
  ci_status?: "success" | "failure" | "pending" | null;
  review_status?: string;
  files?: string[];
}

interface CheckRun {
  name: string;
  status: "queued" | "in_progress" | "completed";
  conclusion: "success" | "failure" | "neutral" | "cancelled" | "skipped" | null;
}

type TabId = "open-prs" | "create-pr" | "ci-status";

export default function PrPanel({ repoPath }: PrPanelProps) {
  const [activeTab, setActiveTab] = useState<TabId>("open-prs");

  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column", background: "var(--bg-primary)", color: "var(--text-primary)", fontFamily: "monospace", fontSize: 12 }}>
      {/* Tab bar */}
      <div style={{ display: "flex", borderBottom: "1px solid var(--border-color)", flexShrink: 0 }}>
        {([
          { id: "open-prs" as TabId, label: "Open PRs" },
          { id: "create-pr" as TabId, label: "Create PR" },
          { id: "ci-status" as TabId, label: "CI Status" },
        ] as const).map((tab) => (
          <button
            key={tab.id}
            onClick={() => setActiveTab(tab.id)}
            style={{
              background: "transparent",
              border: "none",
              borderBottom: activeTab === tab.id ? "2px solid #89b4fa" : "2px solid transparent",
              color: activeTab === tab.id ? "var(--accent-hover)" : "var(--text-muted)",
              padding: "8px 14px",
              cursor: "pointer",
              fontSize: 11,
              fontWeight: activeTab === tab.id ? 700 : 400,
            }}
          >
            {tab.label}
          </button>
        ))}
      </div>

      <div style={{ flex: 1, overflow: "auto" }}>
        {activeTab === "open-prs" && <OpenPrsTab repoPath={repoPath} />}
        {activeTab === "create-pr" && <CreatePrTab repoPath={repoPath} />}
        {activeTab === "ci-status" && <CiStatusTab repoPath={repoPath} />}
      </div>
    </div>
  );
}

function OpenPrsTab({ repoPath }: { repoPath: string }) {
  const [prs, setPrs] = useState<PullRequest[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [expandedPr, setExpandedPr] = useState<number | null>(null);

  const loadPrs = useCallback(async () => {
    if (!repoPath) return;
    setLoading(true);
    setError(null);
    try {
      const result = await invoke<PullRequest[]>("git_list_prs", { repo_path: repoPath, state: "open" });
      setPrs(result ?? []);
    } catch (e) {
      setError(String(e));
      setPrs([]);
    } finally {
      setLoading(false);
    }
  }, [repoPath]);

  useEffect(() => { loadPrs(); }, [loadPrs]);

  const ciIcon = (status?: PullRequest["ci_status"]) => {
    if (status === "success") return <span style={{ color: "#a6e3a1" }}>✓</span>;
    if (status === "failure") return <span style={{ color: "#f38ba8" }}>✗</span>;
    if (status === "pending") return <span style={{ color: "#f9e2af" }}>●</span>;
    return null;
  };

  return (
    <div style={{ padding: 8 }}>
      <div style={{ display: "flex", alignItems: "center", marginBottom: 8 }}>
        <span style={{ fontWeight: "bold", color: "var(--accent-hover)", flex: 1 }}>Pull Requests</span>
        <button
          onClick={loadPrs}
          style={{ background: "var(--bg-hover)", border: "1px solid #45475a", borderRadius: 4, color: "var(--text-primary)", padding: "3px 8px", fontSize: 10, cursor: "pointer" }}
        >
          Refresh
        </button>
      </div>

      {loading && <div style={{ color: "var(--text-muted)", padding: 8 }}>Loading...</div>}
      {error && <div style={{ color: "#f38ba8", padding: 8 }}>Error: {error}</div>}
      {!loading && !error && prs.length === 0 && (
        <div style={{ color: "var(--text-muted)", padding: 8 }}>No open pull requests.</div>
      )}

      {prs.map((pr) => (
        <div key={pr.number} style={{ background: "var(--bg-primary)", borderRadius: 5, marginBottom: 6, border: "1px solid var(--border-color)", overflow: "hidden" }}>
          <div
            style={{ padding: "8px 10px", cursor: "pointer", display: "flex", alignItems: "center", gap: 6 }}
            onClick={() => setExpandedPr(expandedPr === pr.number ? null : pr.number)}
          >
            <span style={{ color: "var(--text-muted)", fontSize: 10, flexShrink: 0 }}>#{pr.number}</span>
            {pr.draft && (
              <span style={{ background: "var(--bg-hover)", color: "var(--text-secondary)", borderRadius: 3, padding: "0 5px", fontSize: 9, flexShrink: 0 }}>Draft</span>
            )}
            <span style={{ flex: 1, color: "var(--text-primary)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{pr.title}</span>
            {ciIcon(pr.ci_status)}
            <span style={{ color: "var(--text-muted)", fontSize: 10, flexShrink: 0 }}>▼</span>
          </div>

          <div style={{ padding: "0 10px 4px", fontSize: 10, color: "var(--text-muted)" }}>
            <span>by {pr.author}</span>
            {pr.labels.map((l) => (
              <span key={l} style={{ background: "var(--bg-hover)", borderRadius: 8, padding: "1px 5px", marginLeft: 4 }}>{l}</span>
            ))}
          </div>

          {expandedPr === pr.number && (
            <div style={{ padding: "8px 10px", borderTop: "1px solid var(--border-color)" }}>
              {pr.body && (
                <div style={{ color: "var(--text-secondary)", fontSize: 11, marginBottom: 8, maxHeight: 120, overflowY: "auto", whiteSpace: "pre-wrap" }}>
                  {pr.body}
                </div>
              )}
              {pr.review_status && (
                <div style={{ color: "#f9e2af", fontSize: 10, marginBottom: 4 }}>Review: {pr.review_status}</div>
              )}
              {pr.files && pr.files.length > 0 && (
                <div style={{ marginBottom: 6 }}>
                  <div style={{ color: "var(--text-muted)", fontSize: 10, marginBottom: 2 }}>Changed files:</div>
                  {pr.files.map((f) => (
                    <div key={f} style={{ color: "var(--text-secondary)", fontSize: 10, paddingLeft: 8 }}>{f}</div>
                  ))}
                </div>
              )}
              {pr.url && (
                <a
                  href={pr.url}
                  target="_blank"
                  rel="noopener noreferrer"
                  style={{ color: "var(--accent-hover)", fontSize: 11 }}
                >
                  View on GitHub →
                </a>
              )}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

function CreatePrTab({ repoPath }: { repoPath: string }) {
  const [title, setTitle] = useState("");
  const [body, setBody] = useState("");
  const [base, setBase] = useState("main");
  const [draft, setDraft] = useState(false);
  const [loading, setLoading] = useState(false);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const createPr = async () => {
    if (!title.trim()) return;
    setLoading(true);
    setError(null);
    setResult(null);
    try {
      const url = await invoke<string>("git_create_pr", { repo_path: repoPath, title, body, base, draft });
      setResult(url ?? "PR created successfully.");
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  return (
    <div style={{ padding: 12 }}>
      <div style={{ fontWeight: "bold", color: "var(--accent-hover)", marginBottom: 10 }}>Create Pull Request</div>

      <label style={{ display: "block", color: "var(--text-secondary)", fontSize: 10, marginBottom: 4 }}>Title</label>
      <input
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        placeholder="PR title..."
        style={{ width: "100%", background: "var(--bg-hover)", border: "1px solid #45475a", borderRadius: 4, color: "var(--text-primary)", padding: "6px 8px", fontSize: 12, outline: "none", marginBottom: 8, boxSizing: "border-box" }}
      />

      <label style={{ display: "block", color: "var(--text-secondary)", fontSize: 10, marginBottom: 4 }}>Body (Markdown)</label>
      <textarea
        value={body}
        onChange={(e) => setBody(e.target.value)}
        placeholder="Describe your changes..."
        rows={6}
        style={{ width: "100%", background: "var(--bg-hover)", border: "1px solid #45475a", borderRadius: 4, color: "var(--text-primary)", padding: "6px 8px", fontSize: 12, outline: "none", resize: "vertical", marginBottom: 8, boxSizing: "border-box", fontFamily: "monospace" }}
      />

      <div style={{ display: "flex", gap: 8, alignItems: "center", marginBottom: 8 }}>
        <label style={{ color: "var(--text-secondary)", fontSize: 10 }}>Base branch:</label>
        <input
          value={base}
          onChange={(e) => setBase(e.target.value)}
          style={{ background: "var(--bg-hover)", border: "1px solid #45475a", borderRadius: 4, color: "var(--text-primary)", padding: "4px 8px", fontSize: 11, outline: "none", width: 100 }}
        />
        <label style={{ display: "flex", alignItems: "center", gap: 4, fontSize: 11, color: "var(--text-secondary)", cursor: "pointer", marginLeft: "auto" }}>
          <input type="checkbox" checked={draft} onChange={(e) => setDraft(e.target.checked)} style={{ accentColor: "var(--accent-hover)" }} />
          Draft
        </label>
      </div>

      <button
        onClick={createPr}
        disabled={loading || !title.trim()}
        style={{ background: "var(--accent-hover)", color: "var(--bg-primary)", border: "none", borderRadius: 4, padding: "6px 16px", fontSize: 12, fontWeight: 700, cursor: "pointer", opacity: loading || !title.trim() ? 0.6 : 1 }}
      >
        {loading ? "Creating..." : "Create PR"}
      </button>

      {result && (
        <div style={{ marginTop: 10, color: "#a6e3a1", fontSize: 11 }}>
          {result.startsWith("http") ? (
            <>PR created: <a href={result} target="_blank" rel="noopener noreferrer" style={{ color: "var(--accent-hover)" }}>{result}</a></>
          ) : result}
        </div>
      )}
      {error && <div style={{ marginTop: 10, color: "#f38ba8", fontSize: 11 }}>Error: {error}</div>}
    </div>
  );
}

function CiStatusTab({ repoPath }: { repoPath: string }) {
  const [runs, setRuns] = useState<CheckRun[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = async () => {
    setLoading(true);
    setError(null);
    try {
      const result = await invoke<CheckRun[]>("git_ci_status", { repo_path: repoPath });
      setRuns(result ?? []);
    } catch (e) {
      setError(String(e));
      setRuns([]);
    } finally {
      setLoading(false);
    }
  };

  const statusIcon = (run: CheckRun) => {
    if (run.status === "queued") return <span style={{ color: "#f9e2af" }}>○</span>;
    if (run.status === "in_progress") return <span style={{ color: "var(--accent-hover)" }}>⏳</span>;
    if (run.conclusion === "success") return <span style={{ color: "#a6e3a1" }}>✓</span>;
    if (run.conclusion === "failure") return <span style={{ color: "#f38ba8" }}>✗</span>;
    return <span style={{ color: "var(--text-muted)" }}>–</span>;
  };

  const conclusionColor = (c: CheckRun["conclusion"]) => {
    if (c === "success") return "#a6e3a1";
    if (c === "failure") return "#f38ba8";
    return "var(--text-muted)";
  };

  return (
    <div style={{ padding: 8 }}>
      <div style={{ display: "flex", alignItems: "center", marginBottom: 8 }}>
        <span style={{ fontWeight: "bold", color: "var(--accent-hover)", flex: 1 }}>CI Status</span>
        <button
          onClick={refresh}
          style={{ background: "var(--bg-hover)", border: "1px solid #45475a", borderRadius: 4, color: "var(--text-primary)", padding: "3px 8px", fontSize: 10, cursor: "pointer" }}
        >
          Refresh
        </button>
      </div>

      {loading && <div style={{ color: "var(--text-muted)", padding: 8 }}>Loading...</div>}
      {error && <div style={{ color: "#f38ba8", padding: 8 }}>Error: {error}</div>}
      {!loading && !error && runs.length === 0 && (
        <div style={{ color: "var(--text-muted)", padding: 8 }}>No CI runs found. Click Refresh to load.</div>
      )}

      {runs.map((run, i) => (
        <div key={i} style={{ display: "flex", alignItems: "center", gap: 8, padding: "6px 8px", background: "var(--bg-primary)", borderRadius: 4, marginBottom: 4, border: "1px solid var(--border-color)" }}>
          <span style={{ fontSize: 14, flexShrink: 0 }}>{statusIcon(run)}</span>
          <span style={{ flex: 1, color: "var(--text-primary)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{run.name}</span>
          <span style={{ fontSize: 9, color: "var(--text-muted)", flexShrink: 0, textTransform: "uppercase" }}>{run.status}</span>
          {run.conclusion && (
            <span style={{ fontSize: 9, color: conclusionColor(run.conclusion), flexShrink: 0, textTransform: "uppercase" }}>{run.conclusion}</span>
          )}
        </div>
      ))}
    </div>
  );
}
