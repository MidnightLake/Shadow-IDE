import React, { useState, useCallback, useEffect, useRef, memo, Suspense } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, emit } from "@tauri-apps/api/event";
import { ask } from "@tauri-apps/plugin-dialog";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ErrorBoundary } from "./components/ErrorBoundary";
import FileExplorer from "./components/FileExplorer";
import { getLanguageFromFilename } from "./components/Editor";
const Editor = React.lazy(() => import("./components/Editor"));
import TerminalPanel, { type SavedTerminalSession } from "./components/Terminal";
import RemoteSettings from "./components/RemoteSettings";
import TodoPanel from "./components/TodoPanel";
import SearchPanel from "./components/SearchPanel";
import SettingsPanel from "./components/SettingsPanel";
import LlmLoader from "./components/LlmLoader";
import LanguagesPanel from "./components/LanguagesPanel";
import LogPanel from "./components/LogPanel";
import RagPanel from "./components/RagPanel";
import { BluetoothPanel } from "./components/BluetoothPanel";
import { FerrumChat } from "./components/FerrumChat";
import { TitleBar } from "./components/TitleBar";
import { ActivityBar } from "./components/ActivityBar";
import { StatusBar } from "./components/StatusBar";
import { DiagnosticPanel } from "./components/DiagnosticPanel";
import CollaborationPanel from "./components/CollaborationPanel";
import GitGraphPanel from "./components/GitGraphPanel";
import TestExplorerPanel from "./components/TestExplorerPanel";
import AgentPanel from "./components/AgentPanel";
import CommandPalette from "./components/CommandPalette";
import DatabasePanel from "./components/DatabasePanel";
import DebugPanel from "./components/DebugPanel";
import DependencyGraph from "./components/DependencyGraph";
import MarkdownPreview from "./components/MarkdownPreview";
import EditHistoryPanel from "./components/EditHistoryPanel";
import KeybindingsPanel from "./components/KeybindingsPanel";
import DocsPanel from "./components/DocsPanel";
import QuickOpenPanel from "./components/QuickOpenPanel";
import ProfilerPanel from "./components/ProfilerPanel";
import GlslPreviewPanel from "./components/GlslPreviewPanel";
import PrPanel from "./components/PrPanel";
import PluginsPanel from "./components/PluginsPanel";
import MutationPanel from "./components/MutationPanel";
import CiCdPanel from "./components/CiCdPanel";
import GameDevPanel from "./components/GameDevPanel";
import ShadowGameWorkspace from "./components/ShadowGameWorkspace";
import PlanenginePanel from "./components/PlanenginePanel";
import JupyterPanel from "./components/JupyterPanel";
import OrgModePanel from "./components/OrgModePanel";
import MeshViewer from "./components/MeshViewer";
import { GAMEDEV_LIVE_VIEW_NAME, GAMEDEV_LIVE_VIEW_PATH, isGameDevLiveViewPath, isShadowIdeVirtualPath } from "./components/gamedevLiveView";
import { summarizePlanengineMarkdown, type PlanengineShellSummary } from "./planengine/summary";
import { useResize } from "./hooks/useResize";
import { useAppState, type PersistedSettings } from "./hooks/useAppState";
import type { SidebarView, PanelZone, RecentProject, WorkspaceSettings } from "./types";
import "./App.css";

// Memoize large components to prevent re-renders on cursor movement
const MemoizedFileExplorer = memo(FileExplorer);
const MemoizedFerrumChat = memo(FerrumChat);
const MemoizedTerminalPanel = memo(TerminalPanel);
const MemoizedLlmLoader = memo(LlmLoader);
const MemoizedTodoPanel = memo(TodoPanel);
const MemoizedSearchPanel = memo(SearchPanel);
const MemoizedSettingsPanel = memo(SettingsPanel);
const MemoizedLanguagesPanel = memo(LanguagesPanel);
const MemoizedLogPanel = memo(LogPanel);
const MemoizedRagPanel = memo(RagPanel);
const MemoizedBluetoothPanel = memo(BluetoothPanel);
const MemoizedCollaborationPanel = memo(CollaborationPanel);
const MemoizedRemoteSettings = memo(RemoteSettings);
const MemoizedGitGraphPanel = memo(GitGraphPanel);
const MemoizedTestExplorerPanel = memo(TestExplorerPanel);
const MemoizedAgentPanel = memo(AgentPanel);
const MemoizedDatabasePanel = memo(DatabasePanel);
const MemoizedDebugPanel = memo(DebugPanel);
const MemoizedDependencyGraph = memo(DependencyGraph);
const MemoizedEditHistoryPanel = memo(EditHistoryPanel);
const MemoizedKeybindingsPanel = memo(KeybindingsPanel);
const MemoizedDocsPanel = memo(DocsPanel);
const MemoizedProfilerPanel = memo(ProfilerPanel);
const MemoizedGlslPreviewPanel = memo(GlslPreviewPanel);
const MemoizedPrPanel = memo(PrPanel);
const MemoizedPluginsPanel = memo(PluginsPanel);
const MemoizedMutationPanel = memo(MutationPanel);
const MemoizedCiCdPanel = memo(CiCdPanel);
const MemoizedGameDevPanel = memo(GameDevPanel);
const MemoizedShadowGameWorkspace = memo(ShadowGameWorkspace);
const MemoizedPlanenginePanel = memo(PlanenginePanel);

const SETTINGS_KEY = "shadowide-settings";
type IntegratedPlanengineDocKey = "plan" | "finish";
type GameDevTab = "overview" | "scene" | "code" | "assets" | "reflect" | "build" | "ai" | "plan";

interface ShadowPlanengineDocs {
  plan_path: string;
  finish_path: string;
  plan_markdown: string;
  finish_markdown: string;
  finish_available: boolean;
}

function canScrollVertically(element: HTMLElement): boolean {
  const style = window.getComputedStyle(element);
  return /(auto|scroll)/.test(style.overflowY) && element.scrollHeight > element.clientHeight;
}

function findScrollableDescendant(start: HTMLElement | null, boundary: HTMLElement): HTMLElement | null {
  let current = start;

  while (current && current !== boundary) {
    if (canScrollVertically(current)) {
      return current;
    }
    current = current.parentElement;
  }

  return canScrollVertically(boundary) ? boundary : null;
}

function loadSettings(): PersistedSettings {
  try {
    const raw = localStorage.getItem(SETTINGS_KEY);
    if (raw) return JSON.parse(raw) as PersistedSettings;
  } catch { /* ignore */ }
  return {};
}

const isMobileDevice = /iPhone|iPad|iPod|Android/i.test(navigator.userAgent);

function isIntegratedPlanengineDocPath(path?: string | null): boolean {
  if (!path) {
    return false;
  }
  const normalized = path.replace(/\\/g, "/").toLowerCase();
  return normalized.endsWith("/planengine.md")
    || normalized.endsWith("/finish.md")
    || normalized === "planengine.md"
    || normalized === "finish.md";
}

function getIntegratedPlanengineDocKey(path?: string | null): IntegratedPlanengineDocKey | null {
  if (!path) {
    return null;
  }
  const normalized = path.replace(/\\/g, "/").toLowerCase();
  if (normalized.endsWith("/finish.md") || normalized === "finish.md") {
    return "finish";
  }
  if (normalized.endsWith("/planengine.md") || normalized === "planengine.md") {
    return "plan";
  }
  return null;
}

function App() {
  const [saved] = useState(loadSettings);
  const [state, dispatch] = useAppState(saved);
  const [hasShadowProject, setHasShadowProject] = useState(false);
  const [gameViewportVisible, setGameViewportVisible] = useState(saved.gameViewportVisible ?? true);
  const [gameViewportWidth, setGameViewportWidth] = useState(saved.gameViewportWidth ?? 560);
  const [resizingGameViewport, setResizingGameViewport] = useState(false);
  const [preferredPlanengineDoc, setPreferredPlanengineDoc] = useState<IntegratedPlanengineDocKey>("plan");
  const [planengineShellSummary, setPlanengineShellSummary] = useState<PlanengineShellSummary | null>(null);

  const {
    rootPath, openFiles, activeFileIndex, recentProjects, workspaceSettings,
    panelZones, leftView, rightView, sidebarAutoHide, sidebarHidden,
    terminalVisible, terminalSessions,
    showRecentMenu, contextMenu, showStatusBar, showErrorPanel, aiFullscreen,
    cursorInfo, diagnosticCounts, diagnosticItems,
    oledMode, aiCompletionEnabled, systemPrompt,
  } = state;

  const sidebarTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const saveTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const editorAreaRef = useRef<HTMLDivElement | null>(null);
  const autoOpenedPlanengineProjectRef = useRef<string | null>(null);
  const everVisited = useRef<Set<string>>(new Set(
    [leftView, rightView].filter(Boolean) as string[]
  ));
  const [quickOpenOpen, setQuickOpenOpen] = useState(false);

  // Resize hook
  const resize = useResize(
    saved.leftSidebarWidth ?? 250,
    saved.rightSidebarWidth ?? 250,
    saved.terminalHeight ?? 250,
  );

  const activeFile = openFiles[activeFileIndex];
  const currentLanguage = activeFile ? getLanguageFromFilename(activeFile.name) : "";
  const hasLeftSidebar = leftView !== null;
  const hasRightSidebar = rightView !== null;
  const activeIntegratedPlanDocKey = getIntegratedPlanengineDocKey(activeFile?.path);
  const isLiveViewportTab = Boolean(activeFile && isGameDevLiveViewPath(activeFile.path));
  const isIntegratedPlanDocActive = Boolean(activeFile && isIntegratedPlanengineDocPath(activeFile.path));
  const showPlanengineLaunchpad = !activeFile && hasShadowProject;
  const showDockedGameViewport = !isMobileDevice && hasShadowProject && gameViewportVisible && !isLiveViewportTab;
  const showGameViewportLauncher = !isMobileDevice && hasShadowProject && !isLiveViewportTab && !isIntegratedPlanDocActive;

  const loadTextFile = useCallback(async (filePath: string, size: number) => {
    if (!isMobileDevice || size <= 256 * 1024) {
      return invoke<string>("read_file_content", { path: filePath });
    }

    let offset = 0;
    let content = "";
    const chunkSize = 64 * 1024;
    while (offset < size) {
      const chunk = await invoke<{ content: string; done: boolean; length: number }>("read_file_chunk", {
        path: filePath,
        offset,
        length: chunkSize,
      });
      content += chunk?.content ?? "";
      if (!chunk || chunk.done || !chunk.length) break;
      offset += chunk.length;
    }
    return content;
  }, []);

  const openVirtualFile = useCallback((path: string, name: string, content = "") => {
    const existingIndex = openFiles.findIndex((file) => file.path === path);
    if (existingIndex !== -1) {
      dispatch({ type: "SET_ACTIVE_FILE_INDEX", payload: existingIndex });
      return;
    }

    dispatch({
      type: "UPDATE_OPEN_FILES",
      payload: (prev) => [...prev, { path, name, content, modified: false, size: 0 }],
    });
    dispatch({ type: "SET_ACTIVE_FILE_INDEX", payload: openFiles.length });
  }, [openFiles]);

  // Helper: open a file by path, reading content and adding to open files
  const openFileByPath = async (filePath: string): Promise<boolean> => {
    const integratedDoc = getIntegratedPlanengineDocKey(filePath);
    if (integratedDoc) {
      focusIntegratedPlanengineDoc(integratedDoc);
      return false;
    }

    if (isShadowIdeVirtualPath(filePath)) {
      openVirtualFile(filePath, isGameDevLiveViewPath(filePath) ? GAMEDEV_LIVE_VIEW_NAME : filePath);
      return true;
    }

    const fileName = filePath.split("/").pop() || filePath;
    try {
      const info = await invoke<{ size: number; is_binary: boolean }>("get_file_info", { path: filePath });
      if (!info.is_binary && info.size <= 50 * 1024 * 1024) {
        const content = await loadTextFile(filePath, info.size);
        dispatch({ type: "UPDATE_OPEN_FILES", payload: (prev) => [...prev, { path: filePath, name: fileName, content, modified: false, size: info.size }] });
        return true;
      }
    } catch { /* skip missing files */ }
    return false;
  };

  useEffect(() => {
    let cancelled = false;

    if (!rootPath) {
      setHasShadowProject(false);
      return;
    }

    const configPath = `${rootPath.replace(/[\\/]+$/, "")}/.shadow_project.toml`.replace(/\\/g, "/");
    invoke("get_file_info", { path: configPath })
      .then(() => {
        if (!cancelled) {
          setHasShadowProject(true);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setHasShadowProject(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [rootPath]);

  useEffect(() => {
    let cancelled = false;

    if (!hasShadowProject || !rootPath) {
      setPlanengineShellSummary(null);
      return;
    }

    invoke<ShadowPlanengineDocs>("shadow_load_planengine_docs")
      .then((docs) => {
        if (!cancelled) {
          setPlanengineShellSummary(summarizePlanengineMarkdown(docs.plan_markdown));
        }
      })
      .catch(() => {
        if (!cancelled) {
          setPlanengineShellSummary(null);
        }
      });

    return () => {
      cancelled = true;
    };
  }, [hasShadowProject, rootPath]);

  // Sync OLED mode to DOM
  useEffect(() => {
    document.documentElement.setAttribute("data-oled", String(oledMode));
  }, [oledMode]);

  // Persist UI settings to localStorage (debounced)
  const settingsTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    if (settingsTimerRef.current) clearTimeout(settingsTimerRef.current);
    settingsTimerRef.current = setTimeout(() => {
      const s: PersistedSettings = {
        oledMode, sidebarAutoHide, showStatusBar,
        leftSidebarWidth: resize.leftSidebarWidth,
        rightSidebarWidth: resize.rightSidebarWidth,
        terminalHeight: resize.terminalHeight,
        terminalVisible,
        gameViewportVisible,
        gameViewportWidth,
        aiCompletionEnabled,
        lastProjectPath: rootPath || undefined,
        panelZones, leftView, rightView, systemPrompt,
      };
      try { localStorage.setItem(SETTINGS_KEY, JSON.stringify(s)); } catch { /* ignore */ }
    }, 500);
    return () => { if (settingsTimerRef.current) clearTimeout(settingsTimerRef.current); };
  }, [oledMode, sidebarAutoHide, showStatusBar, resize.leftSidebarWidth, resize.rightSidebarWidth, resize.terminalHeight, terminalVisible, gameViewportVisible, gameViewportWidth, aiCompletionEnabled, rootPath, panelZones, leftView, rightView, systemPrompt]);

  // Sidebar auto-hide (3s idle timeout)
  const resetSidebarTimer = useCallback(() => {
    if (!sidebarAutoHide) return;
    dispatch({ type: "SET_SIDEBAR_HIDDEN", payload: false });
    if (sidebarTimerRef.current) clearTimeout(sidebarTimerRef.current);
    sidebarTimerRef.current = setTimeout(() => dispatch({ type: "SET_SIDEBAR_HIDDEN", payload: true }), 3000);
  }, [sidebarAutoHide]);

  const closeIntegratedPlanengineTabs = useCallback(() => {
    const integratedIndexes = openFiles
      .map((file, index) => (isIntegratedPlanengineDocPath(file.path) ? index : -1))
      .filter((index) => index !== -1);

    if (integratedIndexes.length === 0) {
      return;
    }

    const remainingFiles = openFiles.filter((file) => !isIntegratedPlanengineDocPath(file.path));
    const removedBeforeActive = integratedIndexes.filter((index) => index < activeFileIndex).length;
    const nextIndex = remainingFiles.length === 0
      ? 0
      : Math.max(0, Math.min(remainingFiles.length - 1, activeFileIndex - removedBeforeActive));

    dispatch({
      type: "UPDATE_OPEN_FILES",
      payload: () => remainingFiles,
    });
    dispatch({ type: "SET_ACTIVE_FILE_INDEX", payload: nextIndex });
  }, [activeFileIndex, openFiles]);

  const focusIntegratedPlanengineDoc = useCallback((docKey: IntegratedPlanengineDocKey) => {
    setPreferredPlanengineDoc(docKey);
    closeIntegratedPlanengineTabs();
    everVisited.current.add("planengine");
    const zone = panelZones.planengine;
    if (zone === "left") {
      dispatch({ type: "SET_LEFT_VIEW", payload: "planengine" });
    } else {
      dispatch({ type: "SET_RIGHT_VIEW", payload: "planengine" });
    }
    if (sidebarAutoHide) {
      resetSidebarTimer();
    }
  }, [closeIntegratedPlanengineTabs, dispatch, panelZones.planengine, resetSidebarTimer, sidebarAutoHide]);

  useEffect(() => {
    if (!sidebarAutoHide) {
      dispatch({ type: "SET_SIDEBAR_HIDDEN", payload: false });
      if (sidebarTimerRef.current) clearTimeout(sidebarTimerRef.current);
    } else {
      resetSidebarTimer();
    }
    return () => { if (sidebarTimerRef.current) clearTimeout(sidebarTimerRef.current); };
  }, [sidebarAutoHide, resetSidebarTimer]);

  useEffect(() => {
    if (!hasShadowProject || !rootPath || openFiles.length > 0) {
      return;
    }
    if (autoOpenedPlanengineProjectRef.current === rootPath) {
      return;
    }
    autoOpenedPlanengineProjectRef.current = rootPath;
    focusIntegratedPlanengineDoc("plan");
  }, [focusIntegratedPlanengineDoc, hasShadowProject, openFiles.length, rootPath]);

  useEffect(() => {
    if (!resizingGameViewport) {
      return;
    }

    const handleMouseMove = (event: MouseEvent) => {
      const rect = editorAreaRef.current?.getBoundingClientRect();
      if (!rect) {
        return;
      }
      const nextWidth = rect.right - event.clientX;
      const maxWidth = Math.max(420, Math.min(980, rect.width - 220));
      setGameViewportWidth(Math.max(360, Math.min(maxWidth, nextWidth)));
    };

    const handleMouseUp = () => {
      setResizingGameViewport(false);
    };

    document.addEventListener("mousemove", handleMouseMove);
    document.addEventListener("mouseup", handleMouseUp);
    document.body.style.userSelect = "none";
    document.body.style.cursor = "col-resize";

    return () => {
      document.removeEventListener("mousemove", handleMouseMove);
      document.removeEventListener("mouseup", handleMouseUp);
      document.body.style.userSelect = "";
      document.body.style.cursor = "";
    };
  }, [resizingGameViewport]);

  // Load home directory + recent projects on mount; restore last project
  useEffect(() => {
    const init = async () => {
      loadRecentProjects();
      const pcRoot = isMobileDevice ? (window as unknown as { __PC_WORKSPACE_ROOT__?: string }).__PC_WORKSPACE_ROOT__ : null;
      if (pcRoot) {
        dispatch({ type: "SET_ROOT_PATH", payload: pcRoot });
      } else if (!isMobileDevice && saved.lastProjectPath) {
        const lastPath = saved.lastProjectPath;
        try {
          await invoke("project_open", { path: lastPath });
          dispatch({ type: "SET_ROOT_PATH", payload: lastPath });
          const projectState = await invoke<{
            root_path: string; open_files: string[]; active_file_index: number;
            sidebar_view: string; sidebar_width: number;
            terminal_visible: boolean; terminal_height: number;
            ai_completion_enabled: boolean;
            terminal_sessions?: SavedTerminalSession[];
          } | null>("project_load_state", { rootPath: lastPath });
          if (projectState) {
            dispatch({ type: "SET_TERMINAL_SESSIONS", payload: projectState.terminal_sessions ?? [] });
            const restorablePaths = projectState.open_files.filter((filePath) =>
              !isShadowIdeVirtualPath(filePath) && !isIntegratedPlanengineDocPath(filePath),
            );
            let restoredCount = 0;
            for (const filePath of restorablePaths) {
              if (await openFileByPath(filePath)) {
                restoredCount += 1;
              }
            }
            dispatch({
              type: "SET_ACTIVE_FILE_INDEX",
              payload: restoredCount > 0 ? Math.max(0, Math.min(projectState.active_file_index, restoredCount - 1)) : 0,
            });
          }
        } catch {
          const home = await invoke<string>("get_home_dir");
          dispatch({ type: "SET_ROOT_PATH", payload: home });
        }
      } else {
        const home = await invoke<string>("get_home_dir");
        dispatch({ type: "SET_ROOT_PATH", payload: home });
      }
    };
    init();
    if (isMobileDevice) {
      const onWsState = (e: Event) => {
        const detail = (e as CustomEvent).detail;
        if (detail?.project_root) dispatch({ type: "SET_ROOT_PATH", payload: detail.project_root });
      };
      window.addEventListener("mobile-workspace-state", onWsState);
      return () => window.removeEventListener("mobile-workspace-state", onWsState);
    }
  }, []);

  // Load workspace settings when project changes
  useEffect(() => {
    if (!rootPath) return;
    invoke<WorkspaceSettings | null>("project_load_config", { rootPath })
      .then((config) => { if (config) dispatch({ type: "SET_WORKSPACE_SETTINGS", payload: config }); })
      .catch(() => {});
  }, [rootPath]);

  // Save workspace settings on change (debounced)
  const wsTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    if (!rootPath) return;
    if (wsTimerRef.current) clearTimeout(wsTimerRef.current);
    wsTimerRef.current = setTimeout(() => {
      invoke("project_save_config", { rootPath, config: workspaceSettings }).catch(() => {});
    }, 1000);
    return () => { if (wsTimerRef.current) clearTimeout(wsTimerRef.current); };
  }, [rootPath, workspaceSettings]);

  const updateWorkspaceSetting = useCallback(<K extends keyof WorkspaceSettings>(key: K, value: WorkspaceSettings[K]) => {
    dispatch({ type: "UPDATE_WORKSPACE_SETTINGS", payload: (prev) => ({ ...prev, [key]: value }) });
  }, []);

  // Auto-save project state
  useEffect(() => {
    if (!rootPath) return;
    if (saveTimerRef.current) clearTimeout(saveTimerRef.current);
    saveTimerRef.current = setTimeout(() => saveProjectState(), 2000);
    return () => { if (saveTimerRef.current) clearTimeout(saveTimerRef.current); };
  }, [rootPath, openFiles, activeFileIndex, leftView, resize.leftSidebarWidth, terminalVisible, resize.terminalHeight, aiCompletionEnabled, terminalSessions]);

  const saveProjectState = async () => {
    if (!rootPath) return;
    try {
      const persistedOpenFiles = openFiles
        .map((f) => f.path)
        .filter((path) => !isShadowIdeVirtualPath(path) && !isIntegratedPlanengineDocPath(path));
      const persistedActiveIndex = persistedOpenFiles.length === 0
        ? 0
        : Math.max(
          0,
          Math.min(
            persistedOpenFiles.length - 1,
            openFiles
              .slice(0, activeFileIndex)
              .filter((file) => !isShadowIdeVirtualPath(file.path) && !isIntegratedPlanengineDocPath(file.path))
              .length,
          ),
        );
      await invoke("project_save_state", {
        projectState: {
          root_path: rootPath,
          open_files: persistedOpenFiles,
          active_file_index: persistedActiveIndex,
          sidebar_view: leftView ?? "explorer",
          sidebar_width: resize.leftSidebarWidth,
          terminal_visible: terminalVisible,
          terminal_height: resize.terminalHeight,
          ai_completion_enabled: aiCompletionEnabled,
          timestamp: Math.floor(Date.now() / 1000),
          terminal_sessions: terminalSessions,
        },
      });
    } catch { /* ignore */ }
  };

  // Remote state sync
  const syncTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    if (syncTimerRef.current) clearTimeout(syncTimerRef.current);
    syncTimerRef.current = setTimeout(() => {
      invoke("remote_update_state", {
        openFiles: openFiles.map((f) => f.path),
        activeFile: activeFile?.path && !isShadowIdeVirtualPath(activeFile.path) ? activeFile.path : null,
        cursorLine: 0, cursorColumn: 0,
        projectRoot: rootPath || null,
      }).catch(() => {});
    }, 500);
    return () => { if (syncTimerRef.current) clearTimeout(syncTimerRef.current); };
  }, [openFiles, activeFileIndex, rootPath]);

  // Emit workspace events for mobile sync
  const prevActivePathRef = useRef<string | null>(null);
  useEffect(() => {
    const currentPath = openFiles[activeFileIndex]?.path ?? null;
    if (currentPath && currentPath !== prevActivePathRef.current) {
      emit("workspace-file-opened", { path: currentPath, name: openFiles[activeFileIndex]?.name }).catch(() => {});
    }
    prevActivePathRef.current = currentPath;
  }, [activeFileIndex, openFiles]);

  const loadRecentProjects = async () => {
    try {
      const recent = await invoke<RecentProject[]>("project_list_recent");
      dispatch({ type: "SET_RECENT_PROJECTS", payload: recent });
    } catch { /* ignore */ }
  };

  const openProject = useCallback(
    async (path: string) => {
      await saveProjectState();
      try { await invoke("project_open", { path }); loadRecentProjects(); } catch { /* ignore */ }
      try {
        const projectState = await invoke<{
          root_path: string; open_files: string[]; active_file_index: number;
          sidebar_view: string; sidebar_width: number;
          terminal_visible: boolean; terminal_height: number;
          ai_completion_enabled: boolean;
          terminal_sessions?: SavedTerminalSession[];
        } | null>("project_load_state", { rootPath: path });
        dispatch({ type: "RESET_FILES_FOR_PROJECT", payload: { rootPath: path } });
        if (projectState) {
          resize.setLeftSidebarWidth(projectState.sidebar_width || 250);
          dispatch({ type: "SET_TERMINAL_VISIBLE", payload: projectState.terminal_visible });
          resize.setTerminalHeight(projectState.terminal_height || 250);
          dispatch({ type: "SET_AI_COMPLETION_ENABLED", payload: projectState.ai_completion_enabled });
          dispatch({ type: "SET_TERMINAL_SESSIONS", payload: projectState.terminal_sessions ?? [] });
          const restorablePaths = projectState.open_files.filter((filePath) =>
            !isShadowIdeVirtualPath(filePath) && !isIntegratedPlanengineDocPath(filePath),
          );
          let restoredCount = 0;
          for (const filePath of restorablePaths) {
            if (await openFileByPath(filePath)) {
              restoredCount += 1;
            }
          }
          dispatch({
            type: "SET_ACTIVE_FILE_INDEX",
            payload: restoredCount > 0 ? Math.max(0, Math.min(projectState.active_file_index, restoredCount - 1)) : 0,
          });
        }
      } catch {
        dispatch({ type: "RESET_FILES_FOR_PROJECT", payload: { rootPath: path } });
      }
      dispatch({ type: "SET_SHOW_RECENT_MENU", payload: false });
    },
    [openFiles, activeFileIndex, leftView, resize.leftSidebarWidth, terminalVisible, resize.terminalHeight, aiCompletionEnabled]
  );

  const handleFileOpen = useCallback(
    async (path: string, name: string) => {
      if (isMobileDevice) { dispatch({ type: "SET_LEFT_VIEW", payload: null }); dispatch({ type: "SET_RIGHT_VIEW", payload: null }); }
      const integratedDoc = getIntegratedPlanengineDocKey(path);
      if (integratedDoc) {
        focusIntegratedPlanengineDoc(integratedDoc);
        emit("workspace-file-opened", { path, name }).catch(() => {});
        return;
      }
      if (isShadowIdeVirtualPath(path)) {
        openVirtualFile(path, name);
        emit("workspace-file-opened", { path, name }).catch(() => {});
        return;
      }
      const existingIndex = openFiles.findIndex((f) => f.path === path);
      if (existingIndex !== -1) { dispatch({ type: "SET_ACTIVE_FILE_INDEX", payload: existingIndex }); return; }
      try {
        const info = await invoke<{ size: number; is_binary: boolean }>("get_file_info", { path });
        if (info.is_binary || info.size > 50 * 1024 * 1024) return;
        const content = await loadTextFile(path, info.size);
        dispatch({ type: "UPDATE_OPEN_FILES", payload: (prev) => [...prev, { path, name, content, modified: false, size: info.size }] });
        dispatch({ type: "SET_ACTIVE_FILE_INDEX", payload: openFiles.length });
        emit("workspace-file-opened", { path, name }).catch(() => {});
      } catch (err) { console.error("Failed to open file:", err); }
    },
    [dispatch, focusIntegratedPlanengineDoc, loadTextFile, openFiles, openVirtualFile]
  );

  const openFullGameViewport = useCallback(() => {
    openVirtualFile(GAMEDEV_LIVE_VIEW_PATH, GAMEDEV_LIVE_VIEW_NAME);
  }, [openVirtualFile]);

  const focusGameDevTab = useCallback((tab: GameDevTab, options?: { openLiveView?: boolean }) => {
    everVisited.current.add("gamedev");
    const zone = panelZones.gamedev;
    if (zone === "left") {
      dispatch({ type: "SET_LEFT_VIEW", payload: "gamedev" });
    } else {
      dispatch({ type: "SET_RIGHT_VIEW", payload: "gamedev" });
    }
    if (options?.openLiveView) {
      openFullGameViewport();
    }
    emit("shadow-gamedev-focus-tab", { tab }).catch(() => {});
    if (sidebarAutoHide) {
      resetSidebarTimer();
    }
  }, [dispatch, openFullGameViewport, panelZones.gamedev, resetSidebarTimer, sidebarAutoHide]);

  useEffect(() => {
    const unlistenPromise = listen<string>("remote-open-file", (e) => {
      const filePath = e.payload;
      handleFileOpen(filePath, filePath.split("/").pop() || filePath);
    });
    return () => { unlistenPromise.then(fn => fn()); };
  }, [handleFileOpen]);

  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent).detail;
      if (detail?.project_root) dispatch({ type: "SET_ROOT_PATH", payload: detail.project_root });
    };
    window.addEventListener("mobile-workspace-state", handler);
    return () => window.removeEventListener("mobile-workspace-state", handler);
  }, []);

  // Live editor update when AI tools modify files
  const fileContentSeqRef = useRef(0);
  useEffect(() => {
    const unlistenPromise = listen<{ path: string; tool: string }>("file-changed-by-tool", async (e) => {
      const { path: filePath, tool } = e.payload;
      const seq = ++fileContentSeqRef.current;
      dispatch({ type: "UPDATE_OPEN_FILES", payload: (prev) => {
        const idx = prev.findIndex((f) => f.path === filePath);
        if (idx === -1) return prev;
        invoke<string>("read_file_content", { path: filePath }).then((content) => {
          if (fileContentSeqRef.current !== seq) return; // stale response -- file was closed/replaced
          dispatch({ type: "UPDATE_OPEN_FILES", payload: (p) => {
            // Verify the file is still open before updating
            const stillOpen = p.findIndex((f) => f.path === filePath);
            if (stillOpen === -1) return p;
            return p.map((f) => f.path === filePath ? { ...f, content, modified: false } : f);
          }});
        });
        return prev.map((f, i) => i === idx ? { ...f, name: f.name.replace(/ \[.*\]$/, "") + ` [${tool}]` } : f);
      }});
      setTimeout(() => {
        dispatch({ type: "UPDATE_OPEN_FILES", payload: (p) => p.map((f) => f.path === filePath ? { ...f, name: f.name.replace(/ \[.*\]$/, "") } : f) });
      }, 2000);
    });
    return () => { unlistenPromise.then(fn => fn()); };
  }, []);

  const handleFileClose = useCallback(
    async (index: number) => {
      const file = openFiles[index];
      if (file?.modified) {
        const confirmed = await ask(`"${file.name}" has unsaved changes. Close anyway?`, { title: "Unsaved Changes", kind: "warning" });
        if (!confirmed) return;
      }
      const closedPath = file?.path;
      dispatch({ type: "UPDATE_OPEN_FILES", payload: (prev) => prev.filter((_, i) => i !== index) });
      if (activeFileIndex >= index && activeFileIndex > 0) dispatch({ type: "SET_ACTIVE_FILE_INDEX", payload: activeFileIndex - 1 });
      if (closedPath) emit("workspace-file-closed", { path: closedPath }).catch(() => {});
    },
    [activeFileIndex, openFiles]
  );

  const dismissIntegratedPlanDoc = useCallback(() => {
    void handleFileClose(activeFileIndex);
  }, [activeFileIndex, handleFileClose]);

  const handleFileContentChange = useCallback((index: number, content: string) => {
    dispatch({ type: "UPDATE_OPEN_FILES", payload: (prev) => prev.map((file, i) => i === index ? { ...file, content, modified: content !== file.content } : file) });
  }, []);

  const handleFileReorder = useCallback(
    (fromIndex: number, toIndex: number) => {
      dispatch({ type: "UPDATE_OPEN_FILES", payload: (prev) => { const u = [...prev]; const [m] = u.splice(fromIndex, 1); u.splice(toIndex, 0, m); return u; } });
      if (activeFileIndex === fromIndex) dispatch({ type: "SET_ACTIVE_FILE_INDEX", payload: toIndex });
      else if (fromIndex < activeFileIndex && toIndex >= activeFileIndex) dispatch({ type: "SET_ACTIVE_FILE_INDEX", payload: activeFileIndex - 1 });
      else if (fromIndex > activeFileIndex && toIndex <= activeFileIndex) dispatch({ type: "SET_ACTIVE_FILE_INDEX", payload: activeFileIndex + 1 });
    },
    [activeFileIndex]
  );

  const handleExplainError = useCallback(
    async (errorText: string) => {
      const zone = panelZones.ai;
      if (zone === "left") dispatch({ type: "SET_LEFT_VIEW", payload: "ai" });
      else dispatch({ type: "SET_RIGHT_VIEW", payload: "ai" });
      try {
        await invoke("ai_explain_error", { errorText, context: activeFile ? `File: ${activeFile.name}` : null, model: null });
      } catch (err) { console.error("Failed to explain error:", err); }
    },
    [activeFile, panelZones]
  );

  const handleActivityClick = useCallback((view: SidebarView) => {
    everVisited.current.add(view);
    const zone = panelZones[view];
    if (zone === "left") dispatch({ type: "UPDATE_LEFT_VIEW", payload: (prev) => prev === view ? null : view });
    else dispatch({ type: "UPDATE_RIGHT_VIEW", payload: (prev) => prev === view ? null : view });
    if (sidebarAutoHide) resetSidebarTimer();
  }, [panelZones, sidebarAutoHide, resetSidebarTimer]);

  const movePanelTo = useCallback((view: SidebarView, zone: PanelZone) => {
    dispatch({ type: "MOVE_PANEL_TO", payload: { view, zone } });
  }, []);

  // Track ever-visited panels for lazy mounting
  useEffect(() => {
    if (leftView) everVisited.current.add(leftView);
  }, [leftView]);

  useEffect(() => {
    if (rightView) everVisited.current.add(rightView);
  }, [rightView]);

  useEffect(() => {
    if (!isIntegratedPlanDocActive) {
      return;
    }
    const docKey = getIntegratedPlanengineDocKey(activeFile?.path);
    if (docKey) {
      setPreferredPlanengineDoc(docKey);
    }
    everVisited.current.add("planengine");
    const zone = panelZones.planengine;
    if (zone === "left") {
      dispatch({ type: "SET_LEFT_VIEW", payload: "planengine" });
    } else {
      dispatch({ type: "SET_RIGHT_VIEW", payload: "planengine" });
    }
  }, [activeFile?.path, dispatch, isIntegratedPlanDocActive, panelZones.planengine]);

  // Keyboard shortcuts
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.key === "`") { e.preventDefault(); dispatch({ type: "TOGGLE_TERMINAL_VISIBLE" }); }
      if (e.ctrlKey && e.shiftKey && e.key === "A") { e.preventDefault(); handleActivityClick("ai"); }
      if (e.ctrlKey && e.shiftKey && e.key === "R") { e.preventDefault(); handleActivityClick("remote"); }
      if (e.ctrlKey && e.shiftKey && e.key === "T") { e.preventDefault(); handleActivityClick("todos"); }
      if (e.ctrlKey && e.shiftKey && e.key === "F") { e.preventDefault(); handleActivityClick("search"); }
      if ((e.ctrlKey || e.metaKey) && e.key === "p" && !e.shiftKey) { e.preventDefault(); setQuickOpenOpen(true); }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [handleActivityClick]);

  // Close menus
  useEffect(() => {
    if (!showRecentMenu) return;
    const handleClick = () => dispatch({ type: "SET_SHOW_RECENT_MENU", payload: false });
    window.addEventListener("click", handleClick);
    return () => window.removeEventListener("click", handleClick);
  }, [showRecentMenu]);

  useEffect(() => {
    if (!contextMenu) return;
    const close = () => dispatch({ type: "SET_CONTEXT_MENU", payload: null });
    const handleKey = (e: KeyboardEvent) => { if (e.key === "Escape") close(); };
    window.addEventListener("click", close);
    window.addEventListener("scroll", close, true);
    window.addEventListener("keydown", handleKey);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("scroll", close, true);
      window.removeEventListener("keydown", handleKey);
    };
  }, [contextMenu]);

  const handlePanelContextMenu = useCallback((e: React.MouseEvent, panel: SidebarView) => {
    e.preventDefault();
    dispatch({ type: "SET_CONTEXT_MENU", payload: { x: e.clientX, y: e.clientY, panel } });
  }, []);

  const handleSidebarWheel = useCallback((e: React.WheelEvent<HTMLDivElement>) => {
    const target = e.target as HTMLElement | null;
    if (target?.closest("textarea, [contenteditable='true']")) return;

    const scrollable = findScrollableDescendant(target, e.currentTarget);
    if (!scrollable) return;

    const maxScrollTop = scrollable.scrollHeight - scrollable.clientHeight;
    if (maxScrollTop <= 0) return;

    const nextScrollTop = Math.max(0, Math.min(maxScrollTop, scrollable.scrollTop + e.deltaY));
    if (nextScrollTop === scrollable.scrollTop) return;

    scrollable.scrollTop = nextScrollTop;
    e.preventDefault();
  }, []);

  // Render a panel by view name (wrapped in ErrorBoundary) — lazy mount
  const renderPanel = (view: SidebarView, isActive: boolean) => {
    if (!everVisited.current.has(view)) return null;
    const panel = renderPanelInner(view, isActive);
    return <ErrorBoundary name={view} key={view}>{panel}</ErrorBoundary>;
  };

  const renderPanelInner = (view: SidebarView, isActive: boolean) => {
    switch (view) {
      case "explorer":
        return <MemoizedFileExplorer onFileOpen={handleFileOpen} rootPath={rootPath} onRootPathChange={(path) => openProject(path)} />;
      case "ai":
        return (
          <MemoizedFerrumChat
            visible={isActive}
            rootPath={rootPath}
            activeFileContent={activeFile?.content}
            activeFileName={activeFile?.name}
            isFullscreen={aiFullscreen}
            onToggleFullscreen={() => dispatch({ type: "TOGGLE_AI_FULLSCREEN" })}
            onPopout={() => {
              const w = window.open(window.location.href + "?popout=ai", "shadowai-popout", "width=520,height=720");
              if (w) w.focus();
            }}
          />
        );
      case "collab":
        return <MemoizedCollaborationPanel visible={isActive} />;
      case "todos":
        return <MemoizedTodoPanel visible={isActive} rootPath={rootPath} onFileOpen={handleFileOpen} />;
      case "search":
        return <MemoizedSearchPanel visible={isActive} rootPath={rootPath} onFileOpen={handleFileOpen} />;
      case "remote":
        return <MemoizedRemoteSettings visible={isActive} />;
      case "llmloader":
        return <MemoizedLlmLoader visible={isActive} rootPath={rootPath} />;
      case "languages":
        return <MemoizedLanguagesPanel visible={isActive} />;
      case "logs":
        return <MemoizedLogPanel visible={isActive} />;
      case "rag":
        return <MemoizedRagPanel visible={isActive} rootPath={rootPath} />;
      case "bluetooth":
        return <MemoizedBluetoothPanel visible={isActive} />;
      case "settings":
        return (
          <MemoizedSettingsPanel
            visible={isActive}
            oledMode={oledMode}
            onOledChange={(v) => dispatch({ type: "SET_OLED_MODE", payload: v })}
            panelZones={panelZones}
            onPanelZoneChange={(view, zone) => movePanelTo(view, zone)}
            sidebarAutoHide={sidebarAutoHide}
            onSidebarAutoHideChange={(v) => dispatch({ type: "SET_SIDEBAR_AUTO_HIDE", payload: v })}
            showStatusBar={showStatusBar}
            onShowStatusBarChange={(v) => dispatch({ type: "SET_SHOW_STATUS_BAR", payload: v })}
            aiCompletionEnabled={aiCompletionEnabled}
            onAiCompletionChange={(v) => dispatch({ type: "SET_AI_COMPLETION_ENABLED", payload: v })}
            fontSize={workspaceSettings.font_size ?? 14}
            onFontSizeChange={(v) => dispatch({ type: "UPDATE_WORKSPACE_SETTINGS", payload: (s) => ({ ...s, font_size: v }) })}
            tabSize={workspaceSettings.tab_size ?? 4}
            onTabSizeChange={(v) => dispatch({ type: "UPDATE_WORKSPACE_SETTINGS", payload: (s) => ({ ...s, tab_size: v }) })}
            minimapEnabled={workspaceSettings.minimap_enabled ?? true}
            onMinimapChange={(v) => { dispatch({ type: "UPDATE_WORKSPACE_SETTINGS", payload: (s) => ({ ...s, minimap_enabled: v }) }); updateWorkspaceSetting("minimap_enabled", v); }}
            useTabs={workspaceSettings.use_tabs ?? false}
            onUseTabsChange={(v) => dispatch({ type: "UPDATE_WORKSPACE_SETTINGS", payload: (s) => ({ ...s, use_tabs: v }) })}
            systemPrompt={systemPrompt}
            onSystemPromptChange={(v) => dispatch({ type: "SET_SYSTEM_PROMPT", payload: v })}
          />
        );
      case "gitgraph":
        return <MemoizedGitGraphPanel rootPath={rootPath ?? ""} />;
      case "testexplorer":
        return <MemoizedTestExplorerPanel rootPath={rootPath ?? ""} visible={isActive} />;
      case "agent":
        return <MemoizedAgentPanel visible={isActive} />;
      case "database":
        return <MemoizedDatabasePanel />;
      case "debug":
        return <MemoizedDebugPanel projectPath={rootPath ?? ""} />;
      case "deps":
        return <MemoizedDependencyGraph projectPath={rootPath ?? ""} />;
      case "edithistory":
        return <MemoizedEditHistoryPanel projectPath={rootPath ?? ""} />;
      case "keybindings":
        return <MemoizedKeybindingsPanel />;
      case "docs":
        return <MemoizedDocsPanel language={currentLanguage} fileUri={activeFile ? `file://${activeFile.path}` : ""} />;
      case "profiler":
        return <MemoizedProfilerPanel projectPath={rootPath ?? ""} />;
      case "glslpreview":
        return <MemoizedGlslPreviewPanel />;
      case "pr":
        return <MemoizedPrPanel repoPath={rootPath ?? ""} />;
      case "plugins":
        return <MemoizedPluginsPanel />;
      case "mutation":
        return <MemoizedMutationPanel rootPath={rootPath ?? ""} visible={isActive} />;
      case "cicd":
        return <MemoizedCiCdPanel rootPath={rootPath ?? ""} />;
      case "gamedev":
        return (
          <MemoizedGameDevPanel
            projectPath={rootPath ?? undefined}
            visible={isActive}
            onOpenFile={(path, name) => { void handleFileOpen(path, name); }}
            onActivatePanel={handleActivityClick}
            viewportDockVisible={gameViewportVisible}
            onViewportDockToggle={() => setGameViewportVisible((value) => !value)}
            onProjectCreated={(path) => { void openProject(path); }}
          />
        );
      case "planengine":
        return (
          <MemoizedPlanenginePanel
            visible={isActive}
            onOpenFile={(path, name) => { void handleFileOpen(path, name); }}
            onActivatePanel={handleActivityClick}
            preferredDoc={preferredPlanengineDoc}
            onFocusGameTab={focusGameDevTab}
            onOpenLiveView={() => focusGameDevTab("overview", { openLiveView: true })}
            projectPath={hasShadowProject ? (rootPath ?? undefined) : undefined}
          />
        );
    }
  };

  const appWindow = (() => {
    try { return getCurrentWindow(); } catch { return null; }
  })();

  // Escape to exit AI fullscreen
  useEffect(() => {
    if (!aiFullscreen) return;
    const handleKey = (e: KeyboardEvent) => { if (e.key === "Escape") dispatch({ type: "SET_AI_FULLSCREEN", payload: false }); };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [aiFullscreen]);

  // Popout mode detection
  const isPopout = new URLSearchParams(window.location.search).get("popout") === "ai";
  if (isPopout) {
    return (
      <div className="app-popout" style={{ height: "100%", background: "var(--bg-secondary)" }}>
        <FerrumChat visible={true} rootPath={rootPath} />
      </div>
    );
  }

  // Build grid columns dynamically
  const gridColumns = isMobileDevice
    ? `${38}px 0px 0px 1fr 0px 0px`
    : [
        `${44}px`,
        hasLeftSidebar ? `${sidebarHidden ? 0 : resize.leftSidebarWidth}px` : "0px",
        hasLeftSidebar && !sidebarHidden ? "4px" : "0px",
        "1fr",
        hasRightSidebar && !sidebarHidden ? "4px" : "0px",
        hasRightSidebar ? `${sidebarHidden ? 0 : resize.rightSidebarWidth}px` : "0px",
      ].join(" ");

  const mainEditorContent = isLiveViewportTab ? (
    <ErrorBoundary name="ShadowGameWorkspace">
      <MemoizedShadowGameWorkspace
        projectPath={rootPath ?? undefined}
        visible={true}
        onOpenFile={(path, name) => { void handleFileOpen(path, name); }}
        onActivatePanel={handleActivityClick}
        autoBuildManagedExternally={leftView === "gamedev" || rightView === "gamedev"}
      />
    </ErrorBoundary>
  ) : isIntegratedPlanDocActive ? (
    <ErrorBoundary name="IntegratedPlanengineDoc">
      <div className="editor-empty integrated-doc-empty">
        <div className="editor-empty-content integrated-doc-empty-content">
          <div className="integrated-doc-kicker">Integrated Sidebar Document</div>
          <h2>PlanEngine Lives In The Sidebar</h2>
          <p>
            <code>{activeFile?.name ?? "planengine.md"}</code> is already integrated into ShadowIDE, so the main
            workspace no longer needs to stay on the full roadmap page.
          </p>
          <p>
            Keep the roadmap in the sidebar and use the center workspace for code, assets, or the live viewport.
          </p>
          <div className="integrated-doc-actions">
            <button
              className="integrated-doc-btn integrated-doc-btn-primary"
              onClick={() => focusIntegratedPlanengineDoc(activeIntegratedPlanDocKey ?? "plan")}
            >
              Open Sidebar
            </button>
            <button className="integrated-doc-btn" onClick={() => focusGameDevTab("overview")}>
              Open Game Panel
            </button>
            <button className="integrated-doc-btn" onClick={dismissIntegratedPlanDoc}>
              Hide This Tab
            </button>
          </div>
        </div>
      </div>
    </ErrorBoundary>
  ) : showPlanengineLaunchpad ? (
    <ErrorBoundary name="PlanengineLaunchpad">
      <div className="editor-empty planengine-launchpad">
        <div className="editor-empty-content planengine-launchpad-content">
          <div className="planengine-launchpad-kicker">Start With PlanEngine</div>
          <h2>ShadowIDE Is Wired Around The Roadmap</h2>
          <p>
            Use the roadmap as the entry point for building, reflecting, live viewport work, and AI-assisted authoring.
          </p>
          <p>
            The core PlanEngine sections now map directly to in-IDE workflows instead of living as detached markdown.
          </p>
          <div className="planengine-launchpad-grid">
            <div className="planengine-launchpad-card">
              <div className="planengine-launchpad-card-title">Roadmap And Audit</div>
              <div className="planengine-launchpad-card-copy">
                Open the integrated roadmap or finish audit from the sidebar and keep the center workspace free for actual authoring.
              </div>
              <div className="planengine-launchpad-actions">
                <button className="planengine-launchpad-btn planengine-launchpad-btn-primary" onClick={() => focusIntegratedPlanengineDoc("plan")}>
                  Open Roadmap
                </button>
                <button className="planengine-launchpad-btn" onClick={() => focusIntegratedPlanengineDoc("finish")}>
                  Open Audit
                </button>
              </div>
            </div>
            <div className="planengine-launchpad-card">
              <div className="planengine-launchpad-card-title">Runtime And Reflection</div>
              <div className="planengine-launchpad-card-copy">
                Jump straight into build and reflection workflows that line up with the C++ runtime and ABI parts of the plan.
              </div>
              <div className="planengine-launchpad-actions">
                <button className="planengine-launchpad-btn planengine-launchpad-btn-primary" onClick={() => focusGameDevTab("build")}>
                  Build Runtime
                </button>
                <button className="planengine-launchpad-btn" onClick={() => focusGameDevTab("reflect")}>
                  Reflect
                </button>
                <button className="planengine-launchpad-btn" onClick={() => focusGameDevTab("scene")}>
                  Scene
                </button>
              </div>
            </div>
            <div className="planengine-launchpad-card">
              <div className="planengine-launchpad-card-title">Viewport Workflow</div>
              <div className="planengine-launchpad-card-copy">
                Launch the live viewport, keep the docked view active, and iterate on terrain, lighting, and runtime behavior from inside ShadowIDE.
              </div>
              <div className="planengine-launchpad-actions">
                <button className="planengine-launchpad-btn planengine-launchpad-btn-primary" onClick={() => focusGameDevTab("overview", { openLiveView: true })}>
                  Open Live View
                </button>
                <button className="planengine-launchpad-btn" onClick={() => focusGameDevTab("assets")}>
                  Assets
                </button>
              </div>
            </div>
            <div className="planengine-launchpad-card">
              <div className="planengine-launchpad-card-title">AI And Tooling</div>
              <div className="planengine-launchpad-card-copy">
                Route directly into the roadmap’s AI surfaces for local models, context, chat, and code-driven iteration.
              </div>
              <div className="planengine-launchpad-actions">
                <button className="planengine-launchpad-btn planengine-launchpad-btn-primary" onClick={() => focusGameDevTab("ai")}>
                  AI Workflow
                </button>
                <button className="planengine-launchpad-btn" onClick={() => handleActivityClick("ai")}>
                  AI Chat
                </button>
                <button className="planengine-launchpad-btn" onClick={() => handleActivityClick("llmloader")}>
                  LLM Loader
                </button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </ErrorBoundary>
  ) : activeFile && (activeFile.name.endsWith(".md") || activeFile.name.endsWith(".mdx")) ? (
    <ErrorBoundary name="MarkdownPreview">
      <MarkdownPreview content={activeFile.content} filePath={activeFile.path} />
    </ErrorBoundary>
  ) : activeFile && activeFile.name.endsWith(".ipynb") ? (
    <ErrorBoundary name="JupyterPanel">
      <JupyterPanel filePath={activeFile.path} />
    </ErrorBoundary>
  ) : activeFile && activeFile.name.endsWith(".org") ? (
    <ErrorBoundary name="OrgModePanel">
      <OrgModePanel filePath={activeFile.path} />
    </ErrorBoundary>
  ) : activeFile && (activeFile.name.endsWith(".obj") || activeFile.name.endsWith(".gltf")) ? (
    <ErrorBoundary name="MeshViewer">
      <MeshViewer filePath={activeFile.path} />
    </ErrorBoundary>
  ) : (
    <ErrorBoundary name="Editor">
      <Suspense fallback={<div>Loading editor...</div>}>
        <Editor
          files={openFiles}
          activeFileIndex={activeFileIndex}
          onActiveFileChange={(i) => dispatch({ type: "SET_ACTIVE_FILE_INDEX", payload: i })}
          onFileClose={handleFileClose}
          onFileContentChange={handleFileContentChange}
          onFileReorder={handleFileReorder}
          aiCompletionEnabled={aiCompletionEnabled}
          rootPath={rootPath}
          minimapEnabled={workspaceSettings.minimap_enabled}
          fontSize={workspaceSettings.font_size}
          tabSize={workspaceSettings.tab_size}
          onMinimapToggle={(v) => updateWorkspaceSetting("minimap_enabled", v)}
          onCursorChange={(info) => dispatch({ type: "SET_CURSOR_INFO", payload: info })}
          onDiagnosticsChange={(counts) => dispatch({ type: "SET_DIAGNOSTIC_COUNTS", payload: counts })}
          onDiagnosticItems={(items) => dispatch({ type: "SET_DIAGNOSTIC_ITEMS", payload: items })}
        />
      </Suspense>
    </ErrorBoundary>
  );

  return (
    <>
    <QuickOpenPanel
      isOpen={quickOpenOpen}
      onClose={() => setQuickOpenOpen(false)}
      onFileSelect={(path) => {
        const name = path.split("/").pop() || path;
        handleFileOpen(path, name);
      }}
      projectPath={rootPath ?? ""}
    />
    <CommandPalette onCommandExecute={(id) => {
      // Route command palette actions to existing handlers
      if (id === "view.toggleTerminal") dispatch({ type: "TOGGLE_TERMINAL_VISIBLE" });
      else if (id === "view.explorerPanel") handleActivityClick("explorer");
      else if (id === "view.searchPanel") handleActivityClick("search");
      else if (id === "view.aiPanel") handleActivityClick("ai");
      else if (id === "view.gamedevPanel") focusGameDevTab("overview");
      else if (id === "view.planenginePanel") focusIntegratedPlanengineDoc("plan");
      else if (id === "view.planengineAudit") focusIntegratedPlanengineDoc("finish");
      else if (id === "view.planengineBuildWorkflow") focusGameDevTab("build");
      else if (id === "view.planengineReflectWorkflow") focusGameDevTab("reflect");
      else if (id === "view.planengineAiWorkflow") focusGameDevTab("ai");
      else if (id === "view.planengineViewportWorkflow") focusGameDevTab("overview", { openLiveView: true });
      else if (id === "view.gitGraph") handleActivityClick("gitgraph");
      else if (id === "view.testExplorer") handleActivityClick("testexplorer");
      else if (id === "view.agentPanel") handleActivityClick("agent");
      else if (id === "settings.open") handleActivityClick("settings");
    }} />
    <div className="app dual-sidebar" data-testid="app-root" style={{ gridTemplateColumns: gridColumns }}>
      <TitleBar
        rootPath={rootPath}
        activeFile={activeFile}
        isMobileDevice={isMobileDevice}
        appWindow={appWindow}
        planSummary={planengineShellSummary ?? undefined}
        onOpenPlanengine={() => focusIntegratedPlanengineDoc("plan")}
        onOpenGamePanel={() => focusGameDevTab("overview")}
        onOpenLiveView={() => focusGameDevTab("overview", { openLiveView: true })}
      />

      <ActivityBar
        leftView={leftView}
        rightView={rightView}
        panelZones={panelZones}
        sidebarAutoHide={sidebarAutoHide}
        sidebarHidden={sidebarHidden}
        sidebarTimerRef={sidebarTimerRef}
        onActivityClick={handleActivityClick}
        onPanelContextMenu={handlePanelContextMenu}
        onSidebarHiddenChange={(v) => dispatch({ type: "SET_SIDEBAR_HIDDEN", payload: v })}
        showRecentMenu={showRecentMenu}
        recentProjects={recentProjects}
        onRecentMenuToggle={() => dispatch({ type: "TOGGLE_SHOW_RECENT_MENU" })}
        onRecentProjectClick={openProject}
        aiCompletionEnabled={aiCompletionEnabled}
        onAiCompletionToggle={() => dispatch({ type: "TOGGLE_AI_COMPLETION_ENABLED" })}
        terminalVisible={terminalVisible}
        onTerminalToggle={() => dispatch({ type: "TOGGLE_TERMINAL_VISIBLE" })}
      />

      {/* Left Sidebar */}
      <div
        className={`sidebar sidebar-left${sidebarHidden && hasLeftSidebar ? " auto-hidden" : ""}${!hasLeftSidebar ? " collapsed" : ""}`}
        onMouseEnter={() => { if (sidebarAutoHide) { dispatch({ type: "SET_SIDEBAR_HIDDEN", payload: false }); if (sidebarTimerRef.current) clearTimeout(sidebarTimerRef.current); } }}
        onMouseLeave={() => { if (sidebarAutoHide) resetSidebarTimer(); }}
        onContextMenu={(e) => { if (leftView) handlePanelContextMenu(e, leftView); }}
      >
        {(Object.entries(panelZones) as [SidebarView, PanelZone][])
          .filter(([, zone]) => zone === "left")
          .map(([view]) => (
            <div
              key={view}
              className="sidebar-panel"
              style={{ display: leftView === view ? undefined : "none" }}
              onWheelCapture={handleSidebarWheel}
            >
              {renderPanel(view, leftView === view || rightView === view)}
            </div>
          ))}
      </div>

      {!isMobileDevice && hasLeftSidebar && !sidebarHidden && (
        <div className="resize-handle-vertical resize-left" onMouseDown={resize.startResizingLeft} />
      )}
      {!hasLeftSidebar && !isMobileDevice && <div />}

      {isMobileDevice && (hasLeftSidebar || hasRightSidebar) && (
        <div className="mobile-sidebar-backdrop" onClick={() => { dispatch({ type: "SET_LEFT_VIEW", payload: null }); dispatch({ type: "SET_RIGHT_VIEW", payload: null }); }} />
      )}

      {/* Main Content Area */}
      <div className="main-content">
        <div ref={editorAreaRef} className="editor-area" style={{ height: terminalVisible ? `calc(100% - ${resize.terminalHeight}px - 4px)` : "100%" }}>
          {showDockedGameViewport ? (
            <div className="workspace-shell">
              <div className="workspace-primary-pane">
                <div className="workspace-primary-pane-body">
                  {mainEditorContent}
                </div>
              </div>
              <div className="workspace-preview-resizer" onMouseDown={() => setResizingGameViewport(true)} />
              <div className="workspace-preview-pane" style={{ width: `${gameViewportWidth}px` }}>
                <ErrorBoundary name="DockedGameViewport">
                  <MemoizedShadowGameWorkspace
                    projectPath={rootPath ?? undefined}
                    visible={true}
                    onOpenFile={(path, name) => { void handleFileOpen(path, name); }}
                    onActivatePanel={handleActivityClick}
                    autoBuildManagedExternally={leftView === "gamedev" || rightView === "gamedev"}
                    layoutMode="dock"
                    onCloseDock={() => setGameViewportVisible(false)}
                    onRequestFullView={openFullGameViewport}
                  />
                </ErrorBoundary>
              </div>
            </div>
          ) : (
            <>
              {showGameViewportLauncher && (
                <div className="game-viewport-launcher">
                  <button
                    className="game-viewport-launcher-btn game-viewport-launcher-btn-primary"
                    onClick={() => setGameViewportVisible(true)}
                  >
                    Open Viewport
                  </button>
                  <button
                    className="game-viewport-launcher-btn"
                    onClick={openFullGameViewport}
                  >
                    Full View
                  </button>
                </div>
              )}
              <div className="workspace-primary-pane workspace-primary-pane-full">
                <div className="workspace-primary-pane-body">
                  {mainEditorContent}
                </div>
              </div>
            </>
          )}
        </div>
        {terminalVisible && <div className="resize-handle-horizontal" onMouseDown={resize.startResizingTerminal} />}
        <div className="terminal-area" style={{ height: terminalVisible ? `${resize.terminalHeight}px` : "0" }}>
          <ErrorBoundary name="Terminal">
            <MemoizedTerminalPanel visible={terminalVisible} cwd={rootPath} onExplainError={handleExplainError} savedSessions={terminalSessions} onSessionsChange={(sessions) => dispatch({ type: "SET_TERMINAL_SESSIONS", payload: sessions })} />
          </ErrorBoundary>
        </div>
      </div>

      {!isMobileDevice && hasRightSidebar && !sidebarHidden && (
        <div className="resize-handle-vertical resize-right" onMouseDown={resize.startResizingRight} />
      )}
      {!hasRightSidebar && !isMobileDevice && <div />}

      {/* Right Sidebar */}
      <div
        className={`sidebar sidebar-right${sidebarHidden && hasRightSidebar ? " auto-hidden" : ""}${!hasRightSidebar ? " collapsed" : ""}`}
        onMouseEnter={() => { if (sidebarAutoHide) { dispatch({ type: "SET_SIDEBAR_HIDDEN", payload: false }); if (sidebarTimerRef.current) clearTimeout(sidebarTimerRef.current); } }}
        onMouseLeave={() => { if (sidebarAutoHide) resetSidebarTimer(); }}
        onContextMenu={(e) => { if (rightView) handlePanelContextMenu(e, rightView); }}
      >
        {(Object.entries(panelZones) as [SidebarView, PanelZone][])
          .filter(([, zone]) => zone === "right")
          .map(([view]) => (
            <div
              key={view}
              className="sidebar-panel"
              style={{ display: rightView === view ? undefined : "none" }}
              onWheelCapture={handleSidebarWheel}
            >
              {renderPanel(view, leftView === view || rightView === view)}
            </div>
          ))}
      </div>

      {showStatusBar && (
        <StatusBar
          diagnosticCounts={diagnosticCounts}
          cursorInfo={cursorInfo}
          currentLanguage={currentLanguage}
          activeFile={!!activeFile}
          aiCompletionEnabled={aiCompletionEnabled}
          planSummary={planengineShellSummary ?? undefined}
          onOpenPlanengine={() => focusIntegratedPlanengineDoc("plan")}
          onToggleErrorPanel={() => dispatch({ type: "TOGGLE_SHOW_ERROR_PANEL" })}
          onHide={() => dispatch({ type: "SET_SHOW_STATUS_BAR", payload: false })}
        />
      )}

      {showErrorPanel && (
        <DiagnosticPanel
          diagnosticCounts={diagnosticCounts}
          diagnosticItems={diagnosticItems}
          onClose={() => dispatch({ type: "SET_SHOW_ERROR_PANEL", payload: false })}
          onFileOpen={handleFileOpen}
          projectRoot={rootPath}
        />
      )}

      {aiFullscreen && (
        <div className="ai-fullscreen-overlay">
          <FerrumChat
            visible={true}
            rootPath={rootPath}
            activeFileContent={activeFile?.content}
            activeFileName={activeFile?.name}
            isFullscreen={true}
            onToggleFullscreen={() => dispatch({ type: "SET_AI_FULLSCREEN", payload: false })}
          />
        </div>
      )}

      {contextMenu && (
        <div
          className="panel-context-menu"
          style={{ left: contextMenu.x, top: contextMenu.y }}
          onClick={(e) => e.stopPropagation()}
        >
          <div className="ctx-menu-title">Move &ldquo;{contextMenu.panel}&rdquo;</div>
          <button
            className={`ctx-menu-item${panelZones[contextMenu.panel] === "left" ? " active" : ""}`}
            onClick={() => { movePanelTo(contextMenu.panel, "left"); dispatch({ type: "SET_CONTEXT_MENU", payload: null }); }}
          >
            Left Sidebar
            {panelZones[contextMenu.panel] === "left" && <span className="ctx-check">&#10003;</span>}
          </button>
          <button
            className={`ctx-menu-item${panelZones[contextMenu.panel] === "right" ? " active" : ""}`}
            onClick={() => { movePanelTo(contextMenu.panel, "right"); dispatch({ type: "SET_CONTEXT_MENU", payload: null }); }}
          >
            Right Sidebar
            {panelZones[contextMenu.panel] === "right" && <span className="ctx-check">&#10003;</span>}
          </button>
        </div>
      )}
    </div>
    </>
  );
}

export default App;
