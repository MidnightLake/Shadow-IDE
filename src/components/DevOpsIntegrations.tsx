import { useEffect, useMemo, useState, type CSSProperties } from "react";
import { invoke } from "@tauri-apps/api/core";

interface CiAdapterConfig {
  id: string;
  name: string;
  provider: string;
  base_url?: string | null;
  token?: string | null;
  project_slug?: string | null;
  organization?: string | null;
  pipeline?: string | null;
  job_path?: string | null;
  branch?: string | null;
  username?: string | null;
}

interface ExternalCiRun {
  id: string;
  name: string;
  status: string;
  conclusion?: string | null;
  branch: string;
  sha: string;
  created_at: string;
  updated_at: string;
  web_url?: string | null;
}

interface ExternalCiJob {
  id: string;
  name: string;
  status: string;
  conclusion?: string | null;
  stage?: string | null;
  started_at?: string | null;
  completed_at?: string | null;
  web_url?: string | null;
}

interface FeatureFlagProviderConfig {
  id: string;
  name: string;
  provider: string;
  base_url?: string | null;
  token: string;
  project_key: string;
  environment: string;
}

interface FeatureFlagRecord {
  key: string;
  name: string;
  description: string;
  enabled: boolean;
  provider: string;
  tags: string[];
}

function ageLabel(value: string): string {
  if (!value) return "unknown";
  const numeric = Number(value);
  const date = Number.isFinite(numeric)
    ? new Date(value.length > 11 ? numeric : numeric * 1000)
    : new Date(value);
  if (Number.isNaN(date.getTime())) return value;
  const diff = Math.floor((Date.now() - date.getTime()) / 1000);
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

export default function DevOpsIntegrations() {
  const [ciConfigs, setCiConfigs] = useState<CiAdapterConfig[]>([]);
  const [selectedCiId, setSelectedCiId] = useState("");
  const [ciRuns, setCiRuns] = useState<ExternalCiRun[]>([]);
  const [ciJobs, setCiJobs] = useState<ExternalCiJob[]>([]);
  const [selectedRunId, setSelectedRunId] = useState("");
  const [ciLog, setCiLog] = useState("");
  const [ciError, setCiError] = useState("");
  const [ciLoading, setCiLoading] = useState(false);
  const [ciForm, setCiForm] = useState<CiAdapterConfig>({
    id: "",
    name: "",
    provider: "circleci",
    base_url: "",
    token: "",
    project_slug: "",
    organization: "",
    pipeline: "",
    job_path: "",
    branch: "",
    username: "",
  });

  const [flagConfigs, setFlagConfigs] = useState<FeatureFlagProviderConfig[]>([]);
  const [selectedFlagConfigId, setSelectedFlagConfigId] = useState("");
  const [flagForm, setFlagForm] = useState<FeatureFlagProviderConfig>({
    id: "",
    name: "",
    provider: "launchdarkly",
    base_url: "",
    token: "",
    project_key: "",
    environment: "",
  });
  const [flags, setFlags] = useState<FeatureFlagRecord[]>([]);
  const [flagError, setFlagError] = useState("");
  const [flagsLoading, setFlagsLoading] = useState(false);

  const selectedCi = useMemo(
    () => ciConfigs.find((config) => config.id === selectedCiId) ?? null,
    [ciConfigs, selectedCiId],
  );
  const selectedFlagConfig = useMemo(
    () => flagConfigs.find((config) => config.id === selectedFlagConfigId) ?? null,
    [flagConfigs, selectedFlagConfigId],
  );

  const loadCiConfigs = async () => {
    const configs = await invoke<CiAdapterConfig[]>("ci_adapter_list_configs");
    setCiConfigs(configs ?? []);
    if (!selectedCiId && configs?.[0]) setSelectedCiId(configs[0].id);
  };

  const loadFlagConfigs = async () => {
    const configs = await invoke<FeatureFlagProviderConfig[]>("feature_flag_list_configs");
    setFlagConfigs(configs ?? []);
    if (!selectedFlagConfigId && configs?.[0]) setSelectedFlagConfigId(configs[0].id);
  };

  useEffect(() => {
    void loadCiConfigs().catch(() => {});
    void loadFlagConfigs().catch(() => {});
  }, []);

  const loadCiRuns = async (configId: string) => {
    setCiLoading(true);
    setCiError("");
    setCiLog("");
    try {
      const runs = await invoke<ExternalCiRun[]>("ci_adapter_list_runs", { id: configId });
      setCiRuns(runs ?? []);
      setSelectedRunId(runs?.[0]?.id ?? "");
      setCiJobs([]);
    } catch (err) {
      setCiError(String(err));
      setCiRuns([]);
      setCiJobs([]);
    } finally {
      setCiLoading(false);
    }
  };

  const loadCiJobs = async (configId: string, runId: string) => {
    try {
      const jobs = await invoke<ExternalCiJob[]>("ci_adapter_list_jobs", { id: configId, runId });
      setCiJobs(jobs ?? []);
    } catch (err) {
      setCiError(String(err));
      setCiJobs([]);
    }
  };

  const loadFlags = async (configId: string) => {
    setFlagsLoading(true);
    setFlagError("");
    try {
      const nextFlags = await invoke<FeatureFlagRecord[]>("feature_flag_list_flags", { id: configId });
      setFlags(nextFlags ?? []);
    } catch (err) {
      setFlagError(String(err));
      setFlags([]);
    } finally {
      setFlagsLoading(false);
    }
  };

  useEffect(() => {
    if (!selectedCiId) return;
    void loadCiRuns(selectedCiId);
  }, [selectedCiId]);

  useEffect(() => {
    if (!selectedCiId || !selectedRunId) return;
    void loadCiJobs(selectedCiId, selectedRunId);
  }, [selectedCiId, selectedRunId]);

  useEffect(() => {
    if (!selectedFlagConfigId) return;
    void loadFlags(selectedFlagConfigId);
  }, [selectedFlagConfigId]);

  return (
    <div style={{ display: "grid", gap: 12, padding: "10px 12px", borderBottom: "1px solid var(--border-color)" }}>
      <div style={{ fontSize: 10, fontWeight: 700, color: "var(--accent)", textTransform: "uppercase", letterSpacing: 0.7 }}>
        Integrations
      </div>

      <div style={{ display: "grid", gap: 12, gridTemplateColumns: "repeat(auto-fit, minmax(280px, 1fr))" }}>
        <div style={{ border: "1px solid var(--border-color)", borderRadius: "var(--radius-md)", background: "var(--bg-secondary)", padding: 12, display: "grid", gap: 10 }}>
          <div style={{ display: "flex", justifyContent: "space-between", gap: 8, alignItems: "center" }}>
            <div>
              <div style={{ fontSize: 11, fontWeight: 700 }}>Generic CI Adapter</div>
              <div style={{ fontSize: 10, color: "var(--text-muted)" }}>CircleCI, Jenkins, Buildkite</div>
            </div>
            <select
              value={selectedCiId}
              onChange={(e) => setSelectedCiId(e.target.value)}
              style={{ background: "var(--bg-primary)", color: "var(--text-primary)", border: "1px solid var(--border-color)", borderRadius: 8, padding: "5px 8px", fontSize: 11 }}
            >
              <option value="">Select adapter</option>
              {ciConfigs.map((config) => (
                <option key={config.id} value={config.id}>{config.name}</option>
              ))}
            </select>
          </div>

          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
            <input value={ciForm.name} onChange={(e) => setCiForm((prev) => ({ ...prev, name: e.target.value }))} placeholder="Adapter name" style={inputStyle} />
            <select value={ciForm.provider} onChange={(e) => setCiForm((prev) => ({ ...prev, provider: e.target.value }))} style={inputStyle}>
              <option value="circleci">CircleCI</option>
              <option value="jenkins">Jenkins</option>
              <option value="buildkite">Buildkite</option>
            </select>
            <input value={ciForm.base_url ?? ""} onChange={(e) => setCiForm((prev) => ({ ...prev, base_url: e.target.value }))} placeholder="Base URL (optional)" style={inputStyle} />
            <input value={ciForm.branch ?? ""} onChange={(e) => setCiForm((prev) => ({ ...prev, branch: e.target.value }))} placeholder="Branch filter" style={inputStyle} />
            <input value={ciForm.token ?? ""} onChange={(e) => setCiForm((prev) => ({ ...prev, token: e.target.value }))} placeholder="Token / API key" style={inputStyle} />
            {ciForm.provider === "circleci" && (
              <input value={ciForm.project_slug ?? ""} onChange={(e) => setCiForm((prev) => ({ ...prev, project_slug: e.target.value }))} placeholder="Project slug" style={inputStyle} />
            )}
            {ciForm.provider === "buildkite" && (
              <>
                <input value={ciForm.organization ?? ""} onChange={(e) => setCiForm((prev) => ({ ...prev, organization: e.target.value }))} placeholder="Organization" style={inputStyle} />
                <input value={ciForm.pipeline ?? ""} onChange={(e) => setCiForm((prev) => ({ ...prev, pipeline: e.target.value }))} placeholder="Pipeline" style={inputStyle} />
              </>
            )}
            {ciForm.provider === "jenkins" && (
              <>
                <input value={ciForm.job_path ?? ""} onChange={(e) => setCiForm((prev) => ({ ...prev, job_path: e.target.value }))} placeholder="Job path" style={inputStyle} />
                <input value={ciForm.username ?? ""} onChange={(e) => setCiForm((prev) => ({ ...prev, username: e.target.value }))} placeholder="Username" style={inputStyle} />
              </>
            )}
          </div>

          <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
            <button
              onClick={async () => {
                const saved = await invoke<CiAdapterConfig>("ci_adapter_save_config", { config: ciForm });
                setCiForm(saved);
                setSelectedCiId(saved.id);
                await loadCiConfigs();
              }}
              style={primaryButton}
            >
              Save Adapter
            </button>
            {selectedCi && (
              <>
                <button onClick={() => void loadCiRuns(selectedCi.id)} style={secondaryButton}>{ciLoading ? "Refreshing..." : "Refresh Runs"}</button>
                <button
                  onClick={async () => {
                    await invoke("ci_adapter_delete_config", { id: selectedCi.id });
                    setSelectedCiId("");
                    setCiForm({ id: "", name: "", provider: "circleci", base_url: "", token: "", project_slug: "", organization: "", pipeline: "", job_path: "", branch: "", username: "" });
                    setCiRuns([]);
                    setCiJobs([]);
                    await loadCiConfigs();
                  }}
                  style={dangerButton}
                >
                  Delete
                </button>
              </>
            )}
          </div>

          {ciError && <div style={{ fontSize: 11, color: "var(--danger)" }}>{ciError}</div>}
          {ciRuns.length > 0 && (
            <div style={{ display: "grid", gap: 6 }}>
              {ciRuns.slice(0, 6).map((run) => (
                <div key={run.id} style={{ border: "1px solid rgba(255,255,255,0.05)", borderRadius: 10, padding: 8, background: selectedRunId === run.id ? "var(--bg-active)" : "transparent" }}>
                  <button
                    onClick={() => setSelectedRunId(run.id)}
                    style={{ background: "transparent", border: "none", color: "inherit", padding: 0, cursor: "pointer", width: "100%", textAlign: "left" }}
                  >
                    <div style={{ fontSize: 11, fontWeight: 700 }}>{run.name}</div>
                    <div style={{ fontSize: 10, color: "var(--text-muted)" }}>
                      {run.branch || "default"} · {run.sha ? run.sha.slice(0, 8) : "n/a"} · {ageLabel(run.updated_at || run.created_at)}
                    </div>
                  </button>
                  {selectedRunId === run.id && (
                    <div style={{ display: "grid", gap: 6, marginTop: 8 }}>
                      <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
                        <button onClick={() => selectedCi && void loadCiJobs(selectedCi.id, run.id)} style={secondaryButton}>Jobs</button>
                        <button onClick={async () => { if (!selectedCi) return; await invoke("ci_adapter_rerun", { id: selectedCi.id, runId: run.id }); await loadCiRuns(selectedCi.id); }} style={secondaryButton}>Re-run</button>
                        <button onClick={async () => { if (!selectedCi) return; try { const log = await invoke<string>("ci_adapter_fetch_log", { id: selectedCi.id, runId: run.id }); setCiLog(log); } catch (err) { setCiError(String(err)); } }} style={secondaryButton}>Log</button>
                        {run.web_url && <a href={run.web_url} target="_blank" rel="noreferrer" style={{ ...linkStyle, alignSelf: "center" }}>Open ↗</a>}
                      </div>
                      {ciJobs.length > 0 && (
                        <div style={{ display: "grid", gap: 4 }}>
                          {ciJobs.map((job) => (
                            <div key={job.id} style={{ fontSize: 10, color: "var(--text-secondary)", display: "flex", justifyContent: "space-between", gap: 8 }}>
                              <span>{job.name}{job.stage ? ` · ${job.stage}` : ""}</span>
                              <span>{job.conclusion ?? job.status}</span>
                            </div>
                          ))}
                        </div>
                      )}
                      {ciLog && (
                        <pre style={{ margin: 0, maxHeight: 180, overflow: "auto", borderRadius: 8, border: "1px solid rgba(255,255,255,0.05)", background: "var(--bg-primary)", padding: 8, fontSize: 10, whiteSpace: "pre-wrap" }}>
                          {ciLog}
                        </pre>
                      )}
                    </div>
                  )}
                </div>
              ))}
            </div>
          )}
        </div>

        <div style={{ border: "1px solid var(--border-color)", borderRadius: "var(--radius-md)", background: "var(--bg-secondary)", padding: 12, display: "grid", gap: 10 }}>
          <div style={{ display: "flex", justifyContent: "space-between", gap: 8, alignItems: "center" }}>
            <div>
              <div style={{ fontSize: 11, fontWeight: 700 }}>Feature Flags</div>
              <div style={{ fontSize: 10, color: "var(--text-muted)" }}>LaunchDarkly and Unleash</div>
            </div>
            <select
              value={selectedFlagConfigId}
              onChange={(e) => setSelectedFlagConfigId(e.target.value)}
              style={{ background: "var(--bg-primary)", color: "var(--text-primary)", border: "1px solid var(--border-color)", borderRadius: 8, padding: "5px 8px", fontSize: 11 }}
            >
              <option value="">Select provider</option>
              {flagConfigs.map((config) => (
                <option key={config.id} value={config.id}>{config.name}</option>
              ))}
            </select>
          </div>

          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
            <input value={flagForm.name} onChange={(e) => setFlagForm((prev) => ({ ...prev, name: e.target.value }))} placeholder="Connection name" style={inputStyle} />
            <select value={flagForm.provider} onChange={(e) => setFlagForm((prev) => ({ ...prev, provider: e.target.value }))} style={inputStyle}>
              <option value="launchdarkly">LaunchDarkly</option>
              <option value="unleash">Unleash</option>
            </select>
            <input value={flagForm.base_url ?? ""} onChange={(e) => setFlagForm((prev) => ({ ...prev, base_url: e.target.value }))} placeholder="Base URL (optional for LaunchDarkly)" style={inputStyle} />
            <input value={flagForm.environment} onChange={(e) => setFlagForm((prev) => ({ ...prev, environment: e.target.value }))} placeholder="Environment" style={inputStyle} />
            <input value={flagForm.project_key} onChange={(e) => setFlagForm((prev) => ({ ...prev, project_key: e.target.value }))} placeholder="Project key" style={inputStyle} />
            <input value={flagForm.token} onChange={(e) => setFlagForm((prev) => ({ ...prev, token: e.target.value }))} placeholder="Token" style={inputStyle} />
          </div>

          <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
            <button
              onClick={async () => {
                const saved = await invoke<FeatureFlagProviderConfig>("feature_flag_save_config", { config: flagForm });
                setFlagForm(saved);
                setSelectedFlagConfigId(saved.id);
                await loadFlagConfigs();
              }}
              style={primaryButton}
            >
              Save Provider
            </button>
            {selectedFlagConfig && (
              <>
                <button onClick={() => void loadFlags(selectedFlagConfig.id)} style={secondaryButton}>{flagsLoading ? "Refreshing..." : "Refresh Flags"}</button>
                <button
                  onClick={async () => {
                    await invoke("feature_flag_delete_config", { id: selectedFlagConfig.id });
                    setSelectedFlagConfigId("");
                    setFlagForm({ id: "", name: "", provider: "launchdarkly", base_url: "", token: "", project_key: "", environment: "" });
                    setFlags([]);
                    await loadFlagConfigs();
                  }}
                  style={dangerButton}
                >
                  Delete
                </button>
              </>
            )}
          </div>

          {flagError && <div style={{ fontSize: 11, color: "var(--danger)" }}>{flagError}</div>}
          {flags.length > 0 && (
            <div style={{ display: "grid", gap: 6, maxHeight: 260, overflowY: "auto" }}>
              {flags.map((flag) => (
                <label key={flag.key} style={{ border: "1px solid rgba(255,255,255,0.05)", borderRadius: 10, padding: 8, display: "grid", gap: 4, cursor: "pointer" }}>
                  <div style={{ display: "flex", justifyContent: "space-between", gap: 8, alignItems: "center" }}>
                    <div>
                      <div style={{ fontSize: 11, fontWeight: 700 }}>{flag.name}</div>
                      <div style={{ fontSize: 10, color: "var(--text-muted)" }}>{flag.key}</div>
                    </div>
                    <input
                      type="checkbox"
                      checked={flag.enabled}
                      onChange={async (e) => {
                        if (!selectedFlagConfig) return;
                        const enabled = e.target.checked;
                        setFlags((prev) => prev.map((entry) => entry.key === flag.key ? { ...entry, enabled } : entry));
                        try {
                          await invoke("feature_flag_set_enabled", { id: selectedFlagConfig.id, key: flag.key, enabled });
                        } catch (err) {
                          setFlagError(String(err));
                          setFlags((prev) => prev.map((entry) => entry.key === flag.key ? { ...entry, enabled: !enabled } : entry));
                        }
                      }}
                    />
                  </div>
                  {flag.description && <div style={{ fontSize: 10, color: "var(--text-secondary)" }}>{flag.description}</div>}
                  {flag.tags.length > 0 && (
                    <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
                      {flag.tags.map((tag) => (
                        <span key={tag} style={{ fontSize: 9, padding: "2px 6px", borderRadius: 999, background: "rgba(125,211,252,0.12)", color: "#7dd3fc" }}>{tag}</span>
                      ))}
                    </div>
                  )}
                </label>
              ))}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

const inputStyle: CSSProperties = {
  background: "var(--bg-primary)",
  color: "var(--text-primary)",
  border: "1px solid var(--border-color)",
  borderRadius: 8,
  padding: "6px 8px",
  fontSize: 11,
};

const primaryButton: CSSProperties = {
  borderRadius: 8,
  border: "1px solid rgba(125, 211, 252, 0.25)",
  background: "#132033",
  color: "#7dd3fc",
  padding: "6px 10px",
  cursor: "pointer",
  fontSize: 11,
};

const secondaryButton: CSSProperties = {
  borderRadius: 8,
  border: "1px solid var(--border-color)",
  background: "transparent",
  color: "var(--text-secondary)",
  padding: "6px 10px",
  cursor: "pointer",
  fontSize: 11,
};

const dangerButton: CSSProperties = {
  borderRadius: 8,
  border: "1px solid rgba(248, 113, 113, 0.24)",
  background: "#2a1418",
  color: "#fca5a5",
  padding: "6px 10px",
  cursor: "pointer",
  fontSize: 11,
};

const linkStyle: CSSProperties = {
  color: "var(--accent)",
  fontSize: 11,
  textDecoration: "none",
};
