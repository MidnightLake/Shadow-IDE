import { useState, useRef, useEffect, memo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, emit } from "@tauri-apps/api/event";
import { MessageBubble, TokenBar, LoadingBar, fmtTokens, timeAgo } from "./FerrumChatMessage";
import type { ChatMessage } from "./FerrumChatMessage";
import FerrumChatInput from "./FerrumChatInput";
import FerrumChatSessions from "./FerrumChatSessions";

// ===== Types =====

interface Profile {
  name: string;
  provider: string;
  model: string;
  base_url: string;
  api_key_env: string;
  max_context_tokens: number;
  system_prompt: string;
  tools: string[];
}

interface Session {
  id: string;
  name: string;
  profile: string;
  created_at: number;
  updated_at: number;
  is_pinned: boolean;
}

interface CompactionCheck {
  should_compact: boolean;
  used_tokens: number;
  max_tokens: number;
  percentage: number;
}

interface FerrumChatProps {
  visible: boolean;
  rootPath: string;
  activeFileContent?: string;
  activeFileName?: string;
  isFullscreen?: boolean;
  onToggleFullscreen?: () => void;
  onPopout?: () => void;
}

type ViewMode = "chat" | "sessions" | "profiles" | "settings" | "memory";
type ChatMode = "plan" | "build" | "auto";

// ===== ShadowEditor Slash Command System (planengine.md §7.4) =====

const SHADOW_BASE_SYSTEM = `You are an expert C++23 game engine developer working on ShadowEditor — a Rust+egui editor with a C++23 ECS game runtime compiled as a hot-reloadable shared library.

Key conventions:
- Components are annotated with SHADOW_COMPONENT() and SHADOW_PROPERTY() macros
- All game code must compile as a C++ shared library with a stable C ABI (extern "C" entry points)
- ECS systems take a pointer to the world and iterate over component arrays
- Use std::expected instead of exceptions, std::flat_map for component storage
- Hot-reload survives: entity IDs, serialized component data; resets: in-memory system state`;

function buildShadowSlashPrompt(cmd: string, arg: string): string {
  const argNote = arg ? `\n\nUser description: "${arg}"` : "";
  switch (cmd) {
    case "/component":
      return `\n\n${SHADOW_BASE_SYSTEM}\n\nTask: Generate a complete C++23 component for ShadowEditor.\n- Produce a .h file with SHADOW_COMPONENT() / SHADOW_PROPERTY() annotations\n- Produce a matching .cpp file with any method implementations\n- Include reasonable defaults and doc comments${argNote}`;
    case "/system":
      return `\n\n${SHADOW_BASE_SYSTEM}\n\nTask: Generate a complete C++23 ECS system for ShadowEditor.\n- The system function must accept a world pointer and iterate components efficiently\n- Use range-based iteration over the relevant component arrays\n- Register the system with a static initializer or via the shadow_init hook${argNote}`;
    case "/shader":
      return `\n\n${SHADOW_BASE_SYSTEM}\n\nTask: Generate a GLSL or WGSL shader for ShadowEditor's wgpu renderer.\n- Prefer WGSL for the native wgpu pipeline, GLSL for Vulkan-specific paths\n- Include vertex and fragment entry points\n- Add uniform buffer declarations and sampler bindings as needed${argNote}`;
    case "/debug":
      return `\n\n${SHADOW_BASE_SYSTEM}\n\nTask: Analyze the ShadowEditor build output, console logs, and scene context below.\n- Identify the root cause of any errors\n- Propose a minimal, targeted patch (diff format preferred)\n- Explain what caused the issue and how the fix addresses it${argNote}`;
    case "/scene":
      return `\n\n${SHADOW_BASE_SYSTEM}\n\nTask: Generate a complete ShadowEditor scene file (.shadow TOML format).\n- Use [[entity]] sections with nested [[entity.component]] arrays\n- Include appropriate Transform, MeshRenderer, and game-specific components\n- Keep entity IDs as short readable strings${argNote}`;
    case "/prefab":
      return `\n\n${SHADOW_BASE_SYSTEM}\n\nTask: Generate a ShadowEditor prefab — a .shadow scene file for a reusable entity group plus the associated C++ component .h/.cpp.\n- Name components clearly, expose configurable properties via SHADOW_PROPERTY\n- The prefab scene should reference asset paths as "assets/..." strings${argNote}`;
    case "/create":
      return `\n\n${SHADOW_BASE_SYSTEM}\n\nTask: Describe the components and initial values needed to create this entity in ShadowEditor.\n- List each component type with its SHADOW_PROPERTY fields and default values\n- Provide the entity entry as a TOML snippet ready to paste into the .shadow scene file${argNote}`;
    case "/explain":
      return `\n\n${SHADOW_BASE_SYSTEM}\n\nTask: Explain the following code or concept in the context of ShadowEditor's C++23 game engine.\n- Relate concepts to the ECS architecture, hot-reload flow, or C ABI boundary where relevant${argNote}`;
    case "/refactor":
      return `\n\n${SHADOW_BASE_SYSTEM}\n\nTask: Refactor the provided C++23 game code.\n- Improve ECS data locality, remove unnecessary copies, simplify component iteration\n- Output a unified diff or complete replacement file${argNote}`;
    case "/fix":
      return `\n\n${SHADOW_BASE_SYSTEM}\n\nTask: Fix the compiler error in the provided C++23 game code.\n- Identify the exact cause, apply the minimal change to fix it\n- Output the corrected code with a brief explanation${argNote}`;
    default:
      return "";
  }
}

// ===== Main Component =====

export function FerrumChat({ visible, rootPath, activeFileContent, activeFileName, isFullscreen, onToggleFullscreen, onPopout }: FerrumChatProps) {
  const [profiles, setProfiles] = useState<Profile[]>([]);
  const [activeProfile, setActiveProfile] = useState<Profile | null>(null);
  const [sessions, setSessions] = useState<Session[]>([]);
  const [activeSession, setActiveSession] = useState<Session | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [streaming, setStreaming] = useState(false);
  const [connected, setConnected] = useState(false);
  const [viewMode, setViewMode] = useState<ViewMode>("chat");
  const [availableModels, setAvailableModels] = useState<string[]>([]);
  const [selectedModel, setSelectedModel] = useState("");
  const [usedTokens, setUsedTokens] = useState(0);
  const [maxTokens, setMaxTokens] = useState(120000);
  const [tokenBreakdown, setTokenBreakdown] = useState<{ system: number; tools: number; history: number; response: number } | null>(null);
  const [toolsEnabled, setToolsEnabled] = useState(true);
  const [temperature, setTemperature] = useState(0.7);

  const [compactNotice, setCompactNotice] = useState(false);
  const [editingProfile, setEditingProfile] = useState<Profile | null>(null);
  const [connectedUrl, setConnectedUrl] = useState("");
  const [loadedModelName, setLoadedModelName] = useState("");
  const [serverStopping, setServerStopping] = useState(false);
  const [chatMode, setChatMode] = useState<ChatMode>("build");
  const [includeFile, setIncludeFile] = useState(false);
  const [gitBranch, setGitBranch] = useState("");
  const [gitDirtyCount, setGitDirtyCount] = useState(0);
  const [toolConfirm, setToolConfirm] = useState<{ name: string; arguments: string; risk: string; streamId: string } | null>(null);
  const [autoApproveSession, setAutoApproveSession] = useState(false);
  const [memories, setMemories] = useState<any[]>([]);
  const [memoryFilter, setMemoryFilter] = useState("");

  const messagesEndRef = useRef<HTMLDivElement>(null);

  const streamIdRef = useRef(0);

  // ===== Auto-connect to LLM Loader =====

  useEffect(() => {
    const unlisteners: (() => void)[] = [];

    // Listen for LLM server start events from LLM Loader
    listen<{ port: number; url: string }>("llm-server-started", (e) => {
      const url = e.payload.url || `http://localhost:${e.payload.port}/v1`;
      setServerStopping(false);
      autoConnectToUrl(url);
    }).then(fn => { unlisteners.push(fn); });

    // Listen for LLM server stop events
    listen("llm-server-stopped", () => {
      setConnected(false);
      setConnectedUrl("");
      setLoadedModelName("");
      setAvailableModels([]);
      setServerStopping(true);
      setTimeout(() => setServerStopping(false), 3000);
    }).then(fn => { unlisteners.push(fn); });

    // Listen for model load/unload events
    listen<{ model: string }>("llm-model-loaded", (e) => {
      if (e.payload.model) {
        setLoadedModelName(e.payload.model);
        setSelectedModel(e.payload.model);
      }
    }).then(fn => { unlisteners.push(fn); });

    // Also check if a server is already running on startup
    checkExistingLlmServer();

    // Listen for mobile auto-detect LLM (DOM event from MobileBridge)
    const onAutoLlm = () => { checkExistingLlmServer(); };
    window.addEventListener("llm-auto-connected", onAutoLlm);

    // Listen for LLM server ready event (health check passed)
    listen<{ port: number; model: string; context_length: number }>("llm-server-ready", (e) => {
      if (e.payload.context_length > 0) setMaxTokens(e.payload.context_length);
      if (e.payload.model) { setLoadedModelName(e.payload.model); setSelectedModel(e.payload.model); }
      checkExistingLlmServer();
    }).then(fn => { unlisteners.push(fn); });

    // Listen for remote message saves (sync between PC and phone)
    listen<{ session_id: string; message: Record<string, unknown> }>("ferrum-message-saved", (e) => {
      // Refresh messages if the saved session matches our active session
      if (e.payload?.session_id) {
        setSessions(prev => prev.map(s => s.id === e.payload.session_id ? { ...s, updated_at: Math.floor(Date.now() / 1000) } : s));
        // Only reload messages if we're viewing this session and not currently streaming
        setActiveSession(current => {
          if (current?.id === e.payload.session_id) {
            invoke<ChatMessage[]>("ferrum_load_messages", { sessionId: e.payload.session_id })
              .then(msgs => { setMessages(prev => { if (msgs.length > prev.length) return msgs; return prev; }); })
              .catch(() => {});
          }
          return current;
        });
      }
    }).then(fn => { unlisteners.push(fn); });

    return () => { unlisteners.forEach(fn => fn()); window.removeEventListener("llm-auto-connected", onAutoLlm); };
  }, []);

  const checkExistingLlmServer = async () => {
    try {
      const status = await invoke<{ running: boolean; port: number; model: string; context_length?: number }>("get_llm_server_status");
      if (status.running && status.port) {
        // Try to get network info for LAN IP (works on both PC and mobile via bridge)
        let url = `http://localhost:${status.port}/v1`;
        try {
          const netInfo = await invoke<{ local_url?: string }>("get_llm_network_info", { port: status.port });
          if (netInfo?.local_url) url = netInfo.local_url;
        } catch { /* fallback to localhost */ }
        if (status.model) {
          setLoadedModelName(status.model);
          setSelectedModel(status.model);
        }
        if (status.context_length && status.context_length > 0) {
          setMaxTokens(status.context_length);
        }
        autoConnectToUrl(url);
      }
    } catch { /* LLM Loader may not be initialized */ }
  };

  const autoConnectToUrl = async (url: string) => {
    try {
      const result = await invoke<{ models: string[]; connected: boolean }>("ferrum_list_provider_models", { baseUrl: url });
      if (result.connected) {
        setConnected(true);
        setConnectedUrl(url);
        setAvailableModels(result.models);
        if (result.models.length > 0) setSelectedModel(result.models[0]);

        // Create or update a "llama.cpp" profile automatically
        const llmProfile: Profile = {
          name: "llama.cpp (Local)",
          provider: "llama.cpp",
          model: result.models[0] || "default",
          base_url: url,
          api_key_env: "",
          max_context_tokens: 120000,
          system_prompt: "You are a helpful assistant.",
          tools: ["shell", "read_file", "write_file"],
        };
        setActiveProfile(llmProfile);
        setMaxTokens(llmProfile.max_context_tokens);

        // Sync model name and context length from LLM Loader
        try {
          const llmStatus = await invoke<{ running: boolean; port: number; model: string; context_length?: number }>("get_llm_server_status");
          if (llmStatus.model) {
            llmProfile.model = llmStatus.model;
            setSelectedModel(llmStatus.model);
            setLoadedModelName(llmStatus.model);
          }
          if (llmStatus.context_length && llmStatus.context_length > 0) {
            llmProfile.max_context_tokens = llmStatus.context_length;
            setMaxTokens(llmStatus.context_length);
          }
        } catch { /* ignore */ }

        // Ensure we have an active session
        if (!activeSession) {
          loadSessions().then((s) => {
            if (s.length === 0) createNewSession(llmProfile);
          });
        }
      }
    } catch { /* ignore */ }
  };

  // ===== Git Status =====

  useEffect(() => {
    if (!visible || !rootPath) return;
    const fetchGit = async () => {
      try {
        const isRepo = await invoke<boolean>("git_is_repo", { path: rootPath });
        if (!isRepo) { setGitBranch(""); setGitDirtyCount(0); return; }
        const status = await invoke<{ branch: string; files: { path: string; status: string }[] }>("git_status", { path: rootPath });
        setGitBranch(status.branch || "");
        setGitDirtyCount(status.files?.length || 0);
      } catch { /* not a git repo */ }
    };
    fetchGit();
    const interval = setInterval(fetchGit, 15000);
    return () => clearInterval(interval);
  }, [visible, rootPath]);

  // ===== Initialization =====

  useEffect(() => {
    if (!visible) return;
    loadProfiles();
    loadSessions();
    // Re-load sessions when mobile WebSocket connects (transport may not be up on first mount)
    const onWorkspaceState = () => { loadProfiles(); loadSessions(); };
    window.addEventListener("mobile-workspace-state", onWorkspaceState);
    return () => window.removeEventListener("mobile-workspace-state", onWorkspaceState);
  }, [visible]);

  // Poll for LLM server connection every 5s if not connected
  useEffect(() => {
    if (connected) return;
    const interval = setInterval(() => {
      checkExistingLlmServer();
      // Also check common provider URLs
      tryConnectProviders();
    }, 5000);
    return () => clearInterval(interval);
  }, [connected]);

  const tryConnectProviders = async () => {
    const urls = [
      "http://localhost:8080/v1",  // llama.cpp default
      "http://localhost:1234/v1",  // LM Studio
      "http://localhost:11434/v1", // Ollama
    ];
    for (const url of urls) {
      try {
        const ok = await invoke<boolean>("ferrum_check_provider", { baseUrl: url });
        if (ok) {
          autoConnectToUrl(url);
          return;
        }
      } catch { /* ignore */ }
    }
  };

  const loadProfiles = async () => {
    try {
      const p = await invoke<Profile[]>("ferrum_get_profiles");
      setProfiles(p);
      // Don't override if already auto-connected to LLM Loader
      if (!connected) {
        const active = await invoke<Profile | null>("ferrum_get_active_profile");
        if (active) {
          setActiveProfile(active);
          setMaxTokens(active.max_context_tokens);
          setSelectedModel(active.model);
          connectToProvider(active);
        }
      }
    } catch { /* config not found */ }
  };

  const loadSessions = async (): Promise<Session[]> => {
    try {
      const s = await invoke<Session[]>("ferrum_list_sessions");
      setSessions(s);
      if (s.length > 0 && !activeSession) {
        const latest = await invoke<Session | null>("ferrum_get_latest_session");
        if (latest) loadSession(latest);
      }
      return s;
    } catch { return []; }
  };

  const connectToProvider = async (profile: Profile) => {
    try {
      const result = await invoke<{ models: string[]; connected: boolean }>("ferrum_list_provider_models", { baseUrl: profile.base_url });
      if (!result) return;
      setConnected(result.connected);
      setConnectedUrl(result.connected ? profile.base_url : "");
      setAvailableModels(result.models ?? []);
      if ((result.models ?? []).length > 0 && !selectedModel) setSelectedModel(result.models[0]);
    } catch { setConnected(false); }
  };

  const switchProfile = async (name: string) => {
    try {
      const profile = await invoke<Profile>("ferrum_set_active_profile", { name });
      setActiveProfile(profile);
      setMaxTokens(profile.max_context_tokens);
      setSelectedModel(profile.model);
      connectToProvider(profile);
    } catch { /* ignore */ }
  };

  // ===== Session Management =====

  const loadSession = async (session: Session) => {
    setActiveSession(session);
    try {
      const msgs = await invoke<ChatMessage[]>("ferrum_load_messages", { sessionId: session.id });
      setMessages(msgs);
      const tokenCount = await invoke<number>("ferrum_get_session_token_count", { sessionId: session.id });
      setUsedTokens(tokenCount);
    } catch { setMessages([]); }
    setViewMode("chat");
  };

  const createNewSession = async (profileOverride?: Profile) => {
    const prof = profileOverride || activeProfile;
    if (!prof) return;
    try {
      const session = await invoke<Session>("ferrum_create_session", {
        name: `Chat ${sessions.length + 1}`,
        profile: prof.name,
      });
      setSessions(prev => [session, ...prev]);
      setActiveSession(session);
      setMessages([]);
      setUsedTokens(0);
      setViewMode("chat");
    } catch { /* ignore */ }
  };

  const deleteSession = async (id: string) => {
    try {
      await invoke("ferrum_delete_session", { sessionId: id });
      setSessions(prev => prev.filter(s => s.id !== id));
      if (activeSession?.id === id) { setActiveSession(null); setMessages([]); }
    } catch { /* ignore */ }
  };

  const renameSession = async (id: string, name: string) => {
    try {
      await invoke("ferrum_rename_session", { sessionId: id, newName: name });
      setSessions(prev => prev.map(s => s.id === id ? { ...s, name } : s));
      if (activeSession?.id === id) setActiveSession(prev => prev ? { ...prev, name } : null);
    } catch { /* ignore */ }
  };

  const exportSession = async () => {
    if (!activeSession) return;
    try {
      const md = await invoke<string>("ferrum_export_session", { sessionId: activeSession.id });
      navigator.clipboard.writeText(md);
    } catch { /* ignore */ }
  };

  // ===== Chat =====

  const sendMessage = async () => {
    if (!input.trim() || streaming || !connected || !activeSession) {
      console.warn("sendMessage blocked:", { hasInput: !!input.trim(), streaming, connected, hasSession: !!activeSession });
      // On mobile, auto-create session if missing
      if (!activeSession && connected && input.trim()) {
        try {
          const s = await invoke<any>("ferrum_create_session", { name: "Chat 1", profile: activeProfile?.name || "default" });
          if (s) { setActiveSession(s); setSessions(prev => [s, ...prev]); }
        } catch {}
      }
      return;
    }

    const prof = activeProfile;
    const baseUrl = connectedUrl || prof?.base_url || "http://localhost:8080/v1";

    const userMsg: ChatMessage = {
      role: "user", content: input.trim(), token_count: 0,
      is_compacted: false, created_at: Math.floor(Date.now() / 1000),
    };

    await invoke("ferrum_save_message", { sessionId: activeSession.id, message: userMsg }).catch(() => {});

    const newMsgs = [...messages, userMsg];
    const assistantPlaceholder: ChatMessage = {
      role: "assistant", content: "", token_count: 0,
      is_compacted: false, created_at: Math.floor(Date.now() / 1000),
    };
    setMessages([...newMsgs, assistantPlaceholder]);
    setInput("");
    setStreaming(true);

    const streamId = `ferrum-${++streamIdRef.current}`;

    const apiMessages = newMsgs.map(m => ({ role: m.role, content: m.content }));
    let systemPrompt = prof?.system_prompt || "You are a helpful assistant.";
    if (includeFile && activeFileContent && activeFileName) {
      systemPrompt += `\n\nContext from ${activeFileName}:\n\n\`\`\`\n${activeFileContent}\n\`\`\``;
    }

    // ===== ShadowEditor auto-context injection (planengine.md §7.4) =====
    // Always inject game project context when a ShadowEditor project is open.
    // Falls through silently if no .shadow_project.toml found.
    try {
      const shadowCtx = await invoke<string>("shadow_get_ai_context", { rootPath: rootPath });
      if (shadowCtx) systemPrompt += `\n\n---\n${shadowCtx}`;
    } catch { /* not a ShadowEditor project — skip */ }

    // ===== ShadowEditor slash command handling (planengine.md §7.4) =====
    const trimmedInput = input.trim();
    const slashMatch = trimmedInput.match(/^(\/\w+)\s*(.*)/s);
    if (slashMatch) {
      const [, slashCmd, slashArg] = slashMatch;
      systemPrompt += buildShadowSlashPrompt(slashCmd, slashArg);
    }

    // RAG context injection — query indexed codebase for relevant context
    try {
      const ragResults = await invoke<{ file_path: string; content: string; score: number; line_start: number; line_end: number }[]>("rag_query_structured", { query: trimmedInput, topK: 3 });
      if (ragResults && ragResults.length > 0) {
        const ragContext = ragResults.map(r => `--- ${r.file_path} (lines ${r.line_start}-${r.line_end}) ---\n${r.content}`).join("\n\n");
        systemPrompt += `\n\nRelevant codebase context (from RAG index):\n\n${ragContext}`;
      }
    } catch { /* RAG not indexed yet, skip */ }

    apiMessages.unshift({ role: "system", content: systemPrompt });

    const unlistenStream = await listen<any>(`ai-chat-stream-${streamId}`, (e) => {
      const content = typeof e.payload === "string" ? e.payload : (e.payload.content || "");
      if (!content) return;
      setMessages(prev => {
        const n = [...prev];
        const last = n[n.length - 1];
        if (last && last.role === "assistant") {
          last.content += content;
        } else {
          // After tool results, create a new assistant message for the continuation
          n.push({
            role: "assistant", content, token_count: 0,
            is_compacted: false, created_at: Math.floor(Date.now() / 1000),
          });
        }
        return n;
      });
    });

    const unlistenThink = await listen<string>(`ai-chat-think-${streamId}`, (e) => {
      setMessages(prev => {
        const n = [...prev];
        const last = n[n.length - 1];
        if (last && last.role === "assistant") last.thinking = (last.thinking || "") + e.payload;
        return n;
      });
    });

    const unlistenToolCall = await listen<any>(`ai-tool-call-${streamId}`, (e) => {
      const toolArgs = e.payload.arguments || "";
      let argsPreview = "";
      try {
        const parsed = JSON.parse(toolArgs);
        argsPreview = Object.entries(parsed)
          .map(([k, v]) => `${k}: ${typeof v === "string" ? v.slice(0, 80) : JSON.stringify(v).slice(0, 80)}`)
          .join(", ");
      } catch { argsPreview = toolArgs.slice(0, 100); }

      setMessages(prev => {
        // If the last message is an assistant placeholder with content, keep it.
        // Add a new assistant placeholder for the next response after tools.
        return [...prev, {
          role: "tool" as const,
          content: `Running ${e.payload.name}(${argsPreview})...`,
          tool_name: e.payload.name,
          tool_calls: toolArgs,
          token_count: 0,
          is_compacted: false,
          created_at: Math.floor(Date.now() / 1000),
        }];
      });
    });

    const unlistenToolStream = await listen<any>(`ai-tool-stream-${streamId}`, (e) => {
      setMessages(prev => {
        const n = [...prev];
        // Find the LAST tool message with this tool name that starts with "Running"
        for (let i = n.length - 1; i >= 0; i--) {
          if (n[i].role === "tool" && n[i].tool_name === e.payload.tool && n[i].content.startsWith("Running")) {
            const existing = n[i].content;
            const newlineIdx = existing.indexOf("\n");
            const header = newlineIdx >= 0 ? existing.slice(0, newlineIdx) : existing;
            const prevOutput = newlineIdx >= 0 ? existing.slice(newlineIdx + 1) : "";
            // Append new chunk, keep last 2000 chars to avoid memory bloat
            const combined = (prevOutput + e.payload.chunk).slice(-2000);
            n[i] = { ...n[i], content: `${header}\n${combined}` };
            break;
          }
        }
        return n;
      });
    });

    const unlistenToolResult = await listen<any>(`ai-tool-result-${streamId}`, (e) => {
      setMessages(prev => {
        const n = [...prev];
        // Find the LAST tool message with this name that starts with "Running"
        for (let i = n.length - 1; i >= 0; i--) {
          if (n[i].role === "tool" && n[i].tool_name === e.payload.name && n[i].content.startsWith("Running")) {
            const status = e.payload.success ? "OK" : "FAILED";
            const duration = e.payload.duration_ms ? ` (${e.payload.duration_ms}ms)` : "";
            const resultPreview = (e.payload.result || "").slice(0, 800);
            n[i] = {
              ...n[i],
              content: `[${status}${duration}] ${e.payload.name}\n${resultPreview}`,
            };
            break;
          }
        }
        return n;
      });
    });

    // File change events — live diff/preview for file-modifying tools
    const unlistenFileChange = await listen<any>(`ai-file-change-${streamId}`, (e) => {
      const { tool, path, action, preview } = e.payload;
      const actionIcon = action === "created" ? "📄" : action === "patched" ? "✏️" : action === "deleted" ? "🗑️" : "💾";
      const shortPath = path.split("/").slice(-2).join("/");

      setMessages(prev => {
        const n = [...prev];
        // Find the last tool message for this tool
        for (let i = n.length - 1; i >= 0; i--) {
          if (n[i].role === "tool" && n[i].tool_name === tool) {
            // Append file change detail to the existing tool result
            const existing = n[i].content;
            const diffBlock = preview
              ? `\n${actionIcon} ${action}: ${shortPath}\n${preview}`
              : `\n${actionIcon} ${action}: ${shortPath}`;
            n[i] = { ...n[i], content: existing + diffBlock };
            break;
          }
        }
        return n;
      });
    });

    // Tool confirmation dialog for risky tools
    const unlistenConfirm = await listen<any>(`ai-tool-confirm-${streamId}`, (e) => {
      if (autoApproveSession || chatMode === "auto") {
        // Auto-approve for this session or in AUTO mode
        emit(`ai-tool-confirm-response-${streamId}`, { approved: true });
        return;
      }
      setToolConfirm({
        name: e.payload.name,
        arguments: e.payload.arguments || "{}",
        risk: e.payload.risk || "Medium",
        streamId,
      });
    });

    const unlistenStats = await listen<any>(`ai-token-stats-${streamId}`, (e) => {
      // Replace (not accumulate) — input_tokens is the full context size, output is the response
      const total = (e.payload.input_tokens || 0) + (e.payload.output_tokens || 0);
      setUsedTokens(total);
      if (e.payload.breakdown) {
        setTokenBreakdown(e.payload.breakdown);
      }
    });

    const unlistenDone = await listen(`ai-chat-done-${streamId}`, () => {
      setStreaming(false);
      unlistenStream(); unlistenThink(); unlistenToolCall(); unlistenToolStream();
      unlistenToolResult(); unlistenFileChange(); unlistenConfirm(); unlistenStats(); unlistenDone();

      // System notification (PC + mobile)
      try {
        if (document.hidden || !document.hasFocus()) {
          // PC notification via Web Notification API
          if (Notification.permission === "granted") {
            new Notification("ShadowAI", { body: "AI response complete", icon: "/icon.png", silent: false });
          } else if (Notification.permission !== "denied") {
            Notification.requestPermission().then(p => {
              if (p === "granted") new Notification("ShadowAI", { body: "AI response complete", icon: "/icon.png" });
            });
          }
        }
        // Mobile notification via bridge event (sent to connected phone)
        emit("ai-chat-complete-notify", { title: "ShadowAI", body: "AI response complete", timestamp: Date.now() });
      } catch {}

      setMessages(prev => {
        if (activeSession) {
          // Save new messages added during this stream (everything after the user's message)
          // The user message was already saved before streaming started.
          // Find where new messages start (after the last user message).
          let startIdx = 0;
          for (let i = prev.length - 1; i >= 0; i--) {
            if (prev[i].role === "user") { startIdx = i + 1; break; }
          }
          for (let i = startIdx; i < prev.length; i++) {
            const msg = prev[i];
            if (msg.content) {
              invoke("ferrum_save_message", {
                sessionId: activeSession.id,
                message: {
                  role: msg.role,
                  content: msg.content,
                  tool_name: msg.tool_name || null,
                  tool_calls: msg.tool_calls || null,
                  token_count: msg.token_count || 0,
                  is_compacted: false,
                  created_at: msg.created_at || Math.floor(Date.now() / 1000),
                },
              }).catch(() => {});
            }
          }
        }
        // Persist user+assistant pair to per-project shadow AI history (.shadoweditor/ai_history.jsonl)
        // Silently ignored if the current project is not a ShadowEditor project.
        if (rootPath) {
          let userMsg: ChatMessage | null = null;
          let assistantContent = "";
          for (let i = prev.length - 1; i >= 0; i--) {
            if (!assistantContent && prev[i].role === "assistant" && prev[i].content) {
              assistantContent = prev[i].content;
            } else if (prev[i].role === "user" && prev[i].content) {
              userMsg = prev[i];
              break;
            }
          }
          if (userMsg && assistantContent) {
            const now = Math.floor(Date.now() / 1000);
            invoke("shadow_ai_history_append", {
              projectPath: rootPath,
              entry: { role: "user", content: userMsg.content, ts: now },
            }).catch(() => {});
            invoke("shadow_ai_history_append", {
              projectPath: rootPath,
              entry: { role: "assistant", content: assistantContent, ts: now },
            }).catch(() => {});
          }
        }
        return prev;
      });

      checkCompaction();
    });

    try {
      // Resolve API key from profile's api_key_env setting
      let resolvedApiKey: string | null = null;
      if (prof?.api_key_env) {
        try {
          const envVal = await invoke<string | null>("env_get_var", { name: prof.api_key_env });
          if (envVal) resolvedApiKey = envVal;
        } catch {
          // env_get_var not available, try passing the value directly
          // (user may have put the key directly in api_key_env)
          if (prof.api_key_env.startsWith("sk-") || prof.api_key_env.length > 20) {
            resolvedApiKey = prof.api_key_env;
          }
        }
      }

      await invoke("ai_chat_with_tools", {
        streamId,
        messages: apiMessages,
        model: selectedModel || null,
        baseUrlOverride: baseUrl,
        apiKey: resolvedApiKey,
        temperature,
        maxTokens: 4096,
        toolsEnabled,
        chatMode,
        rootPath,
      });
    } catch (e) {
      setMessages(prev => {
        const n = [...prev];
        const last = n[n.length - 1];
        if (last) last.content = `Error: ${e}`;
        return n;
      });
      setStreaming(false);
      unlistenStream(); unlistenThink(); unlistenToolCall(); unlistenToolStream();
      unlistenToolResult(); unlistenFileChange(); unlistenConfirm(); unlistenStats(); unlistenDone();
    }
  };

  // ===== Compaction =====

  const checkCompaction = async () => {
    if (!activeSession || !activeProfile) return;
    try {
      const check = await invoke<CompactionCheck>("ferrum_check_compaction", {
        sessionId: activeSession.id,
        maxTokens: activeProfile.max_context_tokens,
        threshold: 0.8,
      });
      if (check.should_compact) await triggerCompaction();
      setUsedTokens(check.used_tokens);
    } catch { /* ignore */ }
  };

  const triggerCompaction = async () => {
    if (!activeSession || !activeProfile) return;
    const baseUrl = connectedUrl || activeProfile.base_url;
    try {
      const prompt = await invoke<string>("ferrum_get_compaction_prompt", { sessionId: activeSession.id });
      const streamId = `compact-${Date.now()}`;
      let summary = "";

      const unlistenStream = await listen<any>(`ai-chat-stream-${streamId}`, (e) => {
        const content = typeof e.payload === "string" ? e.payload : (e.payload.content || "");
        summary += content;
      });

      const done = new Promise<void>((resolve) => {
        listen(`ai-chat-done-${streamId}`, () => { resolve(); });
      });

      // Resolve API key for compaction call
      let compactApiKey: string | null = null;
      if (activeProfile.api_key_env) {
        try {
          const envVal = await invoke<string>("env_get_var", { name: activeProfile.api_key_env });
          if (envVal) compactApiKey = envVal;
        } catch {
          if (activeProfile.api_key_env.startsWith("sk-") || activeProfile.api_key_env.length > 20) {
            compactApiKey = activeProfile.api_key_env;
          }
        }
      }

      await invoke("ai_chat_with_tools", {
        streamId,
        messages: [{ role: "user", content: prompt }],
        model: selectedModel || null,
        baseUrlOverride: baseUrl,
        apiKey: compactApiKey,
        temperature: 0.3,
        maxTokens: 2048,
        toolsEnabled: false,
        chatMode: "plan",
        rootPath,
      });

      await done;
      unlistenStream();

      if (summary.trim()) {
        await invoke("ferrum_apply_compaction", { sessionId: activeSession.id, summary: summary.trim() });
        const msgs = await invoke<ChatMessage[]>("ferrum_load_messages", { sessionId: activeSession.id });
        setMessages(msgs);
        setCompactNotice(true);
        setTimeout(() => setCompactNotice(false), 3000);
      }
    } catch { /* ignore */ }
  };

  // ===== Profile Editor =====

  const saveEditedProfile = async () => {
    if (!editingProfile) return;
    try {
      await invoke("ferrum_add_profile", { profile: editingProfile });
      setEditingProfile(null);
      loadProfiles();
    } catch { /* ignore */ }
  };

  // ===== Effects =====

  useEffect(() => {
    if (messagesEndRef.current) messagesEndRef.current.scrollIntoView({ behavior: "smooth" });
  }, [messages]);


  // Load memories when memory panel is opened
  useEffect(() => {
    if (viewMode === "memory" && rootPath) {
      invoke<any[]>("ai_list_memories", { rootPath }).then(setMemories).catch(() => setMemories([]));
    }
  }, [viewMode, rootPath]);


  if (!visible) return null;

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", background: "var(--bg-primary)", color: "var(--text-primary)", fontFamily: "inherit", fontSize: 13 }}>

      {/* Header */}
      <div className="scroll-no-bar" style={{ padding: "8px 12px", background: "var(--bg-secondary)", borderBottom: "1px solid var(--border-color)", display: "flex", alignItems: "center", gap: 6, flexShrink: 0 }}>
        <span style={{ fontWeight: 700, fontSize: 13, color: "var(--accent)" }}>ShadowAI</span>

        {/* Chat Mode Pills */}
        <div style={{ display: "flex", gap: 2, marginLeft: 4 }}>
          {(["plan", "build", "auto"] as ChatMode[]).map(m => (
            <button key={m} onClick={() => setChatMode(m)}
              style={{
                padding: "2px 6px", fontSize: 9, fontWeight: 600, textTransform: "uppercase",
                border: "1px solid var(--border-color)", borderRadius: "var(--radius-sm)", cursor: "pointer",
                background: chatMode === m ? "var(--accent)" : "transparent",
                color: chatMode === m ? "#fff" : "var(--text-secondary)",
              }}>{m}</button>
          ))}
        </div>

        <span style={{ color: "var(--border-color)" }}>|</span>
        <span style={{ color: connected ? "var(--success)" : serverStopping ? "var(--warning)" : "var(--danger)", fontSize: 9 }}>&#9679;</span>
        <span style={{ fontSize: 11, color: "var(--text-secondary)", maxWidth: 120, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {connected
            ? loadedModelName || activeProfile?.name || "Connected"
            : serverStopping ? "Stopped" : "Disconnected"}
        </span>

        {/* Git status */}
        {gitBranch && (
          <>
            <span style={{ color: "var(--border-color)" }}>|</span>
            <span style={{ fontSize: 10, color: gitDirtyCount > 0 ? "var(--warning)" : "var(--text-muted)", display: "flex", alignItems: "center", gap: 3 }}>
              &#9741; {gitBranch}{gitDirtyCount > 0 && <span style={{ fontSize: 9 }}>+{gitDirtyCount}</span>}
            </span>
          </>
        )}

        <div style={{ minWidth: 8, flexShrink: 0 }} />
        <button className="fc-btn" style={{ flexShrink: 0 }} onClick={() => setViewMode(viewMode === "sessions" ? "chat" : "sessions")}>Sessions</button>
        <button className="fc-btn" style={{ flexShrink: 0 }} onClick={() => setViewMode(viewMode === "profiles" ? "chat" : "profiles")}>Profiles</button>
        <button className="fc-btn" style={{ flexShrink: 0 }} onClick={() => setViewMode(viewMode === "memory" ? "chat" : "memory")}>Memory</button>
        <button className="fc-btn" style={{ flexShrink: 0 }} onClick={() => setViewMode(viewMode === "settings" ? "chat" : "settings")}>Settings</button>
        <button className="fc-btn fc-btn-accent" style={{ flexShrink: 0 }} onClick={() => createNewSession()}>+ New</button>
        {onToggleFullscreen && <button className="fc-btn" style={{ flexShrink: 0 }} onClick={onToggleFullscreen}>{isFullscreen ? "Exit" : "Full"}</button>}
        {onPopout && <button className="fc-btn" style={{ flexShrink: 0 }} onClick={onPopout}>Pop</button>}
      </div>

      {/* Session Sidebar */}
      {viewMode === "sessions" && (
        <FerrumChatSessions
          sessions={sessions}
          activeSessionId={activeSession?.id ?? null}
          onLoadSession={loadSession}
          onDeleteSession={deleteSession}
        />
      )}

      {/* Profile Selector */}
      {viewMode === "profiles" && (
        <div style={{ background: "var(--bg-secondary)", borderBottom: "1px solid var(--border-color)", maxHeight: 400, overflow: "auto", flexShrink: 0 }}>
          <div style={{ padding: "8px 12px", fontSize: 10, color: "var(--text-secondary)", textTransform: "uppercase", fontWeight: 600, letterSpacing: 1 }}>Profiles</div>
          {profiles.map(p => (
            <div key={p.name} style={{
              padding: "8px 12px", cursor: "pointer", display: "flex", alignItems: "center", gap: 8,
              background: activeProfile?.name === p.name ? "var(--bg-hover)" : "transparent",
              borderLeft: activeProfile?.name === p.name ? "3px solid var(--accent)" : "3px solid transparent",
            }}>
              <div style={{ flex: 1 }} onClick={() => switchProfile(p.name)}>
                <div style={{ fontSize: 12, fontWeight: 600 }}>{p.name}</div>
                <div style={{ fontSize: 10, color: "var(--text-secondary)" }}>{p.provider} / {p.model} | {fmtTokens(p.max_context_tokens)} ctx</div>
              </div>
              <button className="fc-btn" style={{ fontSize: 9, padding: "2px 6px" }} onClick={() => setEditingProfile({ ...p })}>Edit</button>
            </div>
          ))}
          <div style={{ padding: "8px 12px" }}>
            <button className="fc-btn fc-btn-accent" onClick={() => setEditingProfile({
              name: "", provider: "openai", model: "", base_url: "http://localhost:8080/v1",
              api_key_env: "", max_context_tokens: 32768, system_prompt: "You are a helpful assistant.",
              tools: ["shell", "read_file", "write_file"],
            })}>+ Add Profile</button>
          </div>

          {editingProfile && (
            <div style={{ padding: "12px", borderTop: "1px solid var(--border-color)", background: "var(--bg-primary)" }}>
              <div style={{ fontSize: 11, fontWeight: 600, marginBottom: 8, color: "var(--accent)" }}>
                {editingProfile.name ? `Edit: ${editingProfile.name}` : "New Profile"}
              </div>
              {([
                ["name", "Name"], ["provider", "Provider"], ["model", "Model"],
                ["base_url", "Base URL"], ["api_key_env", "API Key Env Var"], ["system_prompt", "System Prompt"],
              ] as const).map(([key, label]) => (
                <div key={key} style={{ marginBottom: 6 }}>
                  <label style={{ fontSize: 10, color: "var(--text-secondary)", display: "block", marginBottom: 2 }}>{label}</label>
                  <input type="text" value={String(editingProfile[key] ?? "")}
                    onChange={e => setEditingProfile(prev => prev ? { ...prev, [key]: e.target.value } : null)}
                    style={{ width: "100%", background: "var(--bg-secondary)", border: "1px solid var(--border-color)", borderRadius: "var(--radius-sm)", padding: "4px 8px", color: "var(--text-primary)", fontSize: 11, outline: "none" }} />
                </div>
              ))}
              <div style={{ marginBottom: 6 }}>
                <label style={{ fontSize: 10, color: "var(--text-secondary)", display: "block", marginBottom: 2 }}>Max Context Tokens</label>
                <input type="number" value={editingProfile.max_context_tokens}
                  onChange={e => setEditingProfile(prev => prev ? { ...prev, max_context_tokens: Number(e.target.value) } : null)}
                  style={{ width: "100%", background: "var(--bg-secondary)", border: "1px solid var(--border-color)", borderRadius: "var(--radius-sm)", padding: "4px 8px", color: "var(--text-primary)", fontSize: 11, outline: "none" }} />
              </div>
              <div style={{ display: "flex", gap: 8, marginTop: 8 }}>
                <button className="fc-btn fc-btn-accent" onClick={saveEditedProfile}>Save</button>
                <button className="fc-btn" onClick={() => setEditingProfile(null)}>Cancel</button>
              </div>
            </div>
          )}
        </div>
      )}

      {/* Settings Panel */}
      {viewMode === "settings" && (
        <div style={{ background: "var(--bg-secondary)", borderBottom: "1px solid var(--border-color)", padding: "12px", flexShrink: 0 }}>
          <div style={{ fontSize: 10, color: "var(--text-secondary)", textTransform: "uppercase", fontWeight: 600, letterSpacing: 1, marginBottom: 8 }}>Chat Settings</div>
          <div style={{ marginBottom: 8 }}>
            <label style={{ fontSize: 10, color: "var(--text-secondary)" }}>Model</label>
            <select value={selectedModel} onChange={e => setSelectedModel(e.target.value)}
              style={{ width: "100%", background: "var(--bg-primary)", border: "1px solid var(--border-color)", borderRadius: "var(--radius-sm)", padding: "4px 8px", color: "var(--text-primary)", fontSize: 11, marginTop: 2 }}>
              {availableModels.map(m => <option key={m} value={m}>{m}</option>)}
              {activeProfile && !availableModels.includes(activeProfile.model) && (
                <option value={activeProfile.model}>{activeProfile.model}</option>
              )}
            </select>
          </div>
          <div style={{ marginBottom: 8, display: "flex", alignItems: "center", gap: 8 }}>
            <label style={{ fontSize: 10, color: "var(--text-secondary)" }}>Temperature</label>
            <input type="range" min="0" max="200" value={temperature * 100}
              onChange={e => setTemperature(Number(e.target.value) / 100)} style={{ flex: 1 }} />
            <span style={{ fontSize: 10, color: "var(--text-primary)", fontVariantNumeric: "tabular-nums" }}>{temperature.toFixed(2)}</span>
          </div>
          <div style={{ display: "flex", gap: 12 }}>
            <label style={{ fontSize: 10, color: "var(--text-secondary)", display: "flex", alignItems: "center", gap: 4 }}>
              <input type="checkbox" checked={toolsEnabled} onChange={e => setToolsEnabled(e.target.checked)} /> Tools
            </label>
          </div>
          {connectedUrl && (
            <div style={{ fontSize: 10, color: "var(--text-muted)", marginTop: 8 }}>Connected: {connectedUrl}</div>
          )}
          <div style={{ marginTop: 8 }}>
            <button className="fc-btn" onClick={exportSession}>Export Session as Markdown</button>
          </div>
        </div>
      )}

      {viewMode === "memory" && (
        <div style={{ background: "var(--bg-secondary)", borderBottom: "1px solid var(--border-color)", padding: "12px", flexShrink: 0, maxHeight: "60vh", overflow: "auto" }}>
          <div style={{ fontSize: 10, color: "var(--text-secondary)", textTransform: "uppercase", fontWeight: 600, letterSpacing: 1, marginBottom: 8 }}>Stored Memories</div>
          <input placeholder="Filter memories..." value={memoryFilter} onChange={e => setMemoryFilter(e.target.value)}
            style={{ width: "100%", background: "var(--bg-primary)", border: "1px solid var(--border-color)", borderRadius: "var(--radius-sm)", padding: "4px 8px", color: "var(--text-primary)", fontSize: 11, marginBottom: 8 }} />
          {memories.length === 0 ? (
            <div style={{ fontSize: 11, color: "var(--text-muted)", padding: "12px 0", textAlign: "center" }}>No memories stored yet.</div>
          ) : (
            memories
              .filter(m => {
                if (!memoryFilter) return true;
                const q = memoryFilter.toLowerCase();
                return (m.key || "").toLowerCase().includes(q) || (m.value || "").toLowerCase().includes(q) || (m.category || "").toLowerCase().includes(q);
              })
              .map((m, i) => (
                <div key={i} style={{ padding: "8px", margin: "4px 0", background: "var(--bg-primary)", borderRadius: "var(--radius-sm)", border: "1px solid var(--border-color)" }}>
                  <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 4 }}>
                    <span style={{ fontSize: 11, fontWeight: 600, color: "var(--text-primary)" }}>{m.key}</span>
                    <div style={{ display: "flex", gap: 6, alignItems: "center" }}>
                      <span style={{ fontSize: 9, color: "var(--text-muted)", background: "var(--bg-hover)", padding: "1px 6px", borderRadius: 8 }}>{m.category || "fact"}</span>
                      <button onClick={() => {
                        if (rootPath && m._filename) {
                          invoke("ai_delete_memory", { rootPath, filename: m._filename }).then(() => {
                            setMemories(prev => prev.filter((_, j) => j !== i));
                          });
                        }
                      }} style={{ background: "none", border: "none", color: "var(--danger)", cursor: "pointer", fontSize: 10, padding: "0 2px" }} title="Delete">&times;</button>
                    </div>
                  </div>
                  <div style={{ fontSize: 10, color: "var(--text-secondary)", whiteSpace: "pre-wrap", maxHeight: 100, overflow: "auto" }}>{m.value}</div>
                  {m.timestamp > 0 && (
                    <div style={{ fontSize: 9, color: "var(--text-muted)", marginTop: 4 }}>{timeAgo(m.timestamp)}</div>
                  )}
                </div>
              ))
          )}
        </div>
      )}

      {/* Loading Bar */}
      <LoadingBar active={streaming} />

      {/* Chat Viewport */}
      <div style={{ flex: 1, overflow: "auto", padding: "8px 12px" }}>
        {messages.length === 0 && !streaming && (
          <div style={{ textAlign: "center", padding: "40px 20px", color: "var(--text-muted)" }}>
            <div style={{ fontSize: 24, marginBottom: 8 }}>&#9881;</div>
            <div style={{ fontSize: 13, fontWeight: 600, color: "var(--text-secondary)" }}>FerrumChat</div>
            <div style={{ fontSize: 11, marginTop: 4 }}>
              {connected
                ? `Ready${loadedModelName ? ` — ${loadedModelName}` : ""}. Type a message to start.`
                : serverStopping
                  ? "LLM server stopped. Restart from LLM Loader."
                  : "Waiting for LLM server... Start one from LLM Loader."}
            </div>
            {connected && !activeSession && (
              <button className="fc-btn fc-btn-accent" style={{ marginTop: 12 }} onClick={() => createNewSession()}>Create Session</button>
            )}
          </div>
        )}
        {messages.map((msg, i) => (
          <MessageBubble key={i} msg={msg}
            isStreaming={streaming && i === messages.length - 1 && msg.role === "assistant"}
            onRewind={msg.role === "user" ? () => {
              setMessages(messages.slice(0, i));
              setInput(msg.content);
            } : undefined}
            onToggleThinking={() => setMessages(prev => {
              const n = [...prev];
              if (n[i]) n[i] = { ...n[i], showThinking: !n[i].showThinking };
              return n;
            })} />
        ))}
        {compactNotice && (
          <div style={{ textAlign: "center", padding: "6px", color: "var(--text-muted)", fontStyle: "italic", fontSize: 11 }}>
            &#8635; Context compacted
          </div>
        )}
        <div ref={messagesEndRef} />
      </div>

      {/* Tool Confirmation Dialog */}
      {toolConfirm && (
        <div style={{
          padding: "10px 14px", background: "var(--bg-tertiary)", borderTop: "1px solid var(--border-color)",
          borderBottom: "1px solid var(--border-color)", flexShrink: 0,
        }}>
          <div style={{ fontSize: 12, fontWeight: 600, marginBottom: 6, color: toolConfirm.risk === "High" ? "#ff6b6b" : "#ffa726" }}>
            {toolConfirm.risk === "High" ? "\u26A0\uFE0F" : "\u2139\uFE0F"} Confirm: {toolConfirm.name} ({toolConfirm.risk} Risk)
          </div>
          <pre style={{ fontSize: 11, margin: "0 0 8px", padding: 6, background: "var(--bg-primary)", borderRadius: 4, maxHeight: 100, overflow: "auto", whiteSpace: "pre-wrap" }}>
            {(() => { try { return JSON.stringify(JSON.parse(toolConfirm.arguments), null, 2); } catch { return toolConfirm.arguments; } })()}
          </pre>
          <div style={{ display: "flex", gap: 6 }}>
            <button onClick={() => {
              emit(`ai-tool-confirm-response-${toolConfirm.streamId}`, { approved: true });
              setToolConfirm(null);
            }} style={{ padding: "4px 12px", background: "#4caf50", color: "#fff", border: "none", borderRadius: 4, cursor: "pointer", fontSize: 11 }}>
              Execute
            </button>
            <button onClick={() => {
              setAutoApproveSession(true);
              emit(`ai-tool-confirm-response-${toolConfirm.streamId}`, { approved: true });
              setToolConfirm(null);
            }} style={{ padding: "4px 12px", background: "#2196f3", color: "#fff", border: "none", borderRadius: 4, cursor: "pointer", fontSize: 11 }}>
              Auto-Yes (session)
            </button>
            <button onClick={() => {
              emit(`ai-tool-confirm-response-${toolConfirm.streamId}`, { approved: false });
              setToolConfirm(null);
            }} style={{ padding: "4px 12px", background: "#f44336", color: "#fff", border: "none", borderRadius: 4, cursor: "pointer", fontSize: 11 }}>
              Deny
            </button>
          </div>
        </div>
      )}

      {/* Token Bar */}
      <TokenBar used={usedTokens} max={maxTokens} breakdown={tokenBreakdown} />

      {/* Input Area */}
      <FerrumChatInput
        visible={visible}
        input={input}
        setInput={setInput}
        streaming={streaming}
        connected={connected}
        activeSession={activeSession}
        includeFile={includeFile}
        setIncludeFile={setIncludeFile}
        toolsEnabled={toolsEnabled}
        setToolsEnabled={setToolsEnabled}
        activeFileName={activeFileName}
        rootPath={rootPath}
        onSend={sendMessage}
        onRenameSession={renameSession}
      />

      <style>{`
        .fc-btn {
          background: var(--bg-hover); border: 1px solid var(--border-color); border-radius: var(--radius-sm);
          padding: 4px 10px; color: var(--text-primary); cursor: pointer; font-size: 11px;
          font-family: inherit; transition: background var(--transition-fast);
        }
        .fc-btn:hover { background: var(--bg-active); }
        .fc-btn-accent { background: var(--accent); border-color: var(--accent); color: #fff; }
        .fc-btn-accent:hover { background: var(--accent-hover); }
        .fc-cursor {
          display: inline-block; width: 2px; height: 14px; background: var(--accent);
          animation: fc-blink 1s step-end infinite; vertical-align: text-bottom;
        }
        @keyframes fc-blink { from, to { opacity: 1; } 50% { opacity: 0; } }
        .fc-loading-bar { animation: fc-slide 1.5s ease-in-out infinite; }
        @keyframes fc-slide { 0% { transform: translateX(-100%); } 100% { transform: translateX(350%); } }
      `}</style>
    </div>
  );
}

export default memo(FerrumChat);
