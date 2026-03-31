import React, { memo, useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

// ===== Types =====

export interface ChatMessage {
  role: string;
  content: string;
  tool_calls?: string;
  tool_name?: string;
  token_count: number;
  is_compacted: boolean;
  created_at: number;
  thinking?: string;
  showThinking?: boolean;
}

// ===== Helpers =====

export const fmtTokens = (n: number) =>
  n > 9999 ? `${(n / 1000).toFixed(0)}k` : n > 999 ? `${(n / 1000).toFixed(1)}k` : String(n);

export const tokenBarColor = (pct: number) =>
  pct < 0.5 ? "var(--success)" : pct < 0.8 ? "var(--warning)" : "var(--danger)";

export const timeAgo = (ts: number) => {
  const diff = Math.floor(Date.now() / 1000) - ts;
  if (diff < 60) return "just now";
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
};

// ===== CodeBlock with action buttons =====

interface CodeBlockProps {
  code: string;
  language: string;
}

function CodeBlock({ code, language }: CodeBlockProps) {
  const [hovered, setHovered] = useState(false);
  const [showFilenamePrompt, setShowFilenamePrompt] = useState(false);
  const [filename, setFilename] = useState("");
  const [copied, setCopied] = useState(false);

  const handleInsert = useCallback(() => {
    window.dispatchEvent(new CustomEvent("editor-insert-code", { detail: { code, language } }));
  }, [code, language]);

  const handleDiff = useCallback(() => {
    window.dispatchEvent(new CustomEvent("editor-diff-code", { detail: { proposed: code } }));
  }, [code]);

  const handleCopy = useCallback(() => {
    void navigator.clipboard.writeText(code).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    });
  }, [code]);

  const handleCreateFile = useCallback(async () => {
    if (!showFilenamePrompt) {
      setShowFilenamePrompt(true);
      return;
    }
    if (!filename.trim()) return;
    try {
      await invoke("write_file_content", { path: filename.trim(), content: code });
    } catch (err) {
      console.error("Failed to create file:", err);
    }
    setShowFilenamePrompt(false);
    setFilename("");
  }, [showFilenamePrompt, filename, code]);

  const pillBtn: React.CSSProperties = {
    fontSize: 10,
    padding: "2px 7px",
    borderRadius: 10,
    border: "1px solid rgba(255,255,255,0.14)",
    background: "rgba(15,20,35,0.88)",
    color: "#c0caf5",
    cursor: "pointer",
    whiteSpace: "nowrap",
    transition: "background 0.12s, color 0.12s",
  };

  return (
    <div
      style={{ position: "relative", margin: "6px 0", borderRadius: "var(--radius-md)", overflow: "hidden", border: "1px solid var(--border-subtle)" }}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => { setHovered(false); }}
    >
      {/* Code block header: language label + action buttons */}
      <div style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        padding: "4px 10px",
        background: "rgba(255,255,255,0.03)",
        borderBottom: "1px solid var(--border-subtle)",
      }}>
        <span style={{ fontSize: 10, color: "var(--accent)", fontWeight: 600, fontFamily: "var(--font-mono, monospace)" }}>
          {language || "plaintext"}
        </span>
        <div style={{ display: "flex", gap: 4, opacity: hovered ? 1 : 0, transition: "opacity 0.15s" }}>
          <button style={pillBtn} onClick={handleCopy}>{copied ? "✓ Copied" : "Copy"}</button>
          <button style={pillBtn} onClick={handleInsert} title="Insert at cursor">Insert</button>
          <button style={pillBtn} onClick={handleDiff} title="Diff with current file">Diff</button>
          <button style={pillBtn} onClick={handleCreateFile} title="Create new file">
            {showFilenamePrompt ? "Confirm" : "New File"}
          </button>
        </div>
      </div>

      {showFilenamePrompt && (
        <div style={{ display: "flex", gap: 4, padding: "4px 8px", background: "var(--bg-secondary)", borderBottom: "1px solid var(--border-subtle)" }}>
          <input
            autoFocus
            value={filename}
            onChange={(e) => setFilename(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") { void handleCreateFile(); }
              if (e.key === "Escape") { setShowFilenamePrompt(false); setFilename(""); }
            }}
            placeholder="Enter file path..."
            style={{
              flex: 1,
              fontSize: 11,
              background: "var(--bg-primary)",
              color: "var(--text-primary)",
              border: "1px solid var(--border-color)",
              borderRadius: 4,
              padding: "3px 8px",
            }}
          />
          <button style={{ ...pillBtn, background: "var(--success-dim)", border: "1px solid var(--success)", color: "var(--success)" }} onClick={() => void handleCreateFile()}>Save</button>
          <button style={{ ...pillBtn, background: "var(--danger-dim)", border: "1px solid var(--danger)", color: "var(--danger)" }} onClick={() => { setShowFilenamePrompt(false); setFilename(""); }}>Cancel</button>
        </div>
      )}

      <pre style={{
        background: "var(--bg-secondary, #181825)",
        padding: "10px 12px",
        overflowX: "auto",
        fontSize: 12,
        fontFamily: "var(--font-mono, 'Fira Code', 'JetBrains Mono', monospace)",
        margin: 0,
        whiteSpace: "pre",
        lineHeight: 1.6,
      }}>
        <code>{code}</code>
      </pre>
    </div>
  );
}

/** Parse message content and split into text segments and code blocks. */
function parseContentSegments(content: string): Array<{ type: "text"; text: string } | { type: "code"; code: string; language: string }> {
  const segments: Array<{ type: "text"; text: string } | { type: "code"; code: string; language: string }> = [];
  const fenceRe = /```(\w*)\n?([\s\S]*?)```/g;
  let lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = fenceRe.exec(content)) !== null) {
    if (match.index > lastIndex) {
      segments.push({ type: "text", text: content.slice(lastIndex, match.index) });
    }
    segments.push({ type: "code", language: match[1] ?? "", code: match[2] ?? "" });
    lastIndex = match.index + match[0].length;
  }
  if (lastIndex < content.length) {
    segments.push({ type: "text", text: content.slice(lastIndex) });
  }
  return segments;
}

// ===== MessageBubble =====

export const MessageBubble = memo(({ msg, isStreaming, onToggleThinking, onRewind }: {
  msg: ChatMessage; isStreaming?: boolean; onToggleThinking?: () => void; onRewind?: () => void;
}) => {
  const isUser = msg.role === "user";
  const isSystem = msg.role === "system";
  const isTool = msg.role === "tool";
  const isCompacted = msg.is_compacted;
  const isToolFailed = isTool && msg.content.startsWith("[FAILED");
  const isToolRunning = isTool && msg.content.startsWith("Running");

  return (
    <div style={{
      padding: isTool ? "6px 14px" : "10px 14px",
      margin: "4px 0",
      borderRadius: "var(--radius-lg)",
      background: isUser ? "var(--bg-hover)" : isTool ? "var(--bg-surface)" : isCompacted ? "var(--bg-surface)" : "var(--bg-tertiary)",
      borderLeft: isUser ? "3px solid var(--accent)" : isToolFailed ? "3px solid var(--danger)" : isToolRunning ? "3px solid var(--warning)" : isTool ? "3px solid var(--success)" : isCompacted ? "3px solid var(--text-muted)" : "3px solid var(--success)",
      opacity: isSystem ? 0.6 : 1,
      fontSize: isTool ? 11 : 13,
    }}>
      <div style={{ fontSize: 10, color: "var(--text-secondary)", marginBottom: 4, display: "flex", alignItems: "center", gap: 6 }}>
        <span style={{ fontWeight: 600, textTransform: "uppercase", letterSpacing: "0.5px" }}>
          {isUser ? "You" : isTool ? `Tool: ${msg.tool_name || ""}` : isCompacted ? "Compacted" : msg.role === "assistant" ? "AI" : msg.role}
        </span>
        {isCompacted && <span style={{ color: "var(--text-muted)", fontStyle: "italic" }}>context compacted</span>}
        {onRewind && (
          <button onClick={onRewind} title="Rewind to here" style={{ marginLeft: "auto", background: "none", border: "none", color: "var(--text-muted)", cursor: "pointer", fontSize: 10, padding: "0 4px" }}>&#8634;</button>
        )}
        {msg.token_count > 0 && <span style={{ marginLeft: onRewind ? 0 : "auto", fontSize: 9, color: "var(--text-muted)" }}>{fmtTokens(msg.token_count)} tokens</span>}
      </div>

      {msg.thinking && (
        <div style={{ marginBottom: 6 }}>
          <div onClick={onToggleThinking} style={{ cursor: "pointer", fontSize: 10, color: "var(--accent-hover)", display: "flex", alignItems: "center", gap: 4 }}>
            <span style={{ transform: msg.showThinking ? "rotate(90deg)" : "rotate(0deg)", transition: "transform 0.15s", display: "inline-block" }}>&#9654;</span>
            Thinking...
          </div>
          {msg.showThinking && (
            <div style={{ background: "var(--bg-secondary)", padding: "6px 10px", borderRadius: "var(--radius-sm)", marginTop: 4, fontSize: 11, color: "var(--text-secondary)", maxHeight: 200, overflow: "auto", whiteSpace: "pre-wrap" }}>
              {msg.thinking}
            </div>
          )}
        </div>
      )}

      {isTool && !isToolRunning && msg.content.includes("\n") ? (
        <details style={{ cursor: "pointer" }}>
          <summary style={{ whiteSpace: "pre-wrap", color: "var(--text-primary)", lineHeight: 1.5, outline: "none" }}>
            {msg.content.split("\n")[0]}
          </summary>
          <div style={{ lineHeight: 1.4, marginTop: 4, padding: "4px 0", maxHeight: 400, overflow: "auto", fontSize: 10, fontFamily: "var(--font-mono, monospace)" }}>
            {msg.content.split("\n").slice(1).map((line, idx) => {
              const isAdded = line.startsWith("+ ");
              const isRemoved = line.startsWith("- ");
              const isFileAction = line.match(/^[📄✏️🗑️💾]/u);
              return (
                <div key={idx} style={{
                  color: isAdded ? "#3fb950" : isRemoved ? "#f85149" : isFileAction ? "var(--accent)" : "var(--text-secondary)",
                  background: isAdded ? "rgba(63,185,80,0.08)" : isRemoved ? "rgba(248,81,73,0.08)" : "transparent",
                  padding: "0 4px",
                  whiteSpace: "pre-wrap",
                }}>
                  {line}
                </div>
              );
            })}
          </div>
        </details>
      ) : (
        <div style={{ color: "var(--text-primary)", lineHeight: 1.5 }}>
          {msg.content
            ? parseContentSegments(msg.content).map((seg, i) =>
                seg.type === "code"
                  ? <CodeBlock key={i} code={seg.code} language={seg.language} />
                  : <span key={i} style={{ whiteSpace: "pre-wrap" }}>{seg.text}</span>
              )
            : (isStreaming ? <span className="fc-cursor" /> : "")}
        </div>
      )}
    </div>
  );
});

// ===== TokenBar =====

export const TokenBar = memo(({ used, max, breakdown }: { used: number; max: number; breakdown?: { system: number; tools: number; history: number; response: number } | null }) => {
  const pct = max > 0 ? used / max : 0;
  const color = tokenBarColor(pct);
  const segments = breakdown ? [
    { label: "Sys", value: breakdown.system, color: "#58a6ff" },
    { label: "Tools", value: breakdown.tools, color: "#bc8cff" },
    { label: "Hist", value: breakdown.history, color: "#3fb950" },
    { label: "Out", value: breakdown.response, color: "#f0883e" },
  ] : null;
  const total = segments ? segments.reduce((s, seg) => s + seg.value, 0) : used;
  return (
    <div style={{ padding: "4px 12px", background: "var(--bg-primary)", borderTop: "1px solid var(--border-color)" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, fontSize: 10 }}>
        <span style={{ color: "var(--text-secondary)" }}>Tokens:</span>
        <div style={{ flex: 1, height: 6, background: "var(--bg-hover)", borderRadius: "var(--radius-sm)", overflow: "hidden", display: "flex" }}>
          {segments ? segments.map((seg, i) => {
            const segPct = max > 0 ? (seg.value / max) * 100 : 0;
            return segPct > 0 ? (
              <div key={i} title={`${seg.label}: ${fmtTokens(seg.value)}`} style={{
                width: `${Math.min(segPct, 100)}%`, height: "100%", background: seg.color,
                transition: "width 0.3s ease",
              }} />
            ) : null;
          }) : (
            <div style={{ width: `${Math.min(pct * 100, 100)}%`, height: "100%", background: color, borderRadius: "var(--radius-sm)", transition: "width 0.3s ease" }} />
          )}
        </div>
        <span style={{ color, fontWeight: 600, fontVariantNumeric: "tabular-nums" }}>
          {fmtTokens(total)} / {fmtTokens(max)} ({(pct * 100).toFixed(0)}%)
        </span>
      </div>
      {segments && (
        <div style={{ display: "flex", gap: 10, fontSize: 9, marginTop: 2, color: "var(--text-muted)" }}>
          {segments.map((seg, i) => seg.value > 0 ? (
            <span key={i} style={{ display: "flex", alignItems: "center", gap: 3 }}>
              <span style={{ width: 6, height: 6, borderRadius: 1, background: seg.color, display: "inline-block" }} />
              {seg.label} {fmtTokens(seg.value)}
            </span>
          ) : null)}
        </div>
      )}
    </div>
  );
});

// ===== LoadingBar =====

export const LoadingBar = memo(({ active }: { active: boolean }) => {
  if (!active) return null;
  return (
    <div style={{ height: 3, background: "var(--bg-hover)", overflow: "hidden" }}>
      <div className="fc-loading-bar" style={{ height: "100%", width: "40%", background: "linear-gradient(90deg, var(--accent), var(--accent-hover), var(--accent))", borderRadius: 2 }} />
    </div>
  );
});
