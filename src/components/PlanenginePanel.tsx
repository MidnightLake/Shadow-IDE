import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { SidebarView } from "../types";

interface Props {
  visible: boolean;
  onOpenFile?: (path: string, name: string) => void;
  onActivatePanel?: (view: SidebarView) => void;
  preferredDoc?: DocKey;
  onFocusGameTab?: (tab: GameDevWorkflowTab) => void;
  onOpenLiveView?: () => void;
  projectPath?: string;
}

interface ShadowPlanengineDocs {
  plan_path: string;
  finish_path: string;
  plan_markdown: string;
  finish_markdown: string;
  finish_available: boolean;
}

interface ShadowProjectInfo {
  name: string;
  runtime: string;
  entry_scene: string;
  entry_scene_path: string;
  game_library_path: string;
  game_library_exists: boolean;
  compiler: string;
  standard: string;
  source_file_count: number;
  header_file_count: number;
  has_reflection_json: boolean;
  has_reflection_generated_cpp: boolean;
  has_compile_commands: boolean;
  build_system: string;
  scenes: string[];
}

interface ShadowRuntimeStatus {
  is_live: boolean;
  status_line: string;
  frame_index: number;
  component_count: number;
  entity_count: number;
  last_error?: string | null;
}

interface ShadowAssetItem {
  path: string;
  kind: string;
}

interface MarkdownSection {
  id: string;
  heading: string;
  body: string;
  level: number;
}

type DocKey = "plan" | "finish";
type GameDevWorkflowTab = "overview" | "scene" | "code" | "assets" | "reflect" | "build" | "ai" | "plan";

interface PhaseIntegration {
  key: string;
  heading: string;
  status: string;
  accent: string;
  panel: SidebarView;
  panelLabel: string;
  description: string;
}

interface IntegratedPhaseCard extends PhaseIntegration {
  section: MarkdownSection;
  highlights: string[];
  itemCount: number;
}

type WorkflowActionTarget = SidebarView | GameDevWorkflowTab | "live-view";

interface WorkflowAction {
  label: string;
  target: WorkflowActionTarget;
  kind: "panel" | "game-tab" | "live-view";
  primary?: boolean;
}

interface WorkflowTrackDefinition {
  key: string;
  heading: string;
  accent: string;
  description: string;
  actions: WorkflowAction[];
}

interface IntegratedWorkflowTrack extends WorkflowTrackDefinition {
  section: MarkdownSection;
  highlights: string[];
}

interface ExecutionQueueItem {
  key: string;
  title: string;
  status: string;
  detail: string;
  action: WorkflowAction;
  secondaryAction?: WorkflowAction;
}

interface LaunchCheckpoint {
  key: string;
  step: string;
  status: string;
  detail: string;
  action?: WorkflowAction;
}

interface CriticalGapCard {
  key: string;
  gap: string;
  status: string;
  impact: string;
  effort: string;
  action: WorkflowAction;
}

const PHASE_INTEGRATIONS: PhaseIntegration[] = [
  {
    key: "phase-1",
    heading: "Phase 1",
    status: "Implemented",
    accent: "#7ed4a7",
    panel: "gamedev",
    panelLabel: "Game Panel",
    description: "The editor shell, hierarchy, inspector, and build surface already live in ShadowIDE.",
  },
  {
    key: "phase-1-5",
    heading: "Phase 1.5",
    status: "Integrated",
    accent: "#8eb5c4",
    panel: "gamedev",
    panelLabel: "Game Panel",
    description: "Live preview and terrain iteration map to the docked viewport and runtime workflow.",
  },
  {
    key: "phase-2",
    heading: "Phase 2",
    status: "In Progress",
    accent: "#e9aa5f",
    panel: "gamedev",
    panelLabel: "Game Panel",
    description: "Core runtime, reflection, and build steps plug into the existing authoring workspace.",
  },
  {
    key: "phase-3",
    heading: "Phase 3",
    status: "AI + Rendering",
    accent: "#9fb7ff",
    panel: "ai",
    panelLabel: "AI Chat",
    description: "Advanced rendering and AI work should stay close to the viewport and coding loop.",
  },
  {
    key: "phase-4",
    heading: "Phase 4",
    status: "Ecosystem",
    accent: "#c9a8ff",
    panel: "plugins",
    panelLabel: "Plugins",
    description: "Shipping and ecosystem milestones connect to packaging, plugins, and distribution surfaces.",
  },
];

const WORKFLOW_TRACKS: WorkflowTrackDefinition[] = [
  {
    key: "architecture",
    heading: "Dual-Language Architecture",
    accent: "#7ed4a7",
    description: "The Rust editor shell and the C++ runtime boundary should map directly onto build, reflection, and authoring workflows.",
    actions: [
      { label: "Overview", target: "overview", kind: "game-tab", primary: true },
      { label: "Build", target: "build", kind: "game-tab" },
      { label: "Reflect", target: "reflect", kind: "game-tab" },
    ],
  },
  {
    key: "runtime",
    heading: "C++23 Game Runtime",
    accent: "#e9aa5f",
    description: "Runtime loading, hot reload, scene state, and reflection belong in the integrated game workflow.",
    actions: [
      { label: "Build Runtime", target: "build", kind: "game-tab", primary: true },
      { label: "Scene", target: "scene", kind: "game-tab" },
      { label: "Reflect", target: "reflect", kind: "game-tab" },
    ],
  },
  {
    key: "viewport",
    heading: "Viewport System",
    accent: "#8eb5c4",
    description: "The roadmap’s live viewport work should launch directly into the in-IDE scene preview and authoring loop.",
    actions: [
      { label: "Open Live View", target: "live-view", kind: "live-view", primary: true },
      { label: "Scene Tab", target: "scene", kind: "game-tab" },
      { label: "Assets", target: "assets", kind: "game-tab" },
    ],
  },
  {
    key: "editor-ui",
    heading: "Embedded Code Editor",
    accent: "#9fb7ff",
    description: "Editor, search, and authoring tools should stay close to the roadmap instead of living as disconnected panels.",
    actions: [
      { label: "Explorer", target: "explorer", kind: "panel", primary: true },
      { label: "Search", target: "search", kind: "panel" },
      { label: "Plan Tab", target: "plan", kind: "game-tab" },
    ],
  },
  {
    key: "ai",
    heading: "LLM Integration",
    accent: "#c9a8ff",
    description: "AI workflows from the roadmap should open the exact IDE surfaces used for local models, context, and chat.",
    actions: [
      { label: "AI Tab", target: "ai", kind: "game-tab", primary: true },
      { label: "AI Chat", target: "ai", kind: "panel" },
      { label: "LLM Loader", target: "llmloader", kind: "panel" },
    ],
  },
];

function parseMarkdownSections(markdown: string, prefix: DocKey): MarkdownSection[] {
  const lines = markdown.replace(/\r/g, "").split("\n");
  const sections: MarkdownSection[] = [];
  let currentHeading = "Overview";
  let currentLevel = 1;
  let currentBody: string[] = [];
  let index = 0;

  const pushSection = () => {
    const body = currentBody.join("\n").trim();
    if (!currentHeading.trim() && !body) return;
    sections.push({
      id: `${prefix}-${index++}`,
      heading: currentHeading.trim() || "Overview",
      body,
      level: currentLevel,
    });
  };

  for (const line of lines) {
    const match = line.match(/^(#{1,6})\s+(.*)$/);
    if (match) {
      pushSection();
      currentLevel = match[1].length;
      currentHeading = match[2].trim();
      currentBody = [];
      continue;
    }
    currentBody.push(line);
  }

  pushSection();
  return sections;
}

function normalizeHeading(value: string): string {
  return value.toLowerCase().replace(/[^a-z0-9]+/g, " ").trim();
}

function stripMarkdown(value: string): string {
  return value
    .replace(/`([^`]+)`/g, "$1")
    .replace(/\*\*([^*]+)\*\*/g, "$1")
    .replace(/~~([^~]+)~~/g, "$1")
    .replace(/^\d+[a-z]?\.\s*/, "")
    .replace(/^[-*]\s*/, "")
    .replace(/\s+—\s+/g, " — ")
    .trim();
}

function findSectionByHeading(sections: MarkdownSection[], heading: string): MarkdownSection | null {
  const query = normalizeHeading(heading);
  return sections.find((section) => normalizeHeading(section.heading).includes(query)) ?? null;
}

function extractListItems(body: string, limit?: number): string[] {
  const items = body
    .split("\n")
    .map((line) => line.trim())
    .filter((line) => /^[-*]\s+/.test(line) || /^\d+[a-z]?\.\s+/.test(line))
    .map(stripMarkdown)
    .filter(Boolean);
  return typeof limit === "number" ? items.slice(0, limit) : items;
}

function extractHighlights(body: string, limit = 3): string[] {
  const listItems = extractListItems(body, limit);
  if (listItems.length > 0) {
    return listItems;
  }
  return body
    .split("\n")
    .map((line) => stripMarkdown(line.trim()))
    .filter((line) =>
      Boolean(line)
      && !line.startsWith("```")
      && !line.startsWith("|")
      && !/^[-_]{3,}$/.test(line)
      && !line.includes("shadow-editor/")
    )
    .slice(0, limit);
}

function parseMarkdownTable(body: string): { headers: string[]; rows: string[][] } | null {
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
      .map((cell) => stripMarkdown(cell.trim()));
  const headers = parseCells(lines[0]);
  const rows = lines.slice(2).map(parseCells).filter((row) => row.length === headers.length);
  if (!headers.length || !rows.length) {
    return null;
  }
  return { headers, rows };
}

function inferWorkflowAction(input: string): WorkflowAction {
  const normalized = normalizeHeading(input);
  if (
    normalized.includes("terrain")
    || normalized.includes("viewport")
    || normalized.includes("camera")
    || normalized.includes("live preview")
    || normalized.includes("play in editor")
  ) {
    return { label: "Open Live View", target: "live-view", kind: "live-view", primary: true };
  }
  if (
    normalized.includes("reflect")
    || normalized.includes("header tool")
    || normalized.includes("compile commands")
    || normalized.includes("inspector")
  ) {
    return { label: "Reflect Tab", target: "reflect", kind: "game-tab", primary: true };
  }
  if (
    normalized.includes("physics")
    || normalized.includes("scene")
    || normalized.includes("entity")
    || normalized.includes("prefab")
  ) {
    return { label: "Scene Tab", target: "scene", kind: "game-tab", primary: true };
  }
  if (
    normalized.includes("animation")
    || normalized.includes("asset")
    || normalized.includes("material")
    || normalized.includes("tilemap")
  ) {
    return { label: "Assets Tab", target: "assets", kind: "game-tab", primary: true };
  }
  if (
    normalized.includes("ai")
    || normalized.includes("llm")
    || normalized.includes("loader")
    || normalized.includes("ghost text")
    || normalized.includes("autocomplete")
  ) {
    return { label: "AI Tab", target: "ai", kind: "game-tab", primary: true };
  }
  if (
    normalized.includes("plugin")
    || normalized.includes("marketplace")
    || normalized.includes("ecosystem")
  ) {
    return { label: "Open Plugins", target: "plugins", kind: "panel", primary: true };
  }
  if (
    normalized.includes("build")
    || normalized.includes("runtime")
    || normalized.includes("hot reload")
    || normalized.includes("compiler")
    || normalized.includes("installer")
    || normalized.includes("multi platform")
  ) {
    return { label: "Build Tab", target: "build", kind: "game-tab", primary: true };
  }
  if (
    normalized.includes("code")
    || normalized.includes("editor")
    || normalized.includes("clangd")
    || normalized.includes("lsp")
  ) {
    return { label: "Code Tab", target: "code", kind: "game-tab", primary: true };
  }
  return { label: "Plan Tab", target: "plan", kind: "game-tab", primary: true };
}

function statusBadgeStyle(status: string): React.CSSProperties {
  const normalized = normalizeHeading(status);
  if (
    normalized.includes("ready")
    || normalized.includes("live")
    || normalized.includes("built")
    || normalized.includes("integrated")
    || normalized.includes("complete")
    || normalized.includes("bootstrapped")
    || normalized.includes("project open")
  ) {
    return {
      color: "#7ed4a7",
      borderColor: "#7ed4a733",
      background: "#7ed4a714",
    };
  }
  if (
    normalized.includes("needs")
    || normalized.includes("missing")
    || normalized.includes("pending")
    || normalized.includes("attention")
  ) {
    return {
      color: "#f2b36d",
      borderColor: "#f2b36d33",
      background: "#f2b36d14",
    };
  }
  return {
    color: "#8eb5c4",
    borderColor: "#8eb5c433",
    background: "#8eb5c414",
  };
}

export default function PlanenginePanel({
  visible,
  onOpenFile,
  onActivatePanel,
  preferredDoc = "plan",
  onFocusGameTab,
  onOpenLiveView,
  projectPath,
}: Props) {
  const [docs, setDocs] = useState<ShadowPlanengineDocs | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [activeDoc, setActiveDoc] = useState<DocKey>("plan");
  const [projectInfo, setProjectInfo] = useState<ShadowProjectInfo | null>(null);
  const [runtimeStatus, setRuntimeStatus] = useState<ShadowRuntimeStatus | null>(null);
  const [assetCount, setAssetCount] = useState<number | null>(null);
  const [loadingProjectEvidence, setLoadingProjectEvidence] = useState(false);
  const sectionRefs = useRef<Record<string, HTMLDivElement | null>>({});

  const loadDocs = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const nextDocs = await invoke<ShadowPlanengineDocs>("shadow_load_planengine_docs");
      setDocs(nextDocs);
    } catch (loadError) {
      setDocs(null);
      setError(String(loadError));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (visible && !docs && !loading && !error) {
      void loadDocs();
    }
  }, [docs, error, loadDocs, loading, visible]);

  useEffect(() => {
    if (!visible || !projectPath) {
      return;
    }
    let cancelled = false;
    setLoadingProjectEvidence(true);
    Promise.all([
      invoke<ShadowProjectInfo>("shadow_get_project_info", { projectPath }),
      invoke<ShadowRuntimeStatus>("shadow_runtime_status", { projectPath }).catch(() => null),
      invoke<ShadowAssetItem[]>("shadow_list_assets", { projectPath }).catch(() => []),
    ])
      .then(([info, runtime, assets]) => {
        if (cancelled) {
          return;
        }
        setProjectInfo(info);
        setRuntimeStatus(runtime);
        setAssetCount(Array.isArray(assets) ? assets.length : 0);
      })
      .catch(() => {
        if (cancelled) {
          return;
        }
        setProjectInfo(null);
        setRuntimeStatus(null);
        setAssetCount(null);
      })
      .finally(() => {
        if (!cancelled) {
          setLoadingProjectEvidence(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [projectPath, visible]);

  useEffect(() => {
    if (activeDoc === "finish" && docs && !docs.finish_available) {
      setActiveDoc("plan");
    }
  }, [activeDoc, docs]);

  useEffect(() => {
    if (preferredDoc === "finish" && docs && !docs.finish_available) {
      setActiveDoc("plan");
      return;
    }
    setActiveDoc(preferredDoc);
  }, [docs, preferredDoc]);

  const planSections = useMemo(
    () => parseMarkdownSections(docs?.plan_markdown ?? "", "plan"),
    [docs?.plan_markdown],
  );
  const finishSections = useMemo(
    () => parseMarkdownSections(docs?.finish_markdown ?? "", "finish"),
    [docs?.finish_markdown],
  );

  const currentSections = activeDoc === "plan" ? planSections : finishSections;
  const normalizedSearch = search.trim().toLowerCase();
  const filteredSections = useMemo(() => {
    if (!normalizedSearch) return currentSections;
    return currentSections.filter((section) =>
      section.heading.toLowerCase().includes(normalizedSearch)
      || section.body.toLowerCase().includes(normalizedSearch),
    );
  }, [currentSections, normalizedSearch]);

  const outlineSections = useMemo(
    () => filteredSections.filter((section) => section.level <= 2),
    [filteredSections],
  );

  const auditStamp = useMemo(() => {
    const match = (docs?.plan_markdown ?? "").match(/\*\*Last audited:\*\*\s*([^\n]+)/i);
    return match ? stripMarkdown(match[1]) : null;
  }, [docs?.plan_markdown]);

  const visionSection = useMemo(
    () => findSectionByHeading(planSections, "Vision & North Star"),
    [planSections],
  );
  const visionHighlights = useMemo(
    () => extractListItems(visionSection?.body ?? "", 6),
    [visionSection?.body],
  );

  const summarySection = useMemo(
    () => findSectionByHeading(planSections, "Summary"),
    [planSections],
  );
  const auditSummaryTable = useMemo(
    () => parseMarkdownTable(summarySection?.body ?? ""),
    [summarySection?.body],
  );
  const auditTotals = useMemo(() => {
    if (!auditSummaryTable) return null;
    const totalRow =
      auditSummaryTable.rows.find((row) => normalizeHeading(row[0]).includes("total"))
      ?? auditSummaryTable.rows[auditSummaryTable.rows.length - 1];
    if (!totalRow || totalRow.length < 5) {
      return null;
    }
    const parseValue = (value: string) => {
      const parsed = Number.parseInt(value.replace(/[^0-9-]/g, ""), 10);
      return Number.isFinite(parsed) ? parsed : null;
    };
    return {
      done: parseValue(totalRow[1]),
      partial: parseValue(totalRow[2]),
      notStarted: parseValue(totalRow[3]),
      total: parseValue(totalRow[4]),
    };
  }, [auditSummaryTable]);

  const nextStepsSection = useMemo(
    () => findSectionByHeading(planSections, "Recommended Next Steps"),
    [planSections],
  );
  const pendingNextSteps = useMemo(
    () =>
      extractListItems(nextStepsSection?.body ?? "")
        .filter((item) => !item.includes("\u2705"))
        .slice(0, 4),
    [nextStepsSection?.body],
  );
  const firstLaunchSection = useMemo(
    () => findSectionByHeading(planSections, "First-Launch Experience"),
    [planSections],
  );
  const firstLaunchSteps = useMemo(
    () => extractListItems(firstLaunchSection?.body ?? ""),
    [firstLaunchSection?.body],
  );
  const criticalGapsSection = useMemo(
    () => findSectionByHeading(planSections, "Critical Gaps"),
    [planSections],
  );
  const criticalGapsTable = useMemo(
    () => parseMarkdownTable(criticalGapsSection?.body ?? ""),
    [criticalGapsSection?.body],
  );

  const phaseCards = useMemo<IntegratedPhaseCard[]>(
    () =>
      PHASE_INTEGRATIONS
        .map((phase) => {
          const section = findSectionByHeading(planSections, phase.heading);
          if (!section) return null;
          const highlights = extractHighlights(section.body, 3);
          return {
            ...phase,
            section,
            highlights,
            itemCount: extractListItems(section.body).length,
          };
        })
        .filter((phase): phase is IntegratedPhaseCard => Boolean(phase)),
    [planSections],
  );

  const workflowTracks = useMemo<IntegratedWorkflowTrack[]>(
    () =>
      WORKFLOW_TRACKS
        .map((track) => {
          const section = findSectionByHeading(planSections, track.heading);
          if (!section) return null;
          return {
            ...track,
            section,
            highlights: extractHighlights(section.body, 3),
          };
        })
        .filter((track): track is IntegratedWorkflowTrack => Boolean(track)),
    [planSections],
  );

  const executionQueue = useMemo<ExecutionQueueItem[]>(() => {
    if (!projectInfo) {
      return [
        {
          key: "open-project",
          title: "Open or create a Shadow project",
          status: "Needs Project",
          detail: "PlanEngine can map the roadmap onto build, reflection, scenes, and runtime state once a `.shadow_project.toml` workspace is open.",
          action: { label: "Game Panel", target: "gamedev", kind: "panel", primary: true },
        },
        {
          key: "roadmap-first",
          title: "Start from the roadmap",
          status: "Plan Ready",
          detail: "The roadmap is loaded, but project evidence is not available yet. Use the plan to choose a runtime, viewport, AI, or plugin workflow next.",
          action: { label: "Plan Tab", target: "plan", kind: "game-tab", primary: true },
        },
      ];
    }

    const items: ExecutionQueueItem[] = [];
    const pushUnique = (item: ExecutionQueueItem) => {
      if (items.some((existing) => existing.key === item.key)) {
        return;
      }
      items.push(item);
    };

    if (!projectInfo.game_library_exists) {
      pushUnique({
        key: "build-runtime",
        title: "Build the C++23 runtime",
        status: "Needs Build",
        detail: "The first-launch loop and hot-reload host both depend on a loadable shared library being present for this project.",
        action: { label: "Open Build", target: "build", kind: "game-tab", primary: true },
        secondaryAction: { label: "Open Overview", target: "overview", kind: "game-tab" },
      });
    }

    if (!(projectInfo.has_reflection_json && projectInfo.has_reflection_generated_cpp)) {
      pushUnique({
        key: "reflection",
        title: "Regenerate reflection and compile metadata",
        status: "Needs Reflection",
        detail: "The inspector loop in the plan depends on `reflect.json`, generated reflection code, and fresh compile commands.",
        action: { label: "Reflect Tab", target: "reflect", kind: "game-tab", primary: true },
        secondaryAction: { label: "Build Tab", target: "build", kind: "game-tab" },
      });
    }

    if (!projectInfo.entry_scene_path || projectInfo.scenes.length === 0) {
      pushUnique({
        key: "scene-bootstrap",
        title: "Create or wire the authored scene",
        status: "Needs Scene",
        detail: "PlanEngine’s live viewport, inspector, and runtime loop all expect an entry `.shadow` scene to exist and be connected.",
        action: { label: "Scene Tab", target: "scene", kind: "game-tab", primary: true },
        secondaryAction: { label: "Assets Tab", target: "assets", kind: "game-tab" },
      });
    }

    if ((assetCount ?? 0) === 0) {
      pushUnique({
        key: "assets",
        title: "Import starter assets",
        status: "Needs Assets",
        detail: "The asset-pipeline sections in the roadmap map to the same authoring loop; bringing assets in makes the live viewport immediately more useful.",
        action: { label: "Assets Tab", target: "assets", kind: "game-tab", primary: true },
        secondaryAction: { label: "Open Live View", target: "live-view", kind: "live-view" },
      });
    }

    if (!runtimeStatus?.is_live) {
      pushUnique({
        key: "live-loop",
        title: "Enter the live edit loop",
        status: "Needs Runtime",
        detail: "The roadmap’s core promise is edit → rebuild → see changes in the viewport without restarting the editor.",
        action: { label: "Open Live View", target: "live-view", kind: "live-view", primary: true },
        secondaryAction: { label: "Build Tab", target: "build", kind: "game-tab" },
      });
    }

    for (const item of pendingNextSteps) {
      if (items.length >= 5) {
        break;
      }
      const action = inferWorkflowAction(item);
      pushUnique({
        key: `roadmap-${normalizeHeading(item)}`,
        title: item,
        status: "Roadmap Priority",
        detail: "This unresolved roadmap item is still called out in `planengine.md`, so it remains part of the recommended execution order.",
        action,
        secondaryAction: { label: "Plan Tab", target: "plan", kind: "game-tab" },
      });
    }

    return items.slice(0, 5);
  }, [assetCount, pendingNextSteps, projectInfo, runtimeStatus?.is_live]);

  const launchCheckpoints = useMemo<LaunchCheckpoint[]>(() => {
    const steps = firstLaunchSteps.length > 0 ? firstLaunchSteps : [
      "Install ShadowEditor",
      "Open New Project dialog",
      "Pick a starter template",
      "Editor creates project files",
      "Open LLM Loader to set up local AI",
      "Viewport shows scene and code editor shows example C++ component",
      "Save, auto-rebuild, and see the change in the viewport",
    ];

    return [
      {
        key: "launch-open-project",
        step: steps[1] ?? "Open New Project dialog",
        status: projectInfo ? "Project Open" : "Needs Project",
        detail: projectInfo
          ? `Working in ${projectInfo.name}. PlanEngine is attached to the live Shadow project instead of a detached markdown tab.`
          : "Open or create a Shadow project to translate the roadmap into live build, scene, and runtime evidence.",
        action: { label: "Game Panel", target: "gamedev", kind: "panel", primary: true },
      },
      {
        key: "launch-bootstrap",
        step: steps[3] ?? "Editor creates project files",
        status: projectInfo?.has_compile_commands && Boolean(projectInfo.entry_scene_path) ? "Bootstrapped" : "Needs Setup",
        detail: projectInfo
          ? `compile_commands: ${projectInfo.has_compile_commands ? "ready" : "missing"} · entry scene: ${projectInfo.entry_scene || "not configured"}`
          : "Project bootstrap evidence is not loaded yet.",
        action: inferWorkflowAction("build reflection compile commands scene"),
      },
      {
        key: "launch-ai",
        step: steps[4] ?? "Open LLM Loader to set up local AI",
        status: "Integrated",
        detail: "The roadmap’s local AI setup now maps straight to ShadowIDE’s built-in LLM Loader and AI Chat surfaces.",
        action: { label: "LLM Loader", target: "llmloader", kind: "panel", primary: true },
      },
      {
        key: "launch-live-loop",
        step: steps[6] ?? "Save, auto-rebuild, and see the change in the viewport",
        status: runtimeStatus?.is_live ? "Live" : "Needs Runtime",
        detail: runtimeStatus?.is_live
          ? `Runtime is live at frame ${runtimeStatus.frame_index}; the authoring loop is already active for this project.`
          : "Open the live viewport and load the runtime to complete the edit → build → see-it-live loop described in the plan.",
        action: { label: "Open Live View", target: "live-view", kind: "live-view", primary: true },
      },
    ];
  }, [firstLaunchSteps, projectInfo, runtimeStatus?.frame_index, runtimeStatus?.is_live]);

  const criticalGapCards = useMemo<CriticalGapCard[]>(() => {
    if (!criticalGapsTable) {
      return [];
    }
    return criticalGapsTable.rows
      .map((row, index) => {
        const [gapIndex, gap, status, impact, effort] = row;
        if (!gap) {
          return null;
        }
        const normalizedStatus = normalizeHeading(`${gap} ${status}`);
        const unresolved = !normalizedStatus.includes("done") && !gap.includes("~~");
        if (!unresolved) {
          return null;
        }
        return {
          key: gapIndex || `gap-${index}`,
          gap,
          status: status || "Pending",
          impact: impact || "Impact not listed in the roadmap audit.",
          effort: effort || "Effort not listed",
          action: inferWorkflowAction(`${gap} ${impact}`),
        };
      })
      .filter((gap): gap is CriticalGapCard => Boolean(gap))
      .slice(0, 4);
  }, [criticalGapsTable]);

  const runWorkflowAction = useCallback((action: WorkflowAction) => {
    switch (action.kind) {
      case "panel":
        onActivatePanel?.(action.target as SidebarView);
        break;
      case "game-tab":
        onFocusGameTab?.(action.target as GameDevWorkflowTab);
        break;
      case "live-view":
        onOpenLiveView?.();
        break;
      default:
        break;
    }
  }, [onActivatePanel, onFocusGameTab, onOpenLiveView]);

  const assetEvidence = useMemo(() => {
    if (assetCount == null) {
      return "Not loaded";
    }
    return `${assetCount} tracked asset${assetCount === 1 ? "" : "s"}`;
  }, [assetCount]);
  const headerSummary = useMemo(() => {
    if (auditTotals?.done != null && auditTotals?.total != null) {
      return `${auditTotals.done}/${auditTotals.total} done`;
    }
    if (docs) {
      return `${planSections.length} sections`;
    }
    return "Docs";
  }, [auditTotals?.done, auditTotals?.total, docs, planSections.length]);

  const openCurrentDoc = useCallback(() => {
    if (activeDoc === "plan" && docs?.finish_available) {
      setActiveDoc("finish");
      return;
    }
    setActiveDoc("plan");
  }, [activeDoc, docs?.finish_available]);

  const jumpToSection = useCallback((sectionId: string, doc: DocKey = activeDoc) => {
    if (doc !== activeDoc) {
      setActiveDoc(doc);
    }
    if (search) {
      setSearch("");
    }
    window.setTimeout(() => {
      sectionRefs.current[sectionId]?.scrollIntoView({ block: "start", behavior: "smooth" });
    }, doc !== activeDoc || Boolean(search) ? 90 : 0);
  }, [activeDoc, search]);

  if (!visible) return null;

  return (
    <div style={S.root}>
      <div style={S.header}>
        <div>
          <div style={S.kicker}>Integrated Roadmap</div>
          <div style={S.title}>PlanEngine</div>
        </div>
        <div style={S.headerMeta}>
          {headerSummary}
        </div>
      </div>

      <div style={S.actions}>
        <button
          style={{ ...S.actionButton, opacity: loading ? 0.55 : 1 }}
          onClick={() => { void loadDocs(); }}
          disabled={loading}
        >
          {loading ? "Refreshing..." : "Refresh"}
        </button>
        <button
          style={{ ...S.actionButton, opacity: docs ? 1 : 0.55 }}
          onClick={openCurrentDoc}
          disabled={!docs || (activeDoc === "plan" && !docs.finish_available)}
        >
          {activeDoc === "plan" && docs?.finish_available ? "Open Audit" : "Open Roadmap"}
        </button>
        <button
          style={S.actionButton}
          onClick={() => onActivatePanel?.("gamedev")}
        >
          Game Panel
        </button>
        <button
          style={S.actionButton}
          onClick={() => onActivatePanel?.("ai")}
        >
          AI Chat
        </button>
        <button
          style={S.actionButton}
          onClick={() => onActivatePanel?.("plugins")}
        >
          Plugins
        </button>
      </div>

      <div style={S.docSwitch}>
        <button
          style={{ ...S.docSwitchButton, ...(activeDoc === "plan" ? S.docSwitchButtonActive : {}) }}
          onClick={() => setActiveDoc("plan")}
        >
          planengine.md
        </button>
        <button
          style={{
            ...S.docSwitchButton,
            ...(activeDoc === "finish" ? S.docSwitchButtonActive : {}),
            opacity: docs?.finish_available ? 1 : 0.45,
          }}
          onClick={() => {
            if (docs?.finish_available) setActiveDoc("finish");
          }}
          disabled={!docs?.finish_available}
        >
          finish.md
        </button>
      </div>

      <div style={S.searchWrap}>
        <input
          style={S.searchInput}
          value={search}
          onChange={(event) => setSearch(event.target.value)}
          placeholder={`Search ${activeDoc === "plan" ? "PlanEngine" : "audit"} sections`}
          spellCheck={false}
        />
      </div>

      <div style={S.stats}>
        <div style={S.statChip}>
          <span style={S.statValue}>{currentSections.length}</span>
          <span style={S.statLabel}>sections</span>
        </div>
        <div style={S.statChip}>
          <span style={S.statValue}>{outlineSections.length}</span>
          <span style={S.statLabel}>outline</span>
        </div>
        <div style={S.statChip}>
          <span style={S.statValue}>{filteredSections.length}</span>
          <span style={S.statLabel}>visible</span>
        </div>
      </div>

      {!docs?.finish_available && (
        <div style={S.notice}>
          `finish.md` was not found in the workspace, so this panel is showing the full `planengine.md` integration only.
        </div>
      )}

      {error && <div style={{ ...S.notice, color: "#fca5a5", borderColor: "#7f1d1d" }}>{error}</div>}

      {docs && (
        <div style={S.dashboard}>
          <div style={S.sectionKicker}>Start Here</div>
          <div style={S.overviewGrid}>
            <div style={S.overviewCard}>
              <div style={S.overviewTitle}>North Star</div>
              <div style={S.overviewCopy}>
                PlanEngine now acts like an integrated roadmap surface for ShadowIDE instead of a detached note file.
              </div>
              {visionHighlights.length === 0 ? (
                <div style={S.mutedText}>North-star goals were not found in `planengine.md`.</div>
              ) : (
                <div style={S.inlineList}>
                  {visionHighlights.map((item) => (
                    <div key={item} style={S.inlineListItem}>- {item}</div>
                  ))}
                </div>
              )}
              <div style={S.inlineActions}>
                <button style={S.inlineActionButton} onClick={() => onActivatePanel?.("gamedev")}>
                  Open Game Panel
                </button>
                <button style={S.inlineActionButton} onClick={() => onActivatePanel?.("ai")}>
                  Open AI Chat
                </button>
                <button style={S.inlineActionButton} onClick={() => onActivatePanel?.("plugins")}>
                  Open Plugins
                </button>
              </div>
            </div>

            <div style={S.overviewCard}>
              <div style={S.overviewTitle}>Implementation Audit</div>
              {auditTotals ? (
                <div style={S.auditGrid}>
                  <div style={S.auditChip}>
                    <span style={S.auditValue}>{auditTotals.done ?? "?"}</span>
                    <span style={S.auditLabel}>Done</span>
                  </div>
                  <div style={S.auditChip}>
                    <span style={S.auditValue}>{auditTotals.partial ?? "?"}</span>
                    <span style={S.auditLabel}>Partial</span>
                  </div>
                  <div style={S.auditChip}>
                    <span style={S.auditValue}>{auditTotals.notStarted ?? "?"}</span>
                    <span style={S.auditLabel}>Not Started</span>
                  </div>
                  <div style={S.auditChip}>
                    <span style={S.auditValue}>{auditTotals.total ?? "?"}</span>
                    <span style={S.auditLabel}>Total</span>
                  </div>
                </div>
              ) : (
                <div style={S.mutedText}>Audit summary was not found in `planengine.md`.</div>
              )}
              <div style={S.subsectionTitle}>Recommended Next</div>
              {pendingNextSteps.length === 0 ? (
                <div style={S.mutedText}>Remaining next steps were not found in `planengine.md`.</div>
              ) : (
                <div style={S.inlineList}>
                  {pendingNextSteps.map((item) => (
                    <div key={item} style={S.inlineListItem}>- {item}</div>
                  ))}
                </div>
              )}
            </div>
          </div>

          <div style={S.phaseBlock}>
            <div style={S.phaseHeader}>
              <div style={S.sectionKicker}>Execution Queue</div>
              <div style={S.phaseMeta}>Roadmap priorities mapped onto the current Shadow project</div>
            </div>
            <div style={S.workflowGrid}>
              {executionQueue.map((item) => (
                <div key={item.key} style={S.workflowCard}>
                  <div style={{ ...S.phaseAccent, background: "#e9aa5f" }} />
                  <div style={S.phaseCardHeader}>
                    <div>
                      <div style={S.phaseTitle}>{item.title}</div>
                      <div style={{ ...S.statusBadge, ...statusBadgeStyle(item.status) }}>{item.status}</div>
                    </div>
                  </div>
                  <div style={S.phaseDescription}>{item.detail}</div>
                  <div style={S.workflowActions}>
                    <button
                      style={item.action.primary ? { ...S.phaseButton, ...S.phaseButtonPrimary } : S.phaseButton}
                      onClick={() => runWorkflowAction(item.action)}
                    >
                      {item.action.label}
                    </button>
                    {(() => {
                      const secondaryAction = item.secondaryAction;
                      if (!secondaryAction) {
                        return null;
                      }
                      return (
                        <button
                          style={secondaryAction.primary ? { ...S.phaseButton, ...S.phaseButtonPrimary } : S.phaseButton}
                          onClick={() => runWorkflowAction(secondaryAction)}
                        >
                          {secondaryAction.label}
                        </button>
                      );
                    })()}
                  </div>
                </div>
              ))}
            </div>
          </div>

          <div style={S.phaseBlock}>
            <div style={S.phaseHeader}>
              <div style={S.sectionKicker}>First-Launch Flow</div>
              <div style={S.phaseMeta}>From the roadmap’s onboarding loop into the live IDE</div>
            </div>
            <div style={S.workflowGrid}>
              {launchCheckpoints.map((checkpoint) => (
                <div key={checkpoint.key} style={S.workflowCard}>
                  <div style={{ ...S.phaseAccent, background: "#8eb5c4" }} />
                  <div style={S.phaseCardHeader}>
                    <div>
                      <div style={S.phaseTitle}>{checkpoint.step}</div>
                      <div style={{ ...S.statusBadge, ...statusBadgeStyle(checkpoint.status) }}>{checkpoint.status}</div>
                    </div>
                  </div>
                  <div style={S.phaseDescription}>{checkpoint.detail}</div>
                  {(() => {
                    const action = checkpoint.action;
                    if (!action) {
                      return null;
                    }
                    return (
                      <div style={S.workflowActions}>
                        <button
                          style={action.primary ? { ...S.phaseButton, ...S.phaseButtonPrimary } : S.phaseButton}
                          onClick={() => runWorkflowAction(action)}
                        >
                          {action.label}
                        </button>
                      </div>
                    );
                  })()}
                </div>
              ))}
            </div>
          </div>

          <div style={S.phaseBlock}>
            <div style={S.phaseHeader}>
              <div style={S.sectionKicker}>Current Project Evidence</div>
              <div style={S.phaseMeta}>
                {loadingProjectEvidence ? "Syncing with ShadowIDE" : projectInfo ? projectInfo.name : "No Shadow project detected"}
              </div>
            </div>
            {!projectPath ? (
              <div style={S.mutedText}>Open a ShadowEditor project to map the roadmap onto live project evidence.</div>
            ) : !projectInfo ? (
              <div style={S.mutedText}>This workspace does not currently expose ShadowEditor project evidence.</div>
            ) : (
              <div style={S.workflowGrid}>
                <div style={S.workflowCard}>
                  <div style={{ ...S.phaseAccent, background: "#e9aa5f" }} />
                  <div style={S.phaseCardHeader}>
                    <div>
                      <div style={S.phaseTitle}>Build + Runtime</div>
                      <div style={S.phaseStatus}>{runtimeStatus?.is_live ? "Runtime Live" : "Authoring Mode"}</div>
                    </div>
                  </div>
                  <div style={S.inlineList}>
                    <div style={S.inlineListItem}>- {projectInfo.compiler} ({projectInfo.standard})</div>
                    <div style={S.inlineListItem}>- Build system: {projectInfo.build_system}</div>
                    <div style={S.inlineListItem}>- Library: {projectInfo.game_library_exists ? "Built" : "Missing"}</div>
                    <div style={S.inlineListItem}>- {runtimeStatus?.status_line || "Runtime status not loaded"}</div>
                  </div>
                  <div style={S.workflowActions}>
                    <button style={{ ...S.phaseButton, ...S.phaseButtonPrimary }} onClick={() => onFocusGameTab?.("build")}>
                      Build Tab
                    </button>
                    <button style={S.phaseButton} onClick={() => onFocusGameTab?.("scene")}>
                      Scene Tab
                    </button>
                  </div>
                </div>

                <div style={S.workflowCard}>
                  <div style={{ ...S.phaseAccent, background: "#7ed4a7" }} />
                  <div style={S.phaseCardHeader}>
                    <div>
                      <div style={S.phaseTitle}>Reflection + Code</div>
                      <div style={S.phaseStatus}>{projectInfo.has_reflection_json ? "Inspector Ready" : "Needs Reflection"}</div>
                    </div>
                  </div>
                  <div style={S.inlineList}>
                    <div style={S.inlineListItem}>- {projectInfo.source_file_count} source file(s)</div>
                    <div style={S.inlineListItem}>- {projectInfo.header_file_count} header file(s)</div>
                    <div style={S.inlineListItem}>- compile_commands: {projectInfo.has_compile_commands ? "ready" : "missing"}</div>
                    <div style={S.inlineListItem}>- reflect.json / reflect.cpp: {projectInfo.has_reflection_json && projectInfo.has_reflection_generated_cpp ? "generated" : "missing"}</div>
                  </div>
                  <div style={S.workflowActions}>
                    <button style={{ ...S.phaseButton, ...S.phaseButtonPrimary }} onClick={() => onFocusGameTab?.("reflect")}>
                      Reflect Tab
                    </button>
                    <button style={S.phaseButton} onClick={() => onFocusGameTab?.("plan")}>
                      Plan Tab
                    </button>
                  </div>
                </div>

                <div style={S.workflowCard}>
                  <div style={{ ...S.phaseAccent, background: "#8eb5c4" }} />
                  <div style={S.phaseCardHeader}>
                    <div>
                      <div style={S.phaseTitle}>Scenes + Assets</div>
                      <div style={S.phaseStatus}>{projectInfo.entry_scene || "No entry scene"}</div>
                    </div>
                  </div>
                  <div style={S.inlineList}>
                    <div style={S.inlineListItem}>- Entry scene: {projectInfo.entry_scene || "not configured"}</div>
                    <div style={S.inlineListItem}>- {projectInfo.scenes.length} scene file(s)</div>
                    <div style={S.inlineListItem}>- {assetEvidence}</div>
                    <div style={S.inlineListItem}>- Runtime counts: {runtimeStatus ? `${runtimeStatus.entity_count} entities / ${runtimeStatus.component_count} components` : "not loaded"}</div>
                  </div>
                  <div style={S.workflowActions}>
                    <button style={{ ...S.phaseButton, ...S.phaseButtonPrimary }} onClick={() => onFocusGameTab?.("assets")}>
                      Assets Tab
                    </button>
                    <button style={S.phaseButton} onClick={onOpenLiveView}>
                      Live View
                    </button>
                    <button
                      style={S.phaseButton}
                      onClick={() => {
                        if (projectInfo.entry_scene_path) {
                          onOpenFile?.(projectInfo.entry_scene_path, projectInfo.entry_scene || "Main.shadow");
                        }
                      }}
                    >
                      Open Scene File
                    </button>
                  </div>
                </div>

                <div style={S.workflowCard}>
                  <div style={{ ...S.phaseAccent, background: "#c9a8ff" }} />
                  <div style={S.phaseCardHeader}>
                    <div>
                      <div style={S.phaseTitle}>AI + Tooling</div>
                      <div style={S.phaseStatus}>Mapped To IDE Surfaces</div>
                    </div>
                  </div>
                  <div style={S.inlineList}>
                    <div style={S.inlineListItem}>- AI roadmap opens the Game AI tab and AI Chat</div>
                    <div style={S.inlineListItem}>- Local model setup routes into LLM Loader</div>
                    <div style={S.inlineListItem}>- Build and reflection outputs stay in the same authoring loop</div>
                    <div style={S.inlineListItem}>- PlanEngine audit stays available as `finish.md` in the sidebar</div>
                  </div>
                  <div style={S.workflowActions}>
                    <button style={{ ...S.phaseButton, ...S.phaseButtonPrimary }} onClick={() => onFocusGameTab?.("ai")}>
                      AI Tab
                    </button>
                    <button style={S.phaseButton} onClick={() => onActivatePanel?.("ai")}>
                      AI Chat
                    </button>
                    <button style={S.phaseButton} onClick={() => onActivatePanel?.("llmloader")}>
                      LLM Loader
                    </button>
                  </div>
                </div>
              </div>
            )}
          </div>

          <div style={S.phaseBlock}>
            <div style={S.phaseHeader}>
              <div style={S.sectionKicker}>Blocking Gaps</div>
              <div style={S.phaseMeta}>Unresolved audit blockers still called out by the plan</div>
            </div>
            {criticalGapCards.length === 0 ? (
              <div style={S.mutedText}>Unresolved critical gaps were not found in the audit table.</div>
            ) : (
              <div style={S.workflowGrid}>
                {criticalGapCards.map((gap) => (
                  <div key={gap.key} style={S.workflowCard}>
                    <div style={{ ...S.phaseAccent, background: "#f2b36d" }} />
                    <div style={S.phaseCardHeader}>
                      <div>
                        <div style={S.phaseTitle}>{gap.gap}</div>
                        <div style={{ ...S.statusBadge, ...statusBadgeStyle(gap.status) }}>{gap.status}</div>
                      </div>
                      <div style={S.phaseCount}>{gap.effort}</div>
                    </div>
                    <div style={S.inlineList}>
                      <div style={S.inlineListItem}>- Impact: {gap.impact}</div>
                      <div style={S.inlineListItem}>- Effort: {gap.effort}</div>
                    </div>
                    <div style={S.workflowActions}>
                      <button
                        style={gap.action.primary ? { ...S.phaseButton, ...S.phaseButtonPrimary } : S.phaseButton}
                        onClick={() => runWorkflowAction(gap.action)}
                      >
                        {gap.action.label}
                      </button>
                      {criticalGapsSection && (
                        <button
                          style={S.phaseButton}
                          onClick={() => jumpToSection(criticalGapsSection.id, "plan")}
                        >
                          Jump To Audit
                        </button>
                      )}
                    </div>
                  </div>
                ))}
              </div>
            )}
          </div>

          <div style={S.phaseBlock}>
            <div style={S.phaseHeader}>
              <div style={S.sectionKicker}>Integrated Workflows</div>
              <div style={S.phaseMeta}>PlanEngine sections mapped to live IDE surfaces</div>
            </div>
            <div style={S.workflowGrid}>
              {workflowTracks.length === 0 ? (
                <div style={S.mutedText}>Workflow sections were not found in `planengine.md`.</div>
              ) : (
                workflowTracks.map((track) => (
                  <div key={track.key} style={S.workflowCard}>
                    <div style={{ ...S.phaseAccent, background: track.accent }} />
                    <div style={S.phaseCardHeader}>
                      <div>
                        <div style={S.phaseTitle}>{track.heading}</div>
                        <div style={S.phaseStatus}>Integrated Workflow</div>
                      </div>
                    </div>
                    <div style={S.phaseDescription}>{track.description}</div>
                    {track.highlights.length > 0 ? (
                      <div style={S.inlineList}>
                        {track.highlights.map((item) => (
                          <div key={`${track.key}-${item}`} style={S.inlineListItem}>- {item}</div>
                        ))}
                      </div>
                    ) : (
                      <div style={S.mutedText}>No workflow highlights were found in the roadmap body.</div>
                    )}
                    <div style={S.workflowActions}>
                      {track.actions.map((action) => (
                        <button
                          key={`${track.key}-${action.label}`}
                          style={action.primary ? { ...S.phaseButton, ...S.phaseButtonPrimary } : S.phaseButton}
                          onClick={() => runWorkflowAction(action)}
                        >
                          {action.label}
                        </button>
                      ))}
                      <button
                        style={S.phaseButton}
                        onClick={() => jumpToSection(track.section.id, "plan")}
                      >
                        Jump To Section
                      </button>
                    </div>
                  </div>
                ))
              )}
            </div>
          </div>

          <div style={S.phaseBlock}>
            <div style={S.phaseHeader}>
              <div style={S.sectionKicker}>Roadmap Phases</div>
              <div style={S.phaseMeta}>{auditStamp ?? "Roadmap loaded from planengine.md"}</div>
            </div>
            <div style={S.phaseGrid}>
              {phaseCards.length === 0 ? (
                <div style={S.mutedText}>Roadmap phase sections were not found in `planengine.md`.</div>
              ) : (
                phaseCards.map((phase) => (
                  <div key={phase.key} style={S.phaseCard}>
                    <div style={{ ...S.phaseAccent, background: phase.accent }} />
                    <div style={S.phaseCardHeader}>
                      <div>
                        <div style={S.phaseTitle}>{phase.heading}</div>
                        <div style={S.phaseStatus}>{phase.status}</div>
                      </div>
                      <div style={S.phaseCount}>
                        {phase.itemCount > 0 ? `${phase.itemCount} roadmap items` : "Roadmap section"}
                      </div>
                    </div>
                    <div style={S.phaseDescription}>{phase.description}</div>
                    {phase.highlights.length > 0 ? (
                      <div style={S.inlineList}>
                        {phase.highlights.map((item) => (
                          <div key={`${phase.key}-${item}`} style={S.inlineListItem}>- {item}</div>
                        ))}
                      </div>
                    ) : (
                      <div style={S.mutedText}>No phase details were found in the roadmap body.</div>
                    )}
                    <div style={S.phaseActions}>
                      <button
                        style={S.phaseButton}
                        onClick={() => jumpToSection(phase.section.id, "plan")}
                      >
                        Jump To Section
                      </button>
                      <button
                        style={{ ...S.phaseButton, ...S.phaseButtonPrimary }}
                        onClick={() => onActivatePanel?.(phase.panel)}
                      >
                        Open {phase.panelLabel}
                      </button>
                    </div>
                  </div>
                ))
              )}
            </div>
          </div>
        </div>
      )}

      {outlineSections.length > 0 && (
        <div style={S.outline}>
          <div style={S.outlineTitle}>Outline</div>
          <div style={S.outlineList}>
            {outlineSections.map((section) => (
              <button
                key={section.id}
                style={{ ...S.outlineButton, paddingLeft: 10 + ((section.level - 1) * 8) }}
                onClick={() => jumpToSection(section.id)}
              >
                {section.heading}
              </button>
            ))}
          </div>
        </div>
      )}

      <div style={S.scroll}>
        {loading && !docs ? (
          <div style={S.placeholder}>Loading PlanEngine documents...</div>
        ) : !docs ? (
          <div style={S.placeholder}>PlanEngine docs are not loaded yet.</div>
        ) : filteredSections.length === 0 ? (
          <div style={S.placeholder}>No sections match that search.</div>
        ) : (
          filteredSections.map((section) => (
            <div
              key={section.id}
              ref={(element) => {
                sectionRefs.current[section.id] = element;
              }}
              style={S.card}
            >
              <div style={{ ...S.cardHeading, fontSize: section.level <= 2 ? 13 : 12 }}>
                {"#".repeat(Math.min(section.level, 6))} {section.heading}
              </div>
              {section.body && <pre style={S.cardBody}>{section.body}</pre>}
            </div>
          ))
        )}
      </div>
    </div>
  );
}

const S: Record<string, React.CSSProperties> = {
  root: {
    display: "flex",
    flexDirection: "column",
    height: "100%",
    minHeight: 0,
    background: "var(--bg-primary, #0f1418)",
    color: "var(--text-primary, #ecf1f4)",
  },
  header: {
    display: "flex",
    alignItems: "flex-start",
    justifyContent: "space-between",
    gap: 10,
    padding: "12px 14px 8px",
    borderBottom: "1px solid #1e2a30",
  },
  kicker: {
    fontSize: 10,
    color: "#8eb5c4",
    textTransform: "uppercase",
    letterSpacing: 1,
    marginBottom: 2,
  },
  title: {
    fontSize: 16,
    fontWeight: 700,
    color: "#ecf1f4",
  },
  headerMeta: {
    fontSize: 10,
    color: "#e9aa5f",
    border: "1px solid #e9aa5f33",
    background: "#e9aa5f11",
    borderRadius: 999,
    padding: "3px 8px",
    whiteSpace: "nowrap",
  },
  actions: {
    display: "flex",
    flexWrap: "wrap",
    gap: 6,
    padding: "10px 14px 8px",
  },
  actionButton: {
    padding: "5px 9px",
    borderRadius: 6,
    border: "1px solid #24343d",
    background: "#11181d",
    color: "#8eb5c4",
    fontSize: 11,
    fontWeight: 600,
    cursor: "pointer",
  },
  docSwitch: {
    display: "grid",
    gridTemplateColumns: "repeat(2, minmax(0, 1fr))",
    gap: 6,
    padding: "0 14px 8px",
  },
  docSwitchButton: {
    padding: "7px 8px",
    borderRadius: 8,
    border: "1px solid #1e2a30",
    background: "#0d151a",
    color: "#8eb5c4",
    fontSize: 11,
    fontWeight: 700,
    cursor: "pointer",
  },
  docSwitchButtonActive: {
    color: "#e9aa5f",
    borderColor: "#e9aa5f55",
    background: "#e9aa5f12",
  },
  searchWrap: {
    padding: "0 14px 8px",
  },
  searchInput: {
    width: "100%",
    background: "#0b1116",
    border: "1px solid #1e2a30",
    borderRadius: 8,
    color: "#ecf1f4",
    fontSize: 12,
    padding: "8px 10px",
    boxSizing: "border-box",
    outline: "none",
  },
  stats: {
    display: "grid",
    gridTemplateColumns: "repeat(3, minmax(0, 1fr))",
    gap: 6,
    padding: "0 14px 8px",
  },
  statChip: {
    display: "flex",
    flexDirection: "column",
    gap: 2,
    padding: "8px 9px",
    borderRadius: 8,
    border: "1px solid #1e2a30",
    background: "#11181d",
  },
  statValue: {
    fontSize: 14,
    fontWeight: 700,
    color: "#ecf1f4",
  },
  statLabel: {
    fontSize: 10,
    color: "#8eb5c4",
    textTransform: "uppercase",
    letterSpacing: 0.8,
  },
  notice: {
    margin: "0 14px 8px",
    padding: "8px 10px",
    borderRadius: 8,
    border: "1px solid #1e2a30",
    background: "#11181d",
    color: "#8eb5c4",
    fontSize: 11,
    lineHeight: 1.5,
  },
  dashboard: {
    display: "flex",
    flexDirection: "column",
    gap: 8,
    padding: "0 14px 8px",
  },
  sectionKicker: {
    fontSize: 10,
    fontWeight: 700,
    color: "#e9aa5f",
    textTransform: "uppercase",
    letterSpacing: 1,
  },
  overviewGrid: {
    display: "grid",
    gridTemplateColumns: "minmax(0, 1fr)",
    gap: 8,
  },
  overviewCard: {
    padding: "12px",
    borderRadius: 10,
    border: "1px solid #1e2a30",
    background: "#11181d",
  },
  overviewTitle: {
    fontSize: 12,
    fontWeight: 700,
    color: "#ecf1f4",
    marginBottom: 6,
  },
  overviewCopy: {
    fontSize: 11,
    color: "#8eb5c4",
    lineHeight: 1.55,
    marginBottom: 8,
  },
  subsectionTitle: {
    marginTop: 10,
    marginBottom: 6,
    fontSize: 10,
    fontWeight: 700,
    color: "#e9aa5f",
    textTransform: "uppercase",
    letterSpacing: 0.8,
  },
  inlineList: {
    display: "flex",
    flexDirection: "column",
    gap: 5,
  },
  inlineListItem: {
    fontSize: 11,
    color: "#d6e0e5",
    lineHeight: 1.45,
  },
  inlineActions: {
    display: "flex",
    flexWrap: "wrap",
    gap: 6,
    marginTop: 10,
  },
  inlineActionButton: {
    padding: "5px 9px",
    borderRadius: 6,
    border: "1px solid #24343d",
    background: "#0d151a",
    color: "#8eb5c4",
    fontSize: 11,
    fontWeight: 600,
    cursor: "pointer",
  },
  auditGrid: {
    display: "grid",
    gridTemplateColumns: "repeat(4, minmax(0, 1fr))",
    gap: 6,
  },
  auditChip: {
    display: "flex",
    flexDirection: "column",
    gap: 2,
    padding: "8px 9px",
    borderRadius: 8,
    border: "1px solid #1e2a30",
    background: "#0d151a",
  },
  auditValue: {
    fontSize: 14,
    fontWeight: 700,
    color: "#ecf1f4",
  },
  auditLabel: {
    fontSize: 10,
    color: "#8eb5c4",
    textTransform: "uppercase",
    letterSpacing: 0.8,
  },
  mutedText: {
    fontSize: 11,
    color: "#6f8792",
    lineHeight: 1.45,
  },
  phaseBlock: {
    display: "flex",
    flexDirection: "column",
    gap: 8,
  },
  phaseHeader: {
    display: "flex",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
  },
  phaseMeta: {
    fontSize: 10,
    color: "#8eb5c4",
  },
  phaseGrid: {
    display: "grid",
    gridTemplateColumns: "minmax(0, 1fr)",
    gap: 8,
  },
  workflowGrid: {
    display: "grid",
    gridTemplateColumns: "minmax(0, 1fr)",
    gap: 8,
  },
  phaseCard: {
    position: "relative",
    padding: "12px",
    borderRadius: 10,
    border: "1px solid #1e2a30",
    background: "#11181d",
    overflow: "hidden",
  },
  workflowCard: {
    position: "relative",
    padding: "12px",
    borderRadius: 10,
    border: "1px solid #1e2a30",
    background: "#11181d",
    overflow: "hidden",
  },
  phaseAccent: {
    position: "absolute",
    inset: "0 auto 0 0",
    width: 3,
    opacity: 0.95,
  },
  phaseCardHeader: {
    display: "flex",
    alignItems: "flex-start",
    justifyContent: "space-between",
    gap: 8,
    marginBottom: 6,
  },
  phaseTitle: {
    fontSize: 12,
    fontWeight: 700,
    color: "#ecf1f4",
  },
  phaseStatus: {
    fontSize: 10,
    color: "#8eb5c4",
    marginTop: 2,
  },
  statusBadge: {
    display: "inline-flex",
    alignItems: "center",
    gap: 4,
    marginTop: 4,
    padding: "3px 8px",
    borderRadius: 999,
    border: "1px solid #24343d",
    fontSize: 10,
    fontWeight: 700,
    letterSpacing: 0.3,
    width: "fit-content",
  },
  phaseCount: {
    fontSize: 10,
    color: "#6f8792",
    whiteSpace: "nowrap",
  },
  phaseDescription: {
    fontSize: 11,
    color: "#8eb5c4",
    lineHeight: 1.5,
    marginBottom: 8,
  },
  phaseActions: {
    display: "flex",
    flexWrap: "wrap",
    gap: 6,
    marginTop: 10,
  },
  workflowActions: {
    display: "flex",
    flexWrap: "wrap",
    gap: 6,
    marginTop: 10,
  },
  phaseButton: {
    padding: "5px 9px",
    borderRadius: 6,
    border: "1px solid #24343d",
    background: "#0d151a",
    color: "#8eb5c4",
    fontSize: 11,
    fontWeight: 600,
    cursor: "pointer",
  },
  phaseButtonPrimary: {
    borderColor: "#e9aa5f55",
    background: "#e9aa5f12",
    color: "#e9aa5f",
  },
  outline: {
    margin: "0 14px 8px",
    padding: "10px",
    borderRadius: 10,
    border: "1px solid #1e2a30",
    background: "#0d151a",
  },
  outlineTitle: {
    fontSize: 10,
    fontWeight: 700,
    color: "#e9aa5f",
    textTransform: "uppercase",
    letterSpacing: 1,
    marginBottom: 8,
  },
  outlineList: {
    display: "flex",
    flexDirection: "column",
    gap: 4,
    maxHeight: 160,
    overflowY: "auto",
  },
  outlineButton: {
    padding: "6px 10px",
    borderRadius: 6,
    border: "1px solid transparent",
    background: "#11181d",
    color: "#cfd9de",
    fontSize: 11,
    textAlign: "left",
    cursor: "pointer",
  },
  scroll: {
    flex: 1,
    minHeight: 0,
    overflowY: "auto",
    padding: "0 14px 14px",
  },
  placeholder: {
    padding: "24px 8px",
    color: "#6f8792",
    textAlign: "center",
    fontSize: 12,
  },
  card: {
    marginBottom: 10,
    padding: "10px 12px",
    borderRadius: 10,
    border: "1px solid #1e2a30",
    background: "#11181d",
  },
  cardHeading: {
    color: "#ecf1f4",
    fontWeight: 700,
    marginBottom: 8,
    lineHeight: 1.35,
  },
  cardBody: {
    margin: 0,
    color: "#cfd9de",
    fontSize: 10.5,
    lineHeight: 1.55,
    fontFamily: "monospace",
    whiteSpace: "pre-wrap",
    wordBreak: "break-word",
  },
};
