import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface ServerInfo {
  running: boolean;
  port: number;
  local_ip: string;
  connected_clients: ConnectedClient[];
}

interface ConnectedClient {
  id: string;
  addr: string;
  connected_at: number;
}

interface PairedDevice {
  id: string;
  name: string;
  fingerprint: string;
  paired_at: string;
  permissions: string[];
}

interface NetworkInfo {
  local_ip: string;
  tailscale_ip: string | null;
  tailscale_hostname: string | null;
  wireguard_ip: string | null;
}

interface RemoteSettingsProps {
  visible: boolean;
}

interface RemoteRecordingSummary {
  id: string;
  label: string | null;
  started_at: number;
  ended_at: number | null;
  event_count: number;
  file_path: string;
  active: boolean;
  size_bytes: number;
}

interface RemoteRecordingEntry {
  timestamp: number;
  direction: string;
  client_id: string | null;
  request_id: number | null;
  message_type: string;
  payload: unknown;
}

const REMOTE_PERMISSION_OPTIONS = [
  "filesystem",
  "terminal",
  "llm",
  "workspace",
  "agent",
] as const;

export default function RemoteSettings({ visible }: RemoteSettingsProps) {
  const [serverInfo, setServerInfo] = useState<ServerInfo | null>(null);
  const [pairedDevices, setPairedDevices] = useState<PairedDevice[]>([]);
  const [qrDataUrl, setQrDataUrl] = useState<string | null>(null);
  const [pairingToken, setPairingToken] = useState<string | null>(null);
  const [port, setPort] = useState("9876");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [fingerprint, setFingerprint] = useState<string | null>(null);
  const [certExpiry, setCertExpiry] = useState<number | null>(null);
  const [networkInfo, setNetworkInfo] = useState<NetworkInfo | null>(null);
  const [recordingLabel, setRecordingLabel] = useState("");
  const [recordingStatus, setRecordingStatus] = useState<RemoteRecordingSummary | null>(null);
  const [recordings, setRecordings] = useState<RemoteRecordingSummary[]>([]);
  const [selectedRecordingId, setSelectedRecordingId] = useState<string | null>(null);
  const [recordingEntries, setRecordingEntries] = useState<RemoteRecordingEntry[]>([]);
  const [recordingBusy, setRecordingBusy] = useState(false);
  const [savingDeviceId, setSavingDeviceId] = useState<string | null>(null);

  const refreshInfo = useCallback(async () => {
    try {
      const info = await invoke<ServerInfo>("remote_get_info");
      setServerInfo(info);
      const devices = await invoke<PairedDevice[]>("remote_list_devices");
      setPairedDevices(devices ?? []);
    } catch (err) {
      console.error("Failed to get remote info:", err);
    }
  }, []);

  const refreshRecordings = useCallback(async () => {
    try {
      const [status, list] = await Promise.all([
        invoke<RemoteRecordingSummary | null>("remote_get_recording_status"),
        invoke<RemoteRecordingSummary[]>("remote_list_recordings"),
      ]);
      setRecordingStatus(status ?? null);
      setRecordings(list ?? []);
    } catch (err) {
      console.error("Failed to load remote recordings:", err);
    }
  }, []);

  const loadRecording = useCallback(async (recordingId: string) => {
    try {
      const entries = await invoke<RemoteRecordingEntry[]>("remote_load_recording", {
        recordingId,
        limit: 120,
      });
      setSelectedRecordingId(recordingId);
      setRecordingEntries(entries ?? []);
    } catch (err) {
      setError(String(err));
    }
  }, []);

  useEffect(() => {
    if (visible) {
      refreshInfo();
      refreshRecordings();
    }
  }, [visible, refreshInfo, refreshRecordings]);

  // Fetch cert expiry and network info when component loads
  useEffect(() => {
    invoke<number | null>("remote_check_cert_expiry")
      .then(days => setCertExpiry(days))
      .catch(() => {});
    invoke<NetworkInfo>("remote_detect_network")
      .then(info => setNetworkInfo(info))
      .catch(() => {});
  }, []);

  // Listen for server events
  useEffect(() => {
    const promises = [
      listen("remote-server-started", () => refreshInfo()),
      listen("remote-server-stopped", () => refreshInfo()),
      listen("remote-client-connected", () => refreshInfo()),
      listen("remote-client-disconnected", () => refreshInfo()),
      listen<string>("remote-server-error", (e) => {
        setError(e.payload);
        setLoading(false);
      }),
    ];

    return () => {
      promises.forEach((p) => p.then((fn) => fn()));
    };
  }, [refreshInfo]);

  const generateCert = async () => {
    setLoading(true);
    setError(null);
    try {
      const fp = await invoke<string>("remote_generate_cert");
      setFingerprint(fp);
    } catch (err) {
      setError(String(err));
    }
    setLoading(false);
  };

  const startServer = async () => {
    setLoading(true);
    setError(null);
    try {
      await invoke("remote_start_server", {
        port: parseInt(port) || 9876,
      });
      await refreshInfo();
    } catch (err) {
      setError(String(err));
    }
    setLoading(false);
  };

  const stopServer = async () => {
    setLoading(true);
    try {
      await invoke("remote_stop_server");
      await refreshInfo();
    } catch (err) {
      setError(String(err));
    }
    setLoading(false);
  };

  const showQrCode = async () => {
    try {
      const [dataUrl, token] = await invoke<[string, string]>(
        "remote_get_qr_code"
      );
      setQrDataUrl(dataUrl);
      setPairingToken(token);
    } catch (err) {
      setError(String(err));
    }
  };

  const removeDevice = async (id: string) => {
    try {
      await invoke("remote_remove_device", { id });
      await refreshInfo();
    } catch (err) {
      setError(String(err));
    }
  };

  const updateDevicePermission = async (
    device: PairedDevice,
    permission: typeof REMOTE_PERMISSION_OPTIONS[number],
  ) => {
    if (device.permissions.length === 1 && device.permissions.includes(permission)) {
      setError("Each paired device must keep at least one remote permission.");
      return;
    }
    setSavingDeviceId(device.id);
    const nextPermissions = device.permissions.includes(permission)
      ? device.permissions.filter((entry) => entry !== permission)
      : [...device.permissions, permission];
    try {
      await invoke("remote_update_device_permissions", {
        id: device.id,
        permissions: nextPermissions,
      });
      await refreshInfo();
    } catch (err) {
      setError(String(err));
    } finally {
      setSavingDeviceId(null);
    }
  };

  const startRecording = async () => {
    setRecordingBusy(true);
    setError(null);
    try {
      const summary = await invoke<RemoteRecordingSummary>("remote_start_recording", {
        label: recordingLabel.trim() || null,
      });
      setRecordingStatus(summary);
      setRecordingLabel("");
      await refreshRecordings();
    } catch (err) {
      setError(String(err));
    } finally {
      setRecordingBusy(false);
    }
  };

  const stopRecording = async () => {
    setRecordingBusy(true);
    setError(null);
    try {
      const summary = await invoke<RemoteRecordingSummary>("remote_stop_recording");
      setRecordingStatus(null);
      await refreshRecordings();
      await loadRecording(summary.id);
    } catch (err) {
      setError(String(err));
    } finally {
      setRecordingBusy(false);
    }
  };

  const formatRecordingTime = (timestamp: number | null) => {
    if (!timestamp) return "Still recording";
    return new Date(timestamp * 1000).toLocaleString();
  };

  if (!visible) return null;

  return (
    <div className="remote-settings">
      <div className="remote-header">
        <span className="remote-title">REMOTE ACCESS</span>
      </div>

      {error && (
        <div className="remote-error">
          {error}
          <button className="remote-dismiss" onClick={() => setError(null)}>
            ×
          </button>
        </div>
      )}

      {/* Server Status */}
      <div className="remote-section">
        <div className="remote-section-title">Server</div>

        <div className="remote-status">
          <span
            className={`remote-status-dot ${serverInfo?.running ? "running" : "stopped"}`}
          />
          <span>{serverInfo?.running ? "Running" : "Stopped"}</span>
        </div>

        {serverInfo?.running && (
          <div className="remote-info-grid">
            <span className="remote-label">Address:</span>
            <span className="remote-value">
              {serverInfo.local_ip}:{serverInfo.port}
            </span>
            <span className="remote-label">Clients:</span>
            <span className="remote-value">
              {serverInfo.connected_clients.length}
            </span>
          </div>
        )}

        {!serverInfo?.running && (
          <div className="remote-port-input">
            <label>Port:</label>
            <input
              type="number"
              value={port}
              onChange={(e) => setPort(e.target.value)}
              min="1024"
              max="65535"
            />
          </div>
        )}

        <div className="remote-actions">
          {!serverInfo?.running ? (
            <>
              <button
                className="remote-btn primary"
                onClick={startServer}
                disabled={loading}
              >
                {loading ? "Starting..." : "Start Server"}
              </button>
              <button
                className="remote-btn"
                onClick={generateCert}
                disabled={loading}
              >
                Regenerate Certificate
              </button>
            </>
          ) : (
            <button
              className="remote-btn danger"
              onClick={stopServer}
              disabled={loading}
            >
              Stop Server
            </button>
          )}
        </div>

        {fingerprint && (
          <div className="remote-fingerprint">
            <span className="remote-label">Fingerprint:</span>
            <code className="remote-fp-value">{fingerprint}</code>
          </div>
        )}

        {certExpiry !== null && (
          <div className="remote-cert-expiry">
            Certificate expires in {certExpiry} days
            {certExpiry < 30 && <span className="remote-cert-warning"> (expiring soon!)</span>}
          </div>
        )}
      </div>

      {/* Network */}
      {networkInfo && (
        <div className="remote-section">
          <div className="remote-section-title">Network</div>
          <div className="remote-info-grid">
            <span className="remote-label">Local IP:</span>
            <span className="remote-value">{networkInfo.local_ip}</span>
            {networkInfo.tailscale_ip && (
              <>
                <span className="remote-label">Tailscale IP:</span>
                <span className="remote-value">{networkInfo.tailscale_ip}</span>
              </>
            )}
            {networkInfo.tailscale_hostname && (
              <>
                <span className="remote-label">Tailscale DNS:</span>
                <span className="remote-value" style={{ fontSize: "11px" }}>
                  {networkInfo.tailscale_hostname}
                </span>
              </>
            )}
            {networkInfo.wireguard_ip && (
              <>
                <span className="remote-label">WireGuard IP:</span>
                <span className="remote-value">{networkInfo.wireguard_ip}</span>
              </>
            )}
          </div>
          {networkInfo.tailscale_ip && (
            <p className="remote-hint" style={{ color: "var(--accent)" }}>
              Tailscale detected — use {networkInfo.tailscale_ip}:{port} for secure remote access
            </p>
          )}
          {networkInfo.wireguard_ip && !networkInfo.tailscale_ip && (
            <p className="remote-hint" style={{ color: "var(--accent)" }}>
              WireGuard detected — use {networkInfo.wireguard_ip}:{port} for VPN access
            </p>
          )}
          {serverInfo?.running && (
            <div className="remote-mobile-help" style={{ marginTop: "12px", padding: "10px", background: "#1f242c", borderRadius: "6px", border: "1px solid #30363d" }}>
              <div style={{ fontSize: "12px", fontWeight: "bold", marginBottom: "6px", color: "#58a6ff" }}>📱 MOBILE CONNECTION</div>
              <p className="remote-hint" style={{ fontSize: "11px", marginBottom: "8px" }}>
                Do NOT use "localhost" on your phone. Use one of these addresses:
              </p>
              <div style={{ display: "flex", flexDirection: "column", gap: "6px" }}>
                {[networkInfo.tailscale_ip, networkInfo.wireguard_ip, networkInfo.local_ip]
                  .filter(Boolean)
                  .map(ip => (
                    <div key={ip} style={{ display: "flex", alignItems: "center", gap: "8px" }}>
                      <code style={{ color: "var(--accent)", fontSize: "11px", flex: 1, background: "#0d1117", padding: "4px 8px", borderRadius: "4px" }}>
                        {ip}:{serverInfo.port}
                      </code>
                      <button 
                        className="remote-btn small" 
                        onClick={() => navigator.clipboard.writeText(`${ip}:${serverInfo.port}`)}
                        style={{ padding: "2px 8px" }}
                      >
                        Copy
                      </button>
                    </div>
                  ))
                }
              </div>
              <p style={{ fontSize: "10px", marginTop: "10px", color: "#8b949e", fontStyle: "italic" }}>
                Note: If https fails, try http:// in your browser first.
              </p>
            </div>
          )}
        </div>
      )}

      {/* Pairing */}
      <div className="remote-section">
        <div className="remote-section-title">Pair Device</div>
        <p className="remote-hint">
          Scan the QR code from ShadowIDE on your iPhone to pair.
        </p>
        <button
          className="remote-btn"
          onClick={showQrCode}
          disabled={!serverInfo?.running}
        >
          Show QR Code
        </button>

        {qrDataUrl && (
          <div className="remote-qr">
            <img src={qrDataUrl} alt="Pairing QR Code" width={200} height={200} />
            {pairingToken && (
              <div className="remote-token">
                <span className="remote-label">Token:</span>
                <code>{pairingToken.slice(0, 8)}...</code>
              </div>
            )}
            <button
              className="remote-btn small"
              onClick={() => {
                setQrDataUrl(null);
                setPairingToken(null);
              }}
            >
              Hide
            </button>
          </div>
        )}
      </div>

      {/* Paired Devices */}
      <div className="remote-section">
        <div className="remote-section-title">
          Paired Devices ({pairedDevices.length})
        </div>
        {pairedDevices.length === 0 ? (
          <p className="remote-hint">No devices paired yet.</p>
        ) : (
          <div className="remote-device-list">
            {pairedDevices.map((device) => (
              <div key={device.id} className="remote-device">
                <div className="remote-device-info">
                  <span className="remote-device-name">{device.name}</span>
                  <span className="remote-device-fp">
                    {device.fingerprint.slice(0, 20)}...
                  </span>
                  <div style={{ display: "flex", flexWrap: "wrap", gap: "6px", marginTop: "8px" }}>
                    {REMOTE_PERMISSION_OPTIONS.map((permission) => {
                      const enabled = device.permissions.includes(permission);
                      return (
                        <button
                          key={permission}
                          className="remote-btn small"
                          disabled={savingDeviceId === device.id}
                          onClick={() => void updateDevicePermission(device, permission)}
                          style={{
                            padding: "2px 8px",
                            borderColor: enabled ? "var(--accent)" : "var(--border-color)",
                            color: enabled ? "var(--accent)" : "var(--text-muted)",
                            background: enabled ? "rgba(88, 166, 255, 0.12)" : "transparent",
                          }}
                        >
                          {enabled ? "✓" : "○"} {permission}
                        </button>
                      );
                    })}
                  </div>
                </div>
                <button
                  className="remote-btn small danger"
                  onClick={() => removeDevice(device.id)}
                >
                  Remove
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

      <div className="remote-section">
        <div className="remote-section-title">Session Recording</div>
        <p className="remote-hint">
          Record remote traffic to an NDJSON replay file, then inspect the captured requests and responses here.
        </p>

        <div className="remote-port-input">
          <label>Label:</label>
          <input
            type="text"
            value={recordingLabel}
            onChange={(e) => setRecordingLabel(e.target.value)}
            placeholder="review, bug-repro, pairing-run"
            disabled={recordingBusy || !!recordingStatus}
          />
        </div>

        <div className="remote-actions">
          {!recordingStatus ? (
            <button
              className="remote-btn primary"
              onClick={startRecording}
              disabled={recordingBusy}
            >
              {recordingBusy ? "Starting..." : "Start Recording"}
            </button>
          ) : (
            <button
              className="remote-btn danger"
              onClick={stopRecording}
              disabled={recordingBusy}
            >
              {recordingBusy ? "Stopping..." : "Stop Recording"}
            </button>
          )}
          <button
            className="remote-btn"
            onClick={() => void refreshRecordings()}
            disabled={recordingBusy}
          >
            Refresh
          </button>
        </div>

        {recordingStatus && (
          <div className="remote-info-grid" style={{ marginTop: "10px" }}>
            <span className="remote-label">Active:</span>
            <span className="remote-value">{recordingStatus.label || recordingStatus.id}</span>
            <span className="remote-label">Events:</span>
            <span className="remote-value">{recordingStatus.event_count}</span>
            <span className="remote-label">File:</span>
            <span className="remote-value" style={{ fontSize: "11px" }}>{recordingStatus.file_path}</span>
          </div>
        )}

        {recordings.length === 0 ? (
          <p className="remote-hint" style={{ marginTop: "10px" }}>No recordings yet.</p>
        ) : (
          <div className="remote-device-list" style={{ marginTop: "10px" }}>
            {recordings.map((recording) => (
              <div key={recording.id} className="remote-device">
                <div className="remote-device-info">
                  <span className="remote-device-name">
                    {recording.label || recording.id}
                    {recording.active ? " (live)" : ""}
                  </span>
                  <span className="remote-device-fp">
                    {formatRecordingTime(recording.started_at)} · {recording.event_count} events · {(recording.size_bytes / 1024).toFixed(1)} KB
                  </span>
                  <span className="remote-device-fp">
                    {formatRecordingTime(recording.ended_at)}
                  </span>
                </div>
                <button
                  className="remote-btn small"
                  onClick={() => void loadRecording(recording.id)}
                >
                  Review
                </button>
              </div>
            ))}
          </div>
        )}

        {selectedRecordingId && (
          <div style={{ marginTop: "12px" }}>
            <div className="remote-section-title" style={{ fontSize: "11px" }}>
              Recording Review
            </div>
            <div style={{
              maxHeight: "260px",
              overflowY: "auto",
              border: "1px solid var(--border-color)",
              borderRadius: "8px",
              background: "rgba(13, 17, 23, 0.55)",
              padding: "8px",
              display: "flex",
              flexDirection: "column",
              gap: "8px",
            }}>
              {recordingEntries.map((entry, index) => (
                <div
                  key={`${entry.timestamp}-${index}`}
                  style={{
                    border: "1px solid rgba(255,255,255,0.06)",
                    borderRadius: "6px",
                    padding: "8px",
                    background: entry.direction === "inbound"
                      ? "rgba(88, 166, 255, 0.08)"
                      : entry.direction === "outbound"
                        ? "rgba(46, 160, 67, 0.08)"
                        : "rgba(255, 255, 255, 0.04)",
                  }}
                >
                  <div style={{ fontSize: "11px", fontWeight: 700, color: "var(--text-primary)" }}>
                    {entry.direction} · {entry.message_type}
                  </div>
                  <div style={{ fontSize: "10px", color: "var(--text-muted)", marginTop: "2px" }}>
                    {new Date(entry.timestamp * 1000).toLocaleTimeString()} · client {entry.client_id?.slice(0, 8) || "system"}
                    {entry.request_id !== null ? ` · req ${entry.request_id}` : ""}
                  </div>
                  <pre style={{
                    margin: "8px 0 0",
                    whiteSpace: "pre-wrap",
                    wordBreak: "break-word",
                    fontSize: "10px",
                    color: "var(--text-secondary)",
                    fontFamily: "var(--editor-font-family, monospace)",
                  }}>
                    {JSON.stringify(entry.payload, null, 2)}
                  </pre>
                </div>
              ))}
            </div>
          </div>
        )}
      </div>

      {/* Connected Clients */}
      {serverInfo?.running && serverInfo.connected_clients.length > 0 && (
        <div className="remote-section">
          <div className="remote-section-title">Connected Clients</div>
          <div className="remote-device-list">
            {serverInfo.connected_clients.map((client) => (
              <div key={client.id} className="remote-device">
                <div className="remote-device-info">
                  <span className="remote-device-name">{client.addr}</span>
                  <span className="remote-device-fp">
                    ID: {client.id.slice(0, 8)}...
                  </span>
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
