import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

interface BtStatus {
  running: boolean;
  adapter?: string;
  address?: string;
  psm?: number;
}

interface BtDevice {
  index: number;
  name: string;
  id: string;
  rssi: number;
}

interface BleTransferProgress {
  direction: string;
  path: string;
  transferred: number;
  total: number;
  percent: number;
}

export function BluetoothPanel({ visible }: { visible: boolean }) {
  const [btRunning, setBtRunning] = useState(false);
  const [btStatus, setBtStatus] = useState<BtStatus | null>(null);
  const [btStarting, setBtStarting] = useState(false);
  const [pairQrDataUrl, setPairQrDataUrl] = useState<string | null>(null);
  const [pairToken, setPairToken] = useState<string | null>(null);

  // Mobile-specific state
  const [isMobile, setIsMobile] = useState(false);
  const [btScanning, setBtScanning] = useState(false);
  const [btDevices, setBtDevices] = useState<BtDevice[]>([]);
  const [btState, setBtState] = useState("idle");
  const [transferProgress, setTransferProgress] = useState<BleTransferProgress | null>(null);

  const refreshBtStatus = () => {
    // Only invoke if in real Tauri
    if ((window as unknown as Record<string, unknown>).__TAURI_INTERNALS__) {
      invoke<BtStatus>("bt_get_status").then((s) => {
        setBtStatus(s);
        setBtRunning(s?.running ?? false);
      }).catch(() => {});
    }
  };

  useEffect(() => {
    const desc = Object.getOwnPropertyDescriptor(window, "__TAURI_INTERNALS__");
    const isRealTauri = !!desc && !desc.writable && !desc.configurable;
    setIsMobile(!isRealTauri);

    if (visible && isRealTauri) {
      refreshBtStatus();
    }
  }, [visible]);

  useEffect(() => {
    if (isMobile) {
      // Listen for mobile bridge events
      const handleBtStatus = (e: CustomEvent<{ state: string }>) => {
        setBtState(e.detail.state);
        setBtScanning(e.detail.state === "scanning");
      };
      const handleBtDevices = (e: CustomEvent<{ devices: BtDevice[] }>) => {
        setBtDevices(e.detail.devices);
      };

      window.addEventListener("mobile-bt-status", handleBtStatus as EventListener);
      window.addEventListener("mobile-bt-devices", handleBtDevices as EventListener);
      return () => {
        window.removeEventListener("mobile-bt-status", handleBtStatus as EventListener);
        window.removeEventListener("mobile-bt-devices", handleBtDevices as EventListener);
      };
    } else {
      let unlisten: (() => void) | undefined;
      let unlistenProgress: (() => void) | undefined;
      import("@tauri-apps/api/event").then(({ listen }) => {
        listen<BtStatus>("bt_server_status", (e) => {
          setBtStatus(e.payload);
          setBtRunning(e.payload?.running ?? false);
        }).then(u => unlisten = u);
        listen<BleTransferProgress>("ble-transfer-progress", (e) => {
          setTransferProgress(e.payload);
        }).then(u => unlistenProgress = u);
      }).catch(() => {});
      return () => { if (unlisten) unlisten(); if (unlistenProgress) unlistenProgress(); };
    }
  }, [isMobile]);

  if (!visible) return null;

  const startScan = () => {
    window.dispatchEvent(new CustomEvent("mobile-bt-start-scan"));
  };
  const stopScan = () => {
    window.dispatchEvent(new CustomEvent("mobile-bt-stop-scan"));
  };
  const connectDevice = (device: BtDevice) => {
    window.dispatchEvent(new CustomEvent("mobile-bt-connect", { detail: device }));
  };

  const showPairQr = async () => {
    try {
      const [dataUrl, token] = await invoke<[string, string]>("bt_get_pairing_qr");
      setPairQrDataUrl(dataUrl);
      setPairToken(token);
    } catch (e) {
      console.error("Failed to load BLE pairing QR:", e);
    }
  };

  return (
    <div className="settings-panel" style={{ height: "100%", padding: 16, overflowY: "auto" }}>
      <div className="settings-header">
        <h2>Bluetooth Pairing</h2>
      </div>

      {!isMobile ? (
        <div className="settings-section">
          <div className="settings-section-title">Bluetooth (Offline Mode)</div>
          <div className="settings-row" style={{ justifyContent: "space-between" }}>
            <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <span className={`llm-status-dot ${btRunning ? "running" : "stopped"}`} />
              <span className="settings-label">
                {btRunning ? "BLE Server Running" : "BLE Server Stopped"}
              </span>
            </div>
            <button
              className="settings-about-btn"
              disabled={btStarting}
              onClick={async () => {
                setBtStarting(true);
                try {
                  if (btRunning) {
                    await invoke("bt_stop_server");
                  } else {
                    let token = "shadowide-bt-" + Date.now().toString(36);
                    try {
                      const info = await invoke<{ token?: string }>("remote_get_info");
                      if (info?.token) token = info.token;
                    } catch { /* use generated token */ }
                    await invoke("bt_start_server", { authToken: token });
                  }
                } catch (e) {
                  console.error("BT error:", e);
                }
                setTimeout(() => { refreshBtStatus(); setBtStarting(false); }, 500);
              }}
              style={{ whiteSpace: "nowrap" }}
            >
              {btStarting ? "..." : btRunning ? "Stop" : "Start BLE Server"}
            </button>
          </div>
          {btStatus && (
            <div style={{ fontSize: 10, color: "var(--text-muted)", marginTop: 4 }}>
              Adapter: {btStatus.adapter} {btStatus.address ? `(${btStatus.address})` : ""}
              {btRunning && btStatus.psm ? ` | PSM: ${btStatus.psm}` : ""}
            </div>
          )}
          {btRunning && (
            <div style={{ marginTop: 10 }}>
              <button
                className="settings-about-btn"
                onClick={() => void showPairQr()}
                style={{ width: "100%" }}
              >
                Show BLE Pairing QR
              </button>
            </div>
          )}
          {pairQrDataUrl && (
            <div style={{
              marginTop: 12,
              padding: 10,
              borderRadius: 8,
              border: "1px solid var(--border-color)",
              background: "var(--bg-secondary)",
              display: "flex",
              flexDirection: "column",
              alignItems: "center",
              gap: 8,
            }}>
              <img src={pairQrDataUrl} alt="Bluetooth pairing QR" width={180} height={180} />
              <div style={{ fontSize: 11, color: "var(--text-secondary)", textAlign: "center" }}>
                Scan on mobile to preload the BLE token and start device discovery.
              </div>
              {pairToken && (
                <code style={{ fontSize: 10, color: "var(--accent)" }}>
                  {pairToken.slice(0, 12)}...
                </code>
              )}
              <button
                className="settings-about-btn"
                onClick={() => {
                  setPairQrDataUrl(null);
                  setPairToken(null);
                }}
              >
                Hide QR
              </button>
            </div>
          )}
          <div style={{ fontSize: 10, color: "var(--text-muted)", marginTop: 6, lineHeight: 1.5 }}>
            Enables Bluetooth Low Energy (BLE) server so mobile devices can connect without WiFi.
            Start the server, then tap "Scan for Bluetooth Devices" on your iPhone.
          </div>
          {transferProgress && (
            <div style={{ marginTop: 12, padding: 10, borderRadius: 8, border: "1px solid var(--border-color)", background: "var(--bg-secondary)" }}>
              <div style={{ display: "flex", justifyContent: "space-between", gap: 8, fontSize: 11 }}>
                <span>{transferProgress.direction === "upload" ? "Sending over BLE" : "Receiving over BLE"}</span>
                <span>{Math.round(transferProgress.percent)}%</span>
              </div>
              <div style={{ marginTop: 6, height: 6, borderRadius: 999, background: "rgba(255,255,255,0.08)", overflow: "hidden" }}>
                <div style={{ width: `${transferProgress.percent}%`, height: "100%", background: "linear-gradient(90deg, #38bdf8, #818cf8)" }} />
              </div>
              <div style={{ marginTop: 6, fontSize: 10, color: "var(--text-muted)" }}>
                {transferProgress.path.split("/").pop()} · {transferProgress.transferred}/{transferProgress.total} bytes
              </div>
            </div>
          )}
        </div>
      ) : (
        <div className="settings-section">
          <div className="settings-section-title">Bluetooth Scanning</div>
          <div style={{ marginBottom: 12 }}>
            <button
              className="settings-about-btn"
              style={{ width: "100%", padding: 10, background: btScanning ? "#dc2626" : "var(--accent)" }}
              onClick={btScanning ? stopScan : startScan}
            >
              {btScanning ? "Stop Scanning" : "Scan for Devices"}
            </button>
          </div>

          <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
            {btDevices.length === 0 && !btScanning && (
              <div style={{ textAlign: "center", color: "var(--text-muted)", fontSize: 12, padding: 20 }}>
                No devices found. Tap Scan to look for your PC.
              </div>
            )}
            {btDevices.map((d) => (
              <div
                key={d.id}
                className="settings-row"
                style={{
                  padding: 10,
                  background: "var(--bg-secondary)",
                  borderRadius: 6,
                  justifyContent: "space-between"
                }}
              >
                <div>
                  <div style={{ fontWeight: 600 }}>{d.name || "Unknown Device"}</div>
                  <div style={{ fontSize: 10, color: "var(--text-muted)" }}>RSSI: {d.rssi} dBm</div>
                </div>
                <button
                  className="settings-about-btn"
                  onClick={() => connectDevice(d)}
                >
                  Connect
                </button>
              </div>
            ))}
          </div>

          <div style={{ marginTop: 16, fontSize: 11, color: "var(--text-muted)", lineHeight: 1.4 }}>
            Status: <span style={{ color: "var(--text-primary)" }}>{btState}</span>
            <br /><br />
            Make sure "Bluetooth (Offline Mode)" is started on your desktop ShadowIDE.
          </div>
        </div>
      )}
    </div>
  );
}
