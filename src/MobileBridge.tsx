import { useState, useEffect, useCallback, useRef } from "react";
import App from "./App";
import MobileBridgeConnectionUI from "./components/MobileBridgeConnectionUI";

/* eslint-disable @typescript-eslint/no-explicit-any */
// WebKit message handlers are injected by iOS WKWebView — deep optional chaining
// makes full typing impractical, so we extend Window with a loose webkit property.
interface WebKitMessageHandler {
  postMessage(msg: unknown): void;
}
interface MobileWindow extends Window {
  webkit?: { messageHandlers?: Record<string, WebKitMessageHandler> };
  __PC_WORKSPACE_ROOT__?: string | null;
  __TAURI_INTERNALS__?: Record<string, unknown>;
  __TAURI__?: Record<string, unknown>;
  onQRScanned?: (data: string) => void;
  onBtStatus?: (data: BtStatusData) => void;
  onBtDeviceFound?: (data: BtDeviceData) => void;
  onBtMessage?: (json: string) => void;
}
const mobileWindow = window as unknown as MobileWindow;
/* eslint-enable @typescript-eslint/no-explicit-any */

interface RemoteMessage {
  id?: number;
  type: string;
  [key: string]: unknown;
}

/** Decompress a deflate-compressed message from the remote server.
 *  Messages with `{"type":"z","d":"<base64 deflate>"}` are inflated back to JSON. */
async function decompressMessage(raw: string): Promise<string> {
  try {
    const parsed = JSON.parse(raw);
    if (parsed.type === "z" && typeof parsed.d === "string") {
      const binary = Uint8Array.from(atob(parsed.d), c => c.charCodeAt(0));
      const ds = new DecompressionStream("deflate-raw");
      const writer = ds.writable.getWriter();
      writer.write(binary);
      writer.close();
      const reader = ds.readable.getReader();
      const chunks: Uint8Array[] = [];
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        chunks.push(value);
      }
      const totalLen = chunks.reduce((sum, c) => sum + c.length, 0);
      const result = new Uint8Array(totalLen);
      let offset = 0;
      for (const chunk of chunks) {
        result.set(chunk, offset);
        offset += chunk.length;
      }
      return new TextDecoder().decode(result);
    }
  } catch { /* not compressed, return as-is */ }
  return raw;
}

interface BtStatusData {
  state: string;
  message?: string;
}

interface BtDeviceData {
  index: number;
  name: string;
  id: string;
  rssi: number;
}

let msgCounter = 0;
let resolveQueue: Record<number, { resolve: (val: unknown) => void; reject: (err: unknown) => void }> = {};
let mobileWs: WebSocket | null = null;
let btActive = false;
let aiBaseUrl = localStorage.getItem("ai-base-url") || "";
let reconnectAttempt = 0;
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
let lastConnectionInfo: { host: string; token: string; name: string; id?: string } | null = null;
let lastBtDevice: BtDeviceData | null = null;
let btReconnectPending = false;

// PC workspace root — set after sync.getState, read by App on mount
mobileWindow.__PC_WORKSPACE_ROOT__ = null;

// Event emitters for mobile
const eventListeners: Record<string, Array<(payload: unknown) => void>> = {};
// Tauri v2 callback system for event listeners
let _cbCounter = 0;
const _callbacks: Record<number, Function> = {};
let _eventIdCounter = 0;
const _eventRegistry: Record<string, Array<{ callbackId: number; eventId: number }>> = {};
const emitEvent = (event: string, payload: unknown) => {
  if (eventListeners[event]) {
    eventListeners[event].forEach(l => l(payload));
  }
  // Call Tauri v2 registered event callbacks (from plugin:event|listen)
  if (_eventRegistry[event]) {
    for (const { callbackId } of _eventRegistry[event]) {
      if (_callbacks[callbackId]) {
        try { _callbacks[callbackId]({ event, payload, id: 0 }); } catch {}
      }
    }
  }
  // Also dispatch as DOM event for non-bridge code
  window.dispatchEvent(new CustomEvent(event, { detail: payload }));
};

const extractPayload = (m: RemoteMessage): unknown => {
  if (m.type === "fs.dirEntries") return m.entries;
  if (m.type === "fs.fileContent") return m.content;
  if (m.type === "fs.fileChunk") return { path: m.path, offset: m.offset, content: m.content, done: m.done, length: m.length };
  if (m.type === "fs.chunkWritten") return { path: m.path, offset: m.offset, written: m.written, done: m.done };
  if (m.type === "fs.fileInfo") return { size: m.size, is_binary: m.is_binary, line_count: m.line_count };
  if (m.type === "fs.homeDir") return m.path;
  if (m.type === "term.output") return m.data;
  if (m.type === "ide.state") return m.state;
  if (m.type === "sync.state") return { open_files: m.open_files, active_file: m.active_file, project_root: m.project_root };
  if (m.type === "llm.server_status") return m.status;
  if (m.type === "llm.engine_info") return m.info;
  if (m.type === "llm.hardware_info") return m.info;
  if (m.type === "llm.local_models") return m.models;
  if (m.type === "llm.installed_engines") return m.engines;
  if (m.type === "llm.recommended_backend") return m.backend;
  if (m.type === "chat.sessions") return m.sessions_json;
  if (m.type === "tauri.invokeResult") return m.result;
  // FerrumChat responses
  if (m.type === "ferrum.sessions") return m.sessions;
  if (m.type === "ferrum.session") return m.session;
  if (m.type === "ferrum.messages") return m.messages;
  if (m.type === "ferrum.profiles") return m.profiles;
  if (m.type === "ferrum.tokenCount") return m.count;
  if (m.type === "ferrum.export") return m.markdown;
  if (m.type === "ferrum.ok") return null;
  if (m.type === "ferrum.providerCheck") return m.connected;
  if (m.type === "ferrum.providerModels") return { models: m.models, connected: m.connected };
  // Workspace events
  if (m.type === "workspace.event") { emitEvent(`workspace-${m.event}`, m.payload); return m; }
  // Heartbeat
  if (m.type === "heartbeat.ack") return m;
  
  if (m.type.startsWith("llm.")) {
    if (m.info) return m.info;
    if (m.models) return m.models;
    if (m.engines) return m.engines;
    if (m.status) return m.status;
  }
  return m;
};

// Transport abstraction
const isTransportUp = () => (mobileWs && mobileWs.readyState === WebSocket.OPEN) || btActive;
const transportSend = (json: string) => {
  if (mobileWs && mobileWs.readyState === WebSocket.OPEN) {
    mobileWs.send(json);
  } else if (btActive && mobileWindow.webkit?.messageHandlers?.btSend) {
    mobileWindow.webkit.messageHandlers.btSend.postMessage(json);
  }
};

async function aiDirectFetch(path: string, body: Record<string, unknown>) {
  const url = aiBaseUrl.replace(/\/v1\/?$/, "") + path;
  return fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body)
  });
}

// Global mocks for Tauri API — override the simple HTML mock with our bridge mock,
// but only if not running in real Tauri (which sets __TAURI_INTERNALS__ as non-writable)
const _tauriDesc = Object.getOwnPropertyDescriptor(window, "__TAURI_INTERNALS__");
const _isRealTauri = !!_tauriDesc && !_tauriDesc.writable && !_tauriDesc.configurable;
if (!_isRealTauri) {
mobileWindow.__TAURI_INTERNALS__ = {
    transformCallback: (cb: Function, _once?: boolean) => {
      const id = ++_cbCounter;
      _callbacks[id] = cb;
      return id;
    },
    unregisterCallback: (id: number) => {
      delete _callbacks[id];
    },
    invoke: (cmd: string, args: Record<string, unknown>) => {
      // Handle Tauri event plugin commands
      if (cmd === "plugin:event|listen") {
        const eventId = ++_eventIdCounter;
        const eventName = args.event as string;
        const callbackId = args.handler as number;
        if (!_eventRegistry[eventName]) _eventRegistry[eventName] = [];
        _eventRegistry[eventName].push({ callbackId, eventId });
        return Promise.resolve(eventId);
      }
      if (cmd === "plugin:event|unlisten") {
        const eventName = args.event as string;
        const eid = args.eventId as number;
        if (_eventRegistry[eventName]) {
          const entry = _eventRegistry[eventName].find(e => e.eventId === eid);
          if (entry) delete _callbacks[entry.callbackId];
          _eventRegistry[eventName] = _eventRegistry[eventName].filter(e => e.eventId !== eid);
        }
        return Promise.resolve();
      }
      if (cmd === "plugin:event|emit") {
        emitEvent(args.event as string, args.payload);
        // Also forward to PC via WebSocket for events that need to reach the backend
        if (isTransportUp() && args.event) {
          transportSend(JSON.stringify({ id: ++msgCounter, type: "tauri.emitEvent", event: args.event, payload: args.payload }));
        }
        return Promise.resolve();
      }

      console.log("Bridge invoke:", cmd, args);

      if (cmd === "ai_chat_with_tools" || cmd === "ai_chat_stream") {
        if (isTransportUp()) {
          return new Promise((resolve) => {
            const id = ++msgCounter;
            resolveQueue[id] = { resolve: () => resolve(null), reject: () => resolve(null) };
            transportSend(JSON.stringify({ id, type: "tauri.invoke", cmd, args }));
          });
        }
        const streamId = args.streamId;
        // Stream chat completion from LLM server directly (local fallback)
        (async () => {
          try {
            const resp = await aiDirectFetch("/chat/completions", {
              model: args.model,
              messages: args.messages,
              temperature: args.temperature ?? 0.7,
              max_tokens: args.maxTokens ?? 2048,
              stream: true,
            });
            const reader = resp.body?.getReader();
            if (!reader) throw new Error("No response body");
            const decoder = new TextDecoder();
            let buffer = "";
            while (true) {
              const { done, value } = await reader.read();
              if (done) break;
              buffer += decoder.decode(value, { stream: true });
              const lines = buffer.split("\n");
              buffer = lines.pop() || "";
              for (const line of lines) {
                if (!line.startsWith("data: ")) continue;
                const data = line.slice(6).trim();
                if (data === "[DONE]") continue;
                try {
                  const chunk = JSON.parse(data);
                  const content = chunk.choices?.[0]?.delta?.content || "";
                  if (content) {
                    emitEvent(`ai-chat-stream-${streamId}`, { content });
                  }
                } catch { /* skip malformed chunk */ }
              }
            }
          } catch (err) {
            emitEvent(`ai-chat-stream-${streamId}`, { content: `\nError: ${err}\n` });
          }
          emitEvent(`ai-chat-done-${streamId}`, {});
          emitEvent(`ai-chat-stats-${streamId}`, { input_tokens: 0, output_tokens: 0, cache_stats: { entries: 0, total_hits: 0, enabled: false, ttl_seconds: 0 } });
        })();
        return Promise.resolve(null);
      }

      if (cmd === "ai_complete_code") return Promise.resolve(null);

      if (cmd === "abort_ai_chat") {
        if (isTransportUp()) {
          transportSend(JSON.stringify({ id: ++msgCounter, type: "tauri.invoke", cmd, args }));
        }
        return Promise.resolve(null);
      }

      if (isTransportUp()) {
        return new Promise((resolve, reject) => {
          const id = ++msgCounter;
          resolveQueue[id] = { resolve, reject };
          
          let type = cmd;
          if (cmd === "get_file_info") type = "fs.getFileInfo";
          if (cmd === "read_directory") type = "fs.readDir";
          if (cmd === "read_file_content") type = "fs.readFile";
          if (cmd === "read_file_chunk") type = "fs.readChunk";
          if (cmd === "write_file_content") type = "fs.writeFile";
          if (cmd === "create_directory") type = "fs.createDir";
          if (cmd === "delete_entry") type = "fs.delete";
          if (cmd === "rename_entry") type = "fs.rename";
          if (cmd === "get_home_dir") type = "fs.homeDir";
          
          if (cmd === "create_terminal") type = "term.create";
          if (cmd === "write_terminal") type = "term.write";
          if (cmd === "resize_terminal") type = "term.resize";
          if (cmd === "close_terminal") type = "term.close";

          if (cmd === "get_llm_server_status") type = "llm.get_llm_server_status";
          else if (cmd === "get_llm_network_info") type = "llm.get_llm_network_info";
          else if (cmd.startsWith("llm_") || cmd.startsWith("check_engine") || cmd === "list_installed_engines" || cmd === "detect_recommended_backend") {
            type = `llm.${cmd.replace("llm_", "")}`;
          }
          if (cmd === "chat_load_sessions") type = "chat.getSessions";
          if (cmd === "chat_save_sessions") type = "chat.saveSessions";

          // FerrumChat command mappings
          if (cmd === "ferrum_list_sessions") type = "ferrum.listSessions";
          if (cmd === "ferrum_get_latest_session") type = "ferrum.getLatestSession";
          if (cmd === "ferrum_create_session") type = "ferrum.createSession";
          if (cmd === "ferrum_load_messages") type = "ferrum.loadMessages";
          if (cmd === "ferrum_save_message") type = "ferrum.saveMessage";
          if (cmd === "ferrum_delete_session") type = "ferrum.deleteSession";
          if (cmd === "ferrum_rename_session") type = "ferrum.renameSession";
          if (cmd === "ferrum_get_profiles") type = "ferrum.getProfiles";
          if (cmd === "ferrum_get_session_token_count") type = "ferrum.getTokenCount";
          if (cmd === "ferrum_export_session") type = "ferrum.exportSession";
          if (cmd === "ferrum_get_active_profile") type = "ferrum.getProfiles";
          if (cmd === "ferrum_check_provider") type = "ferrum.checkProvider";
          if (cmd === "ferrum_list_provider_models") type = "ferrum.listProviderModels";

          transportSend(JSON.stringify({ id, type, ...args }));
        });
      }

      // Local fallbacks if not connected
      if (cmd === "get_home_dir") return Promise.resolve("/");
      if (cmd === "read_directory") return Promise.resolve([]);
      if (cmd === "check_engine") return Promise.resolve({ installed: false, binary_path: "", version: "", backend: "" });
      if (cmd === "detect_recommended_backend") return Promise.resolve("cpu");
      if (cmd === "list_installed_engines") return Promise.resolve([]);
      if (cmd === "get_llm_server_status") return Promise.resolve({ running: false, port: 8080, model: "", binary: "", backend: "" });
      if (cmd === "ai_check_connection") return Promise.resolve(false);
      if (cmd === "ai_get_models") return Promise.resolve([]);
      if (cmd === "chat_load_sessions") return Promise.resolve("[]");
      if (cmd === "token_get_cache_stats") return Promise.resolve({ entries: 0, total_hits: 0, enabled: false, ttl_seconds: 0 });
      if (cmd === "rag_build_index") return Promise.resolve(null);
      if (cmd === "rag_query") return Promise.resolve([]);
      if (cmd === "ai_explain_error") return Promise.resolve(null);
      // Project state fallbacks (not connected — can't reach PC)
      if (cmd === "project_list_recent") return Promise.resolve([]);
      if (cmd === "project_load_state") return Promise.resolve(null);
      if (cmd === "project_open") return Promise.resolve(null);
      if (cmd === "project_save_state") return Promise.resolve(null);
      if (cmd === "project_load_config") return Promise.resolve(null);
      if (cmd === "project_save_config") return Promise.resolve(null);
      if (cmd === "get_file_info") return Promise.resolve({ size: 0, is_binary: false });
      if (cmd === "get_hardware_info") return Promise.resolve({ cpu_cores: 4, ram_gb: 8, has_gpu: false, gpus: [] });
      if (cmd === "remote_update_state") return Promise.resolve(null);
      // FerrumChat fallbacks when not connected
      if (cmd === "ferrum_list_sessions") return Promise.resolve([]);
      if (cmd === "ferrum_get_profiles") return Promise.resolve([]);
      if (cmd === "ferrum_get_active_profile") return Promise.resolve(null);
      // ferrum_get_latest_session is now handled via WebSocket bridge
      if (cmd === "ferrum_check_provider") return Promise.resolve(false);
      if (cmd === "ferrum_list_provider_models") return Promise.resolve({ models: [], connected: false });
      if (cmd === "git_is_repo") return Promise.resolve(false);
      if (cmd === "git_status") return Promise.resolve({ branch: "", files: [] });
      // LSP fallbacks (no LSP on mobile)
      if (cmd === "lsp_completion") return Promise.resolve([]);
      if (cmd === "lsp_goto_definition") return Promise.resolve([]);
      if (cmd === "lsp_hover") return Promise.resolve(null);
      if (cmd === "lsp_diagnostics") return Promise.resolve([]);
      // Misc
      if (cmd === "list_hf_repo_files") return Promise.resolve([]);
      if (cmd === "scan_local_models") return Promise.resolve([]);

      return Promise.resolve(null);
    }
  };

// Tauri v2 event plugin internals (used by @tauri-apps/api/event _unlisten)
mobileWindow.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
  unregisterListener: (event: string, eventId: number) => {
    if (_eventRegistry[event]) {
      const entry = _eventRegistry[event].find(e => e.eventId === eventId);
      if (entry) delete _callbacks[entry.callbackId];
      _eventRegistry[event] = _eventRegistry[event].filter(e => e.eventId !== eventId);
    }
  }
};

mobileWindow.__TAURI__ = {
  event: {
    listen: (event: string, cb: (event: { payload: unknown; event: string }) => void) => {
      const wrapper = (payload: unknown) => cb({ payload, event });
      if (!eventListeners[event]) eventListeners[event] = [];
      eventListeners[event].push(wrapper);
      return Promise.resolve(() => {
        eventListeners[event] = eventListeners[event].filter(l => l !== wrapper);
      });
    }
  }
};
} // end if (!_isRealTauri)

// Persist connections in localStorage
interface SavedConnection {
  id: string;
  name: string;
  host: string;
  token: string;
  type?: "wifi" | "bt" | "ssh";
  username?: string;
  lastUsed: number;
}

const CONNECTIONS_KEY = "shadowide-mobile-conns";
const LAST_CONN_KEY = "shadowide-mobile-last";

const loadSavedConnections = (): SavedConnection[] => {
  try {
    const raw = localStorage.getItem(CONNECTIONS_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch { return []; }
};

const saveConnection = (conn: SavedConnection) => {
  const conns = loadSavedConnections();
  const existing = conns.findIndex(c => c.id === conn.id || (c.host === conn.host && c.token === conn.token));
  if (existing >= 0) conns[existing] = { ...conns[existing], ...conn, lastUsed: Date.now() };
  else conns.push({ ...conn, lastUsed: Date.now() });
  localStorage.setItem(CONNECTIONS_KEY, JSON.stringify(conns));
  localStorage.setItem(LAST_CONN_KEY, conn.id);
};

const loadLastConnectionId = () => localStorage.getItem(LAST_CONN_KEY);

export default function MobileBridge() {
  const [connected, setConnected] = useState(false);
  const [host, setHost] = useState("");
  const [token, setToken] = useState("");
  const [connType, setConnType] = useState<"wifi" | "bt" | "ssh">("wifi");
  const [sshUser, setSshUser] = useState("");
  const [sshPass, setSshPass] = useState("");
  const [error, setError] = useState("");
  const [statusLogs, setStatusLogs] = useState<string[]>([]);
  const [savedConnections, setSavedConnections] = useState<SavedConnection[]>(loadSavedConnections);
  const [activeConnectionId, setActiveConnectionId] = useState<string | null>(null);
  const [connectionName, setConnectionName] = useState("");
  const [connecting, setConnecting] = useState(false);
  const connectingRef = useRef(false);
  const [btScanning, setBtScanning] = useState(false);
  const [btDevices, setBtDevices] = useState<Array<{ index: number; name: string; id: string; rssi: number }>>([]);
  const [btState, setBtState] = useState("idle");
  const btReconnectTokenRef = useRef<string | null>(null);

  const log = (msg: string) => {
    setStatusLogs(prev => [...prev.slice(-6), msg]);
  };

  const handleSaveRename = (_e: React.SyntheticEvent, id: string, newName: string) => {
    const next = savedConnections.map(c => c.id === id ? { ...c, name: newName } : c);
    setSavedConnections(next);
    localStorage.setItem(CONNECTIONS_KEY, JSON.stringify(next));
  };

  const autoDetectLlm = async (pcHost: string, log: (m: string) => void) => {
    const ports = [8080, 1234, 11434];
    for (const p of ports) {
      const url = `http://${pcHost}:${p}/v1`;
      try {
        const r = await fetch(`${url}/models`, { signal: AbortSignal.timeout(1500) });
        if (r.ok) {
          log(`Detected LLM server on ${pcHost}:${p}`);
          aiBaseUrl = url;
          localStorage.setItem("ai-base-url", url);
          window.dispatchEvent(new CustomEvent("llm-auto-connected"));
          break;
        }
      } catch { /* skip */ }
    }
  };

  const doConnect = useCallback(async (targetHost: string, targetToken: string, name?: string, id?: string, type: "wifi" | "ssh" = "wifi") => {
    if (connectingRef.current) return;
    connectingRef.current = true;
    setConnecting(true);
    setError("");
    log(`Connecting to ${targetHost} via ${type.toUpperCase()}...`);

    if (type === "ssh") {
      if (mobileWindow.webkit?.messageHandlers?.sshConnect) {
        mobileWindow.webkit?.messageHandlers?.sshConnect?.postMessage({
          host: targetHost,
          username: sshUser,
          password: sshPass,
          token: targetToken
        });
        return; 
      } else {
        setError("SSH not available. Use the ShadowIDE iOS app.");
        setConnecting(false);
        connectingRef.current = false;
        return;
      }
    }

    const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
    const wsUrl = `${protocol}//${targetHost}/remote`;

    try {
      const socket = new WebSocket(wsUrl);
      let completed = false;

      const timeout = setTimeout(() => {
        if (!completed) {
          socket.close();
          setError("Connection timed out.");
          setConnecting(false);
          connectingRef.current = false;
        }
      }, 5000);

      socket.onopen = () => {
        if (completed) return socket?.close();
        clearTimeout(timeout);
        log("Socket open. Authenticating...");
        socket.send(JSON.stringify({ type: "auth", token: targetToken, device_name: "iPhone 16" }));
      };

      socket.onmessage = async (event) => {
        const raw = await decompressMessage(event.data as string);
        const m = JSON.parse(raw);
        if (m.type === "auth.ok") {
          completed = true;
          mobileWs = socket;
          setConnected(true);
          setConnecting(false);
          connectingRef.current = false;
          reconnectAttempt = 0;
          lastConnectionInfo = { host: targetHost, token: targetToken, name: name || targetHost, id };
          log("Authenticated!");

          const connId = id || Math.random().toString(36).slice(2);
          saveConnection({
            id: connId,
            name: name || targetHost,
            host: targetHost,
            token: targetToken,
            type: connType,
            username: sshUser,
            lastUsed: Date.now()
          });
          setActiveConnectionId(connId);
          setSavedConnections(loadSavedConnections());

          // Subscribe to workspace events from PC
          transportSend(JSON.stringify({ id: ++msgCounter, type: "sync.subscribe" }));

          // Fetch workspace state from PC to get project root
          const stateReqId = ++msgCounter;
          resolveQueue[stateReqId] = {
            resolve: (payload: unknown) => {
              const p = payload as Record<string, string> | null | undefined;
              if (p && p.project_root) {
                const pcRoot = p.project_root;
                const wsName = pcRoot.split("/").pop() || pcRoot;
                log(`Workspace: ${wsName}`);
                mobileWindow.__PC_WORKSPACE_ROOT__ = pcRoot;
                window.dispatchEvent(new CustomEvent("mobile-workspace-state", { detail: payload }));
              }
            },
            reject: () => {}
          };
          transportSend(JSON.stringify({ id: stateReqId, type: "sync.getState" }));

          // Start heartbeat interval (every 15s)
          const heartbeatInterval = setInterval(() => {
            if (isTransportUp()) {
              transportSend(JSON.stringify({ id: ++msgCounter, type: "heartbeat" }));
            } else {
              clearInterval(heartbeatInterval);
            }
          }, 15000);

          const pcHost = targetHost.split(":")[0];
          if (pcHost) autoDetectLlm(pcHost, log);
        } else if (m.type === "auth.error") {
          completed = true;
          setError(`Auth failed: ${m.message}`);
          socket.close();
          setConnecting(false);
          connectingRef.current = false;
        } else if (m.id && resolveQueue[m.id]) {
          if (m.type === "error") {
            resolveQueue[m.id].reject(new Error(m.message || "Remote error"));
          } else {
            resolveQueue[m.id].resolve(extractPayload(m));
          }
          delete resolveQueue[m.id];
        } else {
          if (m.type === "tauri.event") {
            emitEvent(m.event, m.payload);
            // Trigger system notification for AI completion
            if (m.event === "ai-chat-complete-notify") {
              const title = m.payload?.title || "ShadowAI";
              const body = m.payload?.body || "AI response complete";
              try {
                if (mobileWindow.webkit?.messageHandlers?.notifyUser) {
                  mobileWindow.webkit?.messageHandlers?.notifyUser?.postMessage({ title, body });
                } else if (typeof Notification !== "undefined" && Notification.permission === "granted") {
                  new Notification(title, { body, icon: "/icon.png", tag: "ai-complete" });
                } else if (typeof Notification !== "undefined" && Notification.permission !== "denied") {
                  Notification.requestPermission();
                }
              } catch {}
            }
          }
          if (m.type === "fs.dirEntries") emitEvent("fs.dirEntries", m);
          if (m.type === "fs.fileContent") emitEvent("fs.fileContent", m);
          if (m.type === "term.output") emitEvent("terminal-output", { id: m.terminal_id, data: m.data });
          if (m.type === "term.exit") emitEvent("terminal-exit", { id: m.terminal_id, code: m.code });
        }
      };

      socket.onclose = () => {
        if (mobileWs === socket) {
          mobileWs = null;
          setConnected(false);
          setActiveConnectionId(null);
          log("Disconnected.");

          // Auto-reconnect — instant on foreground return, gentle backoff otherwise
          if (lastConnectionInfo && reconnectAttempt < 12) {
            const delay = Math.min(500 * Math.pow(1.5, reconnectAttempt), 15000);
            reconnectAttempt++;
            log(`Reconnecting in ${Math.round(delay / 1000)}s...`);
            if (reconnectTimer) clearTimeout(reconnectTimer);
            reconnectTimer = setTimeout(() => {
              if (!mobileWs && lastConnectionInfo) {
                const { host: h, token: t, name: n, id: cid } = lastConnectionInfo;
                doConnect(h, t, n, cid, "wifi");
              }
            }, delay);
          }
        }
        setConnecting(false);
        connectingRef.current = false;
      };

      socket.onerror = () => {
        if (!completed) {
          if (targetHost.toLowerCase().includes("localhost") || targetHost.includes("127.0.0.1")) {
            setError(`Connection failed. Do NOT use "localhost" on your phone. Use your PC's IP address instead.`);
          } else {
            setError(`Could not connect to ${targetHost}: Connection refused. Ensure the ShadowIDE server is running on your PC and your phone is on the same network.`);
          }
        }
        setConnecting(false);
        connectingRef.current = false;
      };

    } catch (e) {
      setError(`Error: ${e}`);
      setConnecting(false);
      connectingRef.current = false;
    }
  }, [sshUser, sshPass, connType]);

  const startBtScan = () => {
    setBtDevices([]);
    setConnType("bt");
    if (mobileWindow.webkit?.messageHandlers?.btScan) {
      mobileWindow.webkit.messageHandlers!.btScan!.postMessage(null);
    } else {
      setError("Bluetooth not available (requires iOS app)");
    }
  };

  const stopBtScan = () => {
    if (mobileWindow.webkit?.messageHandlers?.btStopScan) {
      mobileWindow.webkit.messageHandlers!.btStopScan!.postMessage(null);
    }
  };

  const connectBtDevice = (device: { index: number; name: string; id: string; rssi: number }) => {
    lastBtDevice = device;
    btReconnectPending = false;
    btReconnectTokenRef.current = token || "mobile";
    if (mobileWindow.webkit?.messageHandlers?.btConnect) {
      mobileWindow.webkit.messageHandlers!.btConnect!.postMessage({ index: device.index, token: token || "mobile" });
    }
  };

  const deleteSavedConnection = (id: string) => {
    const next = savedConnections.filter(c => c.id !== id);
    setSavedConnections(next);
    localStorage.setItem(CONNECTIONS_KEY, JSON.stringify(next));
    if (activeConnectionId === id) setActiveConnectionId(null);
  };

  useEffect(() => {
    // Detect real Tauri (not our mock) via property descriptor — real Tauri defines non-writable props
    const desc = Object.getOwnPropertyDescriptor(window, "__TAURI_INTERNALS__");
    const isRealTauri = !!desc && !desc.writable && !desc.configurable;
    if (isRealTauri) setConnected(true);

    mobileWindow.onQRScanned = (data: string) => {
      try {
        const payload = typeof data === "string" ? JSON.parse(data) : data;
        if (payload.host && payload.port && payload.pairing_token) {
          const fullHost = `${payload.host}:${payload.port}`;
          if (mobileWindow.webkit?.messageHandlers?.trustCert) mobileWindow.webkit.messageHandlers.trustCert.postMessage(fullHost);
          setHost(fullHost); setToken(payload.pairing_token); setConnectionName(payload.name || ""); setConnType("wifi");
          setError("QR Scanned! Connecting...");
          setTimeout(() => { doConnect(fullHost, payload.pairing_token, payload.name || "", undefined, "wifi"); }, 1000);
        } else if (payload.transport === "bluetooth" && payload.pairing_token) {
          setToken(payload.pairing_token);
          setConnectionName(payload.name || "ShadowIDE BLE");
          setConnType("bt");
          btReconnectTokenRef.current = payload.pairing_token;
          btReconnectPending = true;
          setError("Bluetooth pairing loaded. Scanning for your desktop...");
          startBtScan();
        }
      } catch (e) { setError("Invalid QR: " + String(e)); }
    };

    mobileWindow.onBtStatus = (data: BtStatusData) => {
      setBtState(data.state || "idle");
      if (data.state === "scanning") setBtScanning(true);
      if (data.state === "disconnected" || data.state === "error" || data.state === "idle") setBtScanning(false);
      if (data.state === "disconnected" && btActive) { btActive = false; setConnected(false); }
      if (
        (data.state === "disconnected" || data.state === "error") &&
        lastBtDevice &&
        btReconnectTokenRef.current
      ) {
        btReconnectPending = true;
        window.setTimeout(() => {
          if (btReconnectPending) startBtScan();
        }, 1500);
      }
      if (data.message) setStatusLogs(prev => [...prev.slice(-6), `BT: ${data.message}`]);
      window.dispatchEvent(new CustomEvent("mobile-bt-status", { detail: data }));
    };

    mobileWindow.onBtDeviceFound = (data: BtDeviceData) => {
      setBtDevices(prev => {
        let next;
        if (prev.some(d => d.id === data.id)) next = prev;
        else next = [...prev, { index: data.index, name: data.name, id: data.id, rssi: data.rssi }];
        if (btReconnectPending && lastBtDevice && data.id === lastBtDevice.id) {
          btReconnectPending = false;
          connectBtDevice({ index: data.index, name: data.name, id: data.id, rssi: data.rssi });
        }
        window.dispatchEvent(new CustomEvent("mobile-bt-devices", { detail: { devices: next } }));
        return next;
      });
    };

    mobileWindow.onBtMessage = (json: string) => {
      try {
        const msg = JSON.parse(json);
        if (msg.type === "auth.ok") {
          btActive = true; setConnected(true); setBtState("authenticated");
          btReconnectPending = false;
          setStatusLogs(prev => [...prev.slice(-6), "Transport up! Authenticated."]);
          transportSend(JSON.stringify({ id: ++msgCounter, type: "sync.subscribe" }));
          const stateReqId = ++msgCounter;
          resolveQueue[stateReqId] = {
            resolve: (payload: unknown) => {
              const p = payload as Record<string, string> | null | undefined;
              if (p && p.project_root) {
                mobileWindow.__PC_WORKSPACE_ROOT__ = p.project_root;
                window.dispatchEvent(new CustomEvent("mobile-workspace-state", { detail: payload }));
              }
            },
            reject: () => {}
          };
          transportSend(JSON.stringify({ id: stateReqId, type: "sync.getState" }));
        }
        if (msg.id && resolveQueue[msg.id]) {
          resolveQueue[msg.id].resolve(extractPayload(msg));
          delete resolveQueue[msg.id];
        } else {
          if (msg.type === "tauri.event") emitEvent(msg.event, msg.payload);
          if (msg.type === "fs.dirEntries") emitEvent("fs.dirEntries", msg);
          if (msg.type === "fs.fileContent") emitEvent("fs.fileContent", msg);
          if (msg.type === "term.output") emitEvent("terminal-output", { id: msg.terminal_id, data: msg.data });
          if (msg.type === "term.exit") emitEvent("terminal-exit", { id: msg.terminal_id, code: msg.code });
        }
      } catch { /* ignore */ }
    };

    const hStart = () => startBtScan();
    const hStop = () => stopBtScan();
    const hConnect = (e: Event) => connectBtDevice((e as CustomEvent).detail);
    window.addEventListener("mobile-bt-start-scan", hStart);
    window.addEventListener("mobile-bt-stop-scan", hStop);
    window.addEventListener("mobile-bt-connect", hConnect);

    // Handle native CLI connection bridge — Swift injects this event after connecting
    const onNativeCLI = (e: Event) => {
      const { host: cliHost, token: cliToken } = (e as CustomEvent).detail || {};
      if (cliHost && cliToken && !mobileWs) {
        console.log("Native CLI bridge: auto-connecting to", cliHost);
        doConnect(cliHost, cliToken, "CLI", undefined, "wifi");
      }
    };
    window.addEventListener("native-cli-connected", onNativeCLI);

    // Also expose _autoConnect for direct JS injection from Swift
    (mobileWindow.__TAURI_INTERNALS__ as Record<string, unknown>)._autoConnect = (h: string, t: string) => {
      if (!mobileWs) doConnect(h, t, "CLI", undefined, "wifi");
    };

    return () => {
      delete mobileWindow.onQRScanned; delete mobileWindow.onBtStatus; delete mobileWindow.onBtDeviceFound; delete mobileWindow.onBtMessage;
      window.removeEventListener("mobile-bt-start-scan", hStart); window.removeEventListener("mobile-bt-stop-scan", hStop); window.removeEventListener("mobile-bt-connect", hConnect);
      window.removeEventListener("native-cli-connected", onNativeCLI);
    };
  }, [doConnect]);

  useEffect(() => {
    const lastId = loadLastConnectionId();
    if (lastId) {
      const conn = loadSavedConnections().find(c => c.id === lastId);
      if (conn) { setHost(conn.host); setToken(conn.token); setConnectionName(conn.name); setConnType(conn.type || "wifi"); setSshUser(conn.username || ""); }
    }
  }, []);

  // Auto-reconnect when app returns from background (iOS kills WebSocket on suspend)
  useEffect(() => {
    const onVisibilityChange = () => {
      if (document.visibilityState === "visible" && !mobileWs && lastConnectionInfo) {
        reconnectAttempt = 0; // Reset backoff
        if (reconnectTimer) { clearTimeout(reconnectTimer); reconnectTimer = null; }
        const { host: h, token: t, name: n, id: cid } = lastConnectionInfo;
        doConnect(h, t, n, cid, "wifi");
      }
    };
    document.addEventListener("visibilitychange", onVisibilityChange);
    return () => document.removeEventListener("visibilitychange", onVisibilityChange);
  }, [doConnect]);

  if (connected) return <App />;

  return (
    <MobileBridgeConnectionUI
      host={host}
      setHost={setHost}
      token={token}
      setToken={setToken}
      connType={connType}
      setConnType={setConnType}
      sshUser={sshUser}
      setSshUser={setSshUser}
      sshPass={sshPass}
      setSshPass={setSshPass}
      connectionName={connectionName}
      setConnectionName={setConnectionName}
      connecting={connecting}
      error={error}
      statusLogs={statusLogs}
      savedConnections={savedConnections}
      btScanning={btScanning}
      btDevices={btDevices}
      btState={btState}
      onConnect={doConnect}
      onStartBtScan={startBtScan}
      onStopBtScan={stopBtScan}
      onConnectBtDevice={connectBtDevice}
      onDeleteSavedConnection={deleteSavedConnection}
      onScanQR={() => mobileWindow.webkit?.messageHandlers?.scanQR?.postMessage(null)}
      onSaveRename={handleSaveRename}
    />
  );
}
