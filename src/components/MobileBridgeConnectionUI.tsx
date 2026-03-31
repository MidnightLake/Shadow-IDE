import { useState } from "react";

interface SavedConnection {
  id: string;
  name: string;
  host: string;
  token: string;
  type?: "wifi" | "bt" | "ssh";
  username?: string;
  lastUsed: number;
}

interface BtDevice {
  index: number;
  name: string;
  id: string;
  rssi: number;
}

interface MobileBridgeConnectionUIProps {
  host: string;
  setHost: (v: string) => void;
  token: string;
  setToken: (v: string) => void;
  connType: "wifi" | "bt" | "ssh";
  setConnType: (v: "wifi" | "bt" | "ssh") => void;
  sshUser: string;
  setSshUser: (v: string) => void;
  sshPass: string;
  setSshPass: (v: string) => void;
  connectionName: string;
  setConnectionName: (v: string) => void;
  connecting: boolean;
  error: string;
  statusLogs: string[];
  savedConnections: SavedConnection[];
  btScanning: boolean;
  btDevices: BtDevice[];
  btState: string;
  onConnect: (host: string, token: string, name?: string, id?: string, type?: "wifi" | "ssh") => void;
  onStartBtScan: () => void;
  onStopBtScan: () => void;
  onConnectBtDevice: (device: BtDevice) => void;
  onDeleteSavedConnection: (id: string) => void;
  onScanQR: () => void;
  onSaveRename: (e: React.SyntheticEvent, id: string, newName: string) => void;
}

export default function MobileBridgeConnectionUI({
  host,
  setHost,
  token,
  setToken,
  connType,
  setConnType,
  sshUser,
  setSshUser,
  sshPass,
  setSshPass,
  connectionName,
  setConnectionName,
  connecting,
  error,
  statusLogs,
  savedConnections,
  btScanning,
  btDevices,
  btState,
  onConnect,
  onStartBtScan,
  onStopBtScan,
  onConnectBtDevice,
  onDeleteSavedConnection,
  onScanQR,
  onSaveRename,
}: MobileBridgeConnectionUIProps) {
  const [showSaved, setShowSaved] = useState(true);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [editName, setEditName] = useState("");

  const startRename = (e: React.MouseEvent, conn: SavedConnection) => {
    e.stopPropagation();
    setEditingId(conn.id);
    setEditName(conn.name);
  };

  const handleSaveRename = (e: React.SyntheticEvent, id: string) => {
    e.stopPropagation();
    onSaveRename(e, id, editName);
    setEditingId(null);
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", minHeight: "100vh", maxHeight: "100vh", background: "#0f172a", color: "#f8fafc", fontFamily: "system-ui, sans-serif", padding: "env(safe-area-inset-top, 20px) 20px 0 20px", boxSizing: "border-box", overflow: "hidden" }}>
      <div style={{ flex: 1, display: "flex", flexDirection: "column", justifyContent: "center", alignItems: "center", maxWidth: 400, width: "100%", margin: "0 auto" }}>
        <div style={{ marginBottom: 24, textAlign: "center" }}>
          <h1 style={{ fontSize: 32, fontWeight: 800, margin: "0 0 8px 0", letterSpacing: "-0.02em" }}>ShadowIDE</h1>
          <p style={{ color: "#94a3b8", margin: 0 }}>Connect to your PC to start coding</p>
        </div>

        <div style={{ display: "flex", width: "100%", marginBottom: 16, background: "#1e293b", borderRadius: 8, padding: 4 }}>
          {["wifi", "ssh", "bt"].map(t => (
            <button key={t} onClick={() => { setConnType(t as "wifi" | "bt" | "ssh"); if (t === "bt") onStartBtScan(); }} style={{ flex: 1, padding: "8px", border: "none", borderRadius: 6, background: connType === t ? "#3b82f6" : "transparent", color: "#fff", fontWeight: 600, fontSize: 13 }}>{t.toUpperCase()}</button>
          ))}
        </div>

        <div style={{ width: "100%", background: "#1e293b", padding: 20, borderRadius: 16, border: "1px solid #334155", boxShadow: "0 10px 25px -5px rgba(0,0,0,0.3)" }}>
          {connType !== "bt" && (
            <>
              <div style={{ marginBottom: 12 }}><label style={{ display: "block", fontSize: 11, fontWeight: 600, color: "#64748b", marginBottom: 4, textTransform: "uppercase" }}>Name</label><input style={{ width: "100%", padding: "10px", background: "#0f172a", border: "1px solid #334155", borderRadius: 8, color: "#fff", fontSize: 15, boxSizing: "border-box" }} placeholder="Work PC" value={connectionName} onChange={e => setConnectionName(e.target.value)} /></div>
              <div style={{ marginBottom: 12 }}><label style={{ display: "block", fontSize: 11, fontWeight: 600, color: "#64748b", marginBottom: 4, textTransform: "uppercase" }}>Host & Port</label><input style={{ width: "100%", padding: "10px", background: "#0f172a", border: "1px solid #334155", borderRadius: 8, color: "#fff", fontSize: 15, boxSizing: "border-box" }} placeholder={connType === "ssh" ? "192.168.1.10:22" : "192.168.1.10:9876"} value={host} onChange={e => setHost(e.target.value)} /></div>
              {connType === "ssh" && (
                <>
                  <div style={{ marginBottom: 12 }}><label style={{ display: "block", fontSize: 11, fontWeight: 600, color: "#64748b", marginBottom: 4, textTransform: "uppercase" }}>Username</label><input style={{ width: "100%", padding: "10px", background: "#0f172a", border: "1px solid #334155", borderRadius: 8, color: "#fff", fontSize: 15, boxSizing: "border-box" }} placeholder="root" value={sshUser} onChange={e => setSshUser(e.target.value)} /></div>
                  <div style={{ marginBottom: 12 }}><label style={{ display: "block", fontSize: 11, fontWeight: 600, color: "#64748b", marginBottom: 4, textTransform: "uppercase" }}>Password</label><input type="password" style={{ width: "100%", padding: "10px", background: "#0f172a", border: "1px solid #334155", borderRadius: 8, color: "#fff", fontSize: 15, boxSizing: "border-box" }} placeholder="SSH Password" value={sshPass} onChange={e => setSshPass(e.target.value)} /></div>
                </>
              )}
              <div style={{ marginBottom: 20 }}><label style={{ display: "block", fontSize: 11, fontWeight: 600, color: "#64748b", marginBottom: 4, textTransform: "uppercase" }}>{connType === "ssh" ? "Pairing Token (optional)" : "Pairing Token"}</label><input type="password" style={{ width: "100%", padding: "10px", background: "#0f172a", border: "1px solid #334155", borderRadius: 8, color: "#fff", fontSize: 15, boxSizing: "border-box" }} placeholder="Token from PC" value={token} onChange={e => setToken(e.target.value)} /></div>
              <button style={{ width: "100%", padding: "14px", background: "#3b82f6", border: "none", borderRadius: 8, color: "#fff", fontWeight: 700, fontSize: 16, cursor: "pointer", opacity: connecting ? 0.7 : 1 }} disabled={connecting} onClick={() => onConnect(host, token, connectionName, undefined, connType as "wifi" | "ssh")}>{connecting ? "Connecting..." : `Connect via ${connType.toUpperCase()}`}</button>
            </>
          )}
          {connType === "bt" && (
            <div style={{ textAlign: "center" }}><div style={{ padding: 20, color: "#94a3b8", fontSize: 14 }}>{btScanning ? "Scanning..." : "Ready to scan."}</div><button style={{ width: "100%", padding: "14px", background: btScanning ? "#dc2626" : "#3b82f6", border: "none", borderRadius: 8, color: "#fff", fontWeight: 700, fontSize: 15, cursor: "pointer" }} onClick={btScanning ? onStopBtScan : onStartBtScan}>{btScanning ? "Stop Scanning" : "Scan for Devices"}</button></div>
          )}
          {connType === "wifi" && <button style={{ width: "100%", marginTop: 12, padding: "12px", background: "transparent", border: "1px solid #334155", borderRadius: 8, color: "#94a3b8", fontWeight: 600, fontSize: 14, cursor: "pointer" }} onClick={onScanQR}>Scan QR Code</button>}
        </div>

        {showSaved && savedConnections.length > 0 && (
          <div style={{ width: "100%", marginTop: 24 }}>
            <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 8 }}><span style={{ fontSize: 12, fontWeight: 600, color: "#64748b", textTransform: "uppercase" }}>Saved PCs</span><button onClick={() => setShowSaved(false)} style={{ background: "none", border: "none", color: "#3b82f6", fontSize: 11 }}>Hide</button></div>
            <div style={{ display: "flex", flexDirection: "column", gap: 8, maxHeight: 150, overflow: "auto" }}>
              {savedConnections.sort((a, b) => b.lastUsed - a.lastUsed).map(c => (
                <div key={c.id} style={{ padding: "10px 12px", background: "#1e293b", border: "1px solid #334155", borderRadius: 10, display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                  <div onClick={() => { if (editingId) return; setHost(c.host); setToken(c.token); setConnectionName(c.name); setConnType(c.type || "wifi"); setSshUser(c.username || ""); onConnect(c.host, c.token, c.name, c.id, (c.type === "ssh" ? "ssh" : "wifi")); }} style={{ flex: 1, cursor: "pointer" }}>
                    {editingId === c.id ? <input autoFocus style={{ background: "#0f172a", border: "1px solid #3b82f6", color: "#fff", borderRadius: 4, padding: "2px 6px", width: "80%" }} value={editName} onChange={e => setEditName(e.target.value)} onBlur={() => setEditingId(null)} onKeyDown={e => e.key === "Enter" && handleSaveRename(e, c.id)} onClick={e => e.stopPropagation()} /> : <div style={{ fontSize: 14, fontWeight: 600 }}>{c.name} <span style={{ fontSize: 9, opacity: 0.5 }}>({c.type || "wifi"})</span></div>}
                    <div style={{ fontSize: 11, color: "#64748b" }}>{c.host}</div>
                  </div>
                  <div style={{ display: "flex", gap: 4 }}>
                    {editingId === c.id ? <button onClick={e => handleSaveRename(e, c.id)} style={{ background: "none", border: "none", color: "#22c55e", padding: 4 }}><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><polyline points="20 6 9 17 4 12"/></svg></button> : <button onClick={e => startRename(e, c)} style={{ background: "none", border: "none", color: "#64748b", padding: 4 }}><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7" /><path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z" /></svg></button>}
                    <button onClick={() => onDeleteSavedConnection(c.id)} style={{ background: "none", border: "none", color: "#64748b", padding: 4 }}><svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M3 6h18M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/></svg></button>
                  </div>
                </div>
              ))}
            </div>
          </div>
        )}

        {statusLogs.length > 0 && <div style={{ marginTop: 12, padding: 10, background: "#000", borderRadius: 8, width: "100%", fontSize: 11, fontFamily: "monospace", color: "#22c55e", border: "1px solid #334155", boxSizing: "border-box", maxHeight: 100, overflow: "auto" }}>{statusLogs.map((l, i) => <div key={i}>{`> ${l}`}</div>)}</div>}

        {btDevices.length > 0 && connType === "bt" && (
          <div style={{ width: "100%", marginTop: 8, display: "flex", flexDirection: "column", gap: 6, maxHeight: 150, overflow: "auto" }}>
            {btDevices.map(d => (
              <div key={d.id} style={{ padding: "10px 12px", background: "#1e293b", border: "1px solid #334155", borderRadius: 8, display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                <div>
                  <div style={{ fontSize: 14, fontWeight: 600 }}>{d.name || "Unknown PC"}</div>
                  <div style={{ fontSize: 10, color: "#64748b" }}>RSSI: {d.rssi} dBm</div>
                </div>
                <button
                  style={{ padding: "6px 12px", background: "#3b82f6", border: "none", borderRadius: 6, color: "#fff", fontSize: 12, fontWeight: 600 }}
                  onClick={() => onConnectBtDevice(d)}
                >Connect</button>
              </div>
            ))}
          </div>
        )}

        {error && <div style={{ color: "#ef4444", marginTop: 10, fontSize: 13, textAlign: "center" }}>{error}</div>}
        <div style={{ fontSize: 9, color: "#475569", marginTop: 8 }}>BT State: {btState}</div>
      </div>
      <div style={{ paddingBottom: "env(safe-area-inset-bottom, 12px)", flexShrink: 0 }} />
    </div>
  );
}
