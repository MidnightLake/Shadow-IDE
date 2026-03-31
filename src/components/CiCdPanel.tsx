import { useState, useCallback, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import DevOpsIntegrations from "./DevOpsIntegrations";

// ── Types ────────────────────────────────────────────────────────────────────

interface WorkflowRun {
  id: number;
  name: string;
  status: "queued" | "in_progress" | "completed";
  conclusion: "success" | "failure" | "cancelled" | "skipped" | "neutral" | null;
  head_branch: string;
  head_sha: string;
  created_at: string;
  updated_at: string;
  html_url: string;
  workflow_name?: string;
}

interface CiJob {
  id: number;
  name: string;
  status: "queued" | "in_progress" | "completed";
  conclusion: "success" | "failure" | "cancelled" | "skipped" | null;
  started_at: string | null;
  completed_at: string | null;
  stage?: string | null;
  html_url?: string | null;
  steps: Array<{ name: string; status: string; conclusion: string | null; number: number }>;
}

interface RepoInfo {
  provider: "github" | "gitlab";
  owner: string;
  repo: string;
}

interface DeploymentStatus {
  environment: "dev" | "staging" | "prod";
  environmentLabel: string;
  provider: "github" | "gitlab";
  ref: string;
  sha: string;
  created_at: string;
  state: string;
  url: string | null;
}

// ── Helpers ──────────────────────────────────────────────────────────────────

const statusIcon = (status: string, conclusion: string | null): string => {
  if (status === "in_progress") return "⟳";
  if (status === "queued") return "…";
  if (conclusion === "success") return "✓";
  if (conclusion === "failure") return "✗";
  if (conclusion === "cancelled") return "⊘";
  if (conclusion === "skipped") return "→";
  return "?";
};

const statusColor = (status: string, conclusion: string | null): string => {
  if (status === "in_progress") return "var(--warning)";
  if (status === "queued") return "var(--text-muted)";
  if (conclusion === "success") return "var(--success)";
  if (conclusion === "failure") return "var(--danger)";
  if (conclusion === "cancelled") return "var(--text-muted)";
  return "var(--text-secondary)";
};

const deploymentStateColor = (state: string): string => {
  const normalized = state.toLowerCase();
  if (normalized.includes("success") || normalized === "active") return "var(--success)";
  if (normalized.includes("progress") || normalized.includes("running") || normalized === "pending") return "var(--warning)";
  if (normalized.includes("fail") || normalized.includes("error")) return "var(--danger)";
  return "var(--text-secondary)";
};

function timeAgo(iso: string): string {
  const numeric = Number(iso);
  const date = Number.isFinite(numeric)
    ? new Date(iso.length > 11 ? numeric : numeric * 1000)
    : new Date(iso);
  const diff = Math.floor((Date.now() - date.getTime()) / 1000);
  if (Number.isNaN(diff)) return iso;
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

function parseRepoInfo(remoteUrl: string): RepoInfo | null {
  const trimmed = remoteUrl.trim();
  if (!trimmed) return null;
  const match = trimmed.match(/(?:git@|https?:\/\/)(?:[^/:]+)[:/]([^/]+)\/(.+?)(?:\.git)?$/i);
  if (!match) return null;
  const owner = match[1];
  const repo = match[2];
  const lower = trimmed.toLowerCase();
  if (lower.includes("github")) return { provider: "github", owner, repo };
  if (lower.includes("gitlab")) return { provider: "gitlab", owner, repo };
  return null;
}

function normalizeEnvironment(rawEnvironment: string): DeploymentStatus["environment"] | null {
  const value = rawEnvironment.toLowerCase();
  if (/\b(prod|production|live)\b/.test(value)) return "prod";
  if (/\b(stage|staging|qa|test)\b/.test(value)) return "staging";
  if (/\b(dev|development|preview)\b/.test(value)) return "dev";
  return null;
}

function environmentLabel(environment: DeploymentStatus["environment"]): string {
  if (environment === "dev") return "Dev";
  if (environment === "staging") return "Staging";
  return "Prod";
}

function gitlabRunState(status: string): Pick<WorkflowRun, "status" | "conclusion"> {
  const normalized = status.toLowerCase();
  if (["created", "pending", "preparing", "waiting_for_resource"].includes(normalized)) {
    return { status: "queued", conclusion: null };
  }
  if (["running", "manual", "scheduled"].includes(normalized)) {
    return { status: "in_progress", conclusion: null };
  }
  if (normalized === "success") {
    return { status: "completed", conclusion: "success" };
  }
  if (["failed", "error"].includes(normalized)) {
    return { status: "completed", conclusion: "failure" };
  }
  if (["canceled", "cancelled"].includes(normalized)) {
    return { status: "completed", conclusion: "cancelled" };
  }
  if (normalized === "skipped") {
    return { status: "completed", conclusion: "skipped" };
  }
  return { status: "completed", conclusion: null };
}

function gitlabJobState(status: string): Pick<CiJob, "status" | "conclusion"> {
  const normalized = status.toLowerCase();
  if (["created", "pending", "preparing", "waiting_for_resource"].includes(normalized)) {
    return { status: "queued", conclusion: null };
  }
  if (["running", "manual", "scheduled"].includes(normalized)) {
    return { status: "in_progress", conclusion: null };
  }
  if (normalized === "success") {
    return { status: "completed", conclusion: "success" };
  }
  if (["failed", "error"].includes(normalized)) {
    return { status: "completed", conclusion: "failure" };
  }
  if (["canceled", "cancelled"].includes(normalized)) {
    return { status: "completed", conclusion: "cancelled" };
  }
  if (normalized === "skipped") {
    return { status: "completed", conclusion: "skipped" };
  }
  return { status: "completed", conclusion: null };
}

// ── Run row ──────────────────────────────────────────────────────────────────

function RunRow({
  run, selected, onSelect, onRerun,
}: {
  run: WorkflowRun;
  selected: boolean;
  onSelect: () => void;
  onRerun: () => void;
}) {
  const color = statusColor(run.status, run.conclusion);
  const icon = statusIcon(run.status, run.conclusion);
  return (
    <div
      onClick={onSelect}
      style={{
        display: "flex", alignItems: "center", gap: 8,
        padding: "6px 10px", cursor: "pointer", fontSize: 12,
        borderRadius: "var(--radius-sm)",
        background: selected ? "var(--bg-active)" : "transparent",
        borderLeft: `3px solid ${selected ? "var(--accent)" : "transparent"}`,
        transition: "background 0.12s, border-color 0.12s",
      }}
    >
      <span style={{ color, fontSize: 13, width: 14, textAlign: "center", flexShrink: 0 }}>{icon}</span>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ color: "var(--text-primary)", fontWeight: 500, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
          {run.workflow_name || run.name}
        </div>
        <div style={{ fontSize: 10, color: "var(--text-muted)", marginTop: 1 }}>
          {run.head_branch} · {run.head_sha.slice(0, 7)} · {timeAgo(run.updated_at)}
        </div>
      </div>
      <button
        onClick={(e) => { e.stopPropagation(); onRerun(); }}
        title="Re-run workflow"
        style={{
          background: "none", border: "1px solid var(--border-color)", borderRadius: "var(--radius-sm)",
          color: "var(--text-muted)", cursor: "pointer", fontSize: 10, padding: "2px 6px",
          transition: "color 0.12s, border-color 0.12s",
        }}
        onMouseEnter={e => { (e.target as HTMLElement).style.color = "var(--accent)"; (e.target as HTMLElement).style.borderColor = "var(--accent)"; }}
        onMouseLeave={e => { (e.target as HTMLElement).style.color = "var(--text-muted)"; (e.target as HTMLElement).style.borderColor = "var(--border-color)"; }}
      >
        ↻ Re-run
      </button>
    </div>
  );
}

// ── Job detail ───────────────────────────────────────────────────────────────

function JobDetail({ job }: { job: CiJob }) {
  const [expanded, setExpanded] = useState(false);
  const color = statusColor(job.status, job.conclusion);
  const icon = statusIcon(job.status, job.conclusion);
  return (
    <div style={{ marginBottom: 4 }}>
      <div
        onClick={() => setExpanded(e => !e)}
        style={{ display: "flex", alignItems: "center", gap: 6, cursor: "pointer", padding: "4px 6px", borderRadius: "var(--radius-sm)", transition: "background 0.1s" }}
        onMouseEnter={e => (e.currentTarget.style.background = "var(--bg-hover)")}
        onMouseLeave={e => (e.currentTarget.style.background = "transparent")}
      >
        <span style={{ color, fontSize: 12, width: 14, textAlign: "center" }}>{icon}</span>
        <span style={{ fontSize: 11, color: "var(--text-primary)", flex: 1 }}>{job.name}</span>
        {job.stage && (
          <span style={{
            fontSize: 9,
            color: "var(--accent)",
            border: "1px solid var(--accent)",
            borderRadius: 999,
            padding: "1px 6px",
            textTransform: "uppercase",
          }}>
            {job.stage}
          </span>
        )}
        <span style={{ fontSize: 10, color: "var(--text-muted)" }}>{expanded ? "▲" : "▼"}</span>
      </div>
      {expanded && job.steps.length > 0 && (
        <div style={{ paddingLeft: 20, borderLeft: "2px solid var(--border-color)", marginLeft: 7 }}>
          {job.steps.map(step => (
            <div key={step.number} style={{
              display: "flex", alignItems: "center", gap: 6, padding: "2px 6px",
              fontSize: 10, color: step.conclusion === "failure" ? "var(--danger)" : step.conclusion === "success" ? "var(--success)" : "var(--text-muted)",
            }}>
              <span style={{ width: 10 }}>{step.conclusion === "failure" ? "✗" : step.conclusion === "success" ? "✓" : step.status === "in_progress" ? "⟳" : "·"}</span>
              <span>{step.name}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Main panel ───────────────────────────────────────────────────────────────

export default function CiCdPanel({ rootPath }: { rootPath: string }) {
  const [runs, setRuns] = useState<WorkflowRun[]>([]);
  const [jobs, setJobs] = useState<CiJob[]>([]);
  const [selectedRun, setSelectedRun] = useState<WorkflowRun | null>(null);
  const [loading, setLoading] = useState(false);
  const [jobsLoading, setJobsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [provider, setProvider] = useState<"github" | "gitlab" | "none">("none");
  const [autoRefresh, setAutoRefresh] = useState(false);
  const [repoInfo, setRepoInfo] = useState<RepoInfo | null>(null);
  const [deployments, setDeployments] = useState<DeploymentStatus[]>([]);
  const [deploymentError, setDeploymentError] = useState<string | null>(null);
  const [jobLog, setJobLog] = useState<{ jobId: number; jobName: string; content: string } | null>(null);
  const [jobLogLoadingId, setJobLogLoadingId] = useState<number | null>(null);

  const runShell = useCallback(async (cmd: string) => {
    return invoke<string>("shell_exec", { command: cmd, cwd: rootPath });
  }, [rootPath]);

  const detectProvider = useCallback(async () => {
    try {
      const result = await runShell("git remote get-url origin");
      const parsed = typeof result === "string" ? parseRepoInfo(result) : null;
      setRepoInfo(parsed);
      setProvider(parsed?.provider ?? "none");
      return parsed;
    } catch {
      setRepoInfo(null);
      setProvider("none");
      return null;
    }
  }, [runShell]);

  const fetchRuns = useCallback(async (info: RepoInfo | null) => {
    if (!info) {
      setRuns([]);
      setLoading(false);
      return;
    }
    setLoading(true);
    setError(null);
    try {
      if (info.provider === "github") {
        const result = await runShell("gh run list --limit 20 --json id,name,status,conclusion,headBranch,headSha,createdAt,updatedAt,url,workflowName");
        const parsed = JSON.parse(result) as Array<{
          id: number; name: string; status: string; conclusion: string;
          headBranch: string; headSha: string; createdAt: string; updatedAt: string;
          url: string; workflowName: string;
        }>;
        setRuns(parsed.map(r => ({
          id: r.id, name: r.name,
          status: r.status as WorkflowRun["status"],
          conclusion: (r.conclusion as WorkflowRun["conclusion"]) || null,
          head_branch: r.headBranch, head_sha: r.headSha,
          created_at: r.createdAt, updated_at: r.updatedAt,
          html_url: r.url, workflow_name: r.workflowName,
        })));
      } else {
        const projectId = encodeURIComponent(`${info.owner}/${info.repo}`);
        const result = await runShell(`glab api projects/${projectId}/pipelines?per_page=20`);
        const parsed = JSON.parse(result) as Array<{
          id: number;
          status: string;
          ref: string;
          sha: string;
          created_at: string;
          updated_at?: string;
          web_url?: string;
          name?: string | null;
          source?: string;
        }>;
        setRuns(parsed.map((pipeline) => {
          const state = gitlabRunState(pipeline.status || "");
          return {
            id: pipeline.id,
            name: pipeline.name || `Pipeline #${pipeline.id}`,
            status: state.status,
            conclusion: state.conclusion,
            head_branch: pipeline.ref || "",
            head_sha: pipeline.sha || "",
            created_at: pipeline.created_at,
            updated_at: pipeline.updated_at || pipeline.created_at,
            html_url: pipeline.web_url || "",
            workflow_name: pipeline.source ? `GitLab · ${pipeline.source}` : "GitLab Pipeline",
          };
        }));
      }
    } catch (e) {
      setError(
        info.provider === "gitlab"
          ? `Failed to load GitLab pipelines: ${String(e)}`
          : `Failed to load GitHub workflow runs: ${String(e)}`
      );
    }
    setLoading(false);
  }, [runShell]);

  const fetchJobs = useCallback(async (runId: number, info: RepoInfo | null) => {
    if (!info) {
      setJobs([]);
      setJobsLoading(false);
      return;
    }
    setJobsLoading(true);
    try {
      if (info.provider === "github") {
        const result = await runShell(`gh run view ${runId} --json jobs`);
        const parsed = JSON.parse(result) as { jobs: CiJob[] };
        setJobs(parsed.jobs || []);
      } else {
        const projectId = encodeURIComponent(`${info.owner}/${info.repo}`);
        const result = await runShell(`glab api projects/${projectId}/pipelines/${runId}/jobs?per_page=100`);
        const parsed = JSON.parse(result) as Array<{
          id: number;
          name: string;
          status: string;
          stage?: string | null;
          started_at?: string | null;
          finished_at?: string | null;
          web_url?: string | null;
        }>;
        setJobs(parsed.map((job) => {
          const state = gitlabJobState(job.status || "");
          return {
            id: job.id,
            name: job.name,
            status: state.status,
            conclusion: state.conclusion,
            started_at: job.started_at || null,
            completed_at: job.finished_at || null,
            stage: job.stage || null,
            html_url: job.web_url || null,
            steps: job.stage ? [{ name: job.stage, status: job.status, conclusion: state.conclusion, number: 1 }] : [],
          };
        }));
      }
    } catch { setJobs([]); }
    setJobsLoading(false);
  }, [runShell]);

  const rerunWorkflow = useCallback(async (runId: number, info: RepoInfo | null) => {
    if (!info) return;
    try {
      if (info.provider === "github") {
        await runShell(`gh run rerun ${runId}`);
      } else {
        const projectId = encodeURIComponent(`${info.owner}/${info.repo}`);
        await runShell(`glab api -X POST projects/${projectId}/pipelines/${runId}/retry`);
      }
      setTimeout(() => void fetchRuns(info), 2000);
    } catch (e) {
      setError(`Re-run failed: ${String(e)}`);
    }
  }, [fetchRuns, runShell]);

  const fetchDeployments = useCallback(async (info: RepoInfo | null) => {
    if (!info) {
      setDeployments([]);
      setDeploymentError(null);
      return;
    }

    try {
      setDeploymentError(null);
      if (info.provider === "github") {
        const raw = await runShell(`gh api repos/${info.owner}/${info.repo}/deployments?per_page=24`);
        const parsed = JSON.parse(raw) as Array<{
          id: number;
          environment: string;
          ref: string;
          sha: string;
          created_at: string;
        }>;
        const latestByEnvironment = new Map<DeploymentStatus["environment"], DeploymentStatus>();
        for (const deployment of parsed.slice(0, 12)) {
          const environment = normalizeEnvironment(deployment.environment || "");
          if (!environment || latestByEnvironment.has(environment)) continue;
          let state = "pending";
          let url: string | null = null;
          try {
            const statusesRaw = await runShell(`gh api repos/${info.owner}/${info.repo}/deployments/${deployment.id}/statuses?per_page=1`);
            const statuses = JSON.parse(statusesRaw) as Array<{ state?: string; log_url?: string | null; environment_url?: string | null }>;
            const latest = statuses[0];
            state = latest?.state || state;
            url = latest?.environment_url || latest?.log_url || null;
          } catch {
            // Leave default status when the deployment has no status records yet.
          }
          latestByEnvironment.set(environment, {
            environment,
            environmentLabel: environmentLabel(environment),
            provider: "github",
            ref: deployment.ref,
            sha: deployment.sha,
            created_at: deployment.created_at,
            state,
            url,
          });
        }
        setDeployments(Array.from(latestByEnvironment.values()));
      } else {
        const projectId = encodeURIComponent(`${info.owner}/${info.repo}`);
        const raw = await runShell(`glab api projects/${projectId}/deployments?per_page=24`);
        const parsed = JSON.parse(raw) as Array<{
          environment?: { name?: string };
          ref?: string;
          sha?: string;
          created_at?: string;
          status?: string;
          deployable?: { web_url?: string | null };
        }>;
        const latestByEnvironment = new Map<DeploymentStatus["environment"], DeploymentStatus>();
        for (const deployment of parsed) {
          const environment = normalizeEnvironment(deployment.environment?.name || "");
          if (!environment || latestByEnvironment.has(environment)) continue;
          latestByEnvironment.set(environment, {
            environment,
            environmentLabel: environmentLabel(environment),
            provider: "gitlab",
            ref: deployment.ref || "",
            sha: deployment.sha || "",
            created_at: deployment.created_at || new Date().toISOString(),
            state: deployment.status || "unknown",
            url: deployment.deployable?.web_url || null,
          });
        }
        setDeployments(Array.from(latestByEnvironment.values()));
      }
    } catch (e) {
      setDeployments([]);
      setDeploymentError(String(e));
    }
  }, [runShell]);

  const fetchJobLog = useCallback(async (job: CiJob, info: RepoInfo | null) => {
    if (!info) return;
    setJobLogLoadingId(job.id);
    try {
      let content = "";
      if (info.provider === "github") {
        content = await runShell(`gh run view --job ${job.id} --log`);
      } else {
        const projectId = encodeURIComponent(`${info.owner}/${info.repo}`);
        content = await runShell(`glab api projects/${projectId}/jobs/${job.id}/trace`);
      }
      setJobLog({ jobId: job.id, jobName: job.name, content });
    } catch (e) {
      setError(`Failed to load job log: ${String(e)}`);
    } finally {
      setJobLogLoadingId(null);
    }
  }, [runShell]);

  useEffect(() => {
    const init = async () => {
      const info = await detectProvider();
      await fetchRuns(info);
      await fetchDeployments(info);
    };
    void init();
  }, [detectProvider, fetchDeployments, fetchRuns]);

  useEffect(() => {
    if (!autoRefresh) return;
    const t = setInterval(() => {
      void fetchRuns(repoInfo);
      void fetchDeployments(repoInfo);
    }, 30_000);
    return () => clearInterval(t);
  }, [autoRefresh, fetchDeployments, fetchRuns, repoInfo]);

  useEffect(() => {
    setJobLog(null);
    if (selectedRun) void fetchJobs(selectedRun.id, repoInfo);
  }, [selectedRun, fetchJobs, repoInfo]);

  const inProgress = runs.filter(r => r.status === "in_progress").length;
  const failures = runs.filter(r => r.conclusion === "failure").length;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", background: "var(--bg-primary)", color: "var(--text-primary)" }}>
      {/* Header */}
      <div style={{
        padding: "8px 12px", background: "var(--bg-secondary)",
        borderBottom: "1px solid var(--border-color)",
        display: "flex", alignItems: "center", gap: 8, flexShrink: 0,
      }}>
        <span style={{ fontWeight: 700, fontSize: 12, color: "var(--accent)" }}>CI/CD</span>
        {inProgress > 0 && (
          <span style={{ background: "var(--warning-dim)", color: "var(--warning)", fontSize: 9, fontWeight: 700, padding: "1px 5px", borderRadius: 8 }}>
            {inProgress} running
          </span>
        )}
        {failures > 0 && (
          <span style={{ background: "var(--danger-dim)", color: "var(--danger)", fontSize: 9, fontWeight: 700, padding: "1px 5px", borderRadius: 8 }}>
            {failures} failed
          </span>
        )}
        <div style={{ marginLeft: "auto", display: "flex", gap: 6, alignItems: "center" }}>
          <label style={{ fontSize: 9, color: "var(--text-muted)", display: "flex", alignItems: "center", gap: 4, cursor: "pointer" }}>
            <input type="checkbox" checked={autoRefresh} onChange={e => setAutoRefresh(e.target.checked)} style={{ width: 10 }} />
            Auto-refresh
          </label>
          <button
            onClick={() => {
              void fetchRuns(repoInfo);
              void fetchDeployments(repoInfo);
            }}
            disabled={loading}
            style={{ background: "none", border: "1px solid var(--border-color)", borderRadius: "var(--radius-sm)", color: "var(--text-secondary)", cursor: "pointer", fontSize: 10, padding: "2px 8px" }}
          >
            {loading ? "⟳" : "Refresh"}
          </button>
        </div>
      </div>

      {error && (
        <div style={{ padding: "8px 12px", background: "var(--danger-dim)", color: "var(--danger)", fontSize: 11, borderBottom: "1px solid var(--border-color)" }}>
          {error}
          <div style={{ marginTop: 4, fontSize: 10, color: "var(--text-secondary)" }}>
            {provider === "gitlab"
              ? <span>Make sure <code>glab</code> is installed and authenticated: <code>glab auth login</code></span>
              : <span>Make sure <code>gh</code> is installed and authenticated: <code>gh auth login</code></span>}
          </div>
        </div>
      )}

      <div style={{
        padding: "8px 12px",
        borderBottom: "1px solid var(--border-color)",
        background: "rgba(255,255,255,0.02)",
        display: "flex",
        flexDirection: "column",
        gap: 8,
      }}>
        <div style={{ fontSize: 10, fontWeight: 700, color: "var(--text-secondary)", textTransform: "uppercase", letterSpacing: 0.5 }}>
          Deployment Status
        </div>
        {deploymentError ? (
          <div style={{ fontSize: 11, color: "var(--warning)" }}>
            Deployment status unavailable. Authenticate the matching provider CLI to load environment data.
          </div>
        ) : deployments.length === 0 ? (
          <div style={{ fontSize: 11, color: "var(--text-muted)" }}>
            No dev, staging, or prod deployments detected for this repository yet.
          </div>
        ) : (
          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fit, minmax(132px, 1fr))", gap: 8 }}>
            {deployments
              .slice()
              .sort((a, b) => ["dev", "staging", "prod"].indexOf(a.environment) - ["dev", "staging", "prod"].indexOf(b.environment))
              .map((deployment) => (
                <div
                  key={deployment.environment}
                  style={{
                    border: "1px solid var(--border-color)",
                    borderRadius: "var(--radius-md)",
                    background: "var(--bg-secondary)",
                    padding: "8px 10px",
                    display: "flex",
                    flexDirection: "column",
                    gap: 4,
                  }}
                >
                  <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8 }}>
                    <span style={{ fontSize: 11, fontWeight: 700, color: "var(--text-primary)" }}>
                      {deployment.environmentLabel}
                    </span>
                    <span style={{ fontSize: 10, color: deploymentStateColor(deployment.state), fontWeight: 700 }}>
                      {deployment.state}
                    </span>
                  </div>
                  <div style={{ fontSize: 11, color: "var(--text-secondary)" }}>
                    {deployment.ref || "unknown-ref"}
                  </div>
                  <div style={{ fontSize: 10, color: "var(--text-muted)" }}>
                    {deployment.sha ? deployment.sha.slice(0, 7) : "unknown"} · {timeAgo(deployment.created_at)}
                  </div>
                  {deployment.url && (
                    <a
                      href={deployment.url}
                      target="_blank"
                      rel="noreferrer"
                      style={{ fontSize: 10, color: "var(--accent)", textDecoration: "none" }}
                    >
                      Open deployment ↗
                    </a>
                  )}
                </div>
              ))}
          </div>
        )}
      </div>

      <DevOpsIntegrations />

      <div style={{ flex: 1, overflow: "hidden", display: "flex" }}>
        {/* Runs list */}
        <div style={{
          width: selectedRun ? "45%" : "100%",
          borderRight: selectedRun ? "1px solid var(--border-color)" : "none",
          overflow: "auto", padding: "6px 4px",
          transition: "width 0.18s ease",
        }}>
          {loading && runs.length === 0 ? (
            <div style={{ padding: 16, display: "flex", flexDirection: "column", gap: 8 }}>
              {[80, 65, 75, 55, 70].map((w, i) => (
                <div key={i} className="skeleton skeleton-text" style={{ width: `${w}%` }} />
              ))}
            </div>
          ) : runs.length === 0 ? (
            <div style={{ padding: 16, color: "var(--text-muted)", fontSize: 12, textAlign: "center" }}>
              <div style={{ fontSize: 24, marginBottom: 8 }}>⚙️</div>
              <div>No workflow runs found.</div>
              <div style={{ marginTop: 4, fontSize: 10 }}>
                {provider === "none"
                  ? "Not a GitHub/GitLab project."
                  : provider === "gitlab"
                    ? "Install and authenticate: glab auth login"
                    : "Install and authenticate: gh auth login"}
              </div>
            </div>
          ) : (
            runs.map(run => (
              <RunRow
                key={run.id}
                run={run}
                selected={selectedRun?.id === run.id}
                onSelect={() => setSelectedRun(prev => prev?.id === run.id ? null : run)}
                onRerun={() => void rerunWorkflow(run.id, repoInfo)}
              />
            ))
          )}
        </div>

        {/* Job detail pane */}
        {selectedRun && (
          <div style={{ flex: 1, overflow: "auto", padding: "8px 10px" }}>
            <div style={{ fontSize: 11, fontWeight: 700, color: "var(--text-secondary)", marginBottom: 8, textTransform: "uppercase", letterSpacing: 0.5 }}>
              {selectedRun.workflow_name || selectedRun.name}
            </div>
            <a
              href={selectedRun.html_url}
              target="_blank" rel="noreferrer"
              style={{ fontSize: 10, color: "var(--accent)", textDecoration: "none", display: "block", marginBottom: 8 }}
            >
              Open in browser ↗
            </a>
            {jobsLoading ? (
              <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                {[90, 70, 80].map((w, i) => (
                  <div key={i} className="skeleton skeleton-text" style={{ width: `${w}%` }} />
                ))}
              </div>
            ) : jobs.length === 0 ? (
              <div style={{ color: "var(--text-muted)", fontSize: 11 }}>No job data available.</div>
            ) : (
              <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                {jobs.map(job => (
                  <div key={job.id} style={{ borderBottom: "1px solid rgba(255,255,255,0.04)", paddingBottom: 6 }}>
                    <JobDetail job={job} />
                    <div style={{ display: "flex", alignItems: "center", gap: 8, paddingLeft: 26, marginTop: 2 }}>
                      <button
                        onClick={() => void fetchJobLog(job, repoInfo)}
                        style={{
                          background: "none",
                          border: "1px solid var(--border-color)",
                          borderRadius: "var(--radius-sm)",
                          color: jobLog?.jobId === job.id ? "var(--accent)" : "var(--text-muted)",
                          cursor: "pointer",
                          fontSize: 10,
                          padding: "2px 8px",
                        }}
                      >
                        {jobLogLoadingId === job.id ? "Loading..." : "View Log"}
                      </button>
                      {job.html_url && (
                        <a
                          href={job.html_url}
                          target="_blank"
                          rel="noreferrer"
                          style={{ fontSize: 10, color: "var(--accent)", textDecoration: "none" }}
                        >
                          Open job ↗
                        </a>
                      )}
                    </div>
                  </div>
                ))}
                {jobLog && (
                  <div style={{
                    marginTop: 8,
                    borderTop: "1px solid var(--border-color)",
                    paddingTop: 8,
                  }}>
                    <div style={{ fontSize: 10, fontWeight: 700, color: "var(--text-secondary)", textTransform: "uppercase", letterSpacing: 0.5, marginBottom: 6 }}>
                      Job Log · {jobLog.jobName}
                    </div>
                    <pre style={{
                      margin: 0,
                      maxHeight: 260,
                      overflow: "auto",
                      background: "var(--bg-secondary)",
                      border: "1px solid var(--border-color)",
                      borderRadius: "var(--radius-sm)",
                      padding: 10,
                      fontSize: 10,
                      lineHeight: 1.5,
                      whiteSpace: "pre-wrap",
                      wordBreak: "break-word",
                    }}>
                      {jobLog.content || "(empty log)"}
                    </pre>
                  </div>
                )}
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
