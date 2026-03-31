import { useState } from "react";
import { timeAgo } from "./FerrumChatMessage";

interface Session {
  id: string;
  name: string;
  profile: string;
  created_at: number;
  updated_at: number;
  is_pinned: boolean;
}

interface FerrumChatSessionsProps {
  sessions: Session[];
  activeSessionId: string | null;
  onLoadSession: (session: Session) => void;
  onDeleteSession: (id: string) => void;
}

export default function FerrumChatSessions({
  sessions,
  activeSessionId,
  onLoadSession,
  onDeleteSession,
}: FerrumChatSessionsProps) {
  const [sessionFilter, setSessionFilter] = useState("");

  const filteredSessions = sessionFilter
    ? sessions.filter(s => s.name.toLowerCase().includes(sessionFilter.toLowerCase()))
    : sessions;

  return (
    <div style={{ background: "var(--bg-secondary)", borderBottom: "1px solid var(--border-color)", maxHeight: 300, overflow: "auto", flexShrink: 0 }}>
      <div style={{ padding: "6px 12px" }}>
        <input type="text" placeholder="Filter sessions..." value={sessionFilter} onChange={e => setSessionFilter(e.target.value)}
          style={{ width: "100%", background: "var(--bg-primary)", border: "1px solid var(--border-color)", borderRadius: "var(--radius-sm)", padding: "4px 8px", color: "var(--text-primary)", fontSize: 11, outline: "none" }} />
      </div>
      {filteredSessions.map(s => (
        <div key={s.id} onClick={() => onLoadSession(s)} style={{
          padding: "8px 12px", cursor: "pointer", display: "flex", alignItems: "center", gap: 8,
          background: s.id === activeSessionId ? "var(--bg-hover)" : "transparent",
          borderLeft: s.id === activeSessionId ? "3px solid var(--accent)" : "3px solid transparent",
        }}>
          {s.is_pinned && <span style={{ color: "var(--warning)", fontSize: 10 }}>&#9733;</span>}
          <span style={{ flex: 1, fontSize: 12 }}>{s.name}</span>
          <span style={{ fontSize: 9, color: "var(--text-muted)" }}>{timeAgo(s.updated_at)}</span>
          <button onClick={(e) => { e.stopPropagation(); onDeleteSession(s.id); }}
            style={{ background: "none", border: "none", color: "var(--danger)", cursor: "pointer", fontSize: 11, padding: "0 4px" }}>x</button>
        </div>
      ))}
      {filteredSessions.length === 0 && (
        <div style={{ padding: "12px", color: "var(--text-muted)", textAlign: "center", fontSize: 11 }}>No sessions found</div>
      )}
    </div>
  );
}
