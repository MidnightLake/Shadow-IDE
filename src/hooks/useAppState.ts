import { useReducer, type Dispatch } from "react";
import type { OpenFile, CursorInfo, DiagnosticCounts, DiagnosticItem } from "../components/Editor";
import type { SavedTerminalSession } from "../components/Terminal";
import type { SidebarView, PanelZone, RecentProject, WorkspaceSettings } from "../types";
import { DEFAULT_ZONES } from "../types";

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

export interface AppState {
  // Project / files
  rootPath: string;
  openFiles: OpenFile[];
  activeFileIndex: number;
  recentProjects: RecentProject[];
  workspaceSettings: WorkspaceSettings;

  // Layout – sidebars
  panelZones: Record<SidebarView, PanelZone>;
  leftView: SidebarView | null;
  rightView: SidebarView | null;
  sidebarAutoHide: boolean;
  sidebarHidden: boolean;

  // Layout – terminal
  terminalVisible: boolean;
  terminalSessions: SavedTerminalSession[];

  // Layout – panels & menus
  showRecentMenu: boolean;
  contextMenu: { x: number; y: number; panel: SidebarView } | null;
  showStatusBar: boolean;
  showErrorPanel: boolean;
  aiFullscreen: boolean;

  // Editor info
  cursorInfo: CursorInfo;
  diagnosticCounts: DiagnosticCounts;
  diagnosticItems: DiagnosticItem[];

  // Preferences
  oledMode: boolean;
  aiCompletionEnabled: boolean;
  systemPrompt: string;
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

export type AppAction =
  // Project / files
  | { type: "SET_ROOT_PATH"; payload: string }
  | { type: "SET_OPEN_FILES"; payload: OpenFile[] }
  | { type: "UPDATE_OPEN_FILES"; payload: (prev: OpenFile[]) => OpenFile[] }
  | { type: "SET_ACTIVE_FILE_INDEX"; payload: number }
  | { type: "SET_RECENT_PROJECTS"; payload: RecentProject[] }
  | { type: "SET_WORKSPACE_SETTINGS"; payload: WorkspaceSettings }
  | { type: "UPDATE_WORKSPACE_SETTINGS"; payload: (prev: WorkspaceSettings) => WorkspaceSettings }

  // Layout – sidebars
  | { type: "SET_PANEL_ZONES"; payload: Record<SidebarView, PanelZone> }
  | { type: "UPDATE_PANEL_ZONES"; payload: (prev: Record<SidebarView, PanelZone>) => Record<SidebarView, PanelZone> }
  | { type: "SET_LEFT_VIEW"; payload: SidebarView | null }
  | { type: "UPDATE_LEFT_VIEW"; payload: (prev: SidebarView | null) => SidebarView | null }
  | { type: "SET_RIGHT_VIEW"; payload: SidebarView | null }
  | { type: "UPDATE_RIGHT_VIEW"; payload: (prev: SidebarView | null) => SidebarView | null }
  | { type: "SET_SIDEBAR_AUTO_HIDE"; payload: boolean }
  | { type: "SET_SIDEBAR_HIDDEN"; payload: boolean }

  // Layout – terminal
  | { type: "SET_TERMINAL_VISIBLE"; payload: boolean }
  | { type: "TOGGLE_TERMINAL_VISIBLE" }
  | { type: "SET_TERMINAL_SESSIONS"; payload: SavedTerminalSession[] }

  // Layout – panels & menus
  | { type: "SET_SHOW_RECENT_MENU"; payload: boolean }
  | { type: "TOGGLE_SHOW_RECENT_MENU" }
  | { type: "SET_CONTEXT_MENU"; payload: { x: number; y: number; panel: SidebarView } | null }
  | { type: "SET_SHOW_STATUS_BAR"; payload: boolean }
  | { type: "SET_SHOW_ERROR_PANEL"; payload: boolean }
  | { type: "TOGGLE_SHOW_ERROR_PANEL" }
  | { type: "SET_AI_FULLSCREEN"; payload: boolean }
  | { type: "TOGGLE_AI_FULLSCREEN" }

  // Editor info
  | { type: "SET_CURSOR_INFO"; payload: CursorInfo }
  | { type: "SET_DIAGNOSTIC_COUNTS"; payload: DiagnosticCounts }
  | { type: "SET_DIAGNOSTIC_ITEMS"; payload: DiagnosticItem[] }

  // Preferences
  | { type: "SET_OLED_MODE"; payload: boolean }
  | { type: "SET_AI_COMPLETION_ENABLED"; payload: boolean }
  | { type: "TOGGLE_AI_COMPLETION_ENABLED" }
  | { type: "SET_SYSTEM_PROMPT"; payload: string }

  // Batch: reset files when opening a new project
  | { type: "RESET_FILES_FOR_PROJECT"; payload: { rootPath: string } }
  // Move panel to a zone, updating leftView/rightView accordingly
  | { type: "MOVE_PANEL_TO"; payload: { view: SidebarView; zone: PanelZone } }
  ;

// ---------------------------------------------------------------------------
// Reducer
// ---------------------------------------------------------------------------

function appReducer(state: AppState, action: AppAction): AppState {
  switch (action.type) {
    // Project / files
    case "SET_ROOT_PATH":
      return { ...state, rootPath: action.payload };
    case "SET_OPEN_FILES":
      return { ...state, openFiles: action.payload };
    case "UPDATE_OPEN_FILES":
      return { ...state, openFiles: action.payload(state.openFiles) };
    case "SET_ACTIVE_FILE_INDEX":
      return { ...state, activeFileIndex: action.payload };
    case "SET_RECENT_PROJECTS":
      return { ...state, recentProjects: action.payload };
    case "SET_WORKSPACE_SETTINGS":
      return { ...state, workspaceSettings: action.payload };
    case "UPDATE_WORKSPACE_SETTINGS":
      return { ...state, workspaceSettings: action.payload(state.workspaceSettings) };

    // Layout – sidebars
    case "SET_PANEL_ZONES":
      return { ...state, panelZones: action.payload };
    case "UPDATE_PANEL_ZONES":
      return { ...state, panelZones: action.payload(state.panelZones) };
    case "SET_LEFT_VIEW":
      return { ...state, leftView: action.payload };
    case "UPDATE_LEFT_VIEW":
      return { ...state, leftView: action.payload(state.leftView) };
    case "SET_RIGHT_VIEW":
      return { ...state, rightView: action.payload };
    case "UPDATE_RIGHT_VIEW":
      return { ...state, rightView: action.payload(state.rightView) };
    case "SET_SIDEBAR_AUTO_HIDE":
      return { ...state, sidebarAutoHide: action.payload };
    case "SET_SIDEBAR_HIDDEN":
      return { ...state, sidebarHidden: action.payload };

    // Layout – terminal
    case "SET_TERMINAL_VISIBLE":
      return { ...state, terminalVisible: action.payload };
    case "TOGGLE_TERMINAL_VISIBLE":
      return { ...state, terminalVisible: !state.terminalVisible };
    case "SET_TERMINAL_SESSIONS":
      return { ...state, terminalSessions: action.payload };

    // Layout – panels & menus
    case "SET_SHOW_RECENT_MENU":
      return { ...state, showRecentMenu: action.payload };
    case "TOGGLE_SHOW_RECENT_MENU":
      return { ...state, showRecentMenu: !state.showRecentMenu };
    case "SET_CONTEXT_MENU":
      return { ...state, contextMenu: action.payload };
    case "SET_SHOW_STATUS_BAR":
      return { ...state, showStatusBar: action.payload };
    case "SET_SHOW_ERROR_PANEL":
      return { ...state, showErrorPanel: action.payload };
    case "TOGGLE_SHOW_ERROR_PANEL":
      return { ...state, showErrorPanel: !state.showErrorPanel };
    case "SET_AI_FULLSCREEN":
      return { ...state, aiFullscreen: action.payload };
    case "TOGGLE_AI_FULLSCREEN":
      return { ...state, aiFullscreen: !state.aiFullscreen };

    // Editor info
    case "SET_CURSOR_INFO":
      return { ...state, cursorInfo: action.payload };
    case "SET_DIAGNOSTIC_COUNTS":
      return { ...state, diagnosticCounts: action.payload };
    case "SET_DIAGNOSTIC_ITEMS":
      return { ...state, diagnosticItems: action.payload };

    // Preferences
    case "SET_OLED_MODE":
      return { ...state, oledMode: action.payload };
    case "SET_AI_COMPLETION_ENABLED":
      return { ...state, aiCompletionEnabled: action.payload };
    case "TOGGLE_AI_COMPLETION_ENABLED":
      return { ...state, aiCompletionEnabled: !state.aiCompletionEnabled };
    case "SET_SYSTEM_PROMPT":
      return { ...state, systemPrompt: action.payload };

    // Compound actions
    case "RESET_FILES_FOR_PROJECT":
      return { ...state, rootPath: action.payload.rootPath, openFiles: [], activeFileIndex: 0 };

    case "MOVE_PANEL_TO": {
      const { view, zone } = action.payload;
      const newZones = { ...state.panelZones, [view]: zone };
      if (zone === "right") {
        return {
          ...state,
          panelZones: newZones,
          leftView: state.leftView === view ? null : state.leftView,
          rightView: view,
        };
      } else {
        return {
          ...state,
          panelZones: newZones,
          rightView: state.rightView === view ? null : state.rightView,
          leftView: view,
        };
      }
    }

    default:
      return state;
  }
}

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

const isMobileDevice = /iPhone|iPad|iPod|Android/i.test(navigator.userAgent);

export interface PersistedSettings {
  oledMode?: boolean;
  sidebarAutoHide?: boolean;
  showStatusBar?: boolean;
  leftSidebarWidth?: number;
  rightSidebarWidth?: number;
  terminalHeight?: number;
  terminalVisible?: boolean;
  gameViewportVisible?: boolean;
  gameViewportWidth?: number;
  aiCompletionEnabled?: boolean;
  lastProjectPath?: string;
  panelZones?: Record<SidebarView, PanelZone>;
  leftView?: SidebarView | null;
  rightView?: SidebarView | null;
  systemPrompt?: string;
}

function createInitialState(saved: PersistedSettings): AppState {
  return {
    rootPath: "",
    openFiles: [],
    activeFileIndex: 0,
    recentProjects: [],
    workspaceSettings: {},

    panelZones: saved.panelZones ? { ...DEFAULT_ZONES, ...saved.panelZones } : { ...DEFAULT_ZONES },
    leftView: isMobileDevice ? "ai" : (saved.leftView ?? "explorer"),
    rightView: isMobileDevice ? null : (saved.rightView ?? null),
    sidebarAutoHide: isMobileDevice ? false : (saved.sidebarAutoHide ?? true),
    sidebarHidden: false,

    terminalVisible: saved.terminalVisible ?? true,
    terminalSessions: [],

    showRecentMenu: false,
    contextMenu: null,
    showStatusBar: saved.showStatusBar ?? true,
    showErrorPanel: false,
    aiFullscreen: false,

    cursorInfo: { line: 1, column: 1, selected: 0 },
    diagnosticCounts: { errors: 0, warnings: 0, infos: 0 },
    diagnosticItems: [],

    oledMode: saved.oledMode ?? false,
    aiCompletionEnabled: saved.aiCompletionEnabled ?? false,
    systemPrompt: saved.systemPrompt ?? "",
  };
}

export function useAppState(saved: PersistedSettings): [AppState, Dispatch<AppAction>] {
  return useReducer(appReducer, saved, createInitialState);
}
