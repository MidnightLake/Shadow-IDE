interface ServerStatus {
  running: boolean;
  port: number;
  model: string;
  binary: string;
  backend: string;
  error?: string;
}

interface EngineInfo {
  installed: boolean;
  binary_path: string;
  version: string;
  backend: string;
}

interface NetworkInfo {
  local_ip: string;
  local_url: string;
  tailscale_ip?: string;
  tailscale_url?: string;
  public_ip?: string;
  public_url?: string;
}

interface LlmServerStatusProps {
  serverStatus: ServerStatus | null;
  engineInfo: EngineInfo | null;
  networkInfo: NetworkInfo | null;
  launching: boolean;
  selectedModel: string;
  port: number;
  setPort: (v: number) => void;
  serverMsg: string;
  onLaunch: () => void;
  onStop: () => void;
  onUnload: () => void;
}

export default function LlmServerStatus({
  serverStatus,
  engineInfo,
  networkInfo,
  launching,
  selectedModel,
  port,
  setPort,
  serverMsg,
  onLaunch,
  onStop,
  onUnload,
}: LlmServerStatusProps) {
  return (
    <div className="llm-section">
      <div className="llm-section-title">SERVER</div>
      <div className="llm-server-status">
        <span className={`llm-status-dot ${serverStatus?.running ? "running" : "stopped"}`} />
        <span>
          {serverStatus?.running
            ? `Running on :${serverStatus.port}`
            : "Stopped"}
        </span>
        {!serverStatus?.running && !engineInfo?.installed && !serverStatus?.binary && (
          <span className="llm-hint">Install llama.cpp in Settings first</span>
        )}
      </div>
      <div className="llm-server-controls">
        <button className="llm-btn llm-btn-primary" onClick={onLaunch} disabled={launching || !selectedModel || serverStatus?.running === true}>
          {launching ? "Starting..." : "Start Server"}
        </button>
        <button className="llm-btn" onClick={onStop} disabled={!serverStatus?.running}>Stop</button>
        <button className="llm-btn" onClick={onUnload} disabled={!serverStatus?.running} title="Unload model and stop server (frees VRAM)">Unload</button>
        <input type="number" className="llm-input llm-port-input" value={port} min={1024} max={65535} onChange={(e) => setPort(Number(e.target.value))} title="Port" />
      </div>
      {serverMsg && <div className="llm-msg">{serverMsg}</div>}
      {!serverStatus?.running && serverStatus?.error && (
        <div className="llm-msg" style={{ color: "#ef4444", whiteSpace: "pre-wrap", maxHeight: 80, overflow: "auto", fontSize: 10 }}>
          {serverStatus.error}
        </div>
      )}
      {networkInfo && (
        <div style={{
          marginTop: 6, padding: "8px 10px", background: "var(--bg-secondary)",
          borderRadius: 6, fontSize: 10, lineHeight: 1.8,
          border: "1px solid var(--border)",
        }}>
          <div style={{ fontWeight: 600, marginBottom: 4, fontSize: 11, color: "var(--text-secondary)" }}>
            Mobile / Network Access
          </div>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ color: "var(--text-secondary)" }}>Local IP:</span>
            <code style={{ background: "var(--bg-primary)", padding: "2px 6px", borderRadius: 3, cursor: "pointer", fontSize: 10 }}
              onClick={() => navigator.clipboard?.writeText(networkInfo.local_url)}
              title="Click to copy">
              {networkInfo.local_url}
            </code>
          </div>
          {networkInfo.tailscale_url && (
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <span style={{ color: "var(--text-secondary)" }}>Tailscale:</span>
              <code style={{ background: "var(--bg-primary)", padding: "2px 6px", borderRadius: 3, cursor: "pointer", fontSize: 10 }}
                onClick={() => navigator.clipboard?.writeText(networkInfo.tailscale_url!)}
                title="Click to copy">
                {networkInfo.tailscale_url}
              </code>
            </div>
          )}
          {networkInfo.public_url && (
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <span style={{ color: "var(--text-secondary)" }}>Public IP:</span>
              <code style={{ background: "var(--bg-primary)", padding: "2px 6px", borderRadius: 3, cursor: "pointer", fontSize: 10 }}
                onClick={() => navigator.clipboard?.writeText(networkInfo.public_url!)}
                title="Click to copy (requires port forwarding)">
                {networkInfo.public_url}
              </code>
            </div>
          )}
          <div style={{ marginTop: 4, color: "var(--text-muted)", fontSize: 9, lineHeight: 1.5 }}>
            Server binds 0.0.0.0 — accessible from any device on the network. Click URL to copy.
            {serverStatus?.running && (
              <span> | <a href="#" style={{ color: "var(--accent)", textDecoration: "none" }}
                onClick={async (e) => {
                  e.preventDefault();
                  try {
                    const r = await fetch(`${networkInfo.local_url}/models`, { signal: AbortSignal.timeout(3000) });
                    alert(r.ok ? "LLM server reachable!" : `Server returned ${r.status}`);
                  } catch (err) { alert(`Cannot reach: ${err}`); }
                }}
              >Test connection</a></span>
            )}
          </div>
        </div>
      )}
    </div>
  );
}
