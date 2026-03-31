import { useState, useRef, useEffect, memo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface ChatMessage {
  role: "user" | "assistant" | "system" | "tool-call" | "tool-result";
  content: string;
  toolName?: string;
  toolArgs?: string;
  success?: boolean;
  durationMs?: number;
  thinking?: string;
  showThinking?: boolean;
}

interface CacheStats {
  entries: number;
  total_hits: number;
  enabled: boolean;
  ttl_seconds: number;
}

interface AiProvider {
  name: string;
  base_url: string;
  available: boolean;
  model_count: number;
}

interface ModelInfo {
  id: string;
}

interface ChatSession {
  id: string;
  name: string;
  messages: ChatMessage[];
  model: string;
  createdAt: number;
  updatedAt: number;
}

const SESSIONS_KEY = "shadowide-chat-sessions";
const ACTIVE_SESSION_KEY = "shadowide-active-session";

function loadSessions(): ChatSession[] {
  try {
    const raw = localStorage.getItem(SESSIONS_KEY);
    if (raw) return JSON.parse(raw) as ChatSession[];
  } catch { /* ignore */ }
  return [];
}

function saveSessions(sessions: ChatSession[]) {
  const json = JSON.stringify(sessions);
  try { localStorage.setItem(SESSIONS_KEY, json); } catch { /* ignore */ }
  invoke("chat_save_sessions", { sessionsJson: json }).catch(() => {});
}

function generateId(): string {
  return Date.now().toString(36) + Math.random().toString(36).slice(2, 7);
}

interface AiChatProps {
  visible: boolean;
  activeFileContent?: string;
  activeFileName?: string;
  rootPath: string;
  systemPrompt: string;
  isFullscreen?: boolean;
  onToggleFullscreen?: () => void;
  onPopout?: () => void;
}

const ChatMessageItem = memo(({ msg, onRewind, onToggleThinking, isStreaming }: { 
  msg: ChatMessage; 
  onRewind?: () => void;
  onToggleThinking?: () => void;
  isStreaming?: boolean;
}) => {
  return (
    <div className={`ai-message ai-message-${msg.role}`}>
      <div className="ai-message-header">
        <span>{msg.role === "user" ? "You" : "ShadowAI"}</span>
        {onRewind && (
          <button className="ai-btn-icon" onClick={onRewind} title="Rewind" style={{ marginLeft: "auto", background: "transparent", border: "none", color: "var(--text-muted)", cursor: "pointer", padding: "2px" }}>
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M3 11l19-9-9 19-2-8-8-2z" transform="rotate(-90 12 12)" /><path d="M3 12a9 9 0 1 0 9-9 9.75 9.75 0 0 0-6.74 2.74L3 8"/><path d="M3 3v5h5"/></svg>
          </button>
        )}
      </div>
      {msg.thinking && (
        <div className="ai-thinking-block">
          <div className="ai-thinking-header" onClick={onToggleThinking}>
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" style={{ transform: msg.showThinking ? "rotate(0deg)" : "rotate(-90deg)", transition: "transform 0.2s ease" }}>
              <polyline points="6 9 12 15 18 9" />
            </svg>
            <span>Thought for {Math.ceil(msg.thinking.length / 15)}s</span>
          </div>
          {msg.showThinking && <div className="ai-thinking-content">{msg.thinking}</div>}
        </div>
      )}
      <div className="ai-message-content" style={{ whiteSpace: "pre-wrap" }}>
        {msg.content || (isStreaming ? "..." : "")}
      </div>
    </div>
  );
});

function NetworkQuickConnect({ onConnect }: { onConnect: (url: string) => void }) {
  const [info, setInfo] = useState<any>(null);
  useEffect(() => { invoke<any>("get_llm_network_info", { port: 8080 }).then(setInfo).catch(() => {}); }, []);
  const quickConnect = (ip: string) => onConnect(`http://${ip}:8080/v1`);
  return (
    <div style={{ padding: "6px 0", borderTop: "1px solid var(--border-subtle)", marginTop: 8 }}>
      <div style={{ fontSize: 10, color: "var(--text-muted)", marginBottom: 6, fontWeight: 600, textTransform: "uppercase" }}>Quick Connect</div>
      {info && (info.local_ip || info.tailscale_ip) && (
        <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
          {info.local_ip && <button className="ai-reconnect-btn" style={{ fontSize: 10, marginTop: 0 }} onClick={() => quickConnect(info.local_ip)}>Local</button>}
          {info.tailscale_ip && <button className="ai-reconnect-btn" style={{ fontSize: 10, marginTop: 0 }} onClick={() => quickConnect(info.tailscale_ip)}>Tailscale</button>}
        </div>
      )}
    </div>
  );
}

function AiChat({ visible, activeFileContent, activeFileName, rootPath, systemPrompt, isFullscreen, onToggleFullscreen, onPopout }: AiChatProps) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [streaming, setStreaming] = useState(false);
  const [connected, setConnected] = useState(false);
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [selectedModel, setSelectedModel] = useState("");
  const [includeFile, setIncludeFile] = useState(false);
  const [toolsEnabled, setToolsEnabled] = useState(true);
  const [showSettings, setShowSettings] = useState(false);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const streamIdCounter = useRef(0);

  // Global Settings with Persistence
  const [chatMode, setChatMode] = useState(() => localStorage.getItem("ai-chat-mode") || "build");
  const [temperature, setTemperature] = useState(() => Number(localStorage.getItem("ai-temp") || "0.7"));
  const [maxTokens] = useState(2048);
  const [cleanMode] = useState("trim");
  const [cacheEnabled, setCacheEnabled] = useState(() => localStorage.getItem("ai-cache-enabled") !== "false");
  const [maxContext, setMaxContext] = useState(() => Number(localStorage.getItem("ai-max-context") || "16384"));

  // Sync settings
  useEffect(() => {
    localStorage.setItem("ai-chat-mode", chatMode);
    localStorage.setItem("ai-temp", String(temperature));
    localStorage.setItem("ai-cache-enabled", String(cacheEnabled));
    localStorage.setItem("ai-max-context", String(maxContext));
    if (connected) {
      invoke("token_update_settings", { clean_mode: cleanMode, cache_enabled: cacheEnabled, max_context: maxContext }).catch(() => {});
    }
  }, [chatMode, temperature, cleanMode, cacheEnabled, maxContext, connected]);

  const [sessions, setSessions] = useState<ChatSession[]>(loadSessions);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(() => localStorage.getItem(ACTIVE_SESSION_KEY));
  const [showSessions, setShowSessions] = useState(false);
  const [sessionTokens, setSessionTokens] = useState({ input: 0, output: 0 });
  const [lastTokenStats, setLastTokenStats] = useState<any>(null);
  const [providers, setProviders] = useState<AiProvider[]>([]);
  const [savedProviders] = useState<AiProvider[]>(() => JSON.parse(localStorage.getItem("ai-saved-providers") || "[]"));
  const [activeProvider, setActiveProvider] = useState(() => localStorage.getItem("ai-active-provider") || "LM Studio");
  const [showProviders, setShowProviders] = useState(false);
  const [cacheStats, setCacheStats] = useState<CacheStats | null>(null);
  const streamingCharsRef = useRef(0);
  const sessionIdRef = useRef<string | null>(null);

  const fmtTokens = (n: number) => n > 9999 ? `${(n / 1000).toFixed(0)}k` : n > 999 ? `${(n / 1000).toFixed(1)}k` : String(n);

  const createSession = () => {
    const id = generateId();
    const newSess: ChatSession = { id, name: "New Chat", messages: [], model: selectedModel, createdAt: Date.now(), updatedAt: Date.now() };
    setSessions(prev => { const n = [newSess, ...prev]; saveSessions(n); return n; });
    setActiveSessionId(id); setMessages([]);
  };

  const loadSession = (id: string) => {
    const sess = sessions.find(s => s.id === id);
    if (sess) { setActiveSessionId(id); setMessages(sess.messages); setShowSessions(false); }
  };

  const deleteSession = (id: string) => {
    setSessions(prev => { const n = prev.filter(s => s.id !== id); saveSessions(n); return n; });
    if (activeSessionId === id) createSession();
  };

  const switchProvider = async (name: string, url: string) => {
    setActiveProvider(name); localStorage.setItem("ai-active-provider", name); localStorage.setItem("ai-base-url", url);
    await invoke("ai_set_base_url", { url });
    const isUp = await invoke<boolean>("ai_check_connection");
    setConnected(isUp);
    if (isUp) { 
      const m = await invoke<ModelInfo[]>("ai_get_models"); 
      setModels(m); 
      if (m.length > 0 && !selectedModel) setSelectedModel(m[0].id); 
      const stats = await invoke<CacheStats>("token_get_cache_stats");
      setCacheStats(stats);
    }
  };

  const detectProviders = async () => {
    try {
      let merged = await invoke<AiProvider[]>("ai_detect_providers");
      for (const sp of savedProviders) { if (!merged.find(p => p.base_url === sp.base_url)) { merged.push({ ...sp, available: false }); } }
      setProviders(merged);
      const lastUrl = localStorage.getItem("ai-base-url");
      if (lastUrl) await switchProvider(activeProvider, lastUrl);
    } catch { setConnected(false); }
  };

  const sendMessage = async () => {
    if (!input.trim() || streaming || !connected) return;
    const userMsg: ChatMessage = { role: "user", content: input };
    const newMsgs = [...messages, userMsg];
    setMessages([...newMsgs, { role: "assistant", content: "" }]);
    setInput(""); setStreaming(true); streamingCharsRef.current = 0;

    const streamId = `chat-${++streamIdCounter.current}`;
    const apiMessages: { role: string; content: string }[] = newMsgs.map(m => ({ role: m.role, content: m.content }));

    let combinedSystem = systemPrompt || "You are ShadowAI, an expert developer.";
    if (includeFile && activeFileContent) {
      combinedSystem += `\n\nContext from ${activeFileName}:\n\n\`\`\`\n${activeFileContent}\n\`\`\``;
    }
    apiMessages.unshift({ role: "system", content: combinedSystem });

    const unlistenStream = await listen<any>(`ai-chat-stream-${streamId}`, (e) => {
      const content = typeof e.payload === 'string' ? e.payload : (e.payload.content || "");
      streamingCharsRef.current += content.length;
      setMessages(prev => {
        const n = [...prev];
        const last = n[n.length-1];
        if (last && last.role === "assistant") {
          last.content += content;
        }
        return n;
      });
    });

    const unlistenDone = await listen(`ai-chat-done-${streamId}`, () => {
      setStreaming(false); unlistenStream(); unlistenDone();
      setSessions(prev => {
        const n = prev.map(s => s.id === activeSessionId ? { ...s, messages: messages, updatedAt: Date.now() } : s);
        saveSessions(n);
        return n;
      });
    });

    const unlistenStats = await listen<any>(`ai-token-stats-${streamId}`, (e) => {
      setSessionTokens(prev => ({
        input: prev.input + (e.payload.input_tokens || 0),
        output: prev.output + (e.payload.output_tokens || 0)
      }));
      setLastTokenStats(e.payload);
      unlistenStats();
    });

    try {
      // Route through session system so phone sees the same chat.
      // session_chat sends the message through the agent queue → agent_runner
      // processes it → fires Tauri events (we listen above) AND puts events
      // in the session buffer (phone gets them via WebSocket).
      if (sessionIdRef.current) {
        await invoke("session_chat", {
          streamId, messages: apiMessages, model: selectedModel,
          temperature, maxTokens, toolsEnabled, chatMode, rootPath
        });
      } else {
        // Fallback: direct invoke if no session (shouldn't happen)
        await invoke("ai_chat_with_tools", { streamId, messages: apiMessages, model: selectedModel, temperature, maxTokens, toolsEnabled, chatMode, rootPath });
      }
    } catch (e) {
      setMessages(prev => { const n = [...prev]; const last = n[n.length-1]; if (last) last.content = `Error: ${e}`; return n; });
      setStreaming(false); unlistenStream(); unlistenDone();
    }
  };

  // Join the primary session on mount — this connects the PC UI to the
  // same session the phone uses, so both see the same chat.
  useEffect(() => {
    let unlistenSession: (() => void) | null = null;

    const joinSession = async () => {
      try {
        const result = await invoke<{ session_id: string; is_new: boolean }>("session_join", { sessionId: null });
        sessionIdRef.current = result.session_id;
        console.log(`[AiChat] Joined session ${result.session_id} (new: ${result.is_new})`);

        // Listen to session events from the phone.
        // When the phone sends a message, the agent_runner fires Tauri events
        // with the phone's streamId. Those events are also emitted as
        // session-agent-event by the session bridge. We display them here.
        const unlisten = await listen<any>("session-agent-event", (e) => {
          const event = e.payload;
          if (!event) return;
          const eventType: string = event.type || event.event_type || "";

          // Handle text chunks from phone-initiated chats
          if (eventType.startsWith("ai-chat-stream-")) {
            const content = typeof event.payload === "string"
              ? event.payload
              : (event.payload?.content || "");
            if (content) {
              setMessages(prev => {
                const n = [...prev];
                const last = n[n.length - 1];
                if (last && last.role === "assistant") {
                  last.content += content;
                } else {
                  n.push({ role: "assistant", content });
                }
                return n;
              });
              if (!streaming) setStreaming(true);
            }
          }

          // Handle user messages from phone
          if (eventType === "user_message") {
            const text = event.payload?.text || "";
            if (text) {
              setMessages(prev => [...prev, { role: "user", content: text }, { role: "assistant", content: "" }]);
            }
          }

          // Handle done events from phone-initiated chats
          if (eventType.startsWith("ai-chat-done-") || eventType === "agent_done") {
            setStreaming(false);
          }

          // Handle token stats from phone-initiated chats
          if (eventType.startsWith("ai-chat-stats-") || eventType.startsWith("ai-token-stats-")) {
            const p = event.payload;
            if (p?.input_tokens || p?.output_tokens) {
              setSessionTokens(prev => ({
                input: prev.input + (p.input_tokens || 0),
                output: prev.output + (p.output_tokens || 0)
              }));
            }
          }
        });
        unlistenSession = unlisten;
      } catch (e) {
        console.warn("[AiChat] Failed to join session:", e);
      }
    };

    joinSession();
    return () => { if (unlistenSession) unlistenSession(); };
  }, []);

  useEffect(() => { detectProviders(); }, []);

  // Auto-sync with LLM Loader: when a local model server starts/stops, update connection
  useEffect(() => {
    const unlisteners: (() => void)[] = [];
    listen<{ port: number; url: string }>("llm-server-started", (e) => {
      const url = e.payload.url || `http://localhost:${e.payload.port}/v1`;
      switchProvider("Local LLM", url);
    }).then((u) => { unlisteners.push(u); });
    listen("llm-server-stopped", () => {
      setConnected(false);
      setModels([]);
    }).then((u) => { unlisteners.push(u); });
    return () => { unlisteners.forEach(u => u()); };
  }, []);
  useEffect(() => { if (activeSessionId) localStorage.setItem(ACTIVE_SESSION_KEY, activeSessionId); }, [activeSessionId]);
  useEffect(() => { if (messagesEndRef.current) messagesEndRef.current.scrollIntoView({ behavior: "smooth" }); }, [messages]);

  if (!visible) return null;

  return (
    <div className={`ai-chat ${isFullscreen ? "fullscreen" : ""}`}>
      <div className="ai-chat-header">
        <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
          <span className="ai-chat-title">SHADOW AI</span>
          <div className="ai-chat-mode-pills">
            {["plan", "build", "auto"].map(m => (
              <button key={m} className={`ai-mode-pill ${chatMode === m ? "active" : ""}`} onClick={() => setChatMode(m)}>{m.toUpperCase()}</button>
            ))}
          </div>
        </div>
        <div className="ai-chat-controls">
          <button className="ai-btn" onClick={() => setShowProviders(!showProviders)}>Providers</button>
          <button className="ai-btn" onClick={() => setShowSettings(!showSettings)}>Settings</button>
          <button className="ai-btn" onClick={() => setShowSessions(!showSessions)}>Sessions</button>
          <button className="ai-btn" onClick={createSession}>New</button>
          {onToggleFullscreen && <button className="ai-btn" onClick={onToggleFullscreen}>{isFullscreen ? "Exit" : "Full"}</button>}
          {onPopout && <button className="ai-btn" onClick={onPopout}>Pop</button>}
        </div>
      </div>

      {showSessions && <div className="ai-sessions">
        {sessions.map(s => (
          <div key={s.id} className={`ai-session-row ${s.id === activeSessionId ? "active" : ""}`} onClick={() => loadSession(s.id)}>
            <span style={{ flex: 1 }}>{s.name}</span>
            <button onClick={(e) => { e.stopPropagation(); deleteSession(s.id); }}>x</button>
          </div>
        ))}
      </div>}

      {showProviders && <div className="ai-providers">
        {providers.map(p => (
          <div key={p.base_url} className={`ai-provider-row ${activeProvider === p.name ? "active" : ""}`} onClick={() => switchProvider(p.name, p.base_url)}>
            <span className={`ai-provider-dot ${p.available ? "on" : "off"}`} /> {p.name}
          </div>
        ))}
        <NetworkQuickConnect onConnect={(url) => switchProvider("Custom", url)} />
      </div>}

      {showSettings && <div className="ai-settings">
        <div className="ai-settings-title">TOKEN TWEAKER</div>
        <div className="ai-setting-row"><label>Model</label>
          <select value={selectedModel} onChange={e => setSelectedModel(e.target.value)}>
            {models.map(m => <option key={m.id} value={m.id}>{m.id}</option>)}
          </select>
        </div>
        <div className="ai-setting-row"><label>Context Budget</label><input type="number" className="ai-setting-input" value={maxContext} onChange={e => setMaxContext(Number(e.target.value))} /></div>
        <div className="ai-setting-row"><label>Temperature</label><input type="range" min="0" max="200" value={temperature * 100} onChange={e => setTemperature(Number(e.target.value) / 100)} /><span>{temperature.toFixed(2)}</span></div>
        <div className="ai-setting-row"><label className="ai-checkbox"><input type="checkbox" checked={cacheEnabled} onChange={e => setCacheEnabled(e.target.checked)} /> LM Cache</label></div>
        {cacheStats && <div style={{ fontSize: 10, color: "var(--text-muted)" }}>{cacheStats.entries} cached / {cacheStats.total_hits} hits</div>}
      </div>}

      <div className="ai-messages">
        {messages.map((msg, i) => <ChatMessageItem key={i} msg={msg} isStreaming={streaming && i === messages.length-1} onRewind={msg.role === "user" ? () => { setMessages(messages.slice(0, i)); setInput(msg.content); } : undefined} onToggleThinking={() => setMessages(prev => { const n = [...prev]; const m = n[i]; if (m) m.showThinking = !m.showThinking; return n; })} />)}
        <div ref={messagesEndRef} />
      </div>

      <div className="ai-status-line">
        <div className="ai-token-ring">
          <span className="ai-token-ring-label">{fmtTokens(sessionTokens.input + sessionTokens.output)}</span>
        </div>
        <div style={{ fontSize: "9px", color: "var(--text-muted)", flex: 1, marginLeft: 8 }}>
          IN: {fmtTokens(sessionTokens.input)} | OUT: {fmtTokens(sessionTokens.output)}
          {lastTokenStats?.cached && " (CACHED)"}
        </div>
        {streaming && <div className="ai-stream-indicator">Streaming...</div>}
      </div>

      <div className="ai-input-area">
        <div className="ai-input-options">
          <label className="ai-checkbox"><input type="checkbox" checked={includeFile} onChange={e => setIncludeFile(e.target.checked)} /> File</label>
          <label className="ai-checkbox"><input type="checkbox" checked={toolsEnabled} onChange={e => setToolsEnabled(e.target.checked)} /> Tools</label>
        </div>
        <div className="ai-input-row">
          <textarea ref={inputRef} className="ai-input" value={input} onChange={e => setInput(e.target.value)} onKeyDown={e => e.key === "Enter" && !e.shiftKey && (e.preventDefault(), sendMessage())} placeholder="Ask ShadowAI..." />
          <button className="ai-send-btn" onClick={sendMessage} disabled={streaming || !connected}>Send</button>
        </div>
      </div>
    </div>
  );
}

export default memo(AiChat);
