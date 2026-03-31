import React, { useState, useEffect, useCallback, useMemo, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type { SidebarView } from "../types";
import { GAMEDEV_LIVE_VIEW_NAME, GAMEDEV_LIVE_VIEW_PATH } from "./gamedevLiveView";
import NewProjectWizard from "./NewProjectWizard";
import { summarizePlanengineMarkdown } from "../planengine/summary";

interface ShadowProjectInfo {
  name: string;
  runtime: string;
  entry_scene: string;
  entry_scene_path: string;
  game_library_name: string;
  game_library_path: string;
  game_library_exists: boolean;
  compiler: string;
  standard: string;
  include_dirs: string[];
  defines: string[];
  link_libs: string[];
  scenes: string[];
  source_file_count: number;
  header_file_count: number;
  has_reflection_json: boolean;
  has_reflection_generated_cpp: boolean;
  has_compile_commands: boolean;
  build_system: string;
}

interface ShadowBuildResult {
  success: boolean;
  output: string;
  duration_ms: number;
}

interface ShadowAssetItem {
  name: string;
  path: string;
  kind: string;
  size_bytes: number;
  sub_dir: string;
}

interface ShadowSourceFile {
  path: string;
  kind: string;
  size_bytes: number;
}

interface ShadowReflectRaw {
  component_count: number;
  headers_scanned: number;
  json: string;
  generated_cpp_path?: string;
}

interface ShadowReflectResult {
  components: ShadowComponent[];
  header_count: number;
  component_count: number;
  generated_cpp_path?: string;
}

interface ShadowComponent {
  name: string;
  properties: ShadowProperty[];
}

interface ShadowProperty {
  name: string;
  ty: string;
  meta?: string[];
}

interface ShadowSceneEntity {
  id: string;
  name: string;
  components: ShadowSceneComponent[];
}

interface ShadowSceneComponent {
  component_type: string;
  fields: [string, string][];
}

interface ShadowScene {
  scene_name: string;
  version: string;
  runtime: string;
  entities: ShadowSceneEntity[];
}

interface ShadowSceneValidationIssue {
  severity: "error" | "warning" | "info";
  entity: string;
  component_type: string;
  message: string;
}

interface ShadowSceneValidationReport {
  scene_path: string;
  issue_count: number;
  issues: ShadowSceneValidationIssue[];
}

interface ShadowRuntimeStatus {
  project_path: string;
  library_path: string;
  library_exists: boolean;
  is_live: boolean;
  status_line: string;
  frame_index: number;
  component_count: number;
  entity_count: number;
  entry_scene_path: string;
  last_scene_path: string;
  last_error?: string | null;
}

interface ShadowSuggestion {
  entity_id: string;
  entity: string;
  message: string;
  kind: "warning" | "info" | "tip";
  action_label?: string | null;
  action_component_type?: string | null;
}

interface LocalLoaderModel {
  name: string;
  path: string;
  model_type: string;
  size_bytes: number;
}

interface LoaderServerStatus {
  running: boolean;
  port: number;
  model: string;
  binary: string;
  backend: string;
  error?: string;
}

interface LoaderEngineInfo {
  installed: boolean;
  binary_path: string;
  version: string;
  backend: string;
}

type ShadowAiHistoryEntry = Record<string, unknown>;

interface ShadowPlanengineDocs {
  plan_path: string;
  finish_path: string;
  plan_markdown: string;
  finish_markdown: string;
  finish_available: boolean;
}

interface ShadowMarkdownSection {
  heading: string;
  body: string;
  level: number;
}

interface ShadowRoadmapPhaseCard {
  key: string;
  heading: string;
  status: string;
  accent: string;
  description: string;
  actionLabel: string;
  section: ShadowMarkdownSection;
  highlights: string[];
  counts: {
    done: number;
    partial: number;
    pending: number;
  };
}

interface Props {
  projectPath?: string;
  visible?: boolean;
  onOpenFile?: (path: string, name: string) => void;
  onActivatePanel?: (view: SidebarView) => void;
  viewportDockVisible?: boolean;
  onViewportDockToggle?: () => void;
  onProjectCreated?: (projectPath: string) => void;
}

const KIND_COLORS: Record<string, string> = {
  Mesh: "#7eb8d4",
  Texture: "#b8a7d4",
  Audio: "#7ed4a7",
  Font: "#d4c27e",
  Tilemap: "#8fd8c1",
  Scene: "#e9aa5f",
  Material: "#d47e7e",
  Shader: "#7ed4d4",
  Data: "#aaaaaa",
  Other: "#666",
};

const SOURCE_KIND_COLORS: Record<string, string> = {
  header: "#7eb8d4",
  source: "#7ed4a7",
  other: "#8eb5c4",
};

const SUGGESTION_COLORS = {
  warning: "#f87171",
  info: "#7eb8d4",
  tip: "#7ed4a7",
};

const SUGGESTION_ICONS = {
  warning: "⚠",
  info: "ℹ",
  tip: "💡",
};

const TABS = ["overview", "scene", "code", "assets", "reflect", "build", "ai", "plan"] as const;
type ActiveTab = (typeof TABS)[number];

function isActiveTab(value: string): value is ActiveTab {
  return (TABS as readonly string[]).includes(value);
}

interface WorkspaceFsChangeEvent {
  kind: string;
  paths: string[];
  dir: string;
}

const PLANENGINE_RECOMMENDED_MODELS = [
  { name: "Qwen/Qwen2.5-Coder-7B-Instruct-GGUF", role: "Fast local autocomplete / ghost-text candidate in the built-in loader." },
  { name: "deepseek-ai/deepseek-coder-6.7b-instruct-GGUF", role: "Balanced local coding model for chat, refactor, and fixes." },
  { name: "microsoft/Phi-4-mini-instruct-gguf", role: "Compact reasoning model for offline coding help." },
  { name: "NousResearch/Hermes-3-Llama-3.1-8B-GGUF", role: "General local assistant option for mixed project work." },
] as const;

const COMMON_COMPONENT_TYPES = [
  "Transform",
  "MeshRenderer",
  "SpriteRenderer",
  "RigidBody",
  "Collider",
  "PlayerController",
  "Lifetime",
  "Health",
] as const;

const ROADMAP_PHASE_META = [
  {
    key: "phase-1",
    heading: "Phase 1",
    status: "Foundation",
    accent: "#7ed4a7",
    description: "Editor shell, viewport foundation, reflection, and authoring core.",
    actionLabel: "Open Overview",
  },
  {
    key: "phase-1-5",
    heading: "Phase 1.5",
    status: "Live Preview",
    accent: "#8eb5c4",
    description: "Terrain iteration and live preview connect directly to the viewport workflow.",
    actionLabel: "Open Live View",
  },
  {
    key: "phase-2",
    heading: "Phase 2",
    status: "Core Features",
    accent: "#e9aa5f",
    description: "Runtime build, hot reload, terrain, and gameplay systems drive the day-to-day loop.",
    actionLabel: "Open Build",
  },
  {
    key: "phase-3",
    heading: "Phase 3",
    status: "Rendering + AI",
    accent: "#9fb7ff",
    description: "Advanced rendering and AI generation belong next to coding and playtesting.",
    actionLabel: "Open AI",
  },
  {
    key: "phase-4",
    heading: "Phase 4",
    status: "Ship + Ecosystem",
    accent: "#c9a8ff",
    description: "Packaging, plugins, and ecosystem work extend ShadowIDE beyond one project.",
    actionLabel: "Open Plugins",
  },
] as const;

function resolveProjectPath(projectPath: string, relativePath: string): string {
  if (relativePath.startsWith("/") || /^[A-Za-z]:[\\/]/.test(relativePath)) {
    return relativePath;
  }
  const base = projectPath.replace(/[\\/]+$/, "");
  return `${base}/${relativePath}`.replace(/\\/g, "/");
}

function formatBytes(b: number): string {
  if (b === 0) return "—";
  if (b < 1024) return `${b} B`;
  if (b < 1024 * 1024) return `${(b / 1024).toFixed(1)} KB`;
  return `${(b / (1024 * 1024)).toFixed(1)} MB`;
}

function normalizeFsPath(path: string): string {
  return path.replace(/\\/g, "/").replace(/\/+/g, "/");
}

function isAutoBuildTriggerPath(relativePath: string): boolean {
  const normalized = relativePath.replace(/^\.\/+/, "");
  if (normalized === ".shadow_project.toml" || normalized === "CMakeLists.txt" || normalized === "build.ninja") {
    return true;
  }
  if (!(normalized.startsWith("src/") || normalized.startsWith("game/"))) {
    return false;
  }
  return /\.(h|hpp|cpp|cxx|cc)$/i.test(normalized);
}

function isProjectDataPath(relativePath: string): boolean {
  const normalized = relativePath.replace(/^\.\/+/, "");
  return normalized.startsWith("assets/") || normalized.startsWith("scenes/") || normalized === ".shadow_project.toml";
}

function normalizeReflectResult(raw: ShadowReflectRaw): ShadowReflectResult {
  let components: ShadowComponent[] = [];
  try {
    const parsed = JSON.parse(raw.json) as {
      components?: Array<{
        name?: string;
        properties?: Array<{
          name?: string;
          ty?: string;
          meta?: string[] | string;
          metadata?: string[] | string;
        }>;
      }>;
    };
    components = (parsed.components ?? []).map((comp) => ({
      name: comp.name ?? "UnnamedComponent",
      properties: (comp.properties ?? []).map((prop) => {
        const metaSource = prop.meta ?? prop.metadata;
        const meta = Array.isArray(metaSource)
          ? metaSource.map((item) => String(item))
          : typeof metaSource === "string"
            ? metaSource.split(",").map((item) => item.trim()).filter(Boolean)
            : [];
        return {
          name: prop.name ?? "field",
          ty: prop.ty ?? "unknown",
          meta,
        };
      }),
    }));
  } catch {
    components = [];
  }

  return {
    components,
    header_count: raw.headers_scanned,
    component_count: raw.component_count,
    generated_cpp_path: raw.generated_cpp_path,
  };
}

function parseMarkdownSections(markdown: string): ShadowMarkdownSection[] {
  const lines = markdown.replace(/\r/g, "").split("\n");
  const sections: ShadowMarkdownSection[] = [];
  let currentHeading = "Overview";
  let currentLevel = 1;
  let currentBody: string[] = [];

  const pushSection = () => {
    const body = currentBody.join("\n").trim();
    if (currentHeading.trim() || body) {
      sections.push({
        heading: currentHeading.trim() || "Overview",
        body,
        level: currentLevel,
      });
    }
  };

  for (const line of lines) {
    const headingMatch = line.match(/^(#{1,6})\s+(.*)$/);
    if (headingMatch) {
      pushSection();
      currentLevel = headingMatch[1].length;
      currentHeading = headingMatch[2].trim();
      currentBody = [];
      continue;
    }
    currentBody.push(line);
  }

  pushSection();
  return sections.filter((section) => section.heading || section.body);
}

function normalizePlanHeading(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9]+/g, " ").trim();
}

function stripPlanMarkdown(value: string): string {
  return value
    .replace(/`([^`]+)`/g, "$1")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/~~([^~]+)~~/g, "$1")
    .replace(/^\d+[a-z]?\.\s*/, "")
    .replace(/^[-*]\s*/, "")
    .replace(/\s+â€”\s+/g, " — ")
    .trim();
}

function findPlanSection(sections: ShadowMarkdownSection[], heading: string): ShadowMarkdownSection | null {
  const query = normalizePlanHeading(heading);
  return sections.find((section) => normalizePlanHeading(section.heading).includes(query)) ?? null;
}

function extractPlanListItems(body: string, limit?: number): string[] {
  const items = body
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => /^[-*]\s+/.test(line) || /^\d+[a-z]?\.\s+/.test(line))
    .map(stripPlanMarkdown)
    .filter(Boolean);
  return typeof limit === "number" ? items.slice(0, limit) : items;
}

function extractPendingPlanItems(body: string, limit?: number): string[] {
  const items = body
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => /^[-*]\s+/.test(line) || /^\d+[a-z]?\.\s+/.test(line))
    .filter((line) => !(line.includes("✅") || line.includes("âœ…") || line.includes("~~")))
    .map(stripPlanMarkdown)
    .filter(Boolean);
  return typeof limit === "number" ? items.slice(0, limit) : items;
}

function parsePlanTable(body: string): { headers: string[]; rows: string[][] } | null {
  const lines = body
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => line.startsWith("|") && line.endsWith("|"));
  if (lines.length < 2) {
    return null;
  }
  const parseCells = (line: string) =>
    line
      .split("|")
      .slice(1, -1)
      .map((cell) => stripPlanMarkdown(cell.trim()));
  const headers = parseCells(lines[0]);
  const rows = lines.slice(2).map(parseCells).filter((row) => row.length === headers.length);
  if (!headers.length || !rows.length) {
    return null;
  }
  return { headers, rows };
}

function countRoadmapStates(body: string): { done: number; partial: number; pending: number } {
  const counts = { done: 0, partial: 0, pending: 0 };
  for (const line of body.split("\n").map((item) => item.trim())) {
    if (!/^[-*]\s+/.test(line)) {
      continue;
    }
    if (line.includes("✅") || line.includes("âœ…")) {
      counts.done += 1;
    } else if (line.includes("🔶") || line.includes("ðŸ”¶")) {
      counts.partial += 1;
    } else if (line.includes("⬜") || line.includes("â¬œ")) {
      counts.pending += 1;
    }
  }
  return counts;
}

function pickPrimaryFile(files: ShadowSourceFile[], preferred: string[]): ShadowSourceFile | null {
  for (const candidate of preferred) {
    const found = files.find((file) => file.path === candidate);
    if (found) return found;
  }
  return files[0] ?? null;
}

function sceneFieldDraftKey(entityId: string, componentType: string, fieldName: string): string {
  return `${entityId}::${componentType}::${fieldName}`;
}

function sceneEntityDraftKey(entityId: string): string {
  return `${entityId}::name`;
}

export default function GameDevPanel({ projectPath, visible, onOpenFile, onActivatePanel, viewportDockVisible = false, onViewportDockToggle, onProjectCreated }: Props) {
  const [projectInfo, setProjectInfo] = useState<ShadowProjectInfo | null>(null);
  const [assets, setAssets] = useState<ShadowAssetItem[]>([]);
  const [sourceFiles, setSourceFiles] = useState<ShadowSourceFile[]>([]);
  const [buildLog, setBuildLog] = useState<string>("");
  const [building, setBuilding] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [activeTab, setActiveTab] = useState<ActiveTab>("overview");
  const [kindFilter, setKindFilter] = useState<string>("All");
  const [assetSearch, setAssetSearch] = useState<string>("");
  const [reflectResult, setReflectResult] = useState<ShadowReflectResult | null>(null);
  const [reflecting, setReflecting] = useState(false);
  const [showWizard, setShowWizard] = useState(false);

  useEffect(() => {
    const unlistenPromise = listen<{ tab?: string }>("shadow-gamedev-focus-tab", (event) => {
      const nextTab = event.payload?.tab;
      if (nextTab && isActiveTab(nextTab)) {
        setActiveTab(nextTab);
      }
    });
    return () => {
      unlistenPromise.then((unlisten) => unlisten());
    };
  }, []);
  const [entryScene, setEntryScene] = useState<ShadowScene | null>(null);
  const [sceneValidation, setSceneValidation] = useState<ShadowSceneValidationReport | null>(null);
  const [suggestions, setSuggestions] = useState<ShadowSuggestion[]>([]);
  const [generatingCC, setGeneratingCC] = useState(false);
  const [ccMsg, setCcMsg] = useState<string | null>(null);
  const [loaderServerStatus, setLoaderServerStatus] = useState<LoaderServerStatus | null>(null);
  const [loaderModels, setLoaderModels] = useState<LocalLoaderModel[]>([]);
  const [loaderEngineInfo, setLoaderEngineInfo] = useState<LoaderEngineInfo | null>(null);
  const [loaderInstalledBackends, setLoaderInstalledBackends] = useState<string[]>([]);
  const [loadingLoaderState, setLoadingLoaderState] = useState(false);
  const [loaderStateError, setLoaderStateError] = useState<string | null>(null);
  const [aiContext, setAiContext] = useState<string>("");
  const [aiHistoryCount, setAiHistoryCount] = useState(0);
  const [loadingAi, setLoadingAi] = useState(false);
  const [aiError, setAiError] = useState<string | null>(null);
  const [planDocs, setPlanDocs] = useState<ShadowPlanengineDocs | null>(null);
  const [loadingPlanDocs, setLoadingPlanDocs] = useState(false);
  const [planDocsError, setPlanDocsError] = useState<string | null>(null);
  const [planSearch, setPlanSearch] = useState("");
  const [runtimeStatus, setRuntimeStatus] = useState<ShadowRuntimeStatus | null>(null);
  const [runtimeBusy, setRuntimeBusy] = useState(false);
  const [runtimePlaying, setRuntimePlaying] = useState(false);
  const [runtimeDelta, setRuntimeDelta] = useState("0.0167");
  const [assetImporting, setAssetImporting] = useState(false);
  const [assetImportMsg, setAssetImportMsg] = useState<string | null>(null);
  const [autoBuildEnabled, setAutoBuildEnabled] = useState(true);
  const [autoBuildStatus, setAutoBuildStatus] = useState<string | null>(null);
  const [queuedAutoBuildReason, setQueuedAutoBuildReason] = useState<string | null>(null);
  const [selectedEntityId, setSelectedEntityId] = useState<string | null>(null);
  const [sceneDrafts, setSceneDrafts] = useState<Record<string, string>>({});
  const [sceneBusy, setSceneBusy] = useState(false);
  const [sceneStatus, setSceneStatus] = useState<string | null>(null);
  const [newEntityName, setNewEntityName] = useState("");
  const [newComponentType, setNewComponentType] = useState("");
  const autoBuildTimerRef = useRef<number | null>(null);
  const dataRefreshTimerRef = useRef<number | null>(null);

  const load = useCallback(async () => {
    if (!projectPath) return;
    setLoadError(null);
    try {
      let info = await invoke<ShadowProjectInfo>("shadow_get_project_info", { projectPath });
      if (!info.has_compile_commands) {
        try {
          const msg = await invoke<string>("shadow_generate_compile_commands", { projectPath });
          setCcMsg(msg);
          info = await invoke<ShadowProjectInfo>("shadow_get_project_info", { projectPath });
        } catch {
          // manual regeneration remains available in the panel
        }
      }
      setProjectInfo(info);

      if (info.entry_scene_path) {
        try {
          const scene = await invoke<ShadowScene>("shadow_parse_scene", { scenePath: resolveProjectPath(projectPath, info.entry_scene_path) });
          setEntryScene(scene);
        } catch {
          setEntryScene(null);
        }
        try {
          const report = await invoke<ShadowSceneValidationReport>("shadow_validate_scene", { projectPath });
          setSceneValidation(report);
        } catch {
          setSceneValidation(null);
        }
      } else {
        setEntryScene(null);
        setSceneValidation(null);
      }

      try {
        const listedSources = await invoke<ShadowSourceFile[]>("shadow_list_source_files", { projectPath });
        setSourceFiles(listedSources);
      } catch {
        setSourceFiles([]);
      }

      try {
        const lastBuild = await invoke<string>("shadow_get_last_build_log", { projectPath });
        setBuildLog(lastBuild);
      } catch {
        setBuildLog("");
      }

      try {
        const s = await invoke<ShadowSuggestion[]>("shadow_inspector_suggestions", { projectPath });
        setSuggestions(s);
      } catch {
        setSuggestions([]);
      }

      try {
        const status = await invoke<ShadowRuntimeStatus>("shadow_runtime_status", { projectPath });
        setRuntimeStatus(status);
      } catch {
        setRuntimeStatus(null);
      }

      if (info.has_reflection_json) {
        try {
          const raw = await invoke<ShadowReflectRaw>("shadow_load_reflection", { projectPath });
          setReflectResult(normalizeReflectResult(raw));
        } catch {
          setReflectResult(null);
        }
      } else {
        setReflectResult(null);
      }
    } catch (e) {
      const msg = String(e);
      if (msg.includes("shadow_project.toml") || msg.includes("No such file") || msg.includes("os error 2")) {
        setLoadError(null);
        setProjectInfo(null);
      } else {
        setLoadError(msg);
      }
      return;
    }

    try {
      const items = await invoke<ShadowAssetItem[]>("shadow_list_assets", { projectPath });
      setAssets(items);
    } catch {
      setAssets([]);
    }

  }, [projectPath]);

  const loadAi = useCallback(async () => {
    if (!projectPath) return;
    setLoadingAi(true);
    setAiError(null);
    try {
      const [context, history] = await Promise.all([
        invoke<string>("shadow_get_ai_context", { rootPath: projectPath }),
        invoke<ShadowAiHistoryEntry[]>("shadow_ai_history_load", { projectPath }),
      ]);
      setAiContext(context);
      setAiHistoryCount(history.length);
    } catch (error) {
      setAiError(String(error));
      setAiContext("");
      setAiHistoryCount(0);
    } finally {
      setLoadingAi(false);
    }
  }, [projectPath]);

  const loadLoaderState = useCallback(async () => {
    setLoadingLoaderState(true);
    setLoaderStateError(null);
    try {
      const statusPromise = invoke<LoaderServerStatus>("get_llm_server_status").catch(() => null);
      const enginePromise = invoke<LoaderEngineInfo>("check_engine", { backend: null }).catch(() => null);
      const installedPromise = invoke<string[]>("list_installed_engines").catch(() => []);
      const projectModelsPromise = projectPath
        ? invoke<LocalLoaderModel[]>("scan_local_models", { basePath: projectPath }).catch(() => [] as LocalLoaderModel[])
        : Promise.resolve([] as LocalLoaderModel[]);

      const [status, engine, installed, projectModels] = await Promise.all([
        statusPromise,
        enginePromise,
        installedPromise,
        projectModelsPromise,
      ]);

      let models = projectModels;
      if (models.length === 0) {
        try {
          const home = await invoke<string>("get_home_dir");
          models = await invoke<LocalLoaderModel[]>("scan_local_models", { basePath: home });
        } catch {
          models = [];
        }
      }

      setLoaderServerStatus(status);
      setLoaderEngineInfo(engine);
      setLoaderInstalledBackends(installed);
      setLoaderModels(models ?? []);
    } catch (error) {
      setLoaderServerStatus(null);
      setLoaderEngineInfo(null);
      setLoaderInstalledBackends([]);
      setLoaderModels([]);
      setLoaderStateError(String(error));
    } finally {
      setLoadingLoaderState(false);
    }
  }, [projectPath]);

  const loadPlanDocs = useCallback(async () => {
    setLoadingPlanDocs(true);
    setPlanDocsError(null);
    try {
      const docs = await invoke<ShadowPlanengineDocs>("shadow_load_planengine_docs");
      setPlanDocs(docs);
    } catch (error) {
      setPlanDocsError(String(error));
      setPlanDocs(null);
    } finally {
      setLoadingPlanDocs(false);
    }
  }, []);

  useEffect(() => {
    setBuildLog("");
    setProjectInfo(null);
    setAssets([]);
    setSourceFiles([]);
    setReflectResult(null);
    setEntryScene(null);
    setSceneValidation(null);
    setSuggestions([]);
    setCcMsg(null);
    setLoaderServerStatus(null);
    setLoaderModels([]);
    setLoaderEngineInfo(null);
    setLoaderInstalledBackends([]);
    setLoadingLoaderState(false);
    setLoaderStateError(null);
    setAiContext("");
    setAiHistoryCount(0);
    setAiError(null);
    setPlanDocs(null);
    setLoadingPlanDocs(false);
    setPlanDocsError(null);
    setPlanSearch("");
    setRuntimeStatus(null);
    setRuntimePlaying(false);
    setAssetImportMsg(null);
    setAutoBuildStatus(null);
    setQueuedAutoBuildReason(null);
    setSelectedEntityId(null);
    setSceneDrafts({});
    setSceneBusy(false);
    setSceneStatus(null);
    setNewEntityName("");
    setNewComponentType("");
    setActiveTab("overview");
  }, [projectPath]);

  useEffect(() => {
    if (visible) {
      load();
    }
  }, [visible, load]);

  useEffect(() => {
    if (!entryScene?.entities.length) {
      setSelectedEntityId(null);
      return;
    }
    setSelectedEntityId((current) => {
      if (current && entryScene.entities.some((entity) => entity.id === current)) {
        return current;
      }
      return entryScene.entities[0]?.id ?? null;
    });
  }, [entryScene]);

  useEffect(() => {
    if (visible && activeTab === "ai" && projectPath) {
      loadAi();
      void loadLoaderState();
    }
  }, [visible, activeTab, projectPath, loadAi, loadLoaderState]);

  useEffect(() => {
    if (visible && projectPath) {
      void loadLoaderState();
    }
  }, [visible, projectPath, loadLoaderState]);

  useEffect(() => {
    if (visible && !loadingPlanDocs && !planDocs) {
      void loadPlanDocs();
    }
  }, [visible, loadingPlanDocs, planDocs, loadPlanDocs]);

  const triggerBuild = useCallback(async (options?: { reason?: string; activateBuildTab?: boolean }) => {
    if (!projectPath) return;
    const reason = options?.reason?.trim();
    setBuilding(true);
    setBuildLog(reason ? `${reason}\n\nBuilding…\n` : "Building…\n");
    if (options?.activateBuildTab ?? true) {
      setActiveTab("build");
    }
    try {
      const result = await invoke<ShadowBuildResult>("shadow_trigger_build", { projectPath });
      const summary = `[${result.success ? "OK" : "FAILED"}] ${result.duration_ms}ms\n\n${result.output}`;
      setBuildLog(reason ? `${reason}\n\n${summary}` : summary);
      setAutoBuildStatus(result.success
        ? reason ?? `Last build finished in ${result.duration_ms}ms.`
        : `${reason ?? "Build failed"} · see build output.`);
    } catch (e) {
      const message = `[ERROR] ${e}`;
      setBuildLog(reason ? `${reason}\n\n${message}` : message);
      setAutoBuildStatus(`${reason ?? "Build failed"} · ${String(e)}`);
    } finally {
      setBuilding(false);
      load();
    }
  }, [projectPath, load]);

  const runHeaderTool = useCallback(async () => {
    if (!projectPath) return;
    setReflecting(true);
    setActiveTab("reflect");
    try {
      const raw = await invoke<ShadowReflectRaw>("shadow_run_header_tool", { projectPath });
      setReflectResult(normalizeReflectResult(raw));
      load();
    } catch (e) {
      setReflectResult({ components: [], header_count: 0, component_count: 0 });
      setBuildLog(`[REFLECT ERROR] ${e}`);
    } finally {
      setReflecting(false);
    }
  }, [projectPath, load]);

  const generateCompileCommands = useCallback(async () => {
    if (!projectPath) return;
    setGeneratingCC(true);
    setCcMsg(null);
    try {
      const msg = await invoke<string>("shadow_generate_compile_commands", { projectPath });
      setCcMsg(msg);
      load();
    } catch (e) {
      setCcMsg(`Error: ${e}`);
    } finally {
      setGeneratingCC(false);
    }
  }, [projectPath, load]);

  const loadRuntime = useCallback(async () => {
    if (!projectPath) return;
    setRuntimeBusy(true);
    try {
      const status = await invoke<ShadowRuntimeStatus>("shadow_runtime_load", {
        projectPath,
        loadEntryScene: true,
      });
      setRuntimeStatus(status);
      load();
    } finally {
      setRuntimeBusy(false);
    }
  }, [projectPath, load]);

  const stopRuntime = useCallback(async () => {
    if (!projectPath) return;
    setRuntimeBusy(true);
    setRuntimePlaying(false);
    try {
      const status = await invoke<ShadowRuntimeStatus>("shadow_runtime_stop", { projectPath });
      setRuntimeStatus(status);
    } finally {
      setRuntimeBusy(false);
    }
  }, [projectPath]);

  const validateScene = useCallback(async () => {
    if (!projectPath) return;
    try {
      const report = await invoke<ShadowSceneValidationReport>("shadow_validate_scene", { projectPath });
      setSceneValidation(report);
    } catch {
      setSceneValidation(null);
    }
  }, [projectPath]);

  const refreshSceneData = useCallback(async (nextScene?: ShadowScene) => {
    if (!projectPath) return;

    if (nextScene) {
      setEntryScene(nextScene);
    } else if (projectInfo?.entry_scene_path) {
      try {
        const scene = await invoke<ShadowScene>("shadow_parse_scene", {
          scenePath: resolveProjectPath(projectPath, projectInfo.entry_scene_path),
        });
        setEntryScene(scene);
      } catch {
        setEntryScene(null);
      }
    }

    try {
      const report = await invoke<ShadowSceneValidationReport>("shadow_validate_scene", { projectPath });
      setSceneValidation(report);
    } catch {
      setSceneValidation(null);
    }

    try {
      const nextSuggestions = await invoke<ShadowSuggestion[]>("shadow_inspector_suggestions", { projectPath });
      setSuggestions(nextSuggestions);
    } catch {
      setSuggestions([]);
    }

    try {
      const status = await invoke<ShadowRuntimeStatus>("shadow_runtime_status", { projectPath });
      setRuntimeStatus(status);
    } catch {
      setRuntimeStatus(null);
    }
  }, [projectInfo?.entry_scene_path, projectPath]);

  const runSceneMutation = useCallback(async (
    successMessage: string,
    action: () => Promise<ShadowScene>,
  ) => {
    setSceneBusy(true);
    setSceneStatus(null);
    try {
      const scene = await action();
      setSceneDrafts({});
      setSceneStatus(successMessage);
      await refreshSceneData(scene);
    } catch (error) {
      setSceneStatus(`Scene update failed: ${String(error)}`);
    } finally {
      setSceneBusy(false);
    }
  }, [refreshSceneData]);

  const addEntityToScene = useCallback(async () => {
    if (!projectPath) return;
    const trimmedName = newEntityName.trim();
    await runSceneMutation(
      `Added ${trimmedName || "new entity"} to the scene.`,
      () => invoke<ShadowScene>("shadow_scene_add_entity", {
        projectPath,
        name: trimmedName || null,
      }),
    );
    setNewEntityName("");
  }, [newEntityName, projectPath, runSceneMutation]);

  const removeEntityFromScene = useCallback(async (entityId: string, entityName: string) => {
    if (!projectPath) return;
    await runSceneMutation(
      `Removed ${entityName} from the scene.`,
      () => invoke<ShadowScene>("shadow_scene_remove_entity", {
        projectPath,
        entityId,
      }),
    );
  }, [projectPath, runSceneMutation]);

  const renameSceneEntity = useCallback(async (entityId: string, name: string) => {
    if (!projectPath || !name.trim()) return;
    await runSceneMutation(
      `Renamed entity to ${name.trim()}.`,
      () => invoke<ShadowScene>("shadow_scene_set_entity_name", {
        projectPath,
        entityId,
        name,
      }),
    );
  }, [projectPath, runSceneMutation]);

  const addComponentToEntity = useCallback(async (entityId: string, componentType: string) => {
    if (!projectPath || !componentType.trim()) return;
    await runSceneMutation(
      `Added ${componentType} to the selected entity.`,
      () => invoke<ShadowScene>("shadow_scene_add_component", {
        projectPath,
        entityId,
        componentType,
      }),
    );
    setNewComponentType("");
  }, [projectPath, runSceneMutation]);

  const removeComponentFromEntity = useCallback(async (entityId: string, componentType: string) => {
    if (!projectPath) return;
    await runSceneMutation(
      `Removed ${componentType} from the selected entity.`,
      () => invoke<ShadowScene>("shadow_scene_remove_component", {
        projectPath,
        entityId,
        componentType,
      }),
    );
  }, [projectPath, runSceneMutation]);

  const commitSceneField = useCallback(async (
    entityId: string,
    componentType: string,
    fieldName: string,
    value: string,
  ) => {
    if (!projectPath) return;
    await runSceneMutation(
      `Updated ${componentType}.${fieldName}.`,
      () => invoke<ShadowScene>("shadow_scene_set_component_field", {
        projectPath,
        entityId,
        componentType,
        fieldName,
        value,
      }),
    );
  }, [projectPath, runSceneMutation]);

  const applySuggestion = useCallback(async (suggestion: ShadowSuggestion) => {
    if (!suggestion.action_component_type || !suggestion.entity_id) return;
    await addComponentToEntity(suggestion.entity_id, suggestion.action_component_type);
  }, [addComponentToEntity]);

  const stepRuntime = useCallback(async (deltaOverride?: number) => {
    if (!projectPath) return;
    const parsedDelta = Number.parseFloat(runtimeDelta);
    const deltaTime = Number.isFinite(deltaOverride) ? deltaOverride : (Number.isFinite(parsedDelta) ? parsedDelta : 1 / 60);
    try {
      const status = await invoke<ShadowRuntimeStatus>("shadow_runtime_step", {
        projectPath,
        deltaTime,
      });
      setRuntimeStatus(status);
    } catch {
      // handled by backend status payload on the next refresh
    }
  }, [projectPath, runtimeDelta]);

  const saveRuntimeScene = useCallback(async () => {
    if (!projectPath) return;
    setRuntimeBusy(true);
    try {
      const status = await invoke<ShadowRuntimeStatus>("shadow_runtime_save_scene", { projectPath });
      setRuntimeStatus(status);
      validateScene();
      load();
    } finally {
      setRuntimeBusy(false);
    }
  }, [projectPath, load, validateScene]);

  const importAssets = useCallback(async () => {
    if (!projectPath) return;
    const picked = await open({
      multiple: true,
      directory: false,
      title: "Import assets into ShadowEditor project",
      filters: [
        { name: "Assets", extensions: ["gltf", "glb", "obj", "fbx", "dae", "blend", "png", "jpg", "jpeg", "webp", "exr", "hdr", "ktx2", "dds", "tga", "wav", "ogg", "mp3", "flac", "ttf", "otf", "woff", "woff2", "wgsl", "glsl", "vert", "frag", "json", "shadow_mat", "tmx", "tsx", "ldtk"] },
      ],
    });
    const selected = Array.isArray(picked) ? picked : (typeof picked === "string" ? [picked] : []);
    if (selected.length === 0) return;

    setAssetImporting(true);
    setAssetImportMsg(null);
    try {
      const imported = await invoke<ShadowAssetItem[]>("shadow_import_assets", {
        projectPath,
        sourcePaths: selected,
      });
      setAssetImportMsg(imported.length > 0 ? `Imported ${imported.length} asset${imported.length > 1 ? "s" : ""}.` : "No files were imported.");
      load();
    } catch (error) {
      setAssetImportMsg(`Import failed: ${error}`);
    } finally {
      setAssetImporting(false);
    }
  }, [projectPath, load]);

  const toggleRuntimePlay = useCallback(async () => {
    if (runtimePlaying) {
      setRuntimePlaying(false);
      return;
    }
    if (!runtimeStatus?.is_live) {
      await loadRuntime();
    }
    setRuntimePlaying(true);
  }, [runtimePlaying, runtimeStatus?.is_live, loadRuntime]);

  useEffect(() => {
    if (!runtimePlaying || !runtimeStatus?.is_live) return;
    const timer = window.setInterval(() => {
      void stepRuntime();
    }, 100);
    return () => window.clearInterval(timer);
  }, [runtimePlaying, runtimeStatus?.is_live, stepRuntime]);

  useEffect(() => {
    if (!runtimeStatus?.is_live && runtimePlaying) {
      setRuntimePlaying(false);
    }
  }, [runtimeStatus?.is_live, runtimePlaying]);

  useEffect(() => {
    if (!projectPath || !visible) return;

    invoke("watch_workspace", { rootPath: projectPath }).catch(() => {});

    const normalizedProjectPath = normalizeFsPath(projectPath).replace(/\/+$/, "");
    const unlistenPromise = listen<WorkspaceFsChangeEvent>("workspace-fs-changed", (event) => {
      const relevantRelativePaths = event.payload.paths
        .map((path) => normalizeFsPath(path))
        .filter((path) => path === normalizedProjectPath || path.startsWith(`${normalizedProjectPath}/`))
        .map((path) => path === normalizedProjectPath ? "" : path.slice(normalizedProjectPath.length + 1))
        .filter((path) => {
          if (!path) return false;
          return !path.startsWith("build/") &&
            !path.startsWith(".shadoweditor/") &&
            path !== "compile_commands.json";
        });

      if (relevantRelativePaths.length === 0) {
        return;
      }

      const projectDataPaths = relevantRelativePaths.filter(isProjectDataPath);
      if (projectDataPaths.length > 0) {
        if (dataRefreshTimerRef.current) {
          window.clearTimeout(dataRefreshTimerRef.current);
        }
        dataRefreshTimerRef.current = window.setTimeout(() => {
          void load();
        }, 350);
      }

      if (!autoBuildEnabled) {
        return;
      }

      const buildTriggerPaths = relevantRelativePaths.filter(isAutoBuildTriggerPath);
      if (buildTriggerPaths.length === 0) {
        return;
      }

      if (autoBuildTimerRef.current) {
        window.clearTimeout(autoBuildTimerRef.current);
      }
      autoBuildTimerRef.current = window.setTimeout(() => {
        const preview = buildTriggerPaths.slice(0, 2).join(", ");
        const suffix = buildTriggerPaths.length > 2 ? ` +${buildTriggerPaths.length - 2} more` : "";
        const reason = `Auto-build: ${preview}${suffix}`;
        setAutoBuildStatus(`${reason} · waiting for compile.`);
        if (building) {
          setQueuedAutoBuildReason(reason);
          return;
        }
        void triggerBuild({ reason, activateBuildTab: false });
      }, 700);
    });

    return () => {
      unlistenPromise.then((unlisten) => unlisten());
      if (autoBuildTimerRef.current) {
        window.clearTimeout(autoBuildTimerRef.current);
        autoBuildTimerRef.current = null;
      }
      if (dataRefreshTimerRef.current) {
        window.clearTimeout(dataRefreshTimerRef.current);
        dataRefreshTimerRef.current = null;
      }
    };
  }, [autoBuildEnabled, building, load, projectPath, triggerBuild, visible]);

  useEffect(() => {
    if (!autoBuildEnabled) {
      setQueuedAutoBuildReason(null);
      return;
    }
    if (building || !queuedAutoBuildReason) return;
    const reason = queuedAutoBuildReason;
    setQueuedAutoBuildReason(null);
    void triggerBuild({ reason, activateBuildTab: false });
  }, [autoBuildEnabled, building, queuedAutoBuildReason, triggerBuild]);

  const openProjectFile = useCallback((relativePath: string) => {
    if (!projectPath || !relativePath) return;
    const path = resolveProjectPath(projectPath, relativePath);
    const name = relativePath.split("/").pop() || path.split("/").pop() || relativePath;
    onOpenFile?.(path, name);
  }, [onOpenFile, projectPath]);

  const openLiveView = useCallback(() => {
    onOpenFile?.(GAMEDEV_LIVE_VIEW_PATH, GAMEDEV_LIVE_VIEW_NAME);
  }, [onOpenFile]);

  const toggleViewportDock = useCallback(() => {
    onViewportDockToggle?.();
  }, [onViewportDockToggle]);

  const openAiPanel = useCallback(() => {
    onActivatePanel?.("ai");
  }, [onActivatePanel]);

  const openPluginsPanel = useCallback(() => {
    onActivatePanel?.("plugins");
  }, [onActivatePanel]);

  const openLlmLoaderPanel = useCallback(() => {
    onActivatePanel?.("llmloader");
  }, [onActivatePanel]);

  const openPlanenginePanel = useCallback(() => {
    onActivatePanel?.("planengine");
  }, [onActivatePanel]);

  const noProject = !projectPath || (!projectInfo && !loadError);
  const allKinds = ["All", ...Array.from(new Set(assets.map((a) => a.kind))).sort()];
  const filteredAssets = assets.filter((a) => {
    if (kindFilter !== "All" && a.kind !== kindFilter) return false;
    if (assetSearch && !a.name.toLowerCase().includes(assetSearch.toLowerCase())) return false;
    return true;
  });
  const preferredAutocompleteModel = loaderModels.find((model) => /coder|code|qwen/i.test(model.name) && /7b|8b/i.test(model.name))
    ?? loaderModels.find((model) => /coder|code/i.test(model.name))
    ?? loaderModels[0];
  const preferredChatModel = [...loaderModels].reverse().find((model) => /coder|code|phi|deepseek|hermes|llama/i.test(model.name))
    ?? loaderModels[0];
  const primaryHeader = pickPrimaryFile(sourceFiles.filter((file) => file.kind === "header"), ["src/game.h", "game/game.h"]);
  const primarySource = pickPrimaryFile(sourceFiles.filter((file) => file.kind === "source"), ["src/game.cpp", "game/game.cpp"]);
  const selectedEntity = entryScene?.entities.find((entity) => entity.id === selectedEntityId) ?? entryScene?.entities[0] ?? null;
  const availableComponentTypes = Array.from(new Set([
    ...COMMON_COMPONENT_TYPES,
    ...(reflectResult?.components.map((component) => component.name) ?? []),
  ])).sort((a, b) => a.localeCompare(b));
  const selectedEntityComponentTypes = new Set(selectedEntity?.components.map((component) => component.component_type) ?? []);
  const addableComponentTypes = availableComponentTypes.filter((componentType) => !selectedEntityComponentTypes.has(componentType));
  const selectedEntitySuggestions = suggestions.filter((suggestion) => suggestion.entity_id === selectedEntity?.id);
  const planSections = useMemo(() => parseMarkdownSections(planDocs?.plan_markdown ?? ""), [planDocs?.plan_markdown]);
  const finishSections = useMemo(() => parseMarkdownSections(planDocs?.finish_markdown ?? ""), [planDocs?.finish_markdown]);
  const normalizedPlanSearch = planSearch.trim().toLowerCase();
  const visiblePlanSections = planSections.filter((section) => {
    if (!normalizedPlanSearch) return true;
    return section.heading.toLowerCase().includes(normalizedPlanSearch)
      || section.body.toLowerCase().includes(normalizedPlanSearch);
  });
  const visibleFinishSections = finishSections.filter((section) => {
    if (!normalizedPlanSearch) return true;
    return section.heading.toLowerCase().includes(normalizedPlanSearch)
      || section.body.toLowerCase().includes(normalizedPlanSearch);
  });
  const planAuditStamp = useMemo(() => {
    const match = (planDocs?.plan_markdown ?? "").match(/\*\*Last audited:\*\*\s*([^\n]+)/i);
    return match ? stripPlanMarkdown(match[1]) : null;
  }, [planDocs?.plan_markdown]);
  const planVisionSection = useMemo(() => findPlanSection(planSections, "Vision & North Star"), [planSections]);
  const planVisionHighlights = useMemo(() => extractPlanListItems(planVisionSection?.body ?? "", 5), [planVisionSection?.body]);
  const planSummarySection = useMemo(() => findPlanSection(planSections, "Summary"), [planSections]);
  const planAuditTable = useMemo(() => parsePlanTable(planSummarySection?.body ?? ""), [planSummarySection?.body]);
  const planAuditTotals = useMemo(() => {
    if (!planAuditTable) {
      return null;
    }
    const totalRow =
      planAuditTable.rows.find((row) => normalizePlanHeading(row[0]).includes("total"))
      ?? planAuditTable.rows[planAuditTable.rows.length - 1];
    if (!totalRow || totalRow.length < 5) {
      return null;
    }
    const parseCount = (value: string) => {
      const parsed = Number.parseInt(value.replace(/[^0-9-]/g, ""), 10);
      return Number.isFinite(parsed) ? parsed : 0;
    };
    return {
      done: parseCount(totalRow[1]),
      partial: parseCount(totalRow[2]),
      pending: parseCount(totalRow[3]),
      total: parseCount(totalRow[4]),
    };
  }, [planAuditTable]);
  const planNextStepsSection = useMemo(() => findPlanSection(planSections, "Recommended Next Steps"), [planSections]);
  const planPendingNextSteps = useMemo(() => extractPendingPlanItems(planNextStepsSection?.body ?? "", 5), [planNextStepsSection?.body]);
  const planShellSummary = useMemo(() => summarizePlanengineMarkdown(planDocs?.plan_markdown ?? ""), [planDocs?.plan_markdown]);
  const roadmapPhaseCards = useMemo<ShadowRoadmapPhaseCard[]>(() => {
    const phases: ShadowRoadmapPhaseCard[] = [];
    for (const phase of ROADMAP_PHASE_META) {
      const section = findPlanSection(planSections, phase.heading);
      if (!section) {
        continue;
      }
      phases.push({
        ...phase,
        section,
        highlights: extractPlanListItems(section.body, 3),
        counts: countRoadmapStates(section.body),
      });
    }
    return phases;
  }, [planSections]);

  const runRoadmapPhaseAction = useCallback((phaseKey: string) => {
    switch (phaseKey) {
      case "phase-1":
        setActiveTab("overview");
        break;
      case "phase-1-5":
        openLiveView();
        break;
      case "phase-2":
        setActiveTab("build");
        break;
      case "phase-3":
        setActiveTab("ai");
        openAiPanel();
        break;
      case "phase-4":
        openPluginsPanel();
        break;
      default:
        openPlanenginePanel();
        break;
    }
  }, [openAiPanel, openLiveView, openPlanenginePanel, openPluginsPanel]);
  const planRecommendedWorkflow = useMemo<"build" | "reflect" | "live" | "ai" | "plan">(() => {
    if (!projectInfo?.game_library_exists) {
      return "build";
    }
    if (!(projectInfo.has_reflection_json && projectInfo.has_reflection_generated_cpp) || !projectInfo.has_compile_commands) {
      return "reflect";
    }
    if (!runtimeStatus?.is_live) {
      return "live";
    }
    if (planShellSummary.criticalGaps.some((gap) => /ai|llm|autocomplete/i.test(gap.gap))) {
      return "ai";
    }
    return "plan";
  }, [
    planShellSummary.criticalGaps,
    projectInfo?.game_library_exists,
    projectInfo?.has_compile_commands,
    projectInfo?.has_reflection_generated_cpp,
    projectInfo?.has_reflection_json,
    runtimeStatus?.is_live,
  ]);

  if (noProject && !loadError) {
    return (
      <>
        <div style={styles.empty}>
          <div style={styles.emptyIcon}>
            <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="#e9aa5f" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
              <rect x="2" y="6" width="20" height="12" rx="3" />
              <circle cx="8" cy="12" r="1.5" fill="#e9aa5f" />
              <path d="M16 10v4M14 12h4" />
            </svg>
          </div>
          <div style={styles.emptyTitle}>ShadowEditor</div>
          <div style={styles.emptyDesc}>
            Open a ShadowEditor project folder to get started.<br />
            The folder must contain a <code style={{ color: "#e9aa5f" }}>.shadow_project.toml</code> file.
          </div>
          <button style={styles.newProjectBtn} onClick={() => setShowWizard(true)}>
            + New Project
          </button>
        </div>
        {showWizard && (
          <NewProjectWizard
            onClose={() => setShowWizard(false)}
            onCreated={(path) => {
              setShowWizard(false);
              onProjectCreated?.(path);
              onActivatePanel?.("planengine");
            }}
          />
        )}
      </>
    );
  }

  if (loadError) {
    return (
      <div style={styles.empty}>
        <div style={{ color: "#f87171", marginBottom: 8 }}>Failed to load project</div>
        <div style={{ color: "#888", fontSize: 12 }}>{loadError}</div>
        <button style={styles.btn} onClick={load}>Retry</button>
      </div>
    );
  }

  return (
    <>
      <div style={styles.container}>
        <div style={styles.header}>
          <span style={styles.headerTitle}>{projectInfo?.name ?? "ShadowEditor"}</span>
          <span style={styles.headerKicker}>ShadowEditor (Game Engine)</span>
          <span style={styles.headerSub}>{projectInfo?.runtime ?? ""}</span>
          <button
            style={styles.headerIconBtn}
            title="Run C++ header reflection tool"
            onClick={runHeaderTool}
            disabled={reflecting || !projectInfo}
          >
            {reflecting ? "…" : (
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4z" />
                <path d="M17 14v6M14 17h6" />
              </svg>
            )}
          </button>
          <button
            style={{ ...styles.buildBtn, opacity: (building || !projectInfo) ? 0.4 : 1 }}
            disabled={building || !projectInfo}
            onClick={() => { void triggerBuild(); }}
            title="Build game library (C++23)"
          >
            {building ? "Building…" : "▶ Build"}
          </button>
        </div>

        {projectInfo && (
          <div style={styles.chips}>
            <Chip label={projectInfo.compiler} sub={projectInfo.standard} />
            <Chip label={projectInfo.build_system} />
            <Chip label={projectInfo.has_compile_commands ? "compile_commands ✓" : "compile_commands ✗"} ok={projectInfo.has_compile_commands} />
            <Chip label={projectInfo.has_reflection_json ? "reflect.json ✓" : "reflect.json ✗"} ok={projectInfo.has_reflection_json} />
            <Chip label={projectInfo.has_reflection_generated_cpp ? "reflect.cpp ✓" : "reflect.cpp ✗"} ok={projectInfo.has_reflection_generated_cpp} />
            <Chip label={projectInfo.game_library_exists ? "runtime ✓" : "runtime ✗"} ok={projectInfo.game_library_exists} />
            <Chip label={`${projectInfo.source_file_count + projectInfo.header_file_count} code`} />
            <Chip label={`${assets.length} assets`} />
            {suggestions.length > 0 && <Chip label={`${suggestions.length} suggestion${suggestions.length > 1 ? "s" : ""}`} ok={false} />}
          </div>
        )}

        {!loadingLoaderState && (!loaderServerStatus?.running || loaderInstalledBackends.length === 0) && (
          <div style={styles.loaderBanner}>
            <span>
              {loaderInstalledBackends.length === 0
                ? "Local AI loader is not configured yet."
                : "Local AI loader is stopped."}
            </span>
            <button style={styles.bannerButton} onClick={openLlmLoaderPanel}>
              Open LLM Loader →
            </button>
          </div>
        )}

        <div style={styles.tabs}>
          {TABS.map((tab) => (
            <button
              key={tab}
              style={{ ...styles.tab, ...(activeTab === tab ? styles.tabActive : {}) }}
              onClick={() => setActiveTab(tab)}
            >
              {tab === "ai" ? "AI" : tab === "reflect" ? "Reflect" : tab.charAt(0).toUpperCase() + tab.slice(1)}
            </button>
          ))}
        </div>

        {activeTab === "overview" && projectInfo && (
          <div style={styles.scroll}>
            <Section title="Quick Actions">
              <div style={styles.actionRow}>
                <ActionButton label={viewportDockVisible ? "Hide Viewport" : "Dock Viewport"} onClick={toggleViewportDock} />
                <ActionButton label="Full View" onClick={openLiveView} />
                <ActionButton label="PlanEngine" onClick={() => onActivatePanel?.("planengine")} />
                <ActionButton label="Open Scene" disabled={!projectInfo.entry_scene_path} onClick={() => openProjectFile(projectInfo.entry_scene_path)} />
                <ActionButton label="Open Header" disabled={!primaryHeader} onClick={() => primaryHeader && openProjectFile(primaryHeader.path)} />
                <ActionButton label="Open Source" disabled={!primarySource} onClick={() => primarySource && openProjectFile(primarySource.path)} />
                <ActionButton label="AI Chat" onClick={openAiPanel} />
                <ActionButton label="Config" onClick={() => openProjectFile(".shadow_project.toml")} />
              </div>
            </Section>

            <Section title="PlanEngine Snapshot">
              <div style={styles.actionRow}>
                <ActionButton label="Open Roadmap" onClick={openPlanenginePanel} />
                <ActionButton label="Plan Tab" onClick={() => setActiveTab("plan")} />
                <ActionButton label="Build Runtime" onClick={() => { void triggerBuild({ reason: "PlanEngine roadmap action", activateBuildTab: true }); }} disabled={building} />
                <ActionButton label="Live View" onClick={openLiveView} />
              </div>
              {planAuditStamp && (
                <div style={styles.subtleText}>Roadmap audit: {planAuditStamp}</div>
              )}
              {planAuditTotals ? (
                <div style={styles.roadmapAuditGrid}>
                  <div style={styles.roadmapAuditChip}>
                    <span style={styles.roadmapAuditValue}>{planAuditTotals.done}</span>
                    <span style={styles.roadmapAuditLabel}>Done</span>
                  </div>
                  <div style={styles.roadmapAuditChip}>
                    <span style={styles.roadmapAuditValue}>{planAuditTotals.partial}</span>
                    <span style={styles.roadmapAuditLabel}>Partial</span>
                  </div>
                  <div style={styles.roadmapAuditChip}>
                    <span style={styles.roadmapAuditValue}>{planAuditTotals.pending}</span>
                    <span style={styles.roadmapAuditLabel}>Not Started</span>
                  </div>
                  <div style={styles.roadmapAuditChip}>
                    <span style={styles.roadmapAuditValue}>{planAuditTotals.total}</span>
                    <span style={styles.roadmapAuditLabel}>Total</span>
                  </div>
                </div>
              ) : (
                <div style={styles.placeholder}>PlanEngine audit summary is not loaded yet.</div>
              )}
              {planVisionHighlights.length > 0 && (
                <div style={{ marginTop: 10 }}>
                  <div style={styles.roadmapBlockTitle}>North Star</div>
                  <div style={styles.roadmapInlineList}>
                    {planVisionHighlights.map((item) => (
                      <div key={item} style={styles.roadmapInlineItem}>- {item}</div>
                    ))}
                  </div>
                </div>
              )}
              {planPendingNextSteps.length > 0 && (
                <div style={{ marginTop: 10 }}>
                  <div style={styles.roadmapBlockTitle}>Next Steps</div>
                  <div style={styles.roadmapInlineList}>
                    {planPendingNextSteps.map((item) => (
                      <div key={item} style={styles.roadmapInlineItem}>- {item}</div>
                    ))}
                  </div>
                </div>
              )}
              {(planShellSummary.criticalGaps.length > 0 || planShellSummary.nextSteps[0]) && (
                <div style={{ marginTop: 10 }}>
                  <div style={styles.roadmapBlockTitle}>Execution Pulse</div>
                  <div style={styles.roadmapAuditGrid}>
                    <div style={styles.roadmapAuditChip}>
                      <span style={styles.roadmapAuditValue}>{planShellSummary.criticalGaps.length}</span>
                      <span style={styles.roadmapAuditLabel}>Blocking Gaps</span>
                    </div>
                    <div style={styles.roadmapAuditChip}>
                      <span style={styles.roadmapAuditValue}>{planShellSummary.nextSteps.length}</span>
                      <span style={styles.roadmapAuditLabel}>Tracked Next Steps</span>
                    </div>
                  </div>
                  <div style={styles.roadmapInlineList}>
                    {planShellSummary.nextSteps[0] && (
                      <div style={styles.roadmapInlineItem}>- Next focus: {planShellSummary.nextSteps[0]}</div>
                    )}
                    {planShellSummary.criticalGaps[0] && (
                      <div style={styles.roadmapInlineItem}>- Blocking gap: {planShellSummary.criticalGaps[0].gap}</div>
                    )}
                  </div>
                  <div style={styles.actionRow}>
                    {planRecommendedWorkflow === "build" && (
                      <ActionButton label="Recommended: Build Runtime" onClick={() => { void triggerBuild({ reason: "PlanEngine execution pulse", activateBuildTab: true }); }} disabled={building} />
                    )}
                    {planRecommendedWorkflow === "reflect" && (
                      <ActionButton label="Recommended: Reflect" onClick={() => { void runHeaderTool(); }} disabled={reflecting} />
                    )}
                    {planRecommendedWorkflow === "live" && (
                      <ActionButton label="Recommended: Live View" onClick={openLiveView} />
                    )}
                    {planRecommendedWorkflow === "ai" && (
                      <ActionButton label="Recommended: AI" onClick={() => { setActiveTab("ai"); openAiPanel(); }} />
                    )}
                    {planRecommendedWorkflow === "plan" && (
                      <ActionButton label="Recommended: Plan Tab" onClick={() => setActiveTab("plan")} />
                    )}
                    <ActionButton label="Open Sidebar" onClick={openPlanenginePanel} />
                  </div>
                </div>
              )}
            </Section>

            {suggestions.length > 0 && (
              <Section title="Inspector Suggestions">
                {suggestions.map((suggestion, index) => (
                  <div key={`${suggestion.entity}-${index}`} style={{ ...styles.suggRow, borderLeftColor: SUGGESTION_COLORS[suggestion.kind] }}>
                    <span style={{ color: SUGGESTION_COLORS[suggestion.kind], marginRight: 6 }}>{SUGGESTION_ICONS[suggestion.kind]}</span>
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <span style={styles.suggEntity}>{suggestion.entity}</span>
                      <span style={styles.suggMsg}>{suggestion.message}</span>
                    </div>
                    {suggestion.action_component_type && suggestion.entity_id && (
                      <button
                        style={{ ...styles.miniBtn, opacity: sceneBusy ? 0.45 : 1 }}
                        disabled={sceneBusy}
                        onClick={() => { void applySuggestion(suggestion); }}
                      >
                        {suggestion.action_label ?? "Apply"}
                      </button>
                    )}
                  </div>
                ))}
              </Section>
            )}

            {!projectInfo.has_compile_commands && (
              <Section title="clangd LSP">
                <div style={{ fontSize: 11, color: "#8eb5c4", marginBottom: 6 }}>
                  No <code>compile_commands.json</code> found. Generate it for C++ autocomplete and diagnostics.
                </div>
                <button
                  style={{ ...styles.btn, opacity: generatingCC ? 0.5 : 1, marginTop: 0 }}
                  disabled={generatingCC}
                  onClick={generateCompileCommands}
                >
                  {generatingCC ? "Generating…" : "Generate compile_commands.json"}
                </button>
                {ccMsg && <div style={{ fontSize: 11, color: ccMsg.startsWith("Error") ? "#f87171" : "#7ed4a7", marginTop: 4 }}>{ccMsg}</div>}
              </Section>
            )}

            <Section title="Project">
              <Row label="Entry scene" value={projectInfo.entry_scene || "—"} />
              <Row label="Game library" value={projectInfo.game_library_path} />
              <Row label="Library status" value={projectInfo.game_library_exists ? "Built" : "Missing"} />
              <Row label="Source files" value={String(projectInfo.source_file_count)} />
              <Row label="Header files" value={String(projectInfo.header_file_count)} />
            </Section>

            <Section title="Build">
              <Row label="Compiler" value={`${projectInfo.compiler} (${projectInfo.standard})`} />
              <Row label="Build system" value={projectInfo.build_system} />
              {projectInfo.include_dirs.length > 0 && <Row label="Include dirs" value={projectInfo.include_dirs.join(", ")} />}
              {projectInfo.defines.length > 0 && <Row label="Defines" value={projectInfo.defines.join(", ")} />}
              {projectInfo.link_libs.length > 0 && <Row label="Link libs" value={projectInfo.link_libs.join(", ")} />}
              <div style={styles.actionRow}>
                <ActionButton label={generatingCC ? "Generating…" : "Regen compile_commands.json"} disabled={generatingCC} onClick={generateCompileCommands} />
                <ActionButton label="Open compile_commands.json" disabled={!projectInfo.has_compile_commands} onClick={() => openProjectFile("compile_commands.json")} />
                <ActionButton label="Open reflect.json" disabled={!projectInfo.has_reflection_json} onClick={() => openProjectFile(".shadoweditor/shadow_reflect.json")} />
                <ActionButton label="Open reflect.cpp" disabled={!projectInfo.has_reflection_generated_cpp} onClick={() => openProjectFile(".shadoweditor/shadow_reflect_generated.cpp")} />
              </div>
              {ccMsg && <div style={{ fontSize: 10, color: ccMsg.startsWith("Error") ? "#f87171" : "#7ed4a7", marginTop: 6 }}>{ccMsg}</div>}
            </Section>

            <Section title="Runtime">
              <div style={styles.actionRow}>
                <ActionButton
                  label={runtimeBusy ? "Loading…" : runtimeStatus?.is_live ? "Reload Runtime" : "Load Runtime"}
                  disabled={runtimeBusy || !projectInfo.game_library_exists}
                  onClick={loadRuntime}
                />
                <ActionButton label={runtimePlaying ? "Pause" : "Play"} disabled={runtimeBusy || !projectInfo.game_library_exists} onClick={() => { void toggleRuntimePlay(); }} />
                <ActionButton label="Step" disabled={runtimeBusy || !runtimeStatus?.is_live} onClick={() => { void stepRuntime(); }} />
                <ActionButton label="Save Live Scene" disabled={runtimeBusy || !runtimeStatus?.is_live} onClick={() => { void saveRuntimeScene(); }} />
                <ActionButton label="Stop Runtime" disabled={runtimeBusy || !runtimeStatus?.is_live} onClick={stopRuntime} />
                <ActionButton label="Open Project Config" onClick={() => openProjectFile(".shadow_project.toml")} />
              </div>
              <div style={{ ...styles.suggRow, borderLeftColor: autoBuildEnabled ? "#7ed4a7" : "#666" }}>
                <span style={{ color: autoBuildEnabled ? "#7ed4a7" : "#666", marginRight: 6 }}>{autoBuildEnabled ? "↻" : "•"}</span>
                <div style={{ display: "flex", flexDirection: "column", gap: 4, width: "100%" }}>
                  <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8 }}>
                    <span style={styles.suggEntity}>Auto Build / Hot Reload</span>
                    <button
                      style={{ ...styles.miniBtn, padding: "3px 8px" }}
                      onClick={() => setAutoBuildEnabled((value) => !value)}
                    >
                      {autoBuildEnabled ? "On" : "Off"}
                    </button>
                  </div>
                  <span style={styles.suggMsg}>
                    {autoBuildStatus ?? "Watching src/, game/, and project config changes to rebuild and hot-reload automatically."}
                  </span>
                </div>
              </div>
              <Row label="Status" value={runtimeStatus?.status_line || "Waiting for first load"} />
              <Row label="Play mode" value={runtimePlaying ? "Running" : "Stopped"} />
              <Row label="Library" value={runtimeStatus?.library_path || projectInfo.game_library_path} />
              <Row label="Live scene" value={runtimeStatus?.last_scene_path || projectInfo.entry_scene || "—"} />
              <Row label="Frame" value={String(runtimeStatus?.frame_index ?? 0)} />
              <Row label="Runtime counts" value={`${runtimeStatus?.entity_count ?? 0} entities · ${runtimeStatus?.component_count ?? 0} components`} />
              <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 6 }}>
                <span style={styles.rowLabel}>Step delta</span>
                <input
                  style={{ ...styles.searchInput, marginBottom: 0, width: 90, padding: "4px 6px", fontSize: 11 }}
                  value={runtimeDelta}
                  onChange={(e) => setRuntimeDelta(e.target.value)}
                  spellCheck={false}
                />
              </div>
              {runtimeStatus?.last_error && (
                <div style={{ fontSize: 11, color: "#f87171", marginTop: 6 }}>{runtimeStatus.last_error}</div>
              )}
            </Section>

            <Section title="Scene Validation">
              <div style={styles.actionRow}>
                <ActionButton label="Revalidate Scene" disabled={!projectInfo.entry_scene_path} onClick={validateScene} />
                <ActionButton label="Open Scene" disabled={!projectInfo.entry_scene_path} onClick={() => openProjectFile(projectInfo.entry_scene_path)} />
              </div>
              {!sceneValidation ? (
                <div style={styles.placeholder}>No validation report loaded yet.</div>
              ) : sceneValidation.issue_count === 0 ? (
                <div style={{ fontSize: 11, color: "#7ed4a7" }}>
                  No reflection mismatches found in {sceneValidation.scene_path.split("/").pop() || sceneValidation.scene_path}.
                </div>
              ) : (
                sceneValidation.issues.map((issue, index) => (
                  <div key={`${issue.entity}-${issue.component_type}-${index}`} style={{ ...styles.suggRow, borderLeftColor: SUGGESTION_COLORS[issue.severity === "error" ? "warning" : issue.severity === "warning" ? "info" : "tip"] }}>
                    <span style={{ color: issue.severity === "error" ? "#f87171" : issue.severity === "warning" ? "#e9aa5f" : "#7eb8d4", marginRight: 6 }}>
                      {issue.severity === "error" ? "⛔" : issue.severity === "warning" ? "⚠" : "ℹ"}
                    </span>
                    <div>
                      <span style={styles.suggEntity}>{issue.entity}</span>
                      <span style={styles.suggMsg}>[{issue.component_type}] {issue.message}</span>
                    </div>
                  </div>
                ))
              )}
            </Section>

            {entryScene && entryScene.entities.length > 0 && (
              <Section title={`Scene — ${entryScene.scene_name} (${entryScene.entities.length} entities)`}>
                {entryScene.entities.map((entity) => (
                  <div key={entity.id} style={styles.entityBlock}>
                    <div style={styles.entityName}>
                      <span style={{ color: "#e9aa5f", marginRight: 4 }}>▸</span>
                      {entity.name || entity.id}
                    </div>
                    {entity.components.map((component, index) => (
                      <div key={`${entity.id}-${index}`} style={styles.compTag}>
                        {component.component_type}
                      </div>
                    ))}
                  </div>
                ))}
              </Section>
            )}

            {(!entryScene || entryScene.entities.length === 0) && projectInfo.scenes.length > 0 && (
              <Section title="Scenes">
                {projectInfo.scenes.map((scene) => (
                  <div key={scene} style={styles.sceneRow}>
                    <span style={{ color: "#e9aa5f", marginRight: 4 }}>▤</span>
                    <button style={styles.inlineLinkBtn} onClick={() => openProjectFile(scene)}>
                      {scene}
                    </button>
                  </div>
                ))}
              </Section>
            )}
          </div>
        )}

        {activeTab === "scene" && projectInfo && (
          <div style={styles.scroll}>
            <Section title="Scene Authoring">
              <div style={styles.actionRow}>
                <input
                  style={{ ...styles.searchInput, marginBottom: 0, flex: 1, minWidth: 0 }}
                  value={newEntityName}
                  onChange={(e) => setNewEntityName(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") {
                      e.preventDefault();
                      void addEntityToScene();
                    }
                  }}
                  placeholder="New entity name"
                  spellCheck={false}
                />
                <ActionButton label="Add Entity" disabled={sceneBusy || !projectPath} onClick={() => { void addEntityToScene(); }} />
                <ActionButton label="Validate Scene" disabled={sceneBusy || !projectInfo.entry_scene_path} onClick={validateScene} />
                <ActionButton label="Open Scene File" disabled={!projectInfo.entry_scene_path} onClick={() => openProjectFile(projectInfo.entry_scene_path)} />
              </div>
              {sceneStatus && (
                <div style={{ fontSize: 11, color: sceneStatus.startsWith("Scene update failed") ? "#f87171" : "#7ed4a7", marginBottom: 8 }}>
                  {sceneStatus}
                </div>
              )}
              {!entryScene ? (
                <div style={styles.placeholder}>No scene loaded yet.</div>
              ) : (
                <>
                  <div style={styles.sceneEntityList}>
                    {entryScene.entities.map((entity) => {
                      const isSelected = entity.id === selectedEntity?.id;
                      return (
                        <button
                          key={entity.id}
                          style={{ ...styles.sceneEntityRow, ...(isSelected ? styles.sceneEntityRowSelected : {}) }}
                          onClick={() => setSelectedEntityId(entity.id)}
                        >
                          <span style={styles.sceneEntityName}>{entity.name || entity.id}</span>
                          <span style={styles.sceneEntityMeta}>{entity.components.length} comp</span>
                        </button>
                      );
                    })}
                  </div>

                  {!selectedEntity ? (
                    <div style={styles.placeholder}>Select an entity to edit its reflected component fields.</div>
                  ) : (
                    <div style={styles.sceneInspector}>
                      <div style={styles.sceneInspectorHeader}>
                        <input
                          style={{ ...styles.searchInput, marginBottom: 0, flex: 1, minWidth: 0 }}
                          value={sceneDrafts[sceneEntityDraftKey(selectedEntity.id)] ?? selectedEntity.name}
                          onChange={(e) => setSceneDrafts((current) => ({
                            ...current,
                            [sceneEntityDraftKey(selectedEntity.id)]: e.target.value,
                          }))}
                          onBlur={(e) => {
                            const nextValue = e.target.value.trim();
                            if (nextValue && nextValue !== selectedEntity.name) {
                              void renameSceneEntity(selectedEntity.id, nextValue);
                            }
                          }}
                          onKeyDown={(e) => {
                            if (e.key === "Enter") {
                              e.currentTarget.blur();
                            }
                          }}
                          spellCheck={false}
                        />
                        <button
                          style={{ ...styles.miniBtn, color: "#f87171", borderColor: "#5a2020" }}
                          disabled={sceneBusy}
                          onClick={() => { void removeEntityFromScene(selectedEntity.id, selectedEntity.name || selectedEntity.id); }}
                        >
                          Remove Entity
                        </button>
                      </div>

                      <div style={{ ...styles.actionRow, marginTop: 8 }}>
                        <select
                          style={{ ...styles.selectInput, flex: 1, minWidth: 0 }}
                          value={newComponentType || addableComponentTypes[0] || ""}
                          onChange={(e) => setNewComponentType(e.target.value)}
                          disabled={sceneBusy || addableComponentTypes.length === 0}
                        >
                          {addableComponentTypes.length === 0 ? (
                            <option value="">No more reflected components</option>
                          ) : (
                            addableComponentTypes.map((componentType) => (
                              <option key={componentType} value={componentType}>
                                {componentType}
                              </option>
                            ))
                          )}
                        </select>
                        <ActionButton
                          label="Add Component"
                          disabled={sceneBusy || addableComponentTypes.length === 0}
                          onClick={() => {
                            const componentType = newComponentType || addableComponentTypes[0];
                            if (componentType) {
                              void addComponentToEntity(selectedEntity.id, componentType);
                            }
                          }}
                        />
                      </div>

                      {selectedEntitySuggestions.length > 0 && (
                        <div style={{ marginBottom: 10 }}>
                          {selectedEntitySuggestions.map((suggestion, index) => (
                            <div key={`${suggestion.entity_id}-${index}`} style={{ ...styles.suggRow, borderLeftColor: SUGGESTION_COLORS[suggestion.kind] }}>
                              <span style={{ color: SUGGESTION_COLORS[suggestion.kind], marginRight: 6 }}>{SUGGESTION_ICONS[suggestion.kind]}</span>
                              <div style={{ flex: 1, minWidth: 0 }}>
                                <span style={styles.suggEntity}>{suggestion.entity}</span>
                                <span style={styles.suggMsg}>{suggestion.message}</span>
                              </div>
                              {suggestion.action_component_type && (
                                <button
                                  style={{ ...styles.miniBtn, opacity: sceneBusy ? 0.45 : 1 }}
                                  disabled={sceneBusy}
                                  onClick={() => { void applySuggestion(suggestion); }}
                                >
                                  {suggestion.action_label ?? "Apply"}
                                </button>
                              )}
                            </div>
                          ))}
                        </div>
                      )}

                      {selectedEntity.components.length === 0 ? (
                        <div style={styles.placeholder}>This entity has no components yet.</div>
                      ) : (
                        selectedEntity.components.map((component) => (
                          <div key={`${selectedEntity.id}-${component.component_type}`} style={styles.compBlock}>
                            <div style={styles.sceneComponentHeader}>
                              <div style={styles.compName}>{component.component_type}</div>
                              <button
                                style={{ ...styles.miniBtn, color: "#f87171", borderColor: "#5a2020" }}
                                disabled={sceneBusy}
                                onClick={() => { void removeComponentFromEntity(selectedEntity.id, component.component_type); }}
                              >
                                Remove
                              </button>
                            </div>
                            {component.fields.length === 0 ? (
                              <div style={{ fontSize: 10, color: "#666" }}>no serialized fields yet</div>
                            ) : (
                              component.fields.map(([fieldName, fieldValue]) => {
                                const draftKey = sceneFieldDraftKey(selectedEntity.id, component.component_type, fieldName);
                                const currentValue = sceneDrafts[draftKey] ?? fieldValue;
                                return (
                                  <div key={draftKey} style={styles.sceneFieldRow}>
                                    <span style={styles.sceneFieldLabel}>{fieldName}</span>
                                    <input
                                      style={{ ...styles.searchInput, marginBottom: 0, flex: 1, minWidth: 0, fontFamily: "monospace" }}
                                      value={currentValue}
                                      onChange={(e) => setSceneDrafts((current) => ({
                                        ...current,
                                        [draftKey]: e.target.value,
                                      }))}
                                      onBlur={(e) => {
                                        const nextValue = e.target.value;
                                        if (nextValue !== fieldValue) {
                                          void commitSceneField(selectedEntity.id, component.component_type, fieldName, nextValue);
                                        }
                                      }}
                                      onKeyDown={(e) => {
                                        if (e.key === "Enter") {
                                          e.currentTarget.blur();
                                        }
                                      }}
                                      spellCheck={false}
                                    />
                                  </div>
                                );
                              })
                            )}
                          </div>
                        ))
                      )}
                    </div>
                  )}
                </>
              )}
            </Section>
          </div>
        )}

        {activeTab === "code" && (
          <div style={styles.scroll}>
            <Section title="Source Files">
              {sourceFiles.length === 0 ? (
                <div style={styles.placeholder}>No C++ source or header files found in <code>src/</code> or <code>game/</code>.</div>
              ) : (
                sourceFiles.map((file) => (
                  <div key={file.path} style={styles.assetRow}>
                    <span style={{ ...styles.kindBadge, color: SOURCE_KIND_COLORS[file.kind] ?? "#8eb5c4" }}>{file.kind}</span>
                    <button style={styles.assetNameButton} onClick={() => openProjectFile(file.path)}>
                      {file.path}
                    </button>
                    <span style={styles.assetSize}>{formatBytes(file.size_bytes)}</span>
                  </div>
                ))
              )}
            </Section>
          </div>
        )}

        {activeTab === "assets" && (
          <div style={styles.scroll}>
            <div style={styles.actionRow}>
              <ActionButton label={assetImporting ? "Importing…" : "Import Assets"} disabled={assetImporting || !projectPath} onClick={() => { void importAssets(); }} />
            </div>
            {assetImportMsg && (
              <div style={{ fontSize: 11, color: assetImportMsg.startsWith("Import failed") ? "#f87171" : "#7ed4a7", marginBottom: 8 }}>
                {assetImportMsg}
              </div>
            )}
            <input
              style={styles.searchInput}
              value={assetSearch}
              onChange={(e) => setAssetSearch(e.target.value)}
              placeholder="Search assets…"
              spellCheck={false}
            />
            <div style={styles.filterRow}>
              {allKinds.map((kind) => (
                <button
                  key={kind}
                  style={{ ...styles.filterBtn, ...(kindFilter === kind ? styles.filterBtnActive : {}) }}
                  onClick={() => setKindFilter(kind)}
                >
                  {kind}
                </button>
              ))}
            </div>
            {filteredAssets.length === 0 ? (
              <div style={styles.placeholder}>
                {assetSearch ? `No assets matching "${assetSearch}"` : "No assets found"}
              </div>
            ) : (
              filteredAssets.map((asset) => (
                <div key={asset.path} style={styles.assetRow}>
                  <span style={{ ...styles.kindBadge, color: KIND_COLORS[asset.kind] ?? "#aaa" }}>{asset.kind}</span>
                  <button style={styles.assetNameButton} onClick={() => openProjectFile(asset.path)}>
                    {asset.path}
                  </button>
                  <span style={styles.assetSize}>{formatBytes(asset.size_bytes)}</span>
                </div>
              ))
            )}
          </div>
        )}

        {activeTab === "reflect" && (
          <div style={styles.scroll}>
            <div style={styles.actionRow}>
              <ActionButton label={reflecting ? "Scanning…" : "Run Header Tool"} disabled={reflecting || !projectInfo} onClick={runHeaderTool} />
              <ActionButton label="Open reflect.json" disabled={!projectInfo?.has_reflection_json} onClick={() => openProjectFile(".shadoweditor/shadow_reflect.json")} />
              <ActionButton label="Open reflect.cpp" disabled={!projectInfo?.has_reflection_generated_cpp} onClick={() => openProjectFile(reflectResult?.generated_cpp_path || ".shadoweditor/shadow_reflect_generated.cpp")} />
            </div>
            {!reflectResult ? (
              <div style={styles.placeholder}>
                Reflection metadata is not loaded yet. Run the header tool or build the project to generate <code>.shadoweditor/shadow_reflect.json</code> and <code>.shadoweditor/shadow_reflect_generated.cpp</code>.
              </div>
            ) : reflectResult.components.length === 0 ? (
              <div style={styles.placeholder}>
                No <code>SHADOW_COMPONENT</code> annotations found in {reflectResult.header_count || "the scanned"} header set.
              </div>
            ) : (
              <>
                <div style={{ fontSize: 10, color: "#8eb5c4", marginBottom: 10 }}>
                  {reflectResult.header_count > 0 ? `${reflectResult.header_count} header(s) scanned · ` : ""}
                  {reflectResult.component_count} component(s)
                </div>
                {reflectResult.components.map((component) => (
                  <div key={component.name} style={styles.compBlock}>
                    <div style={styles.compName}>{component.name}</div>
                    {(component.properties ?? []).map((property) => (
                      <div key={`${component.name}-${property.name}`} style={styles.propRow}>
                        <span style={styles.propType}>{property.ty}</span>
                        <span style={styles.propName}>{property.name}</span>
                        {(property.meta ?? []).length > 0 && (
                          <span style={styles.propMeta}>{(property.meta ?? []).join(", ")}</span>
                        )}
                      </div>
                    ))}
                    {component.properties.length === 0 && (
                      <div style={{ fontSize: 10, color: "#666" }}>no annotated properties</div>
                    )}
                  </div>
                ))}
              </>
            )}
          </div>
        )}

        {activeTab === "build" && (
          <div style={styles.scroll}>
            <div style={styles.actionRow}>
              <ActionButton label={building ? "Building…" : "Build Runtime"} disabled={building || !projectInfo} onClick={() => { void triggerBuild(); }} />
              <ActionButton label={runtimeBusy ? "Loading…" : runtimeStatus?.is_live ? "Reload Runtime" : "Load Runtime"} disabled={runtimeBusy || !projectInfo?.game_library_exists} onClick={loadRuntime} />
              <ActionButton label={viewportDockVisible ? "Hide Viewport" : "Dock Viewport"} disabled={!projectInfo} onClick={toggleViewportDock} />
              <ActionButton label="Full View" disabled={!projectInfo} onClick={openLiveView} />
              <ActionButton label="Open compile_commands.json" disabled={!projectInfo?.has_compile_commands} onClick={() => openProjectFile("compile_commands.json")} />
              <ActionButton label="Open last build log" disabled={!buildLog} onClick={() => openProjectFile(".shadoweditor/last_build.log")} />
            </div>
            {runtimeStatus && (
              <div style={{ fontSize: 11, color: runtimeStatus.last_error ? "#f87171" : "#8eb5c4", marginBottom: 8 }}>
                {runtimeStatus.last_error ? `Runtime host error: ${runtimeStatus.last_error}` : runtimeStatus.status_line}
              </div>
            )}
            {buildLog ? (
              <pre style={styles.buildLog}>{buildLog}</pre>
            ) : (
              <div style={styles.placeholder}>
                Press <strong style={{ color: "#e9aa5f" }}>▶ Build</strong> to compile the C++23 game library. Successful builds also refresh reflection and compile commands for the editor.
              </div>
            )}
          </div>
        )}

        {activeTab === "ai" && (
          <div style={styles.scroll}>
            <Section title="Local AI Models">
              <div style={styles.actionRow}>
                <ActionButton label="Open LLM Loader" onClick={openLlmLoaderPanel} />
                <ActionButton label={loadingLoaderState ? "Refreshing…" : "Refresh Loader"} disabled={loadingLoaderState} onClick={() => { void loadLoaderState(); }} />
                <ActionButton label="Open AI Chat" onClick={openAiPanel} />
              </div>
              <Row label="Server" value={loaderServerStatus?.running ? `Running on :${loaderServerStatus.port}` : "Stopped"} />
              <Row label="Backend" value={loaderServerStatus?.backend || loaderEngineInfo?.backend || "Not configured"} />
              <Row label="Installed engines" value={loaderInstalledBackends.length > 0 ? loaderInstalledBackends.map((backend) => backend.toUpperCase()).join(", ") : "None"} />
              <Row label="Autocomplete" value={preferredAutocompleteModel?.name || "No local model detected"} />
              <Row label="Chat / Refactor" value={preferredChatModel?.name || "No local model detected"} />
              {loaderStateError && <div style={{ color: "#f87171", fontSize: 11, marginTop: 8 }}>{loaderStateError}</div>}
              {loadingLoaderState ? (
                <div style={styles.placeholder}>
                  Refreshing loader status and local models…
                </div>
              ) : loaderModels.length === 0 ? (
                <div style={styles.placeholder}>
                  No local models detected yet. Use the LLM Loader panel to browse a GGUF file or download one from Hugging Face.
                </div>
              ) : (
                <div style={{ marginTop: 8 }}>
                  {loaderModels.map((model) => (
                    <div key={model.path} style={styles.assetRow}>
                      <span style={{ ...styles.kindBadge, color: "#7ed4a7" }}>local</span>
                      <span style={{ color: "#ecf1f4", flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{model.name}</span>
                      <span style={styles.assetSize}>{formatBytes(model.size_bytes)}</span>
                    </div>
                  ))}
                </div>
              )}
              <div style={{ fontSize: 11, color: "#8eb5c4", marginTop: 12, marginBottom: 6 }}>Suggested loader models</div>
              {PLANENGINE_RECOMMENDED_MODELS.map((model) => {
                return (
                  <div key={model.name} style={styles.assetRow}>
                    <span style={{ ...styles.kindBadge, color: "#8eb5c4" }}>suggested</span>
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ color: "#ecf1f4", fontSize: 12 }}>{model.name}</div>
                      <div style={{ color: "#8eb5c4", fontSize: 10 }}>{model.role}</div>
                    </div>
                    <button
                      style={styles.miniBtn}
                      onClick={openLlmLoaderPanel}
                    >
                      Open Loader
                    </button>
                  </div>
                );
              })}
            </Section>

            <Section title="AI Integration">
              <div style={styles.actionRow}>
                <ActionButton label="Open AI Chat" onClick={openAiPanel} />
                <ActionButton label="Open LLM Loader" onClick={openLlmLoaderPanel} />
                <ActionButton label={loadingAi ? "Refreshing…" : "Refresh Context"} disabled={loadingAi || !projectPath} onClick={loadAi} />
                <ActionButton label="Open AI History" onClick={() => openProjectFile(".shadoweditor/ai_history.jsonl")} disabled={aiHistoryCount === 0} />
              </div>
              <Row label="History entries" value={String(aiHistoryCount)} />
              {aiError && <div style={{ color: "#f87171", fontSize: 11, marginTop: 8 }}>{aiError}</div>}
              {!aiError && (
                <pre style={styles.aiContext}>{loadingAi ? "Loading AI context…" : (aiContext || "No AI context loaded yet.")}</pre>
              )}
            </Section>
          </div>
        )}

        {activeTab === "plan" && (
          <div style={styles.scroll}>
            <Section title="Project Plan">
              <div style={styles.actionRow}>
                <ActionButton label={loadingPlanDocs ? "Refreshing…" : "Refresh Plan"} disabled={loadingPlanDocs} onClick={() => { void loadPlanDocs(); }} />
                <ActionButton label="Open in Sidebar" onClick={openPlanenginePanel} />
                <ActionButton label="Open planengine.md" disabled={!planDocs} onClick={() => planDocs && onOpenFile?.(planDocs.plan_path, "planengine.md")} />
                <ActionButton label="Open finish.md" disabled={!planDocs?.finish_available} onClick={() => planDocs?.finish_available && onOpenFile?.(planDocs.finish_path, "finish.md")} />
              </div>
              <input
                style={styles.searchInput}
                value={planSearch}
                onChange={(event) => setPlanSearch(event.target.value)}
                placeholder="Filter plan sections or remaining work"
                spellCheck={false}
              />
              {planDocsError && <div style={{ color: "#f87171", fontSize: 11 }}>{planDocsError}</div>}
              {loadingPlanDocs && !planDocs ? (
                <div style={styles.placeholder}>Loading integrated PlanEngine docs…</div>
              ) : !planDocs ? (
                <div style={styles.placeholder}>PlanEngine documents are not loaded yet.</div>
              ) : (
                <>
                  <div style={styles.roadmapPanelWrap}>
                    <div style={styles.roadmapPanelCard}>
                      <div style={styles.roadmapPanelHeader}>
                        <div>
                          <div style={styles.roadmapPanelTitle}>Roadmap Integration</div>
                          <div style={styles.roadmapPanelMeta}>{planAuditStamp ?? "Loaded from planengine.md"}</div>
                        </div>
                        <div style={styles.roadmapPanelBadge}>
                          {roadmapPhaseCards.length} phases
                        </div>
                      </div>
                      <div style={styles.roadmapPanelCopy}>
                        PlanEngine is integrated into the sidebar, the Game panel, the live viewport flow, and the AI workflow. Use this tab to translate the roadmap into concrete IDE actions.
                      </div>
                      {planAuditTotals ? (
                        <div style={styles.roadmapAuditGrid}>
                          <div style={styles.roadmapAuditChip}>
                            <span style={styles.roadmapAuditValue}>{planAuditTotals.done}</span>
                            <span style={styles.roadmapAuditLabel}>Done</span>
                          </div>
                          <div style={styles.roadmapAuditChip}>
                            <span style={styles.roadmapAuditValue}>{planAuditTotals.partial}</span>
                            <span style={styles.roadmapAuditLabel}>Partial</span>
                          </div>
                          <div style={styles.roadmapAuditChip}>
                            <span style={styles.roadmapAuditValue}>{planAuditTotals.pending}</span>
                            <span style={styles.roadmapAuditLabel}>Not Started</span>
                          </div>
                          <div style={styles.roadmapAuditChip}>
                            <span style={styles.roadmapAuditValue}>{planAuditTotals.total}</span>
                            <span style={styles.roadmapAuditLabel}>Total</span>
                          </div>
                        </div>
                      ) : (
                        <div style={styles.placeholder}>No roadmap audit table was found yet.</div>
                      )}
                      {planPendingNextSteps.length > 0 && (
                        <div style={{ marginTop: 10 }}>
                          <div style={styles.roadmapBlockTitle}>Current Priority</div>
                          <div style={styles.roadmapInlineList}>
                            {planPendingNextSteps.map((item) => (
                              <div key={item} style={styles.roadmapInlineItem}>- {item}</div>
                            ))}
                          </div>
                        </div>
                      )}
                    </div>

                    <div style={styles.roadmapPhaseGrid}>
                      {roadmapPhaseCards.map((phase) => (
                        <div key={phase.key} style={styles.roadmapPhaseCard}>
                          <div style={{ ...styles.roadmapPhaseAccent, background: phase.accent }} />
                          <div style={styles.roadmapPhaseHeader}>
                            <div>
                              <div style={styles.roadmapPhaseTitle}>{phase.heading}</div>
                              <div style={styles.roadmapPhaseStatus}>{phase.status}</div>
                            </div>
                            <div style={styles.roadmapPhaseCounts}>
                              {phase.counts.done}/{phase.counts.partial}/{phase.counts.pending}
                            </div>
                          </div>
                          <div style={styles.roadmapPanelCopy}>{phase.description}</div>
                          {phase.highlights.length > 0 && (
                            <div style={styles.roadmapInlineList}>
                              {phase.highlights.map((item) => (
                                <div key={`${phase.key}-${item}`} style={styles.roadmapInlineItem}>- {item}</div>
                              ))}
                            </div>
                          )}
                          <div style={styles.actionRow}>
                            <ActionButton label={phase.actionLabel} onClick={() => runRoadmapPhaseAction(phase.key)} />
                            <ActionButton label="Open Sidebar" onClick={openPlanenginePanel} />
                          </div>
                        </div>
                      ))}
                    </div>
                  </div>

                  <div style={styles.docPanelTitle}>planengine.md</div>
                  {visiblePlanSections.length === 0 ? (
                    <div style={styles.placeholder}>No plan sections match that filter.</div>
                  ) : (
                    visiblePlanSections.map((section, index) => (
                      <div key={`plan-${section.heading}-${index}`} style={styles.docCard}>
                        <div style={{ ...styles.docHeading, fontSize: section.level <= 2 ? 13 : 12 }}>
                          {"#".repeat(Math.min(section.level, 6))} {section.heading}
                        </div>
                        {section.body && <pre style={styles.docBody}>{section.body}</pre>}
                      </div>
                    ))
                  )}

                  <div style={{ ...styles.docPanelTitle, marginTop: 14 }}>finish.md</div>
                  {!planDocs.finish_available ? (
                    <div style={styles.placeholder}>finish.md was not found in this workspace.</div>
                  ) : visibleFinishSections.length === 0 ? (
                    <div style={styles.placeholder}>No audit sections match that filter.</div>
                  ) : (
                    visibleFinishSections.map((section, index) => (
                      <div key={`finish-${section.heading}-${index}`} style={styles.docCard}>
                        <div style={{ ...styles.docHeading, fontSize: section.level <= 2 ? 13 : 12 }}>
                          {"#".repeat(Math.min(section.level, 6))} {section.heading}
                        </div>
                        {section.body && <pre style={styles.docBody}>{section.body}</pre>}
                      </div>
                    ))
                  )}
                </>
              )}
            </Section>
          </div>
        )}
      </div>

      {showWizard && (
        <NewProjectWizard
          onClose={() => setShowWizard(false)}
          onCreated={(path) => {
            setShowWizard(false);
            onProjectCreated?.(path);
            onActivatePanel?.("planengine");
          }}
        />
      )}
    </>
  );
}

function Chip({ label, sub, ok }: { label: string; sub?: string; ok?: boolean }) {
  const color = ok === undefined ? "#8eb5c4" : ok ? "#7ed4a7" : "#f87171";
  return (
    <div style={{ ...styles.chip, borderColor: color + "55", color }}>
      {label}
      {sub && <span style={{ color: "#8eb5c4", marginLeft: 4, fontSize: 10 }}>{sub}</span>}
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={styles.section}>
      <div style={styles.sectionTitle}>{title}</div>
      {children}
    </div>
  );
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div style={styles.row}>
      <span style={styles.rowLabel}>{label}</span>
      <span style={styles.rowValue}>{value}</span>
    </div>
  );
}

function ActionButton({
  label,
  onClick,
  disabled = false,
}: {
  label: string;
  onClick: () => void;
  disabled?: boolean;
}) {
  return (
    <button
      style={{ ...styles.miniBtn, opacity: disabled ? 0.45 : 1 }}
      disabled={disabled}
      onClick={onClick}
    >
      {label}
    </button>
  );
}

const styles: Record<string, React.CSSProperties> = {
  container: { display: "flex", flexDirection: "column", height: "100%", background: "var(--bg-primary, #0f1418)", color: "var(--text-primary, #ecf1f4)" },
  empty: { display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", height: "100%", padding: 24, textAlign: "center", gap: 12 },
  emptyIcon: { marginBottom: 4 },
  emptyTitle: { fontSize: 16, fontWeight: 600, color: "#e9aa5f" },
  emptyDesc: { fontSize: 12, color: "#888", lineHeight: 1.6 },
  newProjectBtn: { marginTop: 8, padding: "7px 18px", background: "#e9aa5f22", color: "#e9aa5f", border: "1px solid #e9aa5f55", borderRadius: 4, cursor: "pointer", fontSize: 12, fontWeight: 600 },
  header: { display: "flex", alignItems: "center", gap: 8, padding: "10px 14px", borderBottom: "1px solid #1e2a30", flexWrap: "wrap" },
  headerTitle: { fontWeight: 700, fontSize: 14, color: "#ecf1f4" },
  headerKicker: { fontSize: 10, color: "#e9aa5f", border: "1px solid #e9aa5f33", background: "#e9aa5f11", borderRadius: 999, padding: "2px 8px" },
  headerSub: { fontSize: 11, color: "#8eb5c4", marginRight: "auto" },
  headerIconBtn: { background: "transparent", border: "none", color: "#8eb5c4", cursor: "pointer", padding: "2px 4px", borderRadius: 3, display: "flex", alignItems: "center" },
  buildBtn: { padding: "4px 12px", background: "#e9aa5f22", color: "#e9aa5f", border: "1px solid #e9aa5f55", borderRadius: 4, cursor: "pointer", fontSize: 12, fontWeight: 600 },
  chips: { display: "flex", flexWrap: "wrap", gap: 6, padding: "8px 14px", borderBottom: "1px solid #1e2a30" },
  chip: { fontSize: 11, padding: "2px 8px", border: "1px solid", borderRadius: 3 },
  loaderBanner: { display: "flex", alignItems: "center", justifyContent: "space-between", padding: "6px 14px", background: "#e9aa5f11", borderBottom: "1px solid #e9aa5f22", fontSize: 11, color: "#e9aa5f" },
  bannerButton: { color: "#7eb8d4", background: "transparent", border: "none", fontWeight: 600, cursor: "pointer", padding: 0 },
  tabs: { display: "grid", gridTemplateColumns: "repeat(4, minmax(0, 1fr))", borderBottom: "1px solid #1e2a30" },
  tab: { padding: "7px 4px", background: "transparent", border: "none", color: "#8eb5c4", cursor: "pointer", fontSize: 10.5, minWidth: 0 },
  tabActive: { color: "#e9aa5f", borderBottom: "2px solid #e9aa5f" },
  scroll: { flex: 1, overflowY: "auto", padding: "10px 14px" },
  section: { marginBottom: 16 },
  sectionTitle: { fontSize: 11, fontWeight: 700, color: "#8eb5c4", textTransform: "uppercase", letterSpacing: 1, marginBottom: 6 },
  row: { display: "flex", justifyContent: "space-between", gap: 8, padding: "3px 0", borderBottom: "1px solid #1e2a3022", fontSize: 12 },
  rowLabel: { color: "#8eb5c4", flexShrink: 0 },
  rowValue: { color: "#ecf1f4", textAlign: "right", wordBreak: "break-all" },
  subtleText: { fontSize: 11, color: "#8eb5c4", lineHeight: 1.5, marginBottom: 8 },
  actionRow: { display: "flex", flexWrap: "wrap", gap: 6, marginBottom: 8 },
  placeholder: { color: "#666", padding: "16px 0", textAlign: "center", fontSize: 12 },
  suggRow: { display: "flex", alignItems: "flex-start", gap: 8, padding: "5px 8px", borderLeft: "2px solid", marginBottom: 4, borderRadius: "0 3px 3px 0", background: "#1e2a3044" },
  suggEntity: { fontWeight: 600, fontSize: 11, color: "#ecf1f4", marginRight: 4 },
  suggMsg: { fontSize: 11, color: "#8eb5c4" },
  entityBlock: { marginBottom: 8, paddingBottom: 6, borderBottom: "1px solid #1e2a3033" },
  entityName: { fontSize: 12, fontWeight: 600, color: "#ecf1f4", marginBottom: 3 },
  compTag: { display: "inline-block", fontSize: 10, color: "#8eb5c4", background: "#1e2a3066", borderRadius: 3, padding: "1px 6px", marginRight: 4, marginBottom: 2 },
  sceneRow: { fontSize: 12, padding: "3px 0", color: "#ecf1f4" },
  sceneEntityList: { display: "flex", flexDirection: "column", gap: 6, marginBottom: 12 },
  sceneEntityRow: { display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8, padding: "8px 10px", background: "#121b21", border: "1px solid #1e2a30", borderRadius: 4, cursor: "pointer", color: "#ecf1f4", textAlign: "left" },
  sceneEntityRowSelected: { borderColor: "#e9aa5f55", background: "#e9aa5f12" },
  sceneEntityName: { fontSize: 12, fontWeight: 600, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" },
  sceneEntityMeta: { fontSize: 10, color: "#8eb5c4", flexShrink: 0 },
  sceneInspector: { padding: "10px", borderRadius: 4, border: "1px solid #1e2a30", background: "#11181d" },
  sceneInspectorHeader: { display: "flex", alignItems: "center", gap: 8 },
  sceneComponentHeader: { display: "flex", alignItems: "center", justifyContent: "space-between", gap: 8, marginBottom: 8 },
  sceneFieldRow: { display: "flex", alignItems: "center", gap: 8, marginBottom: 6 },
  sceneFieldLabel: { width: 112, flexShrink: 0, fontSize: 11, color: "#8eb5c4", fontFamily: "monospace" },
  searchInput: { width: "100%", background: "var(--bg-primary, #0f1418)", border: "1px solid #1e2a30", borderRadius: 4, color: "#ecf1f4", fontSize: 12, padding: "5px 8px", marginBottom: 8, boxSizing: "border-box", outline: "none" },
  selectInput: { background: "var(--bg-primary, #0f1418)", border: "1px solid #1e2a30", borderRadius: 4, color: "#ecf1f4", fontSize: 12, padding: "5px 8px", boxSizing: "border-box", outline: "none" },
  filterRow: { display: "flex", flexWrap: "wrap", gap: 4, marginBottom: 10 },
  filterBtn: { fontSize: 11, padding: "2px 8px", background: "transparent", color: "#8eb5c4", border: "1px solid #1e2a30", borderRadius: 3, cursor: "pointer" },
  filterBtnActive: { background: "#e9aa5f22", color: "#e9aa5f", borderColor: "#e9aa5f55" },
  assetRow: { display: "flex", alignItems: "center", gap: 8, padding: "4px 0", borderBottom: "1px solid #1e2a3022", fontSize: 12 },
  kindBadge: { fontSize: 10, fontWeight: 700, width: 60, flexShrink: 0, textTransform: "uppercase" },
  assetNameButton: { flex: 1, color: "#ecf1f4", background: "transparent", border: "none", padding: 0, textAlign: "left", cursor: "pointer", overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" },
  assetSize: { fontSize: 10, color: "#8eb5c4", flexShrink: 0 },
  compBlock: { marginBottom: 12, padding: "8px 10px", background: "#1e2a3044", borderRadius: 4, border: "1px solid #1e2a3088" },
  compName: { fontSize: 12, fontWeight: 700, color: "#e9aa5f", marginBottom: 6 },
  propRow: { display: "flex", gap: 8, alignItems: "baseline", padding: "2px 0", fontSize: 11 },
  propType: { color: "#7eb8d4", fontFamily: "monospace", minWidth: 80, flexShrink: 0 },
  propName: { color: "#ecf1f4", fontFamily: "monospace", flex: 1 },
  propMeta: { color: "#8eb5c4", fontSize: 10, fontStyle: "italic" },
  buildLog: { fontFamily: "monospace", fontSize: 11, color: "#ecf1f4", whiteSpace: "pre-wrap", wordBreak: "break-word", margin: 0 },
  aiContext: { marginTop: 8, padding: 10, background: "#11181d", border: "1px solid #1e2a30", borderRadius: 4, fontFamily: "monospace", fontSize: 11, color: "#d2dde2", whiteSpace: "pre-wrap", wordBreak: "break-word" },
  roadmapAuditGrid: { display: "grid", gridTemplateColumns: "repeat(4, minmax(0, 1fr))", gap: 6 },
  roadmapAuditChip: { display: "flex", flexDirection: "column", gap: 2, padding: "8px 9px", borderRadius: 8, border: "1px solid #1e2a30", background: "#0d151a" },
  roadmapAuditValue: { fontSize: 14, fontWeight: 700, color: "#ecf1f4" },
  roadmapAuditLabel: { fontSize: 10, color: "#8eb5c4", textTransform: "uppercase", letterSpacing: 0.8 },
  roadmapBlockTitle: { fontSize: 10, fontWeight: 700, color: "#e9aa5f", textTransform: "uppercase", letterSpacing: 0.8, marginBottom: 6 },
  roadmapInlineList: { display: "flex", flexDirection: "column", gap: 5 },
  roadmapInlineItem: { fontSize: 11, color: "#d6e0e5", lineHeight: 1.45 },
  roadmapPanelWrap: { display: "flex", flexDirection: "column", gap: 10, marginBottom: 14 },
  roadmapPanelCard: { padding: "10px 12px", background: "#11181d", border: "1px solid #1e2a30", borderRadius: 8 },
  roadmapPanelHeader: { display: "flex", alignItems: "flex-start", justifyContent: "space-between", gap: 8, marginBottom: 8 },
  roadmapPanelTitle: { fontSize: 12, fontWeight: 700, color: "#ecf1f4" },
  roadmapPanelMeta: { fontSize: 10, color: "#8eb5c4", marginTop: 2 },
  roadmapPanelBadge: { fontSize: 10, color: "#e9aa5f", border: "1px solid #e9aa5f33", background: "#e9aa5f11", borderRadius: 999, padding: "3px 8px", whiteSpace: "nowrap" },
  roadmapPanelCopy: { fontSize: 11, color: "#8eb5c4", lineHeight: 1.5, marginBottom: 8 },
  roadmapPhaseGrid: { display: "grid", gridTemplateColumns: "minmax(0, 1fr)", gap: 8 },
  roadmapPhaseCard: { position: "relative", padding: "10px 12px", background: "#11181d", border: "1px solid #1e2a30", borderRadius: 8, overflow: "hidden" },
  roadmapPhaseAccent: { position: "absolute", inset: "0 auto 0 0", width: 3, opacity: 0.95 },
  roadmapPhaseHeader: { display: "flex", alignItems: "flex-start", justifyContent: "space-between", gap: 8, marginBottom: 8 },
  roadmapPhaseTitle: { fontSize: 12, fontWeight: 700, color: "#ecf1f4" },
  roadmapPhaseStatus: { fontSize: 10, color: "#8eb5c4", marginTop: 2 },
  roadmapPhaseCounts: { fontSize: 10, color: "#6f8792", whiteSpace: "nowrap" },
  docPanelTitle: { fontSize: 11, fontWeight: 700, color: "#e9aa5f", letterSpacing: 0.6, textTransform: "uppercase", marginBottom: 8 },
  docCard: { marginBottom: 10, padding: "10px 12px", background: "#11181d", border: "1px solid #1e2a30", borderRadius: 4 },
  docHeading: { color: "#ecf1f4", fontWeight: 700, marginBottom: 8, lineHeight: 1.4 },
  docBody: { margin: 0, fontFamily: "monospace", fontSize: 10.5, color: "#cfd9de", whiteSpace: "pre-wrap", wordBreak: "break-word", lineHeight: 1.55 },
  btn: { marginTop: 12, padding: "6px 16px", background: "#1e2a30", color: "#e9aa5f", border: "1px solid #e9aa5f55", borderRadius: 4, cursor: "pointer", fontSize: 12 },
  miniBtn: { padding: "4px 8px", background: "transparent", color: "#8eb5c4", border: "1px solid #1e2a30", borderRadius: 3, cursor: "pointer", fontSize: 10 },
  inlineLinkBtn: { background: "transparent", border: "none", color: "#ecf1f4", cursor: "pointer", padding: 0, fontSize: 12, textAlign: "left" },
};
