import { useEffect, useState, useRef, useCallback } from "react";
import MonacoEditor, { DiffEditor as MonacoDiffEditor, type OnMount, useMonaco } from "@monaco-editor/react";
import { invoke } from "@tauri-apps/api/core";
import { listen, emit } from "@tauri-apps/api/event";
import type { editor, IDisposable, languages, MarkerSeverity } from "monaco-editor";
import { createPortal } from "react-dom";
import { useAiCompletion } from "../hooks/useAiCompletion";
import { useGhostDiff } from "../hooks/useGhostDiff";
import AiActionPopup from "./AiActionPopup";

export interface OpenFile {
  path: string;
  name: string;
  content: string;
  modified: boolean;
  size?: number;
}

export interface CursorInfo {
  line: number;
  column: number;
  selected: number;
}

export interface DiagnosticCounts {
  errors: number;
  warnings: number;
  infos: number;
}

export interface DiagnosticItem {
  file: string;
  line: number;
  column: number;
  endLine: number;
  endColumn: number;
  severity: "error" | "warning" | "info";
  message: string;
}

interface EditorProps {
  files: OpenFile[];
  activeFileIndex: number;
  onActiveFileChange: (index: number) => void;
  onFileClose: (index: number) => void;
  onFileContentChange: (index: number, content: string) => void;
  onFileReorder: (fromIndex: number, toIndex: number) => void;
  aiCompletionEnabled?: boolean;
  rootPath?: string;
  minimapEnabled?: boolean;
  fontSize?: number;
  tabSize?: number;
  onMinimapToggle?: (enabled: boolean) => void;
  onCursorChange?: (info: CursorInfo) => void;
  onDiagnosticsChange?: (counts: DiagnosticCounts) => void;
  onDiagnosticItems?: (items: DiagnosticItem[]) => void;
  // Diff editor support
  diffMode?: boolean;
  originalContent?: string;
  // Split pane support
  splitDirection?: 'horizontal' | 'vertical' | null;
  splitFilePath?: string;
  splitFileContent?: string;
  onSplitFileContentChange?: (content: string) => void;
}

interface LspDiagnosticEvent {
  file: string;
  diagnostics: Array<{
    line: number;
    col: number;
    end_line: number;
    end_col: number;
    severity: string;
    message: string;
    source: string | null;
  }>;
}

interface LspCompletionItem {
  label: string;
  kind: string;
  detail: string | null;
  insert_text: string | null;
  documentation: string | null;
}

interface LspHoverResult {
  contents: string;
}

interface LspLocation {
  file: string;
  line: number;
  col: number;
}

interface CollaboratorIdentity {
  id: string;
  name: string;
  color: string;
}

interface CollaboratorPresence {
  collaborator_id: string;
  name: string;
  color: string;
  file_path: string;
  line: number;
  column: number;
  selection_start?: number | null;
  selection_end?: number | null;
  voice_active: boolean;
  video_active: boolean;
  last_seen: number;
}

interface ReviewComment {
  id: string;
  file_path: string;
  line: number;
  column: number;
  author: string;
  body: string;
  created_at: number;
  resolved: boolean;
  resolved_by?: string | null;
}

interface ReviewApproval {
  reviewer: string;
  status: string;
  note: string;
  updated_at: number;
}

interface CollaborationSnapshot {
  file_path: string;
  content: string;
  version: number;
  source_collaborator_id?: string | null;
  presences: CollaboratorPresence[];
  comments: ReviewComment[];
  approvals: ReviewApproval[];
}

interface CallSignalEvent {
  room_id: string;
  file_path: string;
  sender_id: string;
  sender_name: string;
  target_id?: string | null;
  signal_type: string;
  payload: Record<string, unknown>;
  timestamp: number;
}

function collaboratorIdentity(): CollaboratorIdentity {
  const storageKey = "shadowide-collaborator";
  try {
    const raw = localStorage.getItem(storageKey);
    if (raw) {
      const parsed = JSON.parse(raw) as CollaboratorIdentity;
      if (parsed.id && parsed.name && parsed.color) return parsed;
    }
  } catch { /* ignore */ }
  const palette = ["#7dd3fc", "#fca5a5", "#86efac", "#fcd34d", "#c4b5fd", "#fdba74"];
  const identity = {
    id: `collab-${Math.random().toString(36).slice(2, 10)}`,
    name: localStorage.getItem("shadowide-collaborator-name") || `Shadow ${Math.floor(Math.random() * 900 + 100)}`,
    color: palette[Math.floor(Math.random() * palette.length)],
  };
  try {
    localStorage.setItem(storageKey, JSON.stringify(identity));
  } catch { /* ignore */ }
  return identity;
}

function cssId(value: string): string {
  return value.replace(/[^a-zA-Z0-9_-]/g, "-");
}

const MONACO_THEME_NAME = "shadowide-theme";

function cssVar(name: string, fallback: string): string {
  if (typeof window === "undefined") return fallback;
  const value = window.getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  return value || fallback;
}

function themeRuleColor(color: string): string {
  return color.replace(/^#/, "");
}

function defineShadowIdeTheme(monacoApi: NonNullable<ReturnType<typeof useMonaco>>) {
  const bgPrimary = cssVar("--bg-primary", "#0a0e17");
  const bgSecondary = cssVar("--bg-secondary", "#111827");
  const bgTertiary = cssVar("--bg-tertiary", "#0f172a");
  const bgHover = cssVar("--bg-hover", "#1e293b");
  const textPrimary = cssVar("--text-primary", "#e2e8f0");
  const textSecondary = cssVar("--text-secondary", "#94a3b8");
  const textMuted = cssVar("--text-muted", "#475569");
  const accent = cssVar("--accent", "#6366f1");
  const accentHover = cssVar("--accent-hover", "#818cf8");

  monacoApi.editor.defineTheme(MONACO_THEME_NAME, {
    base: "vs-dark",
    inherit: true,
    rules: [
      { token: "comment", foreground: themeRuleColor(textMuted), fontStyle: "italic" },
      { token: "string", foreground: "86efac" },
      { token: "number", foreground: "fbbf24" },
      { token: "regexp", foreground: "f59e0b" },
      { token: "keyword", foreground: themeRuleColor(accentHover), fontStyle: "bold" },
      { token: "operator", foreground: themeRuleColor(textSecondary) },
      { token: "delimiter", foreground: themeRuleColor(textSecondary) },
      { token: "delimiter.bracket", foreground: themeRuleColor(textPrimary) },
      { token: "type", foreground: "7dd3fc" },
      { token: "type.identifier", foreground: "7dd3fc" },
      { token: "class", foreground: "7dd3fc" },
      { token: "struct", foreground: "7dd3fc" },
      { token: "interface", foreground: "7dd3fc" },
      { token: "enum", foreground: "c4b5fd" },
      { token: "function", foreground: "f8fafc" },
      { token: "method", foreground: "f8fafc" },
      { token: "property", foreground: "fda4af" },
      { token: "parameter", foreground: "f9a8d4" },
      { token: "namespace", foreground: "60a5fa" },
      { token: "variable", foreground: themeRuleColor(textPrimary) },
    ],
    colors: {
      "editor.background": bgPrimary,
      "editor.foreground": textPrimary,
      "editorLineNumber.foreground": textMuted,
      "editorLineNumber.activeForeground": textPrimary,
      "editorCursor.foreground": accentHover,
      "editor.selectionBackground": `${accent}55`,
      "editor.selectionHighlightBackground": `${accent}22`,
      "editor.inactiveSelectionBackground": `${bgHover}cc`,
      "editor.wordHighlightBackground": `${accent}18`,
      "editor.wordHighlightStrongBackground": `${accent}28`,
      "editor.lineHighlightBackground": `${bgSecondary}cc`,
      "editor.lineHighlightBorder": bgHover,
      "editorGutter.background": bgPrimary,
      "editorIndentGuide.background1": `${bgHover}aa`,
      "editorIndentGuide.activeBackground1": `${accent}66`,
      "editorWhitespace.foreground": `${textMuted}66`,
      "editorBracketMatch.background": `${accent}18`,
      "editorBracketMatch.border": accent,
      "editorOverviewRuler.border": bgTertiary,
      "editorError.foreground": "#f87171",
      "editorWarning.foreground": "#fbbf24",
      "editorInfo.foreground": "#34d399",
      "editorHint.foreground": "#7dd3fc",
      "editorError.border": "#f87171",
      "editorWarning.border": "#fbbf24",
      "editorInfo.border": "#34d399",
      "editorHint.border": "#7dd3fc",
      "editorGutter.modifiedBackground": accentHover,
      "editorGutter.addedBackground": "#34d399",
      "editorGutter.deletedBackground": "#f87171",
      "scrollbarSlider.background": `${textMuted}33`,
      "scrollbarSlider.hoverBackground": `${textSecondary}55`,
      "scrollbarSlider.activeBackground": `${textSecondary}88`,
      "minimap.background": bgPrimary,
      "minimap.selectionHighlight": `${accent}44`,
      "minimap.errorHighlight": "#f87171",
      "minimap.warningHighlight": "#fbbf24",
      "minimap.findMatchHighlight": `${accentHover}88`,
      "peekView.border": bgHover,
      "peekViewEditor.background": bgPrimary,
      "peekViewResult.background": bgSecondary,
      "diffEditor.insertedTextBackground": "#34d39922",
      "diffEditor.removedTextBackground": "#f8717122",
    },
  });
}

const LSP_EXTENSIONS = new Set([
  "rs", "ts", "tsx", "js", "jsx", "mjs", "cjs",
  "py", "pyi", "c", "h", "cpp", "cxx", "cc", "hpp", "hxx",
  "go", "zig", "lua",
]);

function serverKeyForExt(ext: string): string | null {
  switch (ext) {
    case "rs": return "rust";
    case "ts": case "tsx": case "js": case "jsx": case "mjs": case "cjs": return "typescript";
    case "py": case "pyi": return "python";
    case "c": case "h": case "cpp": case "cxx": case "cc": case "hpp": case "hxx": return "cpp";
    case "go": return "go";
    case "zig": return "zig";
    case "lua": return "lua";
    default: return null;
  }
}

function completionKindToMonaco(kind: string): number {
  const map: Record<string, number> = {
    method: 0, function: 1, constructor: 2, field: 3, variable: 4,
    class: 5, interface: 7, module: 8, property: 9, unit: 10,
    value: 11, enum: 12, keyword: 13, snippet: 14, color: 15,
    file: 16, reference: 17, folder: 18, enum_member: 19, constant: 20,
    struct: 21, event: 22, operator: 23, type_parameter: 24, text: 25,
  };
  return map[kind] ?? 25;
}

export function getLanguageFromFilename(filename: string): string {
  const lower = filename.toLowerCase();
  const ext = lower.split(".").pop() || "";

  // Handle special filenames
  const nameMap: Record<string, string> = {
    dockerfile: "dockerfile",
    makefile: "makefile",
    gnumakefile: "makefile",
    cmakelists: "cmake",
    gemfile: "ruby",
    rakefile: "ruby",
    vagrantfile: "ruby",
    jenkinsfile: "groovy",
  };
  const baseName = lower.split("/").pop()?.split(".")[0] || "";
  if (nameMap[baseName]) return nameMap[baseName];

  const langMap: Record<string, string> = {
    // Web
    ts: "typescript", tsx: "typescript", js: "javascript", jsx: "javascript",
    mjs: "javascript", cjs: "javascript", mts: "typescript", cts: "typescript",
    html: "html", htm: "html", xhtml: "html",
    css: "css", scss: "scss", sass: "scss", less: "less", styl: "stylus",
    json: "json", jsonc: "json", json5: "json",
    vue: "html", svelte: "html", astro: "html",
    // Systems
    rs: "rust", c: "c", h: "c", cpp: "cpp", cc: "cpp", cxx: "cpp",
    hpp: "cpp", hxx: "cpp", hh: "cpp", ino: "cpp",
    go: "go", zig: "zig", nim: "nim", v: "v",
    // JVM
    java: "java", kt: "kotlin", kts: "kotlin", scala: "scala",
    groovy: "groovy", gradle: "groovy", clj: "clojure", cljs: "clojure",
    // .NET
    cs: "csharp", fs: "fsharp", fsx: "fsharp", vb: "vb",
    xaml: "xml", csproj: "xml", fsproj: "xml", sln: "xml",
    // Scripting
    py: "python", pyw: "python", pyi: "python",
    rb: "ruby", erb: "ruby",
    php: "php", phtml: "php",
    lua: "lua", pl: "perl", pm: "perl",
    r: "r", R: "r",
    jl: "julia",
    ex: "elixir", exs: "elixir",
    erl: "erlang", hrl: "erlang",
    hs: "haskell", lhs: "haskell",
    ml: "fsharp", mli: "fsharp", ocaml: "fsharp",
    // Shell
    sh: "shell", bash: "shell", zsh: "shell", fish: "shell",
    ps1: "powershell", psm1: "powershell", psd1: "powershell",
    bat: "bat", cmd: "bat",
    // Data/Config
    xml: "xml", svg: "xml", xsl: "xml", xsd: "xml", plist: "xml",
    yaml: "yaml", yml: "yaml",
    toml: "ini", ini: "ini", cfg: "ini", conf: "ini", properties: "ini",
    env: "dotenv",
    // Markup/Docs
    md: "markdown", mdx: "markdown", rmd: "markdown",
    tex: "latex", sty: "latex", cls: "latex",
    rst: "restructuredtext",
    // Database
    sql: "sql", mysql: "sql", pgsql: "sql", plsql: "sql",
    prisma: "prisma", graphql: "graphql", gql: "graphql",
    // DevOps/Infra
    tf: "hcl", hcl: "hcl", tfvars: "hcl",
    proto: "protobuf",
    // Mobile
    swift: "swift", m: "objective-c", mm: "objective-c",
    dart: "dart",
    // Shader/GPU
    glsl: "glsl", vert: "glsl", frag: "glsl", comp: "glsl",
    hlsl: "hlsl", wgsl: "wgsl", metal: "cpp",
    // Other
    sol: "sol", move: "move",
    asm: "asm", s: "asm", nasm: "asm",
    d: "d", pas: "pascal", pp: "pascal",
    lisp: "lisp", el: "lisp", scm: "scheme",
    coffee: "coffeescript",
    diff: "diff", patch: "diff",
    log: "log",
    csv: "csv", tsv: "csv",
    lock: "json",
    dockerfile: "dockerfile",
  };
  return langMap[ext] || "plaintext";
}

const isMobileDevice = /iPhone|iPad|iPod|Android/i.test(navigator.userAgent);

export default function Editor({
  files,
  activeFileIndex,
  onActiveFileChange,
  onFileClose,
  onFileContentChange,
  onFileReorder,
  aiCompletionEnabled = false,
  rootPath,
  minimapEnabled,
  fontSize,
  tabSize,
  onMinimapToggle,
  onCursorChange,
  onDiagnosticsChange,
  onDiagnosticItems,
  diffMode = false,
  originalContent,
  splitDirection = null,
  splitFilePath,
  splitFileContent,
  onSplitFileContentChange,
}: EditorProps) {
  const editorRef = useRef<editor.IStandaloneCodeEditor | null>(null);
  const [saving, setSaving] = useState(false);
  const [breakpoints, setBreakpoints] = useState<Set<number>>(new Set());
  const breakpointDecorationsRef = useRef<string[]>([]);
  const [dragIndex, setDragIndex] = useState<number | null>(null);
  const [dragOverIndex, setDragOverIndex] = useState<number | null>(null);
  const [gitChanges, setGitChanges] = useState<Map<string, string>>(new Map());
  const completionDisposable = useRef<IDisposable | null>(null);
  const fimDisposable = useRef<IDisposable | null>(null);
  const fimDebounceTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const [ghostTextEnabled, setGhostTextEnabled] = useState(false);
  const [blameEnabled, setBlameEnabled] = useState(false);
  const blameDecorationsRef = useRef<string[]>([]);
  const [aiPopup, setAiPopup] = useState<{ selectedText: string; language: string; position: { x: number; y: number }; multiCursor?: boolean; selections?: import("monaco-editor").Selection[] } | null>(null);
  const lspDisposables = useRef<IDisposable[]>([]);
  const lspStarted = useRef<Set<string>>(new Set());
  const lspOpenFiles = useRef<Set<string>>(new Set());
  const fileVersions = useRef<Map<string, number>>(new Map());
  const collaboratorDecorationsRef = useRef<string[]>([]);
  const reviewDecorationsRef = useRef<string[]>([]);
  const diagnosticDecorationsRef = useRef<string[]>([]);
  const applyingCollabUpdateRef = useRef(false);
  const collabVersionRef = useRef(0);
  const localIdentityRef = useRef<CollaboratorIdentity>(collaboratorIdentity());
  const activeFileRef = useRef<OpenFile | undefined>(undefined);
  const activeFileIndexRef = useRef(0);
  const onFileContentChangeRef = useRef(onFileContentChange);
  const localVideoRef = useRef<HTMLVideoElement | null>(null);
  const remoteVideoRef = useRef<HTMLVideoElement | null>(null);
  const localStreamRef = useRef<MediaStream | null>(null);
  const peerConnectionsRef = useRef<Map<string, RTCPeerConnection>>(new Map());
  const remoteStreamsRef = useRef<Map<string, MediaStream>>(new Map());
  const callStateRef = useRef<{ active: boolean; video: boolean }>({ active: false, video: false });
  const monaco = useMonaco();
  const { registerProvider } = useAiCompletion("");
  const [collabSnapshot, setCollabSnapshot] = useState<CollaborationSnapshot | null>(null);
  const [reviewDraft, setReviewDraft] = useState("");
  const [reviewNote, setReviewNote] = useState("");
  const [callState, setCallState] = useState<{ active: boolean; video: boolean; status: string }>({
    active: false,
    video: false,
    status: "Idle",
  });
  const [remoteStreamVersion, setRemoteStreamVersion] = useState(0);

  const activeFile = files[activeFileIndex];
  const collabSidebarRoot = typeof document !== "undefined" ? document.getElementById("collab-sidebar-root") : null;
  activeFileRef.current = activeFile;
  activeFileIndexRef.current = activeFileIndex;
  onFileContentChangeRef.current = onFileContentChange;
  callStateRef.current = { active: callState.active, video: callState.video };

  // Ghost diff overlay for AI-proposed changes
  const ghostDiff = useGhostDiff(
    editorRef.current,
    activeFile?.path,
    (path, newContent) => {
      // Accept: update file content with the proposed changes
      const idx = files.findIndex((f) => f.path === path);
      if (idx >= 0) onFileContentChange(idx, newContent);
    }
  );

  // Large file threshold: 1MB
  const isLargeFile = (activeFile?.size ?? 0) > 1024 * 1024;

  // Inject ghost diff CSS for Monaco decorations
  useEffect(() => {
    const styleId = "ghost-diff-styles";
    if (document.getElementById(styleId)) return;
    const style = document.createElement("style");
    style.id = styleId;
    style.textContent = `
      .ghost-diff-addition {
        background: rgba(63, 185, 80, 0.15) !important;
        border-left: 3px solid #3fb950 !important;
      }
      .ghost-diff-glyph-add {
        background: #3fb950;
        border-radius: 2px;
        margin-left: 3px;
        width: 4px !important;
      }
      .ghost-diff-glyph-del {
        background: #f85149;
        border-radius: 2px;
        margin-left: 3px;
        width: 4px !important;
      }
      .ghost-diff-deletion-text {
        color: #f85149;
        text-decoration: line-through;
        opacity: 0.6;
        font-style: italic;
      }
    `;
    document.head.appendChild(style);
  }, []);

  // Inject blame annotation CSS
  useEffect(() => {
    const styleId = "blame-annotation-styles";
    if (document.getElementById(styleId)) return;
    const style = document.createElement("style");
    style.id = styleId;
    style.textContent = `.blame-annotation { color: #888; font-size: 11px; font-style: italic; margin-left: 16px; }`;
    document.head.appendChild(style);
  }, []);

  useEffect(() => {
    const styleId = "collab-editor-styles";
    if (document.getElementById(styleId)) return;
    const style = document.createElement("style");
    style.id = styleId;
    style.textContent = `
      .collab-review-line {
        background: rgba(251, 191, 36, 0.08);
        border-left: 2px solid #fbbf24;
      }
      .collab-approval-chip {
        border-radius: 999px;
        padding: 2px 8px;
        font-size: 10px;
        font-weight: 700;
      }
    `;
    document.head.appendChild(style);
  }, []);

  useEffect(() => {
    const styleId = "editor-diagnostic-styles";
    if (document.getElementById(styleId)) return;
    const style = document.createElement("style");
    style.id = styleId;
    style.textContent = `
      .editor-line-error {
        background: linear-gradient(90deg, rgba(248, 113, 113, 0.16) 0%, rgba(248, 113, 113, 0.05) 24%, transparent 62%) !important;
        border-left: 2px solid #f87171 !important;
      }
      .editor-line-warning {
        background: linear-gradient(90deg, rgba(251, 191, 36, 0.14) 0%, rgba(251, 191, 36, 0.05) 24%, transparent 62%) !important;
        border-left: 2px solid #fbbf24 !important;
      }
      .editor-line-info {
        background: linear-gradient(90deg, rgba(52, 211, 153, 0.14) 0%, rgba(52, 211, 153, 0.05) 24%, transparent 62%) !important;
        border-left: 2px solid #34d399 !important;
      }
      .editor-line-hint {
        background: linear-gradient(90deg, rgba(125, 211, 252, 0.12) 0%, rgba(125, 211, 252, 0.04) 24%, transparent 62%) !important;
        border-left: 2px solid #7dd3fc !important;
      }
      .editor-line-marker-error,
      .editor-line-marker-warning,
      .editor-line-marker-info,
      .editor-line-marker-hint {
        width: 4px !important;
        margin-left: 3px;
        border-radius: 999px;
      }
      .editor-line-marker-error { background: #f87171; }
      .editor-line-marker-warning { background: #fbbf24; }
      .editor-line-marker-info { background: #34d399; }
      .editor-line-marker-hint { background: #7dd3fc; }
    `;
    document.head.appendChild(style);
  }, []);

  // Helper: convert Unix timestamp to relative time string
  function formatRelativeTime(timestamp: number): string {
    const now = Math.floor(Date.now() / 1000);
    const diff = now - timestamp;
    if (diff < 60) return "just now";
    if (diff < 3600) return `${Math.floor(diff / 60)} minutes ago`;
    if (diff < 86400) return `${Math.floor(diff / 3600)} hours ago`;
    if (diff < 2592000) return `${Math.floor(diff / 86400)} days ago`;
    if (diff < 31536000) return `${Math.floor(diff / 2592000)} months ago`;
    return `${Math.floor(diff / 31536000)} years ago`;
  }

  // Git blame decorations
  useEffect(() => {
    const editor = editorRef.current;
    if (!blameEnabled || !activeFile?.path || !editor || !monaco) {
      // Clear existing blame decorations when disabled
      if (!blameEnabled && editor) {
        blameDecorationsRef.current = editor.deltaDecorations(blameDecorationsRef.current, []);
      }
      return;
    }
    const loadBlame = async () => {
      try {
        const entries = await invoke<Array<{ line: number; commit_hash: string; author: string; timestamp: number }>>(
          "git_blame",
          { repo_path: rootPath ?? ".", file_path: activeFile.path }
        );
        const decorations = entries.map((entry) => ({
          range: new monaco.Range(entry.line, 1, entry.line, 1),
          options: {
            after: {
              content: `  ${entry.commit_hash.slice(0, 7)} · ${entry.author} · ${formatRelativeTime(entry.timestamp)}`,
              inlineClassName: "blame-annotation",
            },
          },
        }));
        blameDecorationsRef.current = editor.deltaDecorations(blameDecorationsRef.current, decorations);
      } catch { /* ignore */ }
    };
    loadBlame();
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [blameEnabled, activeFile?.path, rootPath, monaco]);

  // Listen for editor-toggle-blame events
  useEffect(() => {
    const handler = () => setBlameEnabled((prev) => !prev);
    window.addEventListener("editor-toggle-blame", handler);
    return () => window.removeEventListener("editor-toggle-blame", handler);
  }, []);

  // Listen for editor-font-changed events
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<{ family: string; size: number; lineHeight: number; ligatures: boolean }>).detail;
      const editor = editorRef.current;
      if (!editor || !detail) return;
      editor.updateOptions({
        fontFamily: detail.family,
        fontSize: detail.size,
        lineHeight: detail.lineHeight,
        fontLigatures: detail.ligatures,
      });
    };
    window.addEventListener("editor-font-changed", handler);
    return () => window.removeEventListener("editor-font-changed", handler);
  }, []);

  // Listen for editor-insert-code events from chat messages
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<{ code: string; language: string }>).detail;
      const editor = editorRef.current;
      if (!editor || !detail?.code) return;
      const selection = editor.getSelection();
      if (!selection) return;
      editor.executeEdits("editor-insert-code", [
        {
          range: selection,
          text: detail.code,
          forceMoveMarkers: true,
        },
      ]);
      editor.focus();
    };
    window.addEventListener("editor-insert-code", handler);
    return () => window.removeEventListener("editor-insert-code", handler);
  }, []);

  // Listen for editor-go-to events from DiagnosticPanel
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<{ file: string; line: number; column: number }>).detail;
      const editor = editorRef.current;
      if (!editor || !detail) return;
      const model = editor.getModel();
      if (!model) return;
      // Only handle if the current model matches the file
      if (model.uri.path !== detail.file) return;
      editor.revealLineInCenter(detail.line);
      editor.setPosition({ lineNumber: detail.line, column: detail.column });
      editor.focus();
    };
    window.addEventListener("editor-go-to", handler);
    return () => window.removeEventListener("editor-go-to", handler);
  }, []);

  // Fetch git status
  useEffect(() => {
    if (!rootPath) return;
    invoke<boolean>("git_is_repo", { root: rootPath }).then(isRepo => {
      if (isRepo) {
        invoke<Array<{path: string, status: string}>>("git_status", { root: rootPath }).then(statuses => {
          const map = new Map<string, string>();
          for (const s of statuses) {
            map.set(s.path, s.status);
          }
          setGitChanges(map);
        }).catch(() => {});
      }
    }).catch(() => {});
  }, [rootPath, files]);

  // Register/unregister inline completion provider
  useEffect(() => {
    if (completionDisposable.current) {
      completionDisposable.current.dispose();
      completionDisposable.current = null;
    }

    if (aiCompletionEnabled && monaco) {
      completionDisposable.current = registerProvider(monaco);
    }

    return () => {
      if (completionDisposable.current) {
        completionDisposable.current.dispose();
        completionDisposable.current = null;
      }
    };
  }, [aiCompletionEnabled, monaco, registerProvider]);

  // Register FIM ghost text inline completions provider
  useEffect(() => {
    if (fimDisposable.current) {
      fimDisposable.current.dispose();
      fimDisposable.current = null;
    }

    if (ghostTextEnabled && monaco) {
      fimDisposable.current = monaco.languages.registerInlineCompletionsProvider("*", {
        provideInlineCompletions: async (model, position) => {
          const offset = model.getOffsetAt(position);
          const fullText = model.getValue();
          const prefix = fullText.slice(0, offset);
          const suffix = fullText.slice(offset);
          const language = model.getLanguageId();
          try {
            const result = await invoke<string>("ai_fim_complete", { prefix, suffix, language });
            if (!result) return { items: [] };
            return {
              items: [
                {
                  insertText: result,
                  range: {
                    startLineNumber: position.lineNumber,
                    startColumn: position.column,
                    endLineNumber: position.lineNumber,
                    endColumn: position.column,
                  },
                },
              ],
            };
          } catch {
            return { items: [] };
          }
        },
        freeInlineCompletions: () => { /* nothing to free */ },
      });
    }

    return () => {
      if (fimDisposable.current) {
        fimDisposable.current.dispose();
        fimDisposable.current = null;
      }
    };
  }, [ghostTextEnabled, monaco]);

  // Update breakpoint gutter decorations when breakpoints change
  useEffect(() => {
    const editor = editorRef.current;
    if (!editor || !monaco) return;
    const newDecorations = Array.from(breakpoints).map((line) => ({
      range: new monaco.Range(line, 1, line, 1),
      options: {
        isWholeLine: false,
        glyphMarginClassName: "breakpoint-glyph",
        glyphMarginHoverMessage: { value: `Breakpoint at line ${line}` },
      },
    }));
    breakpointDecorationsRef.current = editor.deltaDecorations(
      breakpointDecorationsRef.current,
      newDecorations
    );
  }, [breakpoints, monaco]);

  // Inject breakpoint glyph CSS
  useEffect(() => {
    const styleId = "breakpoint-glyph-styles";
    if (document.getElementById(styleId)) return;
    const style = document.createElement("style");
    style.id = styleId;
    style.textContent = `
      .breakpoint-glyph {
        background: #e51400;
        border-radius: 50%;
        width: 12px !important;
        height: 12px !important;
        margin-top: 3px;
        margin-left: 2px;
        cursor: pointer;
      }
    `;
    document.head.appendChild(style);
  }, []);

  // Expose ghostTextEnabled toggle via command palette event
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<{ enabled?: boolean }>).detail;
      if (typeof detail?.enabled === "boolean") {
        setGhostTextEnabled(detail.enabled);
      } else {
        setGhostTextEnabled((prev) => !prev);
      }
    };
    window.addEventListener("editor-toggle-ghost-text", handler);
    return () => window.removeEventListener("editor-toggle-ghost-text", handler);
  }, []);

  const applyDiagnosticDecorations = useCallback((targetFilePath?: string) => {
    const editor = editorRef.current;
    if (!editor || !monaco) return;
    const model = editor.getModel();
    if (!model) {
      diagnosticDecorationsRef.current = editor.deltaDecorations(diagnosticDecorationsRef.current, []);
      return;
    }
    if (targetFilePath && model.uri.path !== targetFilePath) return;

    const markers = monaco.editor.getModelMarkers({ resource: model.uri });
    const strongestMarkerPerLine = new Map<number, typeof markers[number]>();
    for (const marker of markers) {
      const existing = strongestMarkerPerLine.get(marker.startLineNumber);
      if (!existing || marker.severity > existing.severity) {
        strongestMarkerPerLine.set(marker.startLineNumber, marker);
      }
    }

    const decorations = Array.from(strongestMarkerPerLine.values()).map((marker) => {
      const severity = marker.severity;
      const lineClassName =
        severity >= 8 ? "editor-line-error" :
        severity >= 4 ? "editor-line-warning" :
        severity >= 2 ? "editor-line-info" :
        "editor-line-hint";
      const linesDecorationsClassName =
        severity >= 8 ? "editor-line-marker-error" :
        severity >= 4 ? "editor-line-marker-warning" :
        severity >= 2 ? "editor-line-marker-info" :
        "editor-line-marker-hint";
      const color =
        severity >= 8 ? "#f87171" :
        severity >= 4 ? "#fbbf24" :
        severity >= 2 ? "#34d399" :
        "#7dd3fc";

      return {
        range: new monaco.Range(marker.startLineNumber, 1, marker.startLineNumber, 1),
        options: {
          isWholeLine: true,
          className: lineClassName,
          linesDecorationsClassName,
          overviewRuler: {
            color,
            position: monaco.editor.OverviewRulerLane.Left,
          },
        },
      };
    });

    diagnosticDecorationsRef.current = editor.deltaDecorations(diagnosticDecorationsRef.current, decorations);
  }, [monaco]);

  useEffect(() => {
    if (!monaco) return;
    defineShadowIdeTheme(monaco);
    monaco.editor.setTheme(MONACO_THEME_NAME);

    const observer = new MutationObserver(() => {
      defineShadowIdeTheme(monaco);
      monaco.editor.setTheme(MONACO_THEME_NAME);
    });
    observer.observe(document.documentElement, {
      attributes: true,
      attributeFilter: ["data-oled", "style", "class"],
    });
    return () => observer.disconnect();
  }, [monaco]);

  useEffect(() => {
    applyDiagnosticDecorations(activeFile?.path);
  }, [activeFile?.path, applyDiagnosticDecorations]);

  // Register LSP providers (hover, completion, definition) + diagnostics listener
  useEffect(() => {
    if (!monaco) return;

    const severityMap: Record<string, MarkerSeverity> = {
      error: 8 as MarkerSeverity,
      warning: 4 as MarkerSeverity,
      info: 2 as MarkerSeverity,
      hint: 1 as MarkerSeverity,
    };

    const hoverProvider = monaco.languages.registerHoverProvider("*", {
      provideHover: async (model, position) => {
        const filePath = model.uri.path;
        if (!filePath) return null;
        try {
          const result = await invoke<LspHoverResult | null>("lsp_hover", {
            file: filePath,
            line: position.lineNumber - 1,
            col: position.column - 1,
          });
          if (!result) return null;
          return {
            contents: [{ value: result.contents }],
          };
        } catch {
          return null;
        }
      },
    });

    const completionProvider = monaco.languages.registerCompletionItemProvider("*", {
      triggerCharacters: [".", ":", "<", "(", "@", "#", "/"],
      provideCompletionItems: async (model, position) => {
        const filePath = model.uri.path;
        if (!filePath) return { suggestions: [] };
        try {
          const items = await invoke<LspCompletionItem[]>("lsp_completion", {
            file: filePath,
            line: position.lineNumber - 1,
            col: position.column - 1,
          });
          const word = model.getWordUntilPosition(position);
          const range = {
            startLineNumber: position.lineNumber,
            endLineNumber: position.lineNumber,
            startColumn: word.startColumn,
            endColumn: word.endColumn,
          };
          return {
            suggestions: (items ?? []).map((item) => ({
              label: item.label,
              kind: completionKindToMonaco(item.kind) as languages.CompletionItemKind,
              detail: item.detail ?? undefined,
              documentation: item.documentation ?? undefined,
              insertText: item.insert_text ?? item.label,
              range,
            })),
          };
        } catch {
          return { suggestions: [] };
        }
      },
    });

    const definitionProvider = monaco.languages.registerDefinitionProvider("*", {
      provideDefinition: async (model, position) => {
        const filePath = model.uri.path;
        if (!filePath) return null;
        try {
          const locations = await invoke<LspLocation[]>("lsp_goto_definition", {
            file: filePath,
            line: position.lineNumber - 1,
            col: position.column - 1,
          });
          return (locations ?? []).map((loc) => ({
            uri: monaco.Uri.file(loc.file),
            range: {
              startLineNumber: loc.line + 1,
              startColumn: loc.col + 1,
              endLineNumber: loc.line + 1,
              endColumn: loc.col + 1,
            },
          }));
        } catch {
          return null;
        }
      },
    });

    // Register semantic token providers for LSP-supported languages
    const semanticDisposables: import("monaco-editor").IDisposable[] = [];
    const semanticLanguages = ["typescript", "javascript", "python", "rust", "cpp", "c", "go"];
    for (const lang of semanticLanguages) {
      const disp = monaco.languages.registerDocumentSemanticTokensProvider(lang, {
        getLegend: () => ({
          tokenTypes: ["namespace", "class", "enum", "interface", "struct", "typeParameter", "type", "parameter", "variable", "property", "enumMember", "decorator", "event", "function", "method", "macro", "label", "comment", "string", "keyword", "number", "regexp", "operator"],
          tokenModifiers: ["declaration", "definition", "readonly", "static", "deprecated", "abstract", "async", "modification", "documentation", "defaultLibrary"],
        }),
        provideDocumentSemanticTokens: async (model) => {
          try {
            const result = await invoke<{ data: number[] }>("lsp_semantic_tokens", {
              language: lang,
              fileUri: model.uri.toString(),
            });
            return { data: new Uint32Array(result.data) };
          } catch {
            return null;
          }
        },
        releaseDocumentSemanticTokens: () => { /* nothing to release */ },
      });
      semanticDisposables.push(disp);
    }

    lspDisposables.current = [hoverProvider, completionProvider, definitionProvider, ...semanticDisposables];

    // Listen for LSP diagnostics
    let unlistenFn: (() => void) | null = null;
    const setupListener = async () => {
      unlistenFn = await listen<LspDiagnosticEvent>("lsp-diagnostics", (event) => {
        const { file, diagnostics } = event.payload;
        const uri = monaco.Uri.file(file);
        const model = monaco.editor.getModel(uri);
        if (!model) return;
        const markers = diagnostics.map((d) => ({
          severity: severityMap[d.severity] ?? (2 as MarkerSeverity),
          message: d.source ? `[${d.source}] ${d.message}` : d.message,
          startLineNumber: d.line + 1,
          startColumn: d.col + 1,
          endLineNumber: d.end_line + 1,
          endColumn: d.end_col + 1,
        }));
        monaco.editor.setModelMarkers(model, "lsp", markers);
        applyDiagnosticDecorations(file);

        // Emit diagnostic counts + detailed items across all models
        const allMarkers = monaco.editor.getModelMarkers({});
        let errors = 0, warnings = 0, infos = 0;
        const items: DiagnosticItem[] = [];
        for (const m of allMarkers) {
          const sev = m.severity === 8 ? "error" as const : m.severity === 4 ? "warning" as const : "info" as const;
          if (m.severity === 8) errors++;
          else if (m.severity === 4) warnings++;
          else infos++;
          items.push({
            file: m.resource.path,
            line: m.startLineNumber,
            column: m.startColumn,
            endLine: m.endLineNumber,
            endColumn: m.endColumn,
            severity: sev,
            message: m.message,
          });
        }
        onDiagnosticsChange?.({ errors, warnings, infos });
        onDiagnosticItems?.(items);
      });
    };
    setupListener();

    return () => {
      lspDisposables.current.forEach((d) => d.dispose());
      lspDisposables.current = [];
      if (unlistenFn) unlistenFn();
    };
  }, [monaco, applyDiagnosticDecorations]);

  // LSP: start server and notify didOpen/didChange for active file
  useEffect(() => {
    if (!activeFile || !rootPath) return;
    const ext = activeFile.name.split(".").pop()?.toLowerCase() ?? "";
    const serverKey = serverKeyForExt(ext);
    if (!serverKey || !LSP_EXTENSIONS.has(ext)) return;

    const startAndOpen = async () => {
      // Start server if needed
      if (!lspStarted.current.has(serverKey)) {
        try {
          await invoke("lsp_start", { language: serverKey, rootPath });
          lspStarted.current.add(serverKey);
        } catch {
          return; // No server available — silently continue
        }
      }

      // Send didOpen if not already open
      if (!lspOpenFiles.current.has(activeFile.path)) {
        lspOpenFiles.current.add(activeFile.path);
        fileVersions.current.set(activeFile.path, 1);
        invoke("lsp_did_open", {
          file: activeFile.path,
          content: activeFile.content,
        }).catch(() => {});
      }
    };
    startAndOpen();
  }, [activeFile?.path, rootPath]);

  // LSP: notify didChange on content edits (debounced)
  const lspChangeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    if (!activeFile || !lspOpenFiles.current.has(activeFile.path)) return;
    if (lspChangeTimer.current) clearTimeout(lspChangeTimer.current);

    lspChangeTimer.current = setTimeout(() => {
      const ver = (fileVersions.current.get(activeFile.path) ?? 1) + 1;
      fileVersions.current.set(activeFile.path, ver);
      invoke("lsp_did_change", {
        file: activeFile.path,
        content: activeFile.content,
        version: ver,
      }).catch(() => {});
    }, 300);

    return () => {
      if (lspChangeTimer.current) clearTimeout(lspChangeTimer.current);
    };
  }, [activeFile?.content]);

  // LSP: notify didSave after file save
  const lspDidSave = useCallback((path: string) => {
    if (lspOpenFiles.current.has(path)) {
      invoke("lsp_did_save", { file: path }).catch(() => {});
    }
  }, []);

  // LSP: cleanup on file close — called from onFileClose
  const lspDidClose = useCallback((path: string) => {
    if (lspOpenFiles.current.has(path)) {
      lspOpenFiles.current.delete(path);
      fileVersions.current.delete(path);
      invoke("lsp_did_close", { file: path }).catch(() => {});
    }
  }, []);

  const sendCallSignal = useCallback((signalType: string, payload: Record<string, unknown>, targetId?: string) => {
    const currentFile = activeFileRef.current;
    if (!currentFile) return;
    emit("collab-call-signal", {
      room_id: currentFile.path,
      file_path: currentFile.path,
      sender_id: localIdentityRef.current.id,
      sender_name: localIdentityRef.current.name,
      target_id: targetId ?? null,
      signal_type: signalType,
      payload,
      timestamp: Date.now(),
    }).catch(() => {});
  }, []);

  const ensureCollaboratorStyles = useCallback((presence: CollaboratorPresence) => {
    const collaboratorCssId = cssId(presence.collaborator_id);
    const styleId = `collab-style-${collaboratorCssId}`;
    if (document.getElementById(styleId)) return;
    const style = document.createElement("style");
    style.id = styleId;
    style.textContent = `
      .collab-cursor-${collaboratorCssId} {
        border-left: 2px solid ${presence.color};
        margin-left: -1px;
      }
      .collab-label-${collaboratorCssId} {
        background: ${presence.color};
        color: #0b1020;
        border-radius: 999px;
        padding: 0 6px;
        margin-left: 6px;
        font-weight: 700;
      }
    `;
    document.head.appendChild(style);
  }, []);

  const applyCollaborationDecorations = useCallback((snapshot: CollaborationSnapshot | null) => {
    const editor = editorRef.current;
    if (!editor || !monaco) return;
    const model = editor.getModel();
    if (!model || !snapshot) {
      collaboratorDecorationsRef.current = editor.deltaDecorations(collaboratorDecorationsRef.current, []);
      reviewDecorationsRef.current = editor.deltaDecorations(reviewDecorationsRef.current, []);
      return;
    }

    const presenceDecorations = snapshot.presences
      .filter((presence) => presence.collaborator_id !== localIdentityRef.current.id)
      .map((presence) => {
        ensureCollaboratorStyles(presence);
        const line = Math.max(1, Math.min(model.getLineCount(), presence.line));
        const maxColumn = model.getLineMaxColumn(line);
        const startColumn = Math.max(1, Math.min(maxColumn, presence.column));
        const endColumn = Math.min(maxColumn, startColumn + 1);
        return {
          range: new monaco.Range(line, startColumn, line, endColumn),
          options: {
            className: `collab-cursor-${cssId(presence.collaborator_id)}`,
            hoverMessage: {
              value: `${presence.name}${presence.video_active ? " · video" : presence.voice_active ? " · voice" : ""}`,
            },
            after: {
              content: ` ${presence.name}`,
              inlineClassName: `collab-label-${cssId(presence.collaborator_id)}`,
            },
          },
        };
      });

    const reviewDecorations = snapshot.comments
      .filter((comment) => !comment.resolved)
      .map((comment) => ({
        range: new monaco.Range(comment.line, 1, comment.line, 1),
        options: {
          isWholeLine: true,
          className: "collab-review-line",
          hoverMessage: {
            value: `${comment.author}: ${comment.body}`,
          },
        },
      }));

    collaboratorDecorationsRef.current = editor.deltaDecorations(collaboratorDecorationsRef.current, presenceDecorations);
    reviewDecorationsRef.current = editor.deltaDecorations(reviewDecorationsRef.current, reviewDecorations);
  }, [ensureCollaboratorStyles, monaco]);

  const clearCall = useCallback(() => {
    for (const connection of peerConnectionsRef.current.values()) {
      connection.close();
    }
    peerConnectionsRef.current.clear();
    for (const stream of remoteStreamsRef.current.values()) {
      for (const track of stream.getTracks()) track.stop();
    }
    remoteStreamsRef.current.clear();
    if (localStreamRef.current) {
      for (const track of localStreamRef.current.getTracks()) track.stop();
      localStreamRef.current = null;
    }
    if (localVideoRef.current) localVideoRef.current.srcObject = null;
    if (remoteVideoRef.current) remoteVideoRef.current.srcObject = null;
    setRemoteStreamVersion((value) => value + 1);
  }, []);

  const ensureLocalMedia = useCallback(async (withVideo: boolean) => {
    const existing = localStreamRef.current;
    const needsVideo = withVideo && !existing?.getVideoTracks().length;
    if (existing && !needsVideo) return existing;
    const nextStream = await navigator.mediaDevices.getUserMedia({
      audio: true,
      video: withVideo,
    });
    if (existing) {
      for (const track of existing.getTracks()) track.stop();
    }
    localStreamRef.current = nextStream;
    if (localVideoRef.current) localVideoRef.current.srcObject = nextStream;
    return nextStream;
  }, []);

  const ensurePeerConnection = useCallback(async (presence: CollaboratorPresence, withVideo: boolean) => {
    const existing = peerConnectionsRef.current.get(presence.collaborator_id);
    if (existing) return existing;
    const connection = new RTCPeerConnection({
      iceServers: [{ urls: "stun:stun.l.google.com:19302" }],
    });
    peerConnectionsRef.current.set(presence.collaborator_id, connection);
    const localStream = await ensureLocalMedia(withVideo);
    for (const track of localStream.getTracks()) {
      connection.addTrack(track, localStream);
    }
    connection.onicecandidate = (event) => {
      if (event.candidate) {
        sendCallSignal("ice", event.candidate.toJSON() as unknown as Record<string, unknown>, presence.collaborator_id);
      }
    };
    connection.ontrack = (event) => {
      const [stream] = event.streams;
      if (!stream) return;
      remoteStreamsRef.current.set(presence.collaborator_id, stream);
      setRemoteStreamVersion((value) => value + 1);
    };
    connection.onconnectionstatechange = () => {
      if (connection.connectionState === "failed" || connection.connectionState === "closed" || connection.connectionState === "disconnected") {
        peerConnectionsRef.current.delete(presence.collaborator_id);
        remoteStreamsRef.current.delete(presence.collaborator_id);
        setRemoteStreamVersion((value) => value + 1);
      }
    };
    return connection;
  }, [ensureLocalMedia, sendCallSignal]);

  useEffect(() => {
    applyCollaborationDecorations(collabSnapshot);
  }, [applyCollaborationDecorations, collabSnapshot]);

  useEffect(() => {
    const firstRemoteStream = remoteStreamsRef.current.values().next().value ?? null;
    if (remoteVideoRef.current) remoteVideoRef.current.srcObject = firstRemoteStream;
    if (localVideoRef.current && localStreamRef.current) localVideoRef.current.srcObject = localStreamRef.current;
  }, [remoteStreamVersion]);

  useEffect(() => {
    if (!activeFile) {
      setCollabSnapshot(null);
      collabVersionRef.current = 0;
      return;
    }

    let cancelled = false;
    invoke<CollaborationSnapshot | null>("collab_get_snapshot", { filePath: activeFile.path })
      .then((snapshot) => {
        if (cancelled || !snapshot) return;
        setCollabSnapshot(snapshot);
        collabVersionRef.current = snapshot.version;
        if (snapshot.content && snapshot.content !== activeFile.content && snapshot.source_collaborator_id !== localIdentityRef.current.id) {
          applyingCollabUpdateRef.current = true;
          onFileContentChange(activeFileIndex, snapshot.content);
          window.setTimeout(() => { applyingCollabUpdateRef.current = false; }, 0);
        }
      })
      .catch(() => {});

    emit("collab-join", {
      file_path: activeFile.path,
      collaborator_id: localIdentityRef.current.id,
      name: localIdentityRef.current.name,
      color: localIdentityRef.current.color,
      content: activeFile.content,
    }).catch(() => {});

    return () => {
      cancelled = true;
      emit("collab-leave", {
        file_path: activeFile.path,
        collaborator_id: localIdentityRef.current.id,
      }).catch(() => {});
    };
  }, [activeFile?.path]);

  useEffect(() => {
    if (!activeFile) return;
    let unlistenSnapshot: (() => void) | undefined;
    let unlistenSignal: (() => void) | undefined;

    listen<CollaborationSnapshot>("collab-document-state", (event) => {
      const snapshot = event.payload;
      if (snapshot.file_path !== activeFile.path) return;
      collabVersionRef.current = snapshot.version;
      setCollabSnapshot(snapshot);
      if (
        snapshot.source_collaborator_id &&
        snapshot.source_collaborator_id !== localIdentityRef.current.id &&
        snapshot.content !== activeFileRef.current?.content
      ) {
        applyingCollabUpdateRef.current = true;
        onFileContentChangeRef.current(activeFileIndexRef.current, snapshot.content);
        window.setTimeout(() => { applyingCollabUpdateRef.current = false; }, 0);
      }
    }).then((cleanup) => { unlistenSnapshot = cleanup; }).catch(() => {});

    listen<CallSignalEvent>("collab-call-signal", async (event) => {
      const signal = event.payload;
      if (signal.file_path !== activeFile.path) return;
      if (signal.sender_id === localIdentityRef.current.id) return;
      if (signal.target_id && signal.target_id !== localIdentityRef.current.id) return;

      const remotePresence = collabSnapshot?.presences.find((presence) => presence.collaborator_id === signal.sender_id) ?? {
        collaborator_id: signal.sender_id,
        name: signal.sender_name,
        color: "#7dd3fc",
        file_path: activeFile.path,
        line: 1,
        column: 1,
        voice_active: true,
        video_active: Boolean(signal.payload.video),
        last_seen: Date.now(),
      };

      if (signal.signal_type === "hangup") {
        const connection = peerConnectionsRef.current.get(signal.sender_id);
        if (connection) connection.close();
        peerConnectionsRef.current.delete(signal.sender_id);
        remoteStreamsRef.current.delete(signal.sender_id);
        setRemoteStreamVersion((value) => value + 1);
        if (peerConnectionsRef.current.size === 0) {
          setCallState((current) => ({ ...current, active: false, status: "Ended" }));
          clearCall();
        }
        return;
      }

      if (signal.signal_type === "offer") {
        const connection = await ensurePeerConnection(remotePresence, Boolean(signal.payload.video));
        await connection.setRemoteDescription(new RTCSessionDescription(signal.payload as unknown as RTCSessionDescriptionInit));
        const answer = await connection.createAnswer();
        await connection.setLocalDescription(answer);
        sendCallSignal("answer", { type: answer.type, sdp: answer.sdp ?? "" }, signal.sender_id);
        setCallState({
          active: true,
          video: Boolean(signal.payload.video),
          status: `Connected to ${signal.sender_name}`,
        });
        return;
      }

      if (signal.signal_type === "answer") {
        const connection = peerConnectionsRef.current.get(signal.sender_id);
        if (!connection) return;
        await connection.setRemoteDescription(new RTCSessionDescription(signal.payload as unknown as RTCSessionDescriptionInit));
        setCallState((current) => ({ ...current, status: `Connected to ${signal.sender_name}` }));
        return;
      }

      if (signal.signal_type === "ice") {
        const connection = peerConnectionsRef.current.get(signal.sender_id);
        if (!connection) return;
        try {
          await connection.addIceCandidate(new RTCIceCandidate(signal.payload as RTCIceCandidateInit));
        } catch {
          // Ignore transient ICE ordering issues.
        }
      }
    }).then((cleanup) => { unlistenSignal = cleanup; }).catch(() => {});

    return () => {
      if (unlistenSnapshot) unlistenSnapshot();
      if (unlistenSignal) unlistenSignal();
    };
  }, [activeFile?.path, clearCall, collabSnapshot?.presences, ensurePeerConnection, sendCallSignal]);

  useEffect(() => {
    if (!activeFile || !callState.active || !collabSnapshot) return;
    for (const presence of collabSnapshot.presences) {
      if (presence.collaborator_id === localIdentityRef.current.id) continue;
      if (peerConnectionsRef.current.has(presence.collaborator_id)) continue;
      if (localIdentityRef.current.id > presence.collaborator_id) continue;
      ensurePeerConnection(presence, callState.video)
        .then(async (connection) => {
          const offer = await connection.createOffer();
          await connection.setLocalDescription(offer);
          sendCallSignal("offer", { type: offer.type, sdp: offer.sdp ?? "" }, presence.collaborator_id);
          setCallState((current) => ({ ...current, status: `Calling ${presence.name}` }));
        })
        .catch(() => {});
    }
  }, [activeFile?.path, callState.active, callState.video, collabSnapshot, ensurePeerConnection, sendCallSignal]);

  useEffect(() => {
    if (!activeFile || !editorRef.current) return;
    const position = editorRef.current.getPosition();
    emit("collab-presence", {
      file_path: activeFile.path,
      collaborator_id: localIdentityRef.current.id,
      name: localIdentityRef.current.name,
      color: localIdentityRef.current.color,
      line: position?.lineNumber ?? 1,
      column: position?.column ?? 1,
      voice_active: callState.active,
      video_active: callState.active && callState.video,
    }).catch(() => {});
  }, [activeFile?.path, callState.active, callState.video]);

  useEffect(() => () => clearCall(), [clearCall]);

  const handleEditorMount: OnMount = (editor) => {
    editorRef.current = editor;

    // Track cursor position (debounced to avoid excessive re-renders)
    // Also trigger FIM ghost text after 800ms idle
    const cursorTimer = { current: null as ReturnType<typeof setTimeout> | null };
    editor.onDidChangeCursorPosition((e) => {
      if (cursorTimer.current) clearTimeout(cursorTimer.current);
      cursorTimer.current = setTimeout(() => {
        const sel = editor.getSelection();
        const selected = sel ? editor.getModel()?.getValueInRange(sel)?.length ?? 0 : 0;
        onCursorChange?.({ line: e.position.lineNumber, column: e.position.column, selected });
        emit("workspace-cursor-moved", { line: e.position.lineNumber, column: e.position.column, selected }).catch(() => {});
        const currentFile = activeFileRef.current;
        const model = editor.getModel();
        if (currentFile && model) {
          emit("collab-presence", {
            file_path: currentFile.path,
            collaborator_id: localIdentityRef.current.id,
            name: localIdentityRef.current.name,
            color: localIdentityRef.current.color,
            line: e.position.lineNumber,
            column: e.position.column,
            selection_start: sel ? model.getOffsetAt(sel.getStartPosition()) : null,
            selection_end: sel ? model.getOffsetAt(sel.getEndPosition()) : null,
            voice_active: callStateRef.current.active,
            video_active: callStateRef.current.active && callStateRef.current.video,
          }).catch(() => {});
        }
      }, 50);

      // FIM debounce: trigger ghost text after 800ms idle
      if (ghostTextEnabled) {
        if (fimDebounceTimer.current) clearTimeout(fimDebounceTimer.current);
        fimDebounceTimer.current = setTimeout(async () => {
          const model = editor.getModel();
          if (!model) return;
          const position = editor.getPosition();
          if (!position) return;
          const offset = model.getOffsetAt(position);
          const fullText = model.getValue();
          const prefix = fullText.slice(0, offset);
          const suffix = fullText.slice(offset);
          const language = model.getLanguageId();
          try {
            await invoke("ai_fim_complete", { prefix, suffix, language });
            // Result is surfaced via the inline completions provider
          } catch {
            // Backend command may not exist yet
          }
        }, 800);
      }
    });

    editor.onDidChangeModelContent((event) => {
      if (applyingCollabUpdateRef.current) return;
      const currentFile = activeFileRef.current;
      if (!currentFile) return;
      for (const change of event.changes) {
        emit("collab-edit", {
          file_path: currentFile.path,
          collaborator_id: localIdentityRef.current.id,
          name: localIdentityRef.current.name,
          color: localIdentityRef.current.color,
          base_version: collabVersionRef.current,
          offset: change.rangeOffset,
          delete_len: change.rangeLength,
          text: change.text,
        }).catch(() => {});
      }
    });

    // Track selection for Ctrl+K AI popup
    editor.onDidChangeCursorSelection(() => {
      // Selection tracking is handled inline in the Ctrl+K command below
    });

    // Add Ctrl+S save binding
    editor.addCommand(
      // Monaco KeyMod.CtrlCmd | Monaco KeyCode.KeyS
      2048 | 49, // CtrlCmd + S
      () => saveFile()
    );

    // Add Ctrl+Shift+M minimap toggle
    editor.addCommand(
      // CtrlCmd + Shift + KeyM
      2048 | 1024 | 43,
      () => onMinimapToggle?.(!(minimapEnabled ?? true))
    );

    // Add Ctrl+K binding for AI action popup (multi-cursor aware)
    editor.addCommand(
      2048 | 41, // CtrlCmd + K
      () => {
        const selections = editor.getSelections() ?? [];
        const model = editor.getModel();
        if (!model) return;
        const nonEmpty = selections.filter((s) => !s.isEmpty());
        if (nonEmpty.length === 0) return;
        const language = model.getLanguageId();
        const primarySel = nonEmpty[0];
        const layoutInfo = editor.getLayoutInfo();
        const scrolledVisiblePosition = editor.getScrolledVisiblePosition(primarySel.getStartPosition());
        if (!scrolledVisiblePosition) return;
        const editorDom = editor.getDomNode();
        if (!editorDom) return;
        const rect = editorDom.getBoundingClientRect();
        const x = rect.left + scrolledVisiblePosition.left + layoutInfo.contentLeft;
        const y = rect.top + scrolledVisiblePosition.top + 20;

        if (nonEmpty.length > 1) {
          // Multi-cursor: collect all selected texts
          const selectedTexts = nonEmpty.map((sel) => model.getValueInRange(sel) ?? "");
          const combined = selectedTexts.join("\n---\n");
          setAiPopup({
            selectedText: combined,
            language,
            position: { x, y },
            multiCursor: true,
            selections: nonEmpty,
          });
        } else {
          const selectedText = model.getValueInRange(primarySel);
          if (!selectedText.trim()) return;
          setAiPopup({ selectedText, language, position: { x, y } });
        }
      }
    );

    // Gutter click handler for breakpoints
    editor.onMouseDown((e) => {
      if (
        monaco && (
          e.target.type === monaco.editor.MouseTargetType.GUTTER_LINE_NUMBERS ||
          e.target.type === monaco.editor.MouseTargetType.GUTTER_GLYPH_MARGIN
        )
      ) {
        const line = e.target.position?.lineNumber;
        if (line) {
          setBreakpoints((prev) => {
            const next = new Set(prev);
            if (next.has(line)) {
              next.delete(line);
            } else {
              next.add(line);
            }
            window.dispatchEvent(new CustomEvent("breakpoint-changed", { detail: { line, active: next.has(line) } }));
            return next;
          });
        }
      }
    });

    // Emit cursor-move event for DocsPanel
    editor.onDidChangeCursorPosition((e) => {
      const model = editor.getModel();
      if (!model) return;
      const word = model.getWordAtPosition(e.position);
      window.dispatchEvent(new CustomEvent("editor-cursor-move", {
        detail: {
          line: e.position.lineNumber - 1,
          character: e.position.column - 1,
          word: word?.word ?? "",
        },
      }));
    });
  };

  const saveFile = useCallback(async () => {
    if (!activeFile || !activeFile.modified) return;

    setSaving(true);
    try {
      await invoke("write_file_content", {
        path: activeFile.path,
        content: activeFile.content,
      });
      lspDidSave(activeFile.path);
      onFileContentChange(activeFileIndex, activeFile.content);
      emit("workspace-file-saved", { path: activeFile.path }).catch(() => {});
    } catch (err) {
      console.error("Failed to save file:", err);
    }
    setSaving(false);
  }, [activeFile, activeFileIndex, onFileContentChange]);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === "s") {
        e.preventDefault();
        saveFile();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [saveFile]);

  const startCall = useCallback(async (withVideo: boolean) => {
    try {
      await ensureLocalMedia(withVideo);
      setCallState({
        active: true,
        video: withVideo,
        status: "Waiting for collaborators",
      });
    } catch (err) {
      setCallState({
        active: false,
        video: false,
        status: `Call failed: ${String(err)}`,
      });
    }
  }, [ensureLocalMedia]);

  const endCall = useCallback(() => {
    const currentFile = activeFileRef.current;
    if (currentFile) {
      sendCallSignal("hangup", {}, undefined);
    }
    clearCall();
    setCallState({
      active: false,
      video: false,
      status: "Ended",
    });
  }, [clearCall, sendCallSignal]);

  const addReviewComment = useCallback(() => {
    const currentFile = activeFileRef.current;
    const editor = editorRef.current;
    if (!currentFile || !editor || !reviewDraft.trim()) return;
    const position = editor.getPosition();
    emit("collab-add-comment", {
      file_path: currentFile.path,
      line: position?.lineNumber ?? 1,
      column: position?.column ?? 1,
      author: localIdentityRef.current.name,
      body: reviewDraft.trim(),
    }).catch(() => {});
    setReviewDraft("");
  }, [reviewDraft]);

  const setApproval = useCallback((status: string) => {
    const currentFile = activeFileRef.current;
    if (!currentFile) return;
    emit("collab-set-approval", {
      file_path: currentFile.path,
      reviewer: localIdentityRef.current.name,
      status,
      note: reviewNote,
    }).catch(() => {});
    setReviewNote("");
  }, [reviewNote]);

  const collaborationSidebarPanel = activeFile && collabSidebarRoot ? createPortal(
    <div
      style={{
        minHeight: "100%",
        boxSizing: "border-box",
        padding: 12,
        display: "flex",
        flexDirection: "column",
        gap: 10,
        background: "rgba(10, 14, 24, 0.96)",
      }}
    >
      <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8 }}>
        <div>
          <div style={{ fontSize: 11, fontWeight: 700, color: "#7dd3fc", textTransform: "uppercase", letterSpacing: 0.6 }}>
            Collaboration
          </div>
          <div style={{ fontSize: 11, color: "var(--text-muted)" }}>
            {localIdentityRef.current.name} · v{collabSnapshot?.version ?? 0}
          </div>
          <div style={{ fontSize: 10, color: "var(--text-muted)", marginTop: 2 }}>
            {activeFile.name}
          </div>
        </div>
        <div style={{ fontSize: 11, color: callState.active ? "#86efac" : "var(--text-muted)" }}>
          {callState.status}
        </div>
      </div>

      <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
        {(collabSnapshot?.presences ?? [])
          .filter((presence) => presence.collaborator_id !== localIdentityRef.current.id)
          .map((presence) => (
            <button
              key={presence.collaborator_id}
              onClick={() => editorRef.current?.revealLineInCenter(presence.line)}
              style={{
                border: "none",
                borderRadius: 999,
                background: `${presence.color}22`,
                color: presence.color,
                padding: "4px 8px",
                cursor: "pointer",
                fontSize: 11,
              }}
              title={`${presence.name} · ${presence.line}:${presence.column}`}
            >
              {presence.name}
            </button>
          ))}
        {(!collabSnapshot?.presences || collabSnapshot.presences.length <= 1) && (
          <span style={{ fontSize: 11, color: "var(--text-muted)" }}>No other collaborators in this file</span>
        )}
      </div>

      <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
        <button
          onClick={() => void startCall(false)}
          style={{ flex: "1 1 100px", borderRadius: 8, border: "1px solid rgba(125, 211, 252, 0.25)", background: "#132033", color: "#7dd3fc", padding: "6px 10px", cursor: "pointer", fontSize: 11 }}
        >
          Voice Call
        </button>
        <button
          onClick={() => void startCall(true)}
          style={{ flex: "1 1 100px", borderRadius: 8, border: "1px solid rgba(196, 181, 253, 0.3)", background: "#1b1830", color: "#c4b5fd", padding: "6px 10px", cursor: "pointer", fontSize: 11 }}
        >
          Video Call
        </button>
        <button
          onClick={endCall}
          style={{ flex: "1 1 100px", borderRadius: 8, border: "1px solid rgba(248, 113, 113, 0.28)", background: "#2a1418", color: "#fca5a5", padding: "6px 10px", cursor: "pointer", fontSize: 11 }}
        >
          End
        </button>
      </div>

      {(callState.active || remoteStreamsRef.current.size > 0) && (
        <div style={{ display: "grid", gridTemplateColumns: "1fr", gap: 8 }}>
          <video ref={localVideoRef} autoPlay playsInline muted style={{ width: "100%", minHeight: 88, borderRadius: 10, background: "#050816", objectFit: "cover" }} />
          <video ref={remoteVideoRef} autoPlay playsInline style={{ width: "100%", minHeight: 88, borderRadius: 10, background: "#050816", objectFit: "cover" }} />
        </div>
      )}

      <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
        <div style={{ fontSize: 11, fontWeight: 700, color: "#fbbf24", textTransform: "uppercase", letterSpacing: 0.6 }}>
          Code Review
        </div>
        <textarea
          value={reviewDraft}
          onChange={(e) => setReviewDraft(e.target.value)}
          placeholder="Add an inline review comment for the current cursor line"
          style={{
            minHeight: 72,
            resize: "vertical",
            borderRadius: 10,
            border: "1px solid rgba(251, 191, 36, 0.24)",
            background: "#111827",
            color: "var(--text-primary)",
            padding: 8,
            fontSize: 12,
          }}
        />
        <button
          onClick={addReviewComment}
          style={{ borderRadius: 8, border: "1px solid rgba(251, 191, 36, 0.24)", background: "#30220c", color: "#fbbf24", padding: "6px 10px", cursor: "pointer", fontSize: 11 }}
        >
          Add Comment At Cursor
        </button>
        <input
          value={reviewNote}
          onChange={(e) => setReviewNote(e.target.value)}
          placeholder="Approval note"
          style={{
            borderRadius: 8,
            border: "1px solid rgba(125, 211, 252, 0.16)",
            background: "#0f172a",
            color: "var(--text-primary)",
            padding: "6px 8px",
            fontSize: 11,
          }}
        />
        <div style={{ display: "flex", gap: 6 }}>
          <button onClick={() => setApproval("approved")} style={{ flex: 1, borderRadius: 8, border: "1px solid rgba(134, 239, 172, 0.25)", background: "#0f2217", color: "#86efac", padding: "6px 8px", cursor: "pointer", fontSize: 11 }}>
            Approve
          </button>
          <button onClick={() => setApproval("changes_requested")} style={{ flex: 1, borderRadius: 8, border: "1px solid rgba(248, 113, 113, 0.24)", background: "#2a1418", color: "#fca5a5", padding: "6px 8px", cursor: "pointer", fontSize: 11 }}>
            Request Changes
          </button>
        </div>
        {(collabSnapshot?.approvals ?? []).length > 0 && (
          <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
            {collabSnapshot?.approvals.map((approval) => (
              <span
                key={approval.reviewer}
                className="collab-approval-chip"
                style={{
                  background: approval.status === "approved" ? "rgba(134, 239, 172, 0.18)" : "rgba(248, 113, 113, 0.18)",
                  color: approval.status === "approved" ? "#86efac" : "#fca5a5",
                }}
              >
                {approval.reviewer} · {approval.status}
              </span>
            ))}
          </div>
        )}
        <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
          {(collabSnapshot?.comments ?? []).slice().reverse().map((comment) => (
            <div
              key={comment.id}
              style={{
                borderRadius: 10,
                border: "1px solid rgba(255, 255, 255, 0.08)",
                background: comment.resolved ? "rgba(34, 197, 94, 0.08)" : "rgba(255, 255, 255, 0.03)",
                padding: 8,
                display: "flex",
                flexDirection: "column",
                gap: 6,
              }}
            >
              <div style={{ display: "flex", justifyContent: "space-between", gap: 8 }}>
                <button
                  onClick={() => {
                    editorRef.current?.revealLineInCenter(comment.line);
                    editorRef.current?.setPosition({ lineNumber: comment.line, column: comment.column });
                    editorRef.current?.focus();
                  }}
                  style={{ background: "transparent", border: "none", color: "#7dd3fc", cursor: "pointer", padding: 0, fontSize: 11, fontWeight: 700 }}
                >
                  {comment.author} · L{comment.line}
                </button>
                <button
                  onClick={() => {
                    emit("collab-resolve-comment", {
                      file_path: activeFile.path,
                      comment_id: comment.id,
                      resolved: !comment.resolved,
                      actor: localIdentityRef.current.name,
                    }).catch(() => {});
                  }}
                  style={{ background: "transparent", border: "none", color: comment.resolved ? "#86efac" : "#fbbf24", cursor: "pointer", padding: 0, fontSize: 11 }}
                >
                  {comment.resolved ? "Re-open" : "Resolve"}
                </button>
              </div>
              <div style={{ fontSize: 12, color: "var(--text-primary)", lineHeight: 1.5 }}>{comment.body}</div>
              <div style={{ fontSize: 10, color: "var(--text-muted)" }}>
                {new Date(comment.created_at * 1000).toLocaleString()}
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>,
    collabSidebarRoot,
  ) : null;

  if (files.length === 0) {
    return (
      <div className="editor-empty">
        <div className="editor-empty-content">
          <h2>ShadowIDE</h2>
          <p>Open a file from the explorer to start editing</p>
          <p className="shortcut-hint">
            <kbd>Ctrl</kbd>+<kbd>S</kbd> to save &nbsp;
            <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>A</kbd> AI Chat
          </p>
        </div>
      </div>
    );
  }

  return (
    <div className="editor-container">
      <div className="editor-tabs">
        {files.map((file, index) => (
          <div
            key={file.path}
            className={`editor-tab ${index === activeFileIndex ? "active" : ""}${dragIndex === index ? " dragging" : ""}${dragOverIndex === index ? " drag-over-left" : ""}`}
            onClick={() => onActiveFileChange(index)}
            draggable
            onDragStart={(e) => {
              setDragIndex(index);
              e.dataTransfer.effectAllowed = "move";
            }}
            onDragOver={(e) => {
              e.preventDefault();
              e.dataTransfer.dropEffect = "move";
              if (dragIndex !== null && dragIndex !== index) {
                setDragOverIndex(index);
              }
            }}
            onDragLeave={() => setDragOverIndex(null)}
            onDrop={(e) => {
              e.preventDefault();
              if (dragIndex !== null && dragIndex !== index) {
                onFileReorder(dragIndex, index);
              }
              setDragIndex(null);
              setDragOverIndex(null);
            }}
            onDragEnd={() => {
              setDragIndex(null);
              setDragOverIndex(null);
            }}
          >
            <span className="tab-name">
              {file.modified && <span className="tab-modified">{"\u25CF"}</span>}
              {file.name}
              {(() => {
                if (!rootPath) return null;
                const relativePath = file.path.replace(rootPath + "/", "");
                const status = gitChanges.get(relativePath);
                if (status === "M") return <span className="git-status-badge modified" title="Modified">M</span>;
                if (status === "A" || status === "??") return <span className="git-status-badge added" title="New">A</span>;
                if (status === "D") return <span className="git-status-badge deleted" title="Deleted">D</span>;
                return null;
              })()}
            </span>
            <button
              className="tab-close"
              onClick={(e) => {
                e.stopPropagation();
                lspDidClose(files[index].path);
                onFileClose(index);
              }}
            >
              ×
            </button>
          </div>
        ))}
        {saving && <span className="save-indicator">Saving...</span>}
      </div>
      {ghostDiff.hasDiff && (
        <div style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "4px 12px",
          background: "#1a2332",
          borderBottom: "1px solid #2d4a22",
          fontSize: 12,
          color: "#ccc",
        }}>
          <span style={{ color: "#3fb950" }}>+{ghostDiff.addedLines}</span>
          <span style={{ color: "#f85149" }}>-{ghostDiff.removedLines}</span>
          <span style={{ marginLeft: 4 }}>AI proposed changes</span>
          <div style={{ flex: 1 }} />
          <button
            onClick={ghostDiff.acceptDiff}
            style={{
              background: "#238636",
              color: "#fff",
              border: "none",
              borderRadius: 4,
              padding: "2px 10px",
              cursor: "pointer",
              fontSize: 12,
            }}
          >
            Accept
          </button>
          <button
            onClick={ghostDiff.rejectDiff}
            style={{
              background: "#da3633",
              color: "#fff",
              border: "none",
              borderRadius: 4,
              padding: "2px 10px",
              cursor: "pointer",
              fontSize: 12,
            }}
          >
            Reject
          </button>
        </div>
      )}
      <div className="editor-content" style={{ position: "relative" }}>
        {activeFile && isMobileDevice ? (
          <textarea
            value={activeFile.content}
            onChange={(e) => onFileContentChange(activeFileIndex, e.target.value)}
            spellCheck={false}
            autoCapitalize="off"
            autoCorrect="off"
            style={{
              width: "100%",
              height: "100%",
              background: "var(--bg-primary)",
              color: "var(--text-primary)",
              border: "none",
              outline: "none",
              resize: "none",
              padding: "8px 12px",
              fontSize: `${fontSize ?? 14}px`,
              fontFamily: "'JetBrains Mono', 'Fira Code', 'Cascadia Code', monospace",
              lineHeight: 1.5,
              tabSize: tabSize ?? 2,
              whiteSpace: "pre",
              overflowWrap: "normal",
              overflowX: "auto",
              WebkitOverflowScrolling: "touch" as any,
            }}
          />
        ) : activeFile && diffMode ? (
          <MonacoDiffEditor
            height="100%"
            language={getLanguageFromFilename(activeFile.name)}
            original={originalContent ?? ""}
            modified={activeFile.content}
            theme={MONACO_THEME_NAME}
            options={{
              fontSize: fontSize ?? 14,
              fontFamily: "'JetBrains Mono', 'Fira Code', 'Cascadia Code', monospace",
              renderSideBySide: true,
              automaticLayout: true,
              readOnly: false,
            }}
          />
        ) : activeFile && splitDirection ? (
          <div style={{
            display: "flex",
            flexDirection: splitDirection === "vertical" ? "column" : "row",
            height: "100%",
            width: "100%",
          }}>
            <div style={{ flex: 1, overflow: "hidden" }}>
              <MonacoEditor
                height="100%"
                language={getLanguageFromFilename(activeFile.name)}
                value={activeFile.content}
                theme={MONACO_THEME_NAME}
                onChange={(value) => { if (value !== undefined) onFileContentChange(activeFileIndex, value); }}
                onMount={handleEditorMount}
                options={buildEditorOptions(fontSize, tabSize, minimapEnabled, isLargeFile, aiCompletionEnabled)}
              />
            </div>
            <div style={{ width: splitDirection === "horizontal" ? "1px" : undefined, height: splitDirection === "vertical" ? "1px" : undefined, background: "var(--border)" }} />
            <div style={{ flex: 1, overflow: "hidden" }}>
              <MonacoEditor
                height="100%"
                language={splitFilePath ? getLanguageFromFilename(splitFilePath.split("/").pop() ?? splitFilePath) : "plaintext"}
                value={splitFileContent ?? ""}
                theme={MONACO_THEME_NAME}
                onChange={(value) => { if (value !== undefined) onSplitFileContentChange?.(value); }}
                options={buildEditorOptions(fontSize, tabSize, minimapEnabled, false, aiCompletionEnabled)}
              />
            </div>
          </div>
        ) : activeFile ? (
          <MonacoEditor
            height="100%"
            language={getLanguageFromFilename(activeFile.name)}
            value={activeFile.content}
            theme={MONACO_THEME_NAME}
            onChange={(value) => {
              if (value !== undefined) {
                onFileContentChange(activeFileIndex, value);
              }
            }}
            onMount={handleEditorMount}
            options={buildEditorOptions(fontSize, tabSize, minimapEnabled, isLargeFile, aiCompletionEnabled)}
          />
        ) : null}
      </div>
      {collaborationSidebarPanel}

      {/* AI Action Popup */}
      {aiPopup && (
        <AiActionPopup
          selectedText={aiPopup.selectedText}
          language={aiPopup.language}
          position={aiPopup.position}
          multiCursorNote={aiPopup.multiCursor ? `Applying to ${aiPopup.selections?.length ?? 1} selections` : undefined}
          onClose={() => setAiPopup(null)}
          onAction={(action, text) => {
            if (aiPopup.multiCursor && aiPopup.selections && editorRef.current) {
              const model = editorRef.current.getModel();
              const selections = aiPopup.selections;
              const selectedTexts = selections.map((sel) => model?.getValueInRange(sel) ?? "");
              invoke("ai_action", { selections: selectedTexts, action, language: aiPopup.language }).catch(() => {});
            }
            emit("ai-action-requested", { action, selectedText: text, language: aiPopup.language }).catch(() => {});
            setAiPopup(null);
          }}
        />
      )}
    </div>
  );
}

function buildEditorOptions(
  fontSize: number | undefined,
  tabSize: number | undefined,
  minimapEnabled: boolean | undefined,
  isLargeFile: boolean,
  aiCompletionEnabled: boolean,
): editor.IStandaloneEditorConstructionOptions {
  return {
    fontSize: fontSize ?? 14,
    fontFamily: "'JetBrains Mono', 'Fira Code', 'Cascadia Code', monospace",
    minimap: {
      enabled: !isLargeFile && (minimapEnabled ?? true),
      showSlider: "mouseover",
    },
    stickyScroll: { enabled: !isLargeFile },
    // @ts-expect-error breadcrumbs is a valid option but may not appear in older type defs
    breadcrumbs: { enabled: !isLargeFile },
    folding: !isLargeFile,
    foldingStrategy: "indentation",
    scrollBeyondLastLine: false,
    automaticLayout: true,
    tabSize: tabSize ?? 2,
    wordWrap: "off",
    lineNumbers: "on",
    renderWhitespace: isLargeFile ? "none" : "selection",
    bracketPairColorization: { enabled: !isLargeFile },
    cursorBlinking: isLargeFile ? "blink" : "smooth",
    smoothScrolling: !isLargeFile,
    padding: { top: 8 },
    inlineSuggest: { enabled: (aiCompletionEnabled) && !isLargeFile },
    links: !isLargeFile,
    colorDecorators: !isLargeFile,
    matchBrackets: isLargeFile ? "never" : "always",
    occurrencesHighlight: isLargeFile ? "off" : "singleFile",
    renderLineHighlight: isLargeFile ? "none" : "line",
    suggestOnTriggerCharacters: !isLargeFile,
    quickSuggestions: isLargeFile ? false : undefined,
    largeFileOptimizations: true,
    glyphMargin: true,
  };
}
