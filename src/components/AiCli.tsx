import { useState, useRef, useEffect, memo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface CliLine {
  type: "input" | "output" | "tool-call" | "tool-result" | "info" | "error";
  content: string;
  toolName?: string;
  success?: boolean;
  timestamp: number;
}

interface CliStats {
  model: string;
  inputTokens: number;
  outputTokens: number;
  cacheHits: number;
  totalTasks: number;
}

export function AiCli({ visible, rootPath }: { visible: boolean; rootPath: string }) {
  const [lines, setCliLines] = useState<CliLine[]>([]);
  const [input, setInput] = useState("");
  const [running, setRunning] = useState(false);
  const [stats, setStats] = useState<CliStats>({
    model: "unknown",
    inputTokens: 0,
    outputTokens: 0,
    cacheHits: 0,
    totalTasks: 0
  });
  
  const scrollRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const streamIdCounter = useRef(0);

  const addLine = (line: Omit<CliLine, "timestamp">) => {
    setCliLines(prev => [...prev, { ...line, timestamp: Date.now() }].slice(-500));
  };

  const appendToLastLine = (content: string) => {
    setCliLines(prev => {
      if (prev.length === 0) return [{ type: "output", content, timestamp: Date.now() }];
      const last = prev[prev.length - 1];
      if (last.type === "output") {
        const next = [...prev];
        next[next.length - 1] = { ...last, content: last.content + content };
        return next;
      } else {
        return [...prev, { type: "output", content, timestamp: Date.now() }];
      }
    });
  };

  useEffect(() => {
    if (visible && inputRef.current) {
      inputRef.current.focus();
    }
  }, [visible]);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [lines, running]);

  const handleCommand = async () => {
    if (!input.trim() || running) return;
    
    const cmd = input.trim();
    setInput("");
    setRunning(true);
    addLine({ type: "input", content: cmd });

    const streamId = `cli-${++streamIdCounter.current}`;
    
    // Listen for events
    const unlistenStream = await listen<any>(`ai-chat-stream-${streamId}`, (e) => {
      const content = typeof e.payload === 'string' ? e.payload : (e.payload.content || "");
      appendToLastLine(content);
    });

    const unlistenToolCall = await listen<any>(`ai-tool-call-${streamId}`, (e) => {
      addLine({ type: "tool-call", content: `Executing ${e.payload.name}(${e.payload.arguments})`, toolName: e.payload.name });
    });

    const unlistenToolStream = await listen<any>(`ai-tool-stream-${streamId}`, (e) => {
      appendToLastLine(e.payload.chunk || "");
    });

    const unlistenToolResult = await listen<any>(`ai-tool-result-${streamId}`, (e) => {
      addLine({
        type: "tool-result",
        content: e.payload.result,
        toolName: e.payload.name,
        success: e.payload.success
      });
    });

    const unlistenStats = await listen<any>(`ai-token-stats-${streamId}`, (e) => {
      const payload = e.payload;
      setStats(prev => ({
        ...prev,
        inputTokens: prev.inputTokens + (payload.input_tokens || 0),
        outputTokens: prev.outputTokens + (payload.output_tokens || 0),
        cacheHits: prev.cacheHits + (payload.cache_stats?.total_hits || 0),
        totalTasks: prev.totalTasks + 1
      }));
    });

    const unlistenDone = await listen(`ai-chat-done-${streamId}`, () => {
      setRunning(false);
      unlistenStream();
      unlistenToolCall();
      unlistenToolStream();
      unlistenToolResult();
      unlistenStats();
      unlistenDone();
      addLine({ type: "info", content: "Command complete." });
    });

    try {
      const chatMode = "auto"; // CLI is always automation mode
      const messages = [{ role: "user", content: cmd }];
      
      await invoke("ai_chat_with_tools", {
        streamId,
        messages,
        model: null,
        temperature: 0.2,
        maxTokens: 4096,
        toolsEnabled: true,
        chatMode,
        rootPath
      });
    } catch (e) {
      addLine({ type: "error", content: `Command failed: ${e}` });
      setRunning(false);
    }
  };

  if (!visible) return null;

  return (
    <div className="ai-cli-container" style={{
      display: "flex", flexDirection: "column", height: "100%", 
      background: "#000", color: "#00ff00", fontFamily: "monospace",
      fontSize: "12px", borderLeft: "1px solid #333"
    }}>
      {/* CLI Header / Stats */}
      <div style={{ padding: "8px 12px", borderBottom: "1px solid #222", background: "#0a0a0a", display: "flex", justifyContent: "space-between" }}>
        <div>SHADOW-CLI v0.84</div>
        <div style={{ color: "#888", fontSize: "10px" }}>
          IN: {stats.inputTokens} | OUT: {stats.outputTokens} | HITS: {stats.cacheHits}
        </div>
      </div>

      {/* CLI Output */}
      <div ref={scrollRef} style={{ flex: 1, overflowY: "auto", padding: "12px", scrollBehavior: "smooth" }}>
        {lines.map((line, i) => (
          <div key={i} style={{ marginBottom: "6px", wordBreak: "break-all" }}>
            {line.type === "input" && <div style={{ color: "#fff" }}><span style={{ color: "#555" }}>$</span> {line.content}</div>}
            {line.type === "output" && <div style={{ color: "#bbb" }}>{line.content}</div>}
            {line.type === "tool-call" && <div style={{ color: "#3b82f6" }}><span style={{ color: "#1d4ed8" }}>[TOOL]</span> {line.content}</div>}
            {line.type === "tool-result" && (
              <div style={{ color: line.success ? "#10b981" : "#ef4444", paddingLeft: "12px", opacity: 0.8 }}>
                {line.success ? "✓" : "✗"} {line.content.length > 300 ? line.content.slice(0, 300) + "..." : line.content}
              </div>
            )}
            {line.type === "info" && <div style={{ color: "#666", fontStyle: "italic" }}>// {line.content}</div>}
            {line.type === "error" && <div style={{ color: "#ef4444" }}>ERROR: {line.content}</div>}
          </div>
        ))}
        {running && <div className="cli-cursor" style={{ display: "inline-block", width: "8px", height: "14px", background: "#00ff00", animation: "blink 1s step-end infinite" }} />}
      </div>

      {/* CLI Input */}
      <div style={{ padding: "12px", borderTop: "1px solid #222", display: "flex", alignItems: "center", gap: "8px" }}>
        <span style={{ color: "#555" }}>$</span>
        <input
          ref={inputRef}
          style={{ flex: 1, background: "transparent", border: "none", color: "#fff", outline: "none", fontFamily: "monospace" }}
          value={input}
          onChange={e => setInput(e.target.value)}
          onKeyDown={e => e.key === "Enter" && handleCommand()}
          placeholder={running ? "AI is working..." : "Type a command..."}
          disabled={running}
        />
      </div>

      <style>{`
        @keyframes blink {
          from, to { opacity: 1; }
          50% { opacity: 0; }
        }
        .ai-cli-container::-webkit-scrollbar { width: 6px; }
        .ai-cli-container::-webkit-scrollbar-thumb { background: #333; border-radius: 3px; }
      `}</style>
    </div>
  );
}

export default memo(AiCli);
