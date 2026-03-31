import { useCallback, useRef, type ReactElement } from "react";
import type { SidebarView, PanelZone, RecentProject } from "../types";

interface ActivityBarProps {
  leftView: SidebarView | null;
  rightView: SidebarView | null;
  panelZones: Record<SidebarView, PanelZone>;
  sidebarAutoHide: boolean;
  sidebarHidden: boolean;
  sidebarTimerRef: React.MutableRefObject<ReturnType<typeof setTimeout> | null>;
  onActivityClick: (view: SidebarView) => void;
  onPanelContextMenu: (e: React.MouseEvent, panel: SidebarView) => void;
  onSidebarHiddenChange: (hidden: boolean) => void;
  // Settings button
  // Recent projects
  showRecentMenu: boolean;
  recentProjects: RecentProject[];
  onRecentMenuToggle: () => void;
  onRecentProjectClick: (path: string) => void;
  // Toggles
  aiCompletionEnabled: boolean;
  onAiCompletionToggle: () => void;
  terminalVisible: boolean;
  onTerminalToggle: () => void;
}

const PANEL_ORDER: SidebarView[] = [
  "explorer", "ai", "gamedev", "planengine", "collab", "rag", "todos", "search", "remote", "llmloader", "languages", "logs", "bluetooth",
  "gitgraph", "testexplorer", "agent", "database", "debug", "deps", "edithistory", "keybindings", "docs",
  "profiler", "glslpreview", "pr", "plugins", "mutation", "cicd",
];

const PANEL_TITLES: Record<SidebarView, string> = {
  explorer: "Explorer",
  ai: "ShadowAI (Ctrl+Shift+A)",
  collab: "Collaboration",
  rag: "RAG Documents",
  todos: "Diagnostics (Ctrl+Shift+T)",
  search: "Search (Ctrl+Shift+F)",
  remote: "Remote (Ctrl+Shift+R)",
  llmloader: "LLM Loader",
  languages: "Languages",
  logs: "Console",
  bluetooth: "Bluetooth (Offline)",
  settings: "Settings",
  gitgraph: "Git Graph",
  testexplorer: "Test Explorer",
  agent: "Agent Tasks",
  database: "Database",
  debug: "Debug",
  deps: "Dependency Graph",
  edithistory: "Edit History",
  keybindings: "Keybindings",
  docs: "Docs",
  mutation: "Mutation Testing",
  profiler: "Profiler",
  glslpreview: "GLSL Preview",
  pr: "Pull Requests",
  plugins: "Plugins",
  cicd: "CI/CD (GitHub Actions)",
  gamedev: "ShadowEditor (Game Engine)",
  planengine: "PlanEngine Roadmap",
};

const PANEL_ICONS: Record<SidebarView, ReactElement> = {
  explorer: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" /></svg>,
  ai: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M12 2a7 7 0 017 7v1a7 7 0 01-14 0V9a7 7 0 017-7z" /><path d="M9 22h6" /><path d="M12 17v5" /><circle cx="9" cy="10" r="1" fill="currentColor" /><circle cx="15" cy="10" r="1" fill="currentColor" /></svg>,
  collab: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M16 21v-2a4 4 0 00-4-4H7a4 4 0 00-4 4v2"/><circle cx="9.5" cy="7" r="3"/><path d="M22 21v-2a4 4 0 00-3-3.87"/><path d="M16 3.13a4 4 0 010 7.75"/></svg>,
  rag: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M4 19.5A2.5 2.5 0 016.5 17H20" /><path d="M6.5 2H20v20H6.5A2.5 2.5 0 014 19.5v-15A2.5 2.5 0 016.5 2z" /><line x1="9" y1="7" x2="16" y2="7" /><line x1="9" y1="11" x2="14" y2="11" /></svg>,
  todos: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M9 11l3 3L22 4" /><path d="M21 12v7a2 2 0 01-2 2H5a2 2 0 01-2-2V5a2 2 0 012-2h11" /></svg>,
  search: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><circle cx="11" cy="11" r="8" /><path d="M21 21l-4.35-4.35" /></svg>,
  remote: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M5 12.55a11 11 0 0114.08 0" /><path d="M1.42 9a16 16 0 0121.16 0" /><path d="M8.53 16.11a6 6 0 016.95 0" /><circle cx="12" cy="20" r="1" fill="currentColor" /></svg>,
  llmloader: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><rect x="4" y="4" width="16" height="16" rx="2" /><line x1="9" y1="9" x2="9.01" y2="9" strokeWidth="2" /><line x1="15" y1="9" x2="15.01" y2="9" strokeWidth="2" /><line x1="9" y1="15" x2="9.01" y2="15" strokeWidth="2" /><line x1="15" y1="15" x2="15.01" y2="15" strokeWidth="2" /></svg>,
  languages: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><polyline points="16 18 22 12 16 6" /><polyline points="8 6 2 12 8 18" /><line x1="14" y1="4" x2="10" y2="20" /></svg>,
  logs: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M4 19h16" /><path d="M4 15h16" /><path d="M4 11h16" /><path d="M4 7h16" /></svg>,
  bluetooth: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><polyline points="6.5 6.5 17.5 17.5 12 23 12 1 17.5 6.5 6.5 17.5" /></svg>,
  settings: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z" /></svg>,
  gitgraph: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><circle cx="6" cy="6" r="2"/><circle cx="6" cy="18" r="2"/><circle cx="18" cy="12" r="2"/><line x1="6" y1="8" x2="6" y2="16"/><path d="M6 8c0-2 4-4 12-4v8"/></svg>,
  testexplorer: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><polygon points="5 3 19 12 5 21 5 3"/></svg>,
  agent: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><rect x="3" y="3" width="7" height="7"/><rect x="14" y="3" width="7" height="7"/><rect x="14" y="14" width="7" height="7"/><path d="M3 17h5v4M7 14H3v-4"/></svg>,
  database: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><ellipse cx="12" cy="5" rx="9" ry="3"/><path d="M3 5v14c0 1.66 4.03 3 9 3s9-1.34 9-3V5"/><path d="M3 12c0 1.66 4.03 3 9 3s9-1.34 9-3"/></svg>,
  debug: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M12 22c1.1 0 2-.9 2-2H10c0 1.1.9 2 2 2zm6-6v-5c0-3.07-1.64-5.64-4.5-6.32V4c0-.83-.67-1.5-1.5-1.5S10.5 3.17 10.5 4v.68C7.63 5.36 6 7.92 6 11v5l-2 2v1h16v-1l-2-2z"/></svg>,
  deps: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><circle cx="12" cy="5" r="2"/><circle cx="5" cy="19" r="2"/><circle cx="19" cy="19" r="2"/><line x1="12" y1="7" x2="5" y2="17"/><line x1="12" y1="7" x2="19" y2="17"/></svg>,
  edithistory: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><circle cx="12" cy="12" r="10"/><polyline points="12 6 12 12 16 14"/></svg>,
  keybindings: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><rect x="2" y="6" width="20" height="13" rx="2"/><path d="M6 10h.01M10 10h.01M14 10h.01M18 10h.01M8 14h8"/></svg>,
  docs: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M4 19.5A2.5 2.5 0 016.5 17H20"/><path d="M6.5 2H20v20H6.5A2.5 2.5 0 014 19.5v-15A2.5 2.5 0 016.5 2z"/><line x1="9" y1="7" x2="16" y2="7"/><line x1="9" y1="11" x2="13" y2="11"/></svg>,
  mutation: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M7 20c3-3 3-6 0-9s-3-6 0-9"/><path d="M17 20c-3-3-3-6 0-9s3-6 0-9"/><line x1="7" y1="15" x2="17" y2="15"/><line x1="7" y1="9" x2="17" y2="9"/></svg>,
  profiler: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><line x1="18" y1="20" x2="18" y2="10"/><line x1="12" y1="20" x2="12" y2="4"/><line x1="6" y1="20" x2="6" y2="14"/><line x1="2" y1="20" x2="22" y2="20"/></svg>,
  glslpreview: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M12 2L2 19h20L12 2z"/><line x1="6.5" y1="13" x2="17.5" y2="13"/></svg>,
  pr: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><circle cx="6" cy="6" r="2"/><circle cx="6" cy="18" r="2"/><circle cx="18" cy="6" r="2"/><path d="M6 8v8"/><path d="M18 8v2a4 4 0 01-4 4H9"/><polyline points="6 12 8 14 6 16"/></svg>,
  plugins: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M20.5 11H19V7a2 2 0 00-2-2h-4V3.5a2.5 2.5 0 00-5 0V5H4a2 2 0 00-2 2v3.8h1.5a2.5 2.5 0 010 5H2V20a2 2 0 002 2h3.8v-1.5a2.5 2.5 0 015 0V22H17a2 2 0 002-2v-4h1.5a2.5 2.5 0 000-5z"/></svg>,
  cicd: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><circle cx="12" cy="12" r="3"/><path d="M12 2v3M12 19v3M4.22 4.22l2.12 2.12M17.66 17.66l2.12 2.12M2 12h3M19 12h3M4.22 19.78l2.12-2.12M17.66 6.34l2.12-2.12"/></svg>,
  gamedev: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><rect x="2" y="7" width="20" height="11" rx="3"/><path d="M8 11v4M6 13h4"/><circle cx="16" cy="11.5" r="1" fill="currentColor"/><circle cx="18.5" cy="13.5" r="1" fill="currentColor"/></svg>,
  planengine: <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M4 5.5A2.5 2.5 0 016.5 3H20v18H6.5A2.5 2.5 0 014 18.5v-13z"/><path d="M8 7h8"/><path d="M8 11h8"/><path d="M8 15h5"/><path d="M4 18.5A2.5 2.5 0 016.5 16H20"/></svg>,
};

export function ActivityBar({
  leftView, rightView, panelZones, sidebarAutoHide, sidebarHidden,
  sidebarTimerRef, onActivityClick, onPanelContextMenu, onSidebarHiddenChange,
  showRecentMenu, recentProjects, onRecentMenuToggle, onRecentProjectClick,
  aiCompletionEnabled, onAiCompletionToggle,
  terminalVisible, onTerminalToggle,
}: ActivityBarProps) {
  const scrollRef = useRef<HTMLDivElement>(null);

  const handleWheel = useCallback((e: React.WheelEvent<HTMLDivElement>) => {
    const container = scrollRef.current;
    if (!container) return;

    const maxScrollTop = container.scrollHeight - container.clientHeight;
    if (maxScrollTop <= 0) return;

    const nextScrollTop = Math.max(0, Math.min(maxScrollTop, container.scrollTop + e.deltaY));
    if (nextScrollTop === container.scrollTop) return;

    container.scrollTop = nextScrollTop;
    e.preventDefault();
  }, []);

  return (
    <div className="activity-bar"
      ref={scrollRef}
      data-testid="activity-bar"
      onMouseEnter={() => { if (sidebarAutoHide && sidebarHidden) { onSidebarHiddenChange(false); if (sidebarTimerRef.current) clearTimeout(sidebarTimerRef.current); } }}
      onWheelCapture={handleWheel}
    >
      {PANEL_ORDER.map((view) => (
        <button
          key={view}
          className={`activity-btn${(leftView === view || rightView === view) ? " active" : ""}${panelZones[view] === "right" ? " zone-right" : ""}`}
          title={PANEL_TITLES[view]}
          aria-label={PANEL_TITLES[view]}
          aria-pressed={leftView === view || rightView === view}
          role="button"
          tabIndex={0}
          onClick={() => onActivityClick(view)}
          onContextMenu={(e) => onPanelContextMenu(e, view)}
        >
          {PANEL_ICONS[view]}
        </button>
      ))}
      <div className="activity-spacer" />
      <button
        className={`activity-btn${(leftView === "settings" || rightView === "settings") ? " active" : ""}`}
        title="Settings"
        aria-label="Settings"
        aria-pressed={leftView === "settings" || rightView === "settings"}
        role="button"
        tabIndex={0}
        onClick={() => onActivityClick("settings")}
        onContextMenu={(e) => onPanelContextMenu(e, "settings")}
      >
        {PANEL_ICONS.settings}
      </button>
      {/* Recent Projects */}
      <div className="activity-btn-wrapper">
        <button className="activity-btn" title="Recent Projects" aria-label="Recent Projects" onClick={(e) => { e.stopPropagation(); onRecentMenuToggle(); }}>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="12" cy="12" r="10" /><polyline points="12 6 12 12 16 14" />
          </svg>
        </button>
        {showRecentMenu && recentProjects.length > 0 && (
          <div className="recent-menu">
            <div className="recent-menu-title">Recent Projects</div>
            {recentProjects.map((p) => (
              <div key={p.path} className="recent-menu-item" onClick={() => onRecentProjectClick(p.path)}>
                <span className="recent-name">{p.name}</span>
                <span className="recent-path">{p.path}</span>
              </div>
            ))}
          </div>
        )}
      </div>
      <button
        className={`activity-btn ${aiCompletionEnabled ? "active" : ""}`}
        title={`AI Completion: ${aiCompletionEnabled ? "ON" : "OFF"}`}
        aria-label={`AI Completion: ${aiCompletionEnabled ? "ON" : "OFF"}`}
        onClick={onAiCompletionToggle}
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <polyline points="16 18 22 12 16 6" /><polyline points="8 6 2 12 8 18" />
        </svg>
      </button>
      <button
        className={`activity-btn ${terminalVisible ? "active" : ""}`}
        title="Toggle Terminal (Ctrl+`)"
        aria-label="Toggle Terminal"
        onClick={onTerminalToggle}
      >
        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <polyline points="4 17 10 11 4 5" /><line x1="12" y1="19" x2="20" y2="19" />
        </svg>
      </button>
    </div>
  );
}
