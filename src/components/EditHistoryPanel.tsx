import React, { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import InlineDiffPreview from "./InlineDiffPreview";

export interface EditRecord {
  id: string;
  timestamp: number;
  filePath: string;
  description: string;
  before: string;
  after: string;
  agentTaskId?: string;
}

interface EditHistoryPanelProps {
  projectPath: string;
}

const STORAGE_KEY = "shadow-ide-edit-history";
const MAX_EDITS = 50;

function loadFromStorage(): EditRecord[] {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw) return JSON.parse(raw) as EditRecord[];
  } catch { /* ignore */ }
  return [];
}

function saveToStorage(edits: EditRecord[]): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(edits));
  } catch { /* ignore */ }
}

function timeAgo(ts: number): string {
  const diff = Math.floor(Date.now() / 1000) - Math.floor(ts / 1000);
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)} min ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

export default function EditHistoryPanel({ projectPath }: EditHistoryPanelProps) {
  const [edits, setEdits] = useState<EditRecord[]>(() => loadFromStorage());
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [undoingId, setUndoingId] = useState<string | null>(null);
  const [undoingAll, setUndoingAll] = useState(false);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const [, forceUpdate] = useState(0);

  // Persist on change
  useEffect(() => {
    saveToStorage(edits);
  }, [edits]);

  // Force re-render every 30s to update relative timestamps
  useEffect(() => {
    timerRef.current = setInterval(() => forceUpdate((n) => n + 1), 30000);
    return () => { if (timerRef.current) clearInterval(timerRef.current); };
  }, []);

  // Listen for new AI edits
  useEffect(() => {
    const unsub = listen<EditRecord>("ai-edit-applied", (e) => {
      setEdits((prev) => {
        const next = [e.payload, ...prev];
        return next.slice(0, MAX_EDITS);
      });
    });
    return () => { unsub.then((fn) => fn()); };
  }, []);

  const undoEdit = useCallback(async (record: EditRecord) => {
    setUndoingId(record.id);
    try {
      await invoke("write_file_content", { path: record.filePath, content: record.before });
    } catch (err) {
      console.error("Failed to undo edit:", err);
    }
    setUndoingId(null);
  }, []);

  const undoAll = useCallback(async () => {
    setUndoingAll(true);
    // Undo in reverse chronological order (oldest first since array is newest-first)
    const reversed = [...edits].reverse();
    for (const record of reversed) {
      try {
        await invoke("write_file_content", { path: record.filePath, content: record.before });
      } catch { /* skip failed undos */ }
    }
    setUndoingAll(false);
  }, [edits]);

  const selectedEdit = edits.find((e) => e.id === selectedId) ?? null;

  // Group by agentTaskId
  const groups: Map<string, EditRecord[]> = new Map();
  for (const edit of edits) {
    const key = edit.agentTaskId ?? "__ungrouped__";
    const arr = groups.get(key) ?? [];
    arr.push(edit);
    groups.set(key, arr);
  }

  const relPath = (filePath: string) => {
    if (projectPath && filePath.startsWith(projectPath)) {
      return filePath.slice(projectPath.length).replace(/^\//, "");
    }
    return filePath.split("/").pop() ?? filePath;
  };

  const btnStyle: React.CSSProperties = {
    fontSize: 11,
    padding: "2px 8px",
    borderRadius: 4,
    border: "1px solid var(--border-color, #313244)",
    background: "transparent",
    color: "var(--text-muted, #6c7086)",
    cursor: "pointer",
  };

  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column", fontFamily: "monospace", fontSize: 12, color: "var(--text-primary, #cdd6f4)" }}>
      {/* Header */}
      <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "8px 10px", borderBottom: "1px solid var(--border-color, #313244)", flexShrink: 0 }}>
        <span style={{ fontWeight: 700, color: "var(--accent, #89b4fa)" }}>Edit History</span>
        <span style={{ fontSize: 10, color: "var(--text-muted)" }}>({edits.length}/{MAX_EDITS})</span>
        <div style={{ flex: 1 }} />
        <button
          style={{ ...btnStyle, color: undoingAll ? "var(--warning)" : "var(--danger, #f38ba8)" }}
          onClick={undoAll}
          disabled={undoingAll || edits.length === 0}
          title="Undo all edits in reverse order"
        >
          {undoingAll ? "Undoing..." : "Undo All"}
        </button>
        <button
          style={btnStyle}
          onClick={() => setEdits([])}
          title="Clear history"
        >
          Clear
        </button>
      </div>

      <div style={{ display: "flex", flex: 1, overflow: "hidden" }}>
        {/* Timeline list */}
        <div style={{ width: selectedEdit ? "45%" : "100%", overflowY: "auto", borderRight: selectedEdit ? "1px solid var(--border-color, #313244)" : "none" }}>
          {edits.length === 0 && (
            <div style={{ padding: 16, color: "var(--text-muted)", textAlign: "center" }}>
              No edits yet. AI-applied edits will appear here.
            </div>
          )}

          {Array.from(groups.entries()).map(([groupKey, groupEdits]) => (
            <div key={groupKey}>
              {groupKey !== "__ungrouped__" && (
                <div style={{ padding: "4px 10px", background: "var(--bg-surface, #181825)", fontSize: 10, color: "var(--accent)", borderBottom: "1px solid var(--border-color, #313244)" }}>
                  Agent Task: {groupKey.slice(0, 8)}...
                </div>
              )}
              {groupEdits.map((edit) => (
                <div
                  key={edit.id}
                  onClick={() => setSelectedId(selectedId === edit.id ? null : edit.id)}
                  style={{
                    padding: "6px 10px",
                    borderBottom: "1px solid var(--border-color, #313244)",
                    cursor: "pointer",
                    background: selectedId === edit.id ? "var(--bg-hover, #313244)" : "transparent",
                    display: "flex",
                    flexDirection: "column",
                    gap: 2,
                  }}
                >
                  <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                    <span style={{ fontSize: 10, color: "var(--text-muted)" }}>{timeAgo(edit.timestamp)}</span>
                    <div style={{ flex: 1 }} />
                    <button
                      style={{ ...btnStyle, fontSize: 10 }}
                      onClick={(e) => { e.stopPropagation(); void undoEdit(edit); }}
                      disabled={undoingId === edit.id}
                    >
                      {undoingId === edit.id ? "..." : "Undo"}
                    </button>
                  </div>
                  <div style={{ color: "var(--text-secondary, #bac2de)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {edit.description}
                  </div>
                  <div style={{ fontSize: 10, color: "var(--text-muted)", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {relPath(edit.filePath)}
                  </div>
                </div>
              ))}
            </div>
          ))}
        </div>

        {/* Diff preview */}
        {selectedEdit && (
          <div style={{ flex: 1, overflow: "hidden" }}>
            <InlineDiffPreview
              originalContent={selectedEdit.before}
              proposedContent={selectedEdit.after}
              filePath={selectedEdit.filePath}
              onAccept={() => setSelectedId(null)}
              onReject={() => void undoEdit(selectedEdit).then(() => setSelectedId(null))}
              onAcceptHunk={() => { /* hunk accept not applicable in history view */ }}
            />
          </div>
        )}
      </div>
    </div>
  );
}
