import { useState, useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";

interface LogEntry {
  timestamp: number;
  level: "info" | "warn" | "error" | "debug" | "trace";
  message: string;
  source?: string;
}

export default function LogPanel({ visible }: { visible: boolean }) {
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [filter, setFilter] = useState("");
  const [levelFilter, setLevelFilter] = useState<string>("all");
  const scrollRef = useRef<HTMLDivElement>(null);
  const [autoScroll, setAutoScroll] = useState(true);

  useEffect(() => {
    if (!visible) return;

    // Listen for backend log events
    // In a real scenario, you'd hook into tauri-plugin-log or emit custom events
    const unlisten = listen<LogEntry>("log-event", (event) => {
      setLogs((prev) => [...prev, event.payload].slice(-1000));
    });

    return () => {
      unlisten.then((u) => u());
    };
  }, [visible]);

  useEffect(() => {
    if (autoScroll && scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [logs, autoScroll]);

  if (!visible) return null;

  const filteredLogs = logs.filter((log) => {
    if (levelFilter !== "all" && log.level !== levelFilter) return false;
    if (filter && !log.message.toLowerCase().includes(filter.toLowerCase())) return false;
    return true;
  });

  return (
    <div className="log-panel" style={{ display: "flex", flexDirection: "column", height: "100%", background: "var(--bg-tertiary)" }}>
      <div className="explorer-header" style={{ flexShrink: 0 }}>
        <span className="explorer-title">CONSOLE</span>
        <button className="explorer-btn" onClick={() => setLogs([])} title="Clear Logs">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M3 6h18" /><path d="M19 6v14c0 1-1 2-2 2H7c-1 0-2-1-2-2V6" /><path d="M8 6V4c0-1 1-2 2-2h4c1 0 2 1 2 2v2" /></svg>
        </button>
        <button 
          className="explorer-btn" 
          onClick={() => setAutoScroll(!autoScroll)} 
          title="Toggle Auto-scroll"
          style={{ color: autoScroll ? "var(--accent)" : "inherit" }}
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M7 13l5 5 5-5M7 6l5 5 5-5" /></svg>
        </button>
      </div>

      <div style={{ padding: "8px", borderBottom: "1px solid var(--border-color)", display: "flex", gap: "6px" }}>
        <input
          className="ai-setting-input"
          style={{ flex: 1, fontSize: "11px", padding: "4px 8px" }}
          placeholder="Filter logs..."
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
        />
        <select 
          className="ai-setting-select" 
          style={{ fontSize: "11px", padding: "2px 4px" }}
          value={levelFilter}
          onChange={(e) => setLevelFilter(e.target.value)}
        >
          <option value="all">All Levels</option>
          <option value="info">Info</option>
          <option value="warn">Warn</option>
          <option value="error">Error</option>
          <option value="debug">Debug</option>
        </select>
      </div>

      <div 
        ref={scrollRef}
        style={{ flex: 1, overflowY: "auto", padding: "8px", fontFamily: "'JetBrains Mono', 'Fira Code', monospace", fontSize: "11px" }}
      >
        {filteredLogs.length === 0 && (
          <div style={{ color: "var(--text-muted)", textAlign: "center", marginTop: "20px" }}>
            No logs to display
          </div>
        )}
        {filteredLogs.map((log, i) => (
          <div key={i} style={{ marginBottom: "4px", borderBottom: "1px solid var(--border-subtle)", paddingBottom: "2px" }}>
            <span style={{ color: "var(--text-muted)", marginRight: "8px" }}>
              {new Date(log.timestamp).toLocaleTimeString([], { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' })}
            </span>
            <span style={{ 
              color: log.level === "error" ? "var(--danger)" : 
                     log.level === "warn" ? "var(--warning)" : 
                     log.level === "info" ? "var(--accent)" : 
                     "var(--text-muted)",
              fontWeight: "bold",
              marginRight: "8px",
              textTransform: "uppercase",
              fontSize: "10px"
            }}>
              [{log.level}]
            </span>
            <span style={{ color: "var(--text-primary)", wordBreak: "break-word" }}>{log.message}</span>
            {log.source && (
              <span style={{ color: "var(--text-muted)", marginLeft: "8px", fontSize: "10px" }}>
                ({log.source})
              </span>
            )}
          </div>
        ))}
      </div>
    </div>
  );
}
