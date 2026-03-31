import { useRef, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface FerrumChatInputProps {
  visible: boolean;
  input: string;
  setInput: (v: string) => void;
  streaming: boolean;
  connected: boolean;
  activeSession: { id: string; name: string } | null;
  includeFile: boolean;
  setIncludeFile: (v: boolean) => void;
  toolsEnabled: boolean;
  setToolsEnabled: (v: boolean) => void;
  activeFileName?: string;
  rootPath: string;
  onSend: () => void;
  onRenameSession: (id: string, name: string) => void;
}

// Slash commands from planengine.md §7.4 AI Chat Panel
const SLASH_COMMANDS = [
  { cmd: "/create",    desc: "Place a new entity with components in the scene" },
  { cmd: "/component", desc: "Generate a C++23 component (.h + .cpp)" },
  { cmd: "/system",    desc: "Generate a C++23 ECS system" },
  { cmd: "/shader",    desc: "Generate a GLSL/WGSL material shader" },
  { cmd: "/debug",     desc: "Analyze build errors + logs and propose a fix" },
  { cmd: "/scene",     desc: "Generate a full scene layout from description" },
  { cmd: "/prefab",    desc: "Generate a prefab + associated C++ class" },
  { cmd: "/explain",   desc: "Explain selected code or concept" },
  { cmd: "/refactor",  desc: "Refactor the current file or selection" },
  { cmd: "/fix",       desc: "Fix a compiler error with AI" },
];

export default function FerrumChatInput({
  visible,
  input,
  setInput,
  streaming,
  connected,
  activeSession,
  includeFile,
  setIncludeFile,
  toolsEnabled,
  setToolsEnabled,
  activeFileName,
  rootPath,
  onSend,
  onRenameSession,
}: FerrumChatInputProps) {
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const [slashOpen, setSlashOpen] = useState(false);
  const [slashFilter, setSlashFilter] = useState("");
  const [slashIdx, setSlashIdx] = useState(0);

  useEffect(() => {
    if (visible && inputRef.current) inputRef.current.focus();
  }, [visible]);

  // Detect /command typing
  const handleInput = (value: string) => {
    setInput(value);
    // Show slash menu when the input starts with / or has a slash after whitespace
    const lastToken = value.split(/\s/).pop() ?? "";
    if (lastToken.startsWith("/")) {
      setSlashOpen(true);
      setSlashFilter(lastToken.slice(1).toLowerCase());
      setSlashIdx(0);
    } else {
      setSlashOpen(false);
    }
  };

  const filteredCmds = SLASH_COMMANDS.filter(
    c => !slashFilter || c.cmd.slice(1).startsWith(slashFilter)
  );

  const acceptSlashCmd = (cmd: string) => {
    // Replace the trailing /... token with the chosen command + space
    const parts = input.split(/(\s+)/);
    const lastIdx = parts.length - 1;
    if (parts[lastIdx].startsWith("/")) {
      parts[lastIdx] = cmd + " ";
    } else {
      parts.push(cmd + " ");
    }
    setInput(parts.join(""));
    setSlashOpen(false);
    inputRef.current?.focus();
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (slashOpen && filteredCmds.length > 0) {
      if (e.key === "ArrowDown") { e.preventDefault(); setSlashIdx(i => (i + 1) % filteredCmds.length); return; }
      if (e.key === "ArrowUp")   { e.preventDefault(); setSlashIdx(i => (i - 1 + filteredCmds.length) % filteredCmds.length); return; }
      if (e.key === "Tab" || e.key === "Enter") {
        e.preventDefault();
        acceptSlashCmd(filteredCmds[slashIdx].cmd);
        return;
      }
      if (e.key === "Escape") { setSlashOpen(false); return; }
    }
    if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); onSend(); }
  };

  return (
    <div style={{ padding: "8px 12px", background: "var(--bg-secondary)", borderTop: "1px solid var(--border-color)", flexShrink: 0 }}>
      {/* Slash command autocomplete */}
      {slashOpen && filteredCmds.length > 0 && (
        <div style={{
          position: "absolute", bottom: "100%", left: 12, right: 12,
          background: "var(--bg-primary)", border: "1px solid var(--border-color)",
          borderRadius: "var(--radius-md)", boxShadow: "0 -4px 16px rgba(0,0,0,.4)",
          zIndex: 200, overflow: "hidden", marginBottom: 4,
        }}>
          <div style={{ padding: "4px 10px", fontSize: 9, color: "var(--text-muted)", borderBottom: "1px solid var(--border-color)" }}>
            ShadowEditor AI commands — Tab/Enter to accept
          </div>
          {filteredCmds.map((c, i) => (
            <div key={c.cmd}
              onMouseDown={e => { e.preventDefault(); acceptSlashCmd(c.cmd); }}
              style={{
                padding: "6px 12px", cursor: "pointer", display: "flex", gap: 10, alignItems: "baseline",
                background: i === slashIdx ? "var(--bg-hover)" : "transparent",
                borderBottom: i < filteredCmds.length - 1 ? "1px solid var(--border-color)" : undefined,
              }}>
              <span style={{ fontFamily: "monospace", fontSize: 12, color: "var(--accent)", minWidth: 96 }}>{c.cmd}</span>
              <span style={{ fontSize: 11, color: "var(--text-secondary)" }}>{c.desc}</span>
            </div>
          ))}
        </div>
      )}
      <div style={{ display: "flex", gap: 8, position: "relative" }}>
        <textarea ref={inputRef} value={input}
          onChange={e => handleInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={streaming ? "AI is responding..." : connected ? "Type a message or /command..." : "Waiting for LLM server..."}
          disabled={streaming || !connected || !activeSession}
          rows={1}
          style={{
            flex: 1, background: "var(--bg-primary)", border: "1px solid var(--border-color)", borderRadius: "var(--radius-md)",
            padding: "8px 12px", color: "var(--text-primary)", fontSize: 13, resize: "none",
            outline: "none", fontFamily: "inherit", lineHeight: 1.4, minHeight: 38, maxHeight: 120,
          }}
          onInput={(e) => {
            const el = e.target as HTMLTextAreaElement;
            el.style.height = "auto";
            el.style.height = Math.min(el.scrollHeight, 120) + "px";
          }} />
        <button onClick={onSend}
          disabled={streaming || !connected || !activeSession || !input.trim()}
          style={{
            background: streaming || !connected ? "var(--bg-hover)" : "var(--accent)",
            border: "none", borderRadius: "var(--radius-md)", padding: "0 16px",
            color: "#fff", cursor: streaming || !connected ? "not-allowed" : "pointer",
            fontWeight: 600, fontSize: 12, flexShrink: 0,
            transition: "background var(--transition-fast)",
          }}>
          {streaming ? "..." : "Send"}
        </button>
      </div>
      <div style={{ display: "flex", gap: 8, marginTop: 4, fontSize: 9, color: "var(--text-muted)", alignItems: "center" }}>
        <label style={{ display: "flex", alignItems: "center", gap: 3, cursor: "pointer" }}>
          <input type="checkbox" checked={includeFile} onChange={e => setIncludeFile(e.target.checked)}
            style={{ width: 10, height: 10 }} />
          File
        </label>
        <label style={{ display: "flex", alignItems: "center", gap: 3, cursor: "pointer" }}>
          <input type="checkbox" checked={toolsEnabled} onChange={e => setToolsEnabled(e.target.checked)}
            style={{ width: 10, height: 10 }} />
          Tools
        </label>
        <button className="fc-btn" style={{ fontSize: 9, padding: "1px 6px" }}
          onClick={async () => {
            try {
              await invoke("rag_build_index", { rootPath: rootPath, maxFiles: 500 });
            } catch { /* ignore */ }
          }}>Index RAG</button>
        {activeFileName && includeFile && (
          <span style={{ color: "var(--accent)", fontSize: 9 }}>+ {activeFileName}</span>
        )}
        {activeSession && (
          <span style={{ marginLeft: "auto", cursor: "pointer", color: "var(--text-secondary)" }}
            onClick={() => {
              const name = prompt("Rename session:", activeSession.name);
              if (name) onRenameSession(activeSession.id, name);
            }}>
            Rename
          </span>
        )}
      </div>
    </div>
  );
}
