import { useEffect, useRef, useCallback, useState, memo } from "react";
import { Terminal as XTerminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { WebLinksAddon } from "@xterm/addon-web-links";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import "@xterm/xterm/css/xterm.css";

export interface SavedTerminalSession {
  name: string;
  shell: string;
}

interface TerminalProps {
  visible: boolean;
  cwd: string;
  onExplainError?: (errorText: string) => void;
  savedSessions?: SavedTerminalSession[];
  onSessionsChange?: (sessions: SavedTerminalSession[]) => void;
}

interface TerminalSession {
  id: string;
  name: string;
  shell: string;
  sharedSourceId?: string;
  sharedMode?: "read" | "read-write";
  owner?: string;
}

interface SharedTerminalState {
  terminal_id: string;
  mode: "read" | "read-write";
  owner: string;
  active: boolean;
  updated_at?: number;
}

const XTERM_THEME = {
  background: "#1a1a2e",
  foreground: "#e0e0e0",
  cursor: "#e0e0e0",
  selectionBackground: "#3a3a5e",
  black: "#1a1a2e",
  red: "#ff6b6b",
  green: "#51cf66",
  yellow: "#ffd43b",
  blue: "#5c7cfa",
  magenta: "#cc5de8",
  cyan: "#22b8cf",
  white: "#e0e0e0",
  brightBlack: "#495057",
  brightRed: "#ff8787",
  brightGreen: "#69db7c",
  brightYellow: "#ffe066",
  brightBlue: "#748ffc",
  brightMagenta: "#da77f2",
  brightCyan: "#3bc9db",
  brightWhite: "#f8f9fa",
};

function TerminalPanel({ visible, cwd, onExplainError, savedSessions, onSessionsChange }: TerminalProps) {
  const [sessions, setSessions] = useState<TerminalSession[]>([]);
  const [activeSession, setActiveSession] = useState<string>("");
  const [availableShells, setAvailableShells] = useState<string[]>([]);
  const [showShellMenu, setShowShellMenu] = useState(false);
  const [editingName, setEditingName] = useState<string | null>(null);
  const [hasSelection, setHasSelection] = useState(false);
  const [shareMode, setShareMode] = useState<"read" | "read-write">("read");
  const [sharedTerminals, setSharedTerminals] = useState<SharedTerminalState[]>([]);
  const terminalRefs = useRef<Map<string, HTMLDivElement>>(new Map());
  const xtermRefs = useRef<Map<string, XTerminal>>(new Map());
  const fitAddonRefs = useRef<Map<string, FitAddon>>(new Map());
  const initializedSessions = useRef<Set<string>>(new Set());
  const sessionCounter = useRef(0);
  const cleanupFns = useRef<Map<string, () => void>>(new Map());
  const restoredRef = useRef(false);

  // Detect shells on mount
  useEffect(() => {
    invoke<string[]>("detect_shell").then(s => setAvailableShells(s ?? ["sh"])).catch(() => {});
  }, []);

  useEffect(() => {
    if (!visible) return;
    invoke<SharedTerminalState[]>("terminal_share_status").then((items) => {
      setSharedTerminals((items ?? []).filter((item) => item.active));
    }).catch(() => {});
  }, [visible]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<SharedTerminalState | { terminal_id: string; active: boolean }>("terminal-share-state", (event) => {
      const next = event.payload as SharedTerminalState & { active: boolean };
      setSharedTerminals((prev) => {
        if (!next.active) {
          return prev.filter((item) => item.terminal_id !== next.terminal_id);
        }
        const existing = prev.find((item) => item.terminal_id === next.terminal_id);
        if (existing) {
          return prev.map((item) => item.terminal_id === next.terminal_id ? { ...item, ...next } : item);
        }
        return [...prev, next as SharedTerminalState];
      });
    }).then((cleanup) => { unlisten = cleanup; }).catch(() => {});
    return () => { if (unlisten) unlisten(); };
  }, []);

  // Notify parent when sessions change (for persistence)
  const reportSessions = useCallback((current: TerminalSession[]) => {
    if (onSessionsChange) {
      onSessionsChange(current.map(s => ({ name: s.name, shell: s.shell })));
    }
  }, [onSessionsChange]);

  // Restore saved sessions on first mount, or create default
  useEffect(() => {
    if (!visible || restoredRef.current || sessions.length > 0) return;
    restoredRef.current = true;

    if (savedSessions && savedSessions.length > 0) {
      for (const saved of savedSessions) {
        const num = ++sessionCounter.current;
        const id = `term-${Date.now()}-${num}`;
        setSessions(prev => [...prev, { id, name: saved.name, shell: saved.shell }]);
        setActiveSession(id);
      }
    } else {
      createSession();
    }
  }, [visible]);

  // Re-fit active terminal when visibility changes
  useEffect(() => {
    if (visible && activeSession) {
      setTimeout(() => {
        const fitAddon = fitAddonRefs.current.get(activeSession);
        if (fitAddon) {
          fitAddon.fit();
        }
      }, 50);
    }
  }, [visible, activeSession]);

  const createSession = useCallback((shell?: string) => {
    const num = ++sessionCounter.current;
    const id = `term-${Date.now()}-${num}`;
    const name = `Terminal ${num}`;
    const shellName = shell || availableShells[0] || "";
    const newSession = { id, name, shell: shellName };
    setSessions(prev => {
      const next = [...prev, newSession];
      setTimeout(() => reportSessions(next), 0);
      return next;
    });
    setActiveSession(id);
    setShowShellMenu(false);
  }, [availableShells, reportSessions]);

  const collaboratorName = (() => {
    try {
      const raw = localStorage.getItem("shadowide-collaborator");
      if (raw) return (JSON.parse(raw) as { name?: string }).name || "ShadowIDE";
    } catch { /* ignore */ }
    return "ShadowIDE";
  })();

  const joinSharedTerminal = useCallback((shared: SharedTerminalState) => {
    const existing = sessions.find((session) => session.sharedSourceId === shared.terminal_id);
    if (existing) {
      setActiveSession(existing.id);
      return;
    }
    const id = `shared-${shared.terminal_id}`;
    const nextSession: TerminalSession = {
      id,
      name: `${shared.owner} · Shared`,
      shell: "shared",
      sharedSourceId: shared.terminal_id,
      sharedMode: shared.mode,
      owner: shared.owner,
    };
    setSessions((prev) => {
      const next = [...prev, nextSession];
      setTimeout(() => reportSessions(next.filter((session) => !session.sharedSourceId)), 0);
      return next;
    });
    setActiveSession(id);
  }, [reportSessions, sessions]);

  // Initialize xterm for sessions when their container refs become available
  useEffect(() => {
    for (const session of sessions) {
      if (initializedSessions.current.has(session.id)) continue;
      const container = terminalRefs.current.get(session.id);
      if (!container) continue;

      initializedSessions.current.add(session.id);
      initSession(session, container);
    }
  }, [sessions, cwd]);

  const initSession = async (session: TerminalSession, container: HTMLDivElement) => {
    const xterm = new XTerminal({
      cursorBlink: true,
      fontSize: 14,
      fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
      theme: XTERM_THEME,
      allowProposedApi: true,
      disableStdin: Boolean(session.sharedSourceId && session.sharedMode !== "read-write"),
    });

    const fitAddon = new FitAddon();
    const webLinksAddon = new WebLinksAddon();

    xterm.loadAddon(fitAddon);
    xterm.loadAddon(webLinksAddon);
    xterm.open(container);

    xtermRefs.current.set(session.id, xterm);
    fitAddonRefs.current.set(session.id, fitAddon);

    // Fit after a short delay to ensure container has dimensions
    setTimeout(() => {
      fitAddon.fit();
    }, 100);

    const cols = xterm.cols;
    const rows = xterm.rows;

    let unlistenOutput = () => {};
    let unlistenExit = () => {};
    let unlistenShared = () => {};

    if (session.sharedSourceId) {
      xterm.writeln(`\r\n[Joined shared terminal from ${session.owner ?? "collaborator"}]\r\n`);
      unlistenShared = await listen<{ terminal_id: string; data: string }>("terminal-share-output", (event) => {
        if (event.payload.terminal_id === session.sharedSourceId) {
          xterm.write(event.payload.data);
        }
      });
      xterm.onData((data) => {
        if (session.sharedMode === "read-write") {
          invoke("terminal_share_write", { id: session.sharedSourceId, data }).catch((err) =>
            console.error("Failed to write to shared terminal:", err)
          );
        }
      });
    } else {
      // Create PTY session on the backend
      try {
        await invoke("create_terminal", {
          id: session.id,
          rows,
          cols,
          cwd: cwd || undefined,
          shell: session.shell || undefined,
        });
      } catch (err) {
        console.error("Failed to create terminal:", err);
        xterm.writeln(`\r\nError creating terminal: ${err}\r\n`);
        return;
      }

      // Listen for PTY output
      unlistenOutput = await listen<string>(
        `terminal-output-${session.id}`,
        (event) => {
          xterm.write(event.payload);
        }
      );

      // Listen for PTY exit
      unlistenExit = await listen(
        `terminal-exit-${session.id}`,
        () => {
          xterm.writeln("\r\n[Process exited]");
        }
      );

      // Send input from xterm to PTY
      xterm.onData((data) => {
        invoke("write_terminal", { id: session.id, data }).catch((err) =>
          console.error("Failed to write to terminal:", err)
        );
      });

      // Handle resize
      xterm.onResize(({ rows, cols }) => {
        invoke("resize_terminal", { id: session.id, rows, cols }).catch((err) =>
          console.error("Failed to resize terminal:", err)
        );
      });
    }

    // Track selection for "Explain Error" button
    xterm.onSelectionChange(() => {
      if (session.id === activeSession || sessions.length === 1) {
        const sel = xterm.getSelection();
        setHasSelection(!!sel && sel.trim().length > 0);
      }
    });

    // Observe container resize
    const resizeObserver = new ResizeObserver(() => {
      const fa = fitAddonRefs.current.get(session.id);
      if (fa) {
        fa.fit();
      }
    });
    resizeObserver.observe(container);

    // Store cleanup function
    cleanupFns.current.set(session.id, () => {
      unlistenOutput();
      unlistenExit();
      unlistenShared();
      resizeObserver.disconnect();
    });
  };

  const closeSession = useCallback((id: string) => {
    const session = sessions.find((item) => item.id === id);
    if (!session?.sharedSourceId) {
      invoke("close_terminal", { id }).catch(() => {});
    }
    const xterm = xtermRefs.current.get(id);
    if (xterm) xterm.dispose();
    xtermRefs.current.delete(id);
    fitAddonRefs.current.delete(id);
    terminalRefs.current.delete(id);
    initializedSessions.current.delete(id);

    const cleanup = cleanupFns.current.get(id);
    if (cleanup) cleanup();
    cleanupFns.current.delete(id);

    setSessions(prev => {
      const next = prev.filter(s => s.id !== id);
      setTimeout(() => {
        if (activeSession === id && next.length > 0) {
          setActiveSession(next[next.length - 1].id);
        }
        reportSessions(next.filter((item) => !item.sharedSourceId));
      }, 0);
      return next;
    });
  }, [activeSession, reportSessions, sessions]);

  const renameSession = useCallback((id: string, newName: string) => {
    const trimmed = newName.trim();
    if (!trimmed) return;
    setSessions(prev => {
      const next = prev.map(s => (s.id === id ? { ...s, name: trimmed } : s));
      setTimeout(() => reportSessions(next.filter((session) => !session.sharedSourceId)), 0);
      return next;
    });
    setEditingName(null);
  }, [reportSessions]);

  const handleExplainError = useCallback(() => {
    if (!onExplainError) return;
    const xterm = xtermRefs.current.get(activeSession);
    if (!xterm) return;
    const sel = xterm.getSelection();
    if (sel) onExplainError(sel);
  }, [activeSession, onExplainError]);

  const setTerminalRef = useCallback((id: string, el: HTMLDivElement | null) => {
    if (el) {
      terminalRefs.current.set(id, el);
    }
  }, []);

  // Cleanup all sessions on unmount
  useEffect(() => {
    return () => {
      for (const [id, cleanup] of cleanupFns.current) {
        cleanup();
        const xterm = xtermRefs.current.get(id);
        if (xterm) xterm.dispose();
        if (!sessions.find((session) => session.id === id)?.sharedSourceId) {
          invoke("close_terminal", { id }).catch(() => {});
        }
      }
      cleanupFns.current.clear();
      xtermRefs.current.clear();
      fitAddonRefs.current.clear();
      terminalRefs.current.clear();
      initializedSessions.current.clear();
    };
  }, [sessions]);

  // Re-fit when active session changes
  useEffect(() => {
    if (activeSession) {
      setTimeout(() => {
        const fitAddon = fitAddonRefs.current.get(activeSession);
        if (fitAddon) {
          fitAddon.fit();
        }
      }, 50);
    }
  }, [activeSession]);

  const activeSessionMeta = sessions.find((session) => session.id === activeSession) ?? null;
  const activeShare = activeSessionMeta ? sharedTerminals.find((terminal) => terminal.terminal_id === activeSessionMeta.id) : null;

  return (
    <div
      className="terminal-panel"
      style={{ display: visible ? "flex" : "none" }}
    >
      <div className="terminal-header">
        <div className="terminal-tabs">
          {sessions.map(session => (
            <div
              key={session.id}
              className={`terminal-tab ${session.id === activeSession ? "active" : ""}`}
              onClick={() => setActiveSession(session.id)}
            >
              {editingName === session.id ? (
                <input
                  className="terminal-tab-name"
                  defaultValue={session.name}
                  autoFocus
                  onBlur={(e) => renameSession(session.id, e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      renameSession(session.id, (e.target as HTMLInputElement).value);
                    }
                    if (e.key === "Escape") {
                      setEditingName(null);
                    }
                  }}
                  onClick={(e) => e.stopPropagation()}
                  style={{
                    background: "transparent",
                    border: "none",
                    color: "inherit",
                    font: "inherit",
                    width: "80px",
                    outline: "1px solid var(--accent)",
                    padding: "0 2px",
                  }}
                />
              ) : (
                <span
                  className="terminal-tab-name"
                  onDoubleClick={(e) => {
                    e.stopPropagation();
                    setEditingName(session.id);
                  }}
                >
                  {session.name}
                </span>
              )}
              {sessions.length > 1 && (
                <button
                  className="terminal-tab-close"
                  onClick={(e) => {
                    e.stopPropagation();
                    closeSession(session.id);
                  }}
                  title="Close terminal"
                >
                  ×
                </button>
              )}
            </div>
          ))}
          <div style={{ position: "relative" }}>
            <button
              className="terminal-new-btn"
              onClick={() => {
                if (availableShells.length > 1) {
                  setShowShellMenu(prev => !prev);
                } else {
                  createSession();
                }
              }}
              title="New terminal"
            >
              +
            </button>
            {showShellMenu && availableShells.length > 1 && (
              <div className="terminal-shell-menu">
                {availableShells.map(shell => (
                  <div
                    key={shell}
                    className="terminal-shell-item"
                    onClick={() => createSession(shell)}
                  >
                    {shell.split("/").pop()}
                  </div>
                ))}
              </div>
            )}
          </div>
        </div>
        <div className="terminal-header-spacer" />
        {activeSessionMeta && !activeSessionMeta.sharedSourceId && (
          <div style={{ display: "flex", alignItems: "center", gap: 8, marginRight: 8 }}>
            <select
              value={shareMode}
              onChange={(e) => setShareMode(e.target.value as "read" | "read-write")}
              style={{
                background: "var(--panel-bg, #111827)",
                color: "var(--text-secondary, #cbd5e1)",
                border: "1px solid var(--border-color, #334155)",
                borderRadius: 6,
                fontSize: 11,
                padding: "4px 6px",
              }}
            >
              <option value="read">Read-only</option>
              <option value="read-write">Read/write</option>
            </select>
            <button
              className="terminal-explain-btn"
              onClick={() => {
                const command = activeShare ? "terminal_share_stop" : "terminal_share_start";
                const args = activeShare
                  ? { id: activeSessionMeta.id }
                  : { id: activeSessionMeta.id, mode: shareMode, owner: collaboratorName };
                invoke(command, args).catch((err) => console.error("Failed to update shared terminal:", err));
              }}
              title="Share terminal with collaborators"
            >
              {activeShare ? "Stop Sharing" : "Share"}
            </button>
          </div>
        )}
        {hasSelection && onExplainError && (
          <button
            className="terminal-explain-btn"
            onClick={handleExplainError}
            title="Explain selected error with AI"
          >
            Explain Error
          </button>
        )}
      </div>
      {sharedTerminals.length > 0 && (
        <div style={{
          display: "flex",
          gap: 8,
          alignItems: "center",
          padding: "6px 10px",
          borderBottom: "1px solid rgba(255,255,255,0.06)",
          background: "rgba(125, 211, 252, 0.05)",
          overflowX: "auto",
        }}>
          <span style={{ fontSize: 11, color: "var(--text-muted)" }}>Shared:</span>
          {sharedTerminals.map((shared) => (
            <button
              key={shared.terminal_id}
              onClick={() => joinSharedTerminal(shared)}
              style={{
                borderRadius: 999,
                border: "1px solid rgba(125, 211, 252, 0.22)",
                background: "rgba(15, 23, 42, 0.9)",
                color: "#7dd3fc",
                padding: "4px 10px",
                fontSize: 11,
                cursor: "pointer",
                whiteSpace: "nowrap",
              }}
            >
              {shared.owner} · {shared.mode}
            </button>
          ))}
        </div>
      )}
      <div className="terminal-instances">
        {sessions.map(session => (
          <div
            key={session.id}
            className="terminal-instance"
            style={{
              display: session.id === activeSession ? "block" : "none",
            }}
            ref={(el) => setTerminalRef(session.id, el)}
          />
        ))}
      </div>
    </div>
  );
}
export default memo(TerminalPanel);
