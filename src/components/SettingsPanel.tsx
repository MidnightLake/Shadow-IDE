import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useUpdater } from "../hooks/useUpdater";
import ThemeEditor from "./ThemeEditor";
import ThemeMarketplace from "./ThemeMarketplace";

type SidebarView = "explorer" | "ai" | "remote" | "todos" | "search" | "settings" | "llmloader" | "languages" | "gamedev" | "planengine";
type PanelZone = "left" | "right";

interface CloudSnippet {
  id: string;
  title: string;
  language: string;
  content: string;
  tags: string[];
  created_at: number;
  updated_at: number;
}

interface CloudBundleStatus {
  bundle_path: string;
  exists: boolean;
  modified_at: number | null;
  size_bytes: number | null;
}

interface CloudSyncImportResult {
  bundle_path: string;
  imported_at: number;
  snippet_count: number;
  skill_count: number;
  restored_session_count: number;
  frontend: {
    settings_json: string;
    themes_json: string;
    keybindings_json: string;
    ui_skills_json: string;
  };
}

const CLOUD_PATH_KEY = "shadowide-cloud-sync-path";
const CLOUD_INCLUDE_SESSIONS_KEY = "shadowide-cloud-sync-include-sessions";

const PANEL_LABELS: Record<SidebarView, string> = {
  explorer: "File Explorer",
  ai: "AI Chat",
  todos: "Diagnostics",
  search: "Search",
  remote: "Remote",
  settings: "Settings",
  llmloader: "LLM Loader",
  languages: "Languages",
  gamedev: "ShadowEditor",
  planengine: "PlanEngine",
};

function readJsonStorage<T>(key: string, fallback: T): T {
  try {
    const raw = localStorage.getItem(key);
    if (raw) return JSON.parse(raw) as T;
  } catch { /* ignore */ }
  return fallback;
}

function formatEpoch(epoch: number | null | undefined): string {
  if (!epoch) return "Never";
  return new Date(epoch * 1000).toLocaleString();
}

interface SettingsPanelProps {
  visible: boolean;
  oledMode: boolean;
  onOledChange: (v: boolean) => void;
  panelZones: Record<SidebarView, PanelZone>;
  onPanelZoneChange: (view: SidebarView, zone: PanelZone) => void;
  sidebarAutoHide: boolean;
  onSidebarAutoHideChange: (v: boolean) => void;
  showStatusBar: boolean;
  onShowStatusBarChange: (v: boolean) => void;
  aiCompletionEnabled: boolean;
  onAiCompletionChange: (v: boolean) => void;
  fontSize: number;
  onFontSizeChange: (v: number) => void;
  tabSize: number;
  onTabSizeChange: (v: number) => void;
  minimapEnabled: boolean;
  onMinimapChange: (v: boolean) => void;
  useTabs: boolean;
  onUseTabsChange: (v: boolean) => void;
  systemPrompt: string;
  onSystemPromptChange: (v: string) => void;
}

export default function SettingsPanel({
  visible,
  oledMode,
  onOledChange,
  panelZones,
  onPanelZoneChange,
  sidebarAutoHide,
  onSidebarAutoHideChange,
  showStatusBar,
  onShowStatusBarChange,
  aiCompletionEnabled,
  onAiCompletionChange,
  fontSize,
  onFontSizeChange,
  tabSize,
  onTabSizeChange,
  minimapEnabled,
  onMinimapChange,
  useTabs,
  onUseTabsChange,
  systemPrompt,
  onSystemPromptChange,
}: SettingsPanelProps) {
  const { status: updateStatus, installUpdate, dismiss: dismissUpdate } = useUpdater();
  const [showAbout, setShowAbout] = useState(false);
  const [showThemeEditor, setShowThemeEditor] = useState(false);
  const [showThemeMarketplace, setShowThemeMarketplace] = useState(false);
  const [glassMode, setGlassMode] = useState(() => document.body.classList.contains("glass-mode"));
  const [fontFamily, setFontFamily] = useState("JetBrains Mono, Fira Code, monospace");
  const [editorFontSize, setEditorFontSize] = useState(14);
  const [lineHeight, setLineHeight] = useState(1.5);
  const [ligatures, setLigatures] = useState(true);
  const [engineInstalling, setEngineInstalling] = useState(false);
  const [engineMsg, setEngineMsg] = useState("");
  const [enginePercent, setEnginePercent] = useState(0);
  const [backend, setBackend] = useState("vulkan");
  const [cloudPath, setCloudPath] = useState(() => localStorage.getItem(CLOUD_PATH_KEY) || "");
  const [cloudPassphrase, setCloudPassphrase] = useState("");
  const [cloudIncludeSessions, setCloudIncludeSessions] = useState(
    () => localStorage.getItem(CLOUD_INCLUDE_SESSIONS_KEY) !== "false"
  );
  const [cloudBusy, setCloudBusy] = useState(false);
  const [cloudMessage, setCloudMessage] = useState("");
  const [cloudStatus, setCloudStatus] = useState<CloudBundleStatus | null>(null);
  const [cloudSnippets, setCloudSnippets] = useState<CloudSnippet[]>([]);
  const [editingSnippetId, setEditingSnippetId] = useState<string | null>(null);
  const [snippetTitle, setSnippetTitle] = useState("");
  const [snippetLanguage, setSnippetLanguage] = useState("text");
  const [snippetTags, setSnippetTags] = useState("");
  const [snippetContent, setSnippetContent] = useState("");

  const [installedBackends, setInstalledBackends] = useState<string[]>([]);

  const fetchEngineInfo = () => {
    invoke<string[]>("list_installed_engines")
      .then((installed) => setInstalledBackends(installed ?? []))
      .catch(() => {});
  };

  useEffect(() => {
    if (visible) {
      setShowAbout(false);
      fetchEngineInfo();
      invoke<string>("detect_recommended_backend")
        .then((b) => setBackend(b ?? "cpu"))
        .catch(() => {});
    }
  }, [visible]);

  useEffect(() => {
    try { localStorage.setItem(CLOUD_PATH_KEY, cloudPath); } catch { /* ignore */ }
  }, [cloudPath]);

  useEffect(() => {
    try { localStorage.setItem(CLOUD_INCLUDE_SESSIONS_KEY, String(cloudIncludeSessions)); } catch { /* ignore */ }
  }, [cloudIncludeSessions]);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<{ stage: string; percent: number; detail: string }>("engine-install-progress", (e) => {
      setEngineMsg(e.payload.detail);
      setEnginePercent(e.payload.percent);
      if (e.payload.stage === "done") {
        setEngineInstalling(false);
        fetchEngineInfo();
      }
    }).then((u) => { unlisten = u; });
    return () => { if (unlisten) unlisten(); };
  }, []);

  const loadCloudSnippets = async () => {
    try {
      const snippets = await invoke<CloudSnippet[]>("cloud_list_snippets");
      setCloudSnippets(snippets ?? []);
    } catch {
      setCloudSnippets([]);
    }
  };

  const loadCloudStatus = async (path = cloudPath) => {
    const trimmed = path.trim();
    if (!trimmed) {
      setCloudStatus(null);
      return;
    }
    try {
      const status = await invoke<CloudBundleStatus>("cloud_get_bundle_status", { cloudPath: trimmed });
      setCloudStatus(status);
    } catch {
      setCloudStatus(null);
    }
  };

  useEffect(() => {
    if (!visible) return;
    void loadCloudSnippets();
    void loadCloudStatus();
  }, [visible]);

  const doInstall = async () => {
    setEngineInstalling(true);
    setEngineMsg("Starting...");
    setEnginePercent(0);
    try {
      await invoke<string>("install_engine", { backend });
    } catch (e) {
      setEngineMsg(String(e));
      setEngineInstalling(false);
    }
  };

  const handleApplyFont = async () => {
    window.dispatchEvent(new CustomEvent("editor-font-changed", {
      detail: { family: fontFamily, size: editorFontSize, lineHeight, ligatures },
    }));
    try {
      await invoke("update_editor_font", { family: fontFamily, size: editorFontSize, lineHeight, ligatures });
    } catch { /* backend may not implement */ }
  };

  const toggleGlassMode = (enabled: boolean) => {
    setGlassMode(enabled);
    if (enabled) document.body.classList.add("glass-mode");
    else document.body.classList.remove("glass-mode");
  };

  const resetSnippetEditor = () => {
    setEditingSnippetId(null);
    setSnippetTitle("");
    setSnippetLanguage("text");
    setSnippetTags("");
    setSnippetContent("");
  };

  const saveSnippet = async () => {
    const title = snippetTitle.trim();
    const content = snippetContent.trim();
    if (!title || !content) {
      setCloudMessage("Snippet title and content are required.");
      return;
    }

    try {
      setCloudBusy(true);
      setCloudMessage("");
      await invoke<CloudSnippet>("cloud_save_snippet", {
        snippet: {
          id: editingSnippetId,
          title,
          language: snippetLanguage.trim() || "text",
          content: snippetContent,
          tags: snippetTags.split(",").map((tag) => tag.trim()).filter(Boolean),
        },
      });
      await loadCloudSnippets();
      resetSnippetEditor();
      setCloudMessage("Snippet library updated.");
    } catch (e) {
      setCloudMessage(String(e));
    } finally {
      setCloudBusy(false);
    }
  };

  const exportCloudBundle = async () => {
    if (!cloudPath.trim()) {
      setCloudMessage("Cloud folder path is required.");
      return;
    }
    if (cloudPassphrase.trim().length < 8) {
      setCloudMessage("Use a passphrase with at least 8 characters.");
      return;
    }

    const themeSnapshot = {
      theme: localStorage.getItem("shadowide-theme"),
      colorFilter: localStorage.getItem("shadow-ide-color-filter"),
      fontScale: localStorage.getItem("shadow-ide-font-scale"),
      customThemes: readJsonStorage<Record<string, unknown>>("shadow-custom-themes", {}),
      marketplaceInstalled: readJsonStorage<string[]>("shadow-marketplace-installed", []),
    };

    try {
      setCloudBusy(true);
      setCloudMessage("");
      const result = await invoke<{
        bundle_path: string;
        snippet_count: number;
        skill_count: number;
        session_count: number;
      }>("cloud_export_bundle", {
        cloudPath: cloudPath.trim(),
        passphrase: cloudPassphrase,
        settingsJson: localStorage.getItem("shadowide-settings") ?? "",
        themesJson: JSON.stringify(themeSnapshot),
        keybindingsJson: localStorage.getItem("shadowide-keybindings-overrides") ?? "",
        uiSkillsJson: localStorage.getItem("shadowide-ui-skills") ?? "",
        includeSessions: cloudIncludeSessions,
      });
      await loadCloudStatus(cloudPath);
      setCloudMessage(
        `Synced ${result.snippet_count} snippets, ${result.skill_count} skills, and ${result.session_count} sessions to ${result.bundle_path}.`
      );
    } catch (e) {
      setCloudMessage(String(e));
    } finally {
      setCloudBusy(false);
    }
  };

  const importCloudBundle = async () => {
    if (!cloudPath.trim()) {
      setCloudMessage("Cloud folder path is required.");
      return;
    }
    if (cloudPassphrase.trim().length < 8) {
      setCloudMessage("Use a passphrase with at least 8 characters.");
      return;
    }

    try {
      setCloudBusy(true);
      setCloudMessage("");
      const result = await invoke<CloudSyncImportResult>("cloud_import_bundle", {
        cloudPath: cloudPath.trim(),
        passphrase: cloudPassphrase,
        restoreSessions: cloudIncludeSessions,
      });

      if (result.frontend.settings_json) {
        localStorage.setItem("shadowide-settings", result.frontend.settings_json);
      }

      if (result.frontend.themes_json) {
        const themeSnapshot = JSON.parse(result.frontend.themes_json) as {
          theme?: string | null;
          colorFilter?: string | null;
          fontScale?: string | null;
          customThemes?: unknown;
          marketplaceInstalled?: unknown;
        };
        if (themeSnapshot.theme) localStorage.setItem("shadowide-theme", themeSnapshot.theme);
        if (themeSnapshot.colorFilter) localStorage.setItem("shadow-ide-color-filter", themeSnapshot.colorFilter);
        if (themeSnapshot.fontScale) localStorage.setItem("shadow-ide-font-scale", themeSnapshot.fontScale);
        if (themeSnapshot.customThemes) localStorage.setItem("shadow-custom-themes", JSON.stringify(themeSnapshot.customThemes));
        if (themeSnapshot.marketplaceInstalled) localStorage.setItem("shadow-marketplace-installed", JSON.stringify(themeSnapshot.marketplaceInstalled));
      }

      if (result.frontend.keybindings_json) {
        localStorage.setItem("shadowide-keybindings-overrides", result.frontend.keybindings_json);
      }
      if (result.frontend.ui_skills_json) {
        localStorage.setItem("shadowide-ui-skills", result.frontend.ui_skills_json);
      }

      await loadCloudSnippets();
      await loadCloudStatus(cloudPath);

      setCloudMessage(
        `Imported ${result.snippet_count} synced snippets, ${result.skill_count} skills, and restored ${result.restored_session_count} sessions. Reloading UI...`
      );

      setTimeout(() => window.location.reload(), 700);
    } catch (e) {
      setCloudMessage(String(e));
    } finally {
      setCloudBusy(false);
    }
  };

  if (!visible) return null;

  if (showThemeMarketplace) {
    return (
      <div className="settings-panel" style={{ padding: 0, height: "100%", display: "flex", flexDirection: "column" }}>
        <div style={{ padding: "6px 10px", borderBottom: "1px solid var(--theme-border, #313244)", display: "flex", alignItems: "center", gap: 8, flexShrink: 0 }}>
          <button
            className="settings-about-btn"
            onClick={() => setShowThemeMarketplace(false)}
          >
            ← Back
          </button>
          <span style={{ fontWeight: 700, fontSize: 12, color: "var(--theme-accent, #89b4fa)" }}>Theme Marketplace</span>
        </div>
        <div style={{ flex: 1, overflow: "hidden" }}>
          <ThemeMarketplace />
        </div>
      </div>
    );
  }

  return (
    <div className="settings-panel">
      {showThemeEditor && <ThemeEditor onClose={() => setShowThemeEditor(false)} />}
      <div className="settings-header">
        <span className="settings-title">SETTINGS</span>
      </div>

      {/* Appearance */}
      <div className="settings-section">
        <div className="settings-section-title">Appearance</div>

        <label className="settings-row">
          <span className="settings-label">OLED Mode</span>
          <input
            type="checkbox"
            className="settings-toggle"
            checked={oledMode}
            onChange={(e) => onOledChange(e.target.checked)}
          />
        </label>

        <div className="settings-subsection">
          <span className="settings-label" style={{ marginBottom: 6, display: "block" }}>Panel Positions</span>
          {(Object.keys(PANEL_LABELS) as SidebarView[]).map((view) => (
            <div key={view} className="settings-row settings-panel-zone">
              <span className="settings-label settings-label-sm">{PANEL_LABELS[view]}</span>
              <select
                className="settings-select settings-select-sm"
                value={panelZones[view]}
                onChange={(e) => onPanelZoneChange(view, e.target.value as PanelZone)}
              >
                <option value="left">Left</option>
                <option value="right">Right</option>
              </select>
            </div>
          ))}
        </div>

        <label className="settings-row">
          <span className="settings-label">Auto-Hide Sidebar</span>
          <input
            type="checkbox"
            className="settings-toggle"
            checked={sidebarAutoHide}
            onChange={(e) => onSidebarAutoHideChange(e.target.checked)}
          />
        </label>

        <label className="settings-row">
          <span className="settings-label">Show Status Bar</span>
          <input
            type="checkbox"
            className="settings-toggle"
            checked={showStatusBar}
            onChange={(e) => onShowStatusBarChange(e.target.checked)}
          />
        </label>

        <label className="settings-row">
          <span className="settings-label">Font Size</span>
          <input
            type="number"
            className="settings-number"
            min={10}
            max={24}
            value={fontSize}
            onChange={(e) => onFontSizeChange(Number(e.target.value))}
          />
        </label>

        <div className="settings-row" style={{ marginTop: 8 }}>
          <button
            className="settings-about-btn"
            onClick={() => setShowThemeEditor(true)}
          >
            Customize Theme
          </button>
        </div>

        <div className="settings-row" style={{ marginTop: 4 }}>
          <button
            className="settings-about-btn"
            onClick={() => setShowThemeMarketplace(true)}
          >
            Theme Marketplace
          </button>
        </div>

        <label className="settings-row" style={{ marginTop: 8 }}>
          <span className="settings-label">Glass UI</span>
          <input
            type="checkbox"
            className="settings-toggle"
            checked={glassMode}
            onChange={(e) => toggleGlassMode(e.target.checked)}
          />
        </label>
      </div>

      {/* Font */}
      <div className="settings-section">
        <div className="settings-section-title">Font</div>

        <label className="settings-row">
          <span className="settings-label">Font Family</span>
          <input
            type="text"
            className="settings-number"
            style={{ width: 180 }}
            value={fontFamily}
            onChange={(e) => setFontFamily(e.target.value)}
          />
        </label>

        <label className="settings-row">
          <span className="settings-label">Font Size</span>
          <input
            type="number"
            className="settings-number"
            min={10}
            max={24}
            value={editorFontSize}
            onChange={(e) => setEditorFontSize(Number(e.target.value))}
          />
        </label>

        <label className="settings-row">
          <span className="settings-label">Line Height</span>
          <input
            type="number"
            className="settings-number"
            min={1.0}
            max={2.5}
            step={0.1}
            value={lineHeight}
            onChange={(e) => setLineHeight(Number(e.target.value))}
          />
        </label>

        <label className="settings-row">
          <span className="settings-label">Ligatures</span>
          <input
            type="checkbox"
            className="settings-toggle"
            checked={ligatures}
            onChange={(e) => setLigatures(e.target.checked)}
          />
        </label>

        <div className="settings-row" style={{ marginTop: 4 }}>
          <button className="settings-about-btn" onClick={handleApplyFont}>
            Apply
          </button>
        </div>
      </div>

      {/* Editor */}
      <div className="settings-section">
        <div className="settings-section-title">Editor</div>

        <label className="settings-row">
          <span className="settings-label">Tab Size</span>
          <input
            type="number"
            className="settings-number"
            min={1}
            max={8}
            value={tabSize}
            onChange={(e) => onTabSizeChange(Number(e.target.value))}
          />
        </label>

        <label className="settings-row">
          <span className="settings-label">Use Tabs</span>
          <input
            type="checkbox"
            className="settings-toggle"
            checked={useTabs}
            onChange={(e) => onUseTabsChange(e.target.checked)}
          />
        </label>

        <label className="settings-row">
          <span className="settings-label">Minimap</span>
          <input
            type="checkbox"
            className="settings-toggle"
            checked={minimapEnabled}
            onChange={(e) => onMinimapChange(e.target.checked)}
          />
        </label>

        <label className="settings-row">
          <span className="settings-label">AI Inline Completion</span>
          <input
            type="checkbox"
            className="settings-toggle"
            checked={aiCompletionEnabled}
            onChange={(e) => onAiCompletionChange(e.target.checked)}
          />
        </label>
      </div>

      {/* Keyboard Shortcuts */}
      <div className="settings-section">
        <div className="settings-section-title">Keyboard Shortcuts</div>
        <div className="settings-shortcuts">
          <div className="shortcut-row">
            <span>Toggle Terminal</span><kbd>Ctrl+`</kbd>
          </div>
          <div className="shortcut-row">
            <span>AI Chat</span><kbd>Ctrl+Shift+A</kbd>
          </div>
          <div className="shortcut-row">
            <span>Search</span><kbd>Ctrl+Shift+F</kbd>
          </div>
          <div className="shortcut-row">
            <span>Diagnostics</span><kbd>Ctrl+Shift+T</kbd>
          </div>
          <div className="shortcut-row">
            <span>Remote</span><kbd>Ctrl+Shift+R</kbd>
          </div>
        </div>
      </div>

      {/* System Prompt */}
      <div className="settings-section">
        <div className="settings-section-title">System Prompt</div>
        <textarea
          className="settings-system-prompt"
          value={systemPrompt}
          onChange={(e) => onSystemPromptChange(e.target.value)}
          placeholder="Custom system prompt for AI (leave empty for template default)"
          rows={4}
        />
        {systemPrompt.trim() && (
          <button
            className="settings-reset-btn"
            onClick={() => onSystemPromptChange("")}
          >
            Reset to default
          </button>
        )}
      </div>

      {/* LLM Engine */}
      <div className="settings-section">
        <div className="settings-section-title">LLM Engine</div>
        <div style={{ marginBottom: 12 }}>
          {installedBackends.length > 0 ? (
            <div className="settings-row" style={{ flexDirection: "column", alignItems: "flex-start", gap: 4 }}>
              <span className="settings-label" style={{ marginBottom: 4 }}>Installed Backends</span>
              <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
                {installedBackends.map((b) => (
                  <span key={b} className="llm-engine-backend" style={{ display: "inline-flex", alignItems: "center", gap: 4 }}>
                    {b.toUpperCase()}
                    <button
                      onClick={async () => {
                        if (confirm(`Uninstall ${b}?`)) {
                          await invoke("uninstall_engine", { backend: b });
                          fetchEngineInfo();
                        }
                      }}
                      style={{ background: "transparent", border: "none", color: "#ef4444", cursor: "pointer", padding: 0, marginLeft: 4 }}
                      title="Uninstall"
                    >
                      &times;
                    </button>
                  </span>
                ))}
              </div>
            </div>
          ) : (
            <span className="settings-label" style={{ display: "block", marginBottom: 8 }}>No engines installed</span>
          )}
        </div>
        
        <div>
          <div className="settings-row" style={{ gap: 6 }}>
            <select className="settings-select settings-select-sm" value={backend} onChange={(e) => setBackend(e.target.value)} disabled={engineInstalling}>
              <option value="cuda">CUDA (NVIDIA)</option>
              <option value="rocm">ROCm (AMD)</option>
              <option value="vulkan">Vulkan (Any GPU)</option>
              <option value="cpu">CPU Only</option>
            </select>
            <button className="settings-about-btn" onClick={doInstall} disabled={engineInstalling || installedBackends.includes(backend)} style={{ whiteSpace: "nowrap" }}>
              {engineInstalling ? "Installing..." : (installedBackends.includes(backend) ? "Installed" : "Install Engine")}
            </button>
          </div>
        </div>
        {engineMsg && (
          <div style={{ fontSize: 10, color: "var(--text-muted)", marginTop: 4 }}>
            {engineMsg}
            {engineInstalling && enginePercent > 0 && (
              <div className="llm-progress-bar" style={{ marginTop: 4 }}>
                <div className="llm-progress-fill" style={{ width: `${enginePercent}%` }} />
              </div>
            )}
          </div>
        )}
      </div>

      {/* Cloud Sync */}
      <div className="settings-section">
        <div className="settings-section-title">Cloud Sync</div>
        <div style={{ fontSize: 11, color: "var(--text-muted)", marginBottom: 10 }}>
          Point this at a Dropbox, Nextcloud, iCloud, or other synced folder. ShadowIDE writes one encrypted sync bundle and keeps snippets plus optional AI sessions portable across devices.
        </div>

        <label className="settings-row" style={{ alignItems: "flex-start", gap: 8 }}>
          <span className="settings-label">Cloud Folder</span>
          <input
            type="text"
            className="settings-number"
            style={{ width: 220 }}
            value={cloudPath}
            placeholder="/path/to/cloud-sync-folder"
            onChange={(e) => setCloudPath(e.target.value)}
            onBlur={() => void loadCloudStatus()}
          />
        </label>

        <label className="settings-row" style={{ alignItems: "flex-start", gap: 8 }}>
          <span className="settings-label">Passphrase</span>
          <input
            type="password"
            className="settings-number"
            style={{ width: 220 }}
            value={cloudPassphrase}
            placeholder="Minimum 8 characters"
            onChange={(e) => setCloudPassphrase(e.target.value)}
          />
        </label>

        <label className="settings-row">
          <span className="settings-label">Include AI Sessions</span>
          <input
            type="checkbox"
            className="settings-toggle"
            checked={cloudIncludeSessions}
            onChange={(e) => setCloudIncludeSessions(e.target.checked)}
          />
        </label>

        {cloudStatus && (
          <div style={{ fontSize: 10, color: "var(--text-muted)", marginBottom: 8, lineHeight: 1.5 }}>
            <div>Bundle: {cloudStatus.exists ? cloudStatus.bundle_path : "No encrypted bundle exported yet"}</div>
            {cloudStatus.exists && (
              <div>
                Updated {formatEpoch(cloudStatus.modified_at)} · {cloudStatus.size_bytes ?? 0} bytes
              </div>
            )}
          </div>
        )}

        <div style={{ display: "flex", gap: 6, marginBottom: 10 }}>
          <button className="settings-about-btn" onClick={exportCloudBundle} disabled={cloudBusy}>
            {cloudBusy ? "Working..." : "Export Sync Bundle"}
          </button>
          <button className="settings-about-btn" onClick={importCloudBundle} disabled={cloudBusy}>
            Import & Restore
          </button>
        </div>

        <div style={{
          border: "1px solid var(--border-color)",
          borderRadius: 6,
          padding: 8,
          background: "rgba(255,255,255,0.02)",
        }}>
          <div style={{ fontSize: 11, fontWeight: 700, color: "var(--accent)", marginBottom: 8 }}>
            Snippet Library ({cloudSnippets.length})
          </div>
          <label className="settings-row" style={{ gap: 8, alignItems: "flex-start" }}>
            <span className="settings-label">Title</span>
            <input
              type="text"
              className="settings-number"
              style={{ width: 180 }}
              value={snippetTitle}
              onChange={(e) => setSnippetTitle(e.target.value)}
            />
          </label>
          <label className="settings-row" style={{ gap: 8, alignItems: "flex-start" }}>
            <span className="settings-label">Language</span>
            <input
              type="text"
              className="settings-number"
              style={{ width: 180 }}
              value={snippetLanguage}
              onChange={(e) => setSnippetLanguage(e.target.value)}
            />
          </label>
          <label className="settings-row" style={{ gap: 8, alignItems: "flex-start" }}>
            <span className="settings-label">Tags</span>
            <input
              type="text"
              className="settings-number"
              style={{ width: 180 }}
              value={snippetTags}
              placeholder="shader, util, api"
              onChange={(e) => setSnippetTags(e.target.value)}
            />
          </label>
          <textarea
            className="settings-system-prompt"
            value={snippetContent}
            onChange={(e) => setSnippetContent(e.target.value)}
            placeholder="Snippet content..."
            rows={5}
            style={{ marginTop: 8 }}
          />
          <div style={{ display: "flex", gap: 6, marginTop: 8 }}>
            <button className="settings-about-btn" onClick={saveSnippet} disabled={cloudBusy}>
              {editingSnippetId ? "Update Snippet" : "Save Snippet"}
            </button>
            {editingSnippetId && (
              <button className="settings-about-btn" onClick={resetSnippetEditor} disabled={cloudBusy}>
                Cancel
              </button>
            )}
          </div>

          <div style={{ marginTop: 10, maxHeight: 220, overflowY: "auto", display: "flex", flexDirection: "column", gap: 6 }}>
            {cloudSnippets.length === 0 ? (
              <div style={{ fontSize: 11, color: "var(--text-muted)" }}>
                No synced snippets yet.
              </div>
            ) : (
              cloudSnippets.map((snippet) => (
                <div
                  key={snippet.id}
                  style={{
                    border: "1px solid var(--border-color)",
                    borderRadius: 6,
                    padding: "8px 10px",
                    background: "var(--bg-secondary)",
                  }}
                >
                  <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div style={{ fontSize: 12, fontWeight: 600, color: "var(--text-primary)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                        {snippet.title}
                      </div>
                      <div style={{ fontSize: 10, color: "var(--text-muted)" }}>
                        {snippet.language || "text"} · {snippet.tags.join(", ") || "untagged"} · updated {formatEpoch(snippet.updated_at)}
                      </div>
                    </div>
                    <button
                      className="settings-about-btn"
                      onClick={() => {
                        setEditingSnippetId(snippet.id);
                        setSnippetTitle(snippet.title);
                        setSnippetLanguage(snippet.language || "text");
                        setSnippetTags(snippet.tags.join(", "));
                        setSnippetContent(snippet.content);
                      }}
                    >
                      Edit
                    </button>
                    <button
                      className="settings-about-btn"
                      onClick={async () => {
                        try {
                          await navigator.clipboard.writeText(snippet.content);
                          setCloudMessage(`Copied snippet "${snippet.title}" to clipboard.`);
                        } catch {
                          setCloudMessage(`Unable to copy snippet "${snippet.title}".`);
                        }
                      }}
                    >
                      Copy
                    </button>
                    <button
                      className="settings-about-btn"
                      onClick={async () => {
                        try {
                          setCloudBusy(true);
                          await invoke("cloud_delete_snippet", { id: snippet.id });
                          await loadCloudSnippets();
                          if (editingSnippetId === snippet.id) resetSnippetEditor();
                          setCloudMessage(`Removed snippet "${snippet.title}".`);
                        } catch (e) {
                          setCloudMessage(String(e));
                        } finally {
                          setCloudBusy(false);
                        }
                      }}
                    >
                      Delete
                    </button>
                  </div>
                </div>
              ))
            )}
          </div>
        </div>

        {cloudMessage && (
          <div style={{ fontSize: 10, color: "var(--text-secondary)", marginTop: 8, lineHeight: 1.5 }}>
            {cloudMessage}
          </div>
        )}
      </div>

      {/* Bluetooth removed - moved to BluetoothPanel */}

      {/* Updates */}
      {updateStatus.available && (
        <div className="settings-section">
          <div className="settings-section-title">Update Available</div>
          <div style={{ fontSize: 12, marginBottom: 8 }}>
            Version <strong>{updateStatus.version}</strong> is available.
            {updateStatus.body && (
              <div style={{ color: "var(--text-muted)", marginTop: 4, fontSize: 11 }}>
                {updateStatus.body}
              </div>
            )}
          </div>
          {updateStatus.error && (
            <div style={{ color: "#ef4444", fontSize: 11, marginBottom: 6 }}>{updateStatus.error}</div>
          )}
          <div style={{ display: "flex", gap: 6 }}>
            <button
              className="settings-about-btn"
              onClick={installUpdate}
              disabled={updateStatus.installing}
            >
              {updateStatus.installing ? "Installing..." : "Install & Restart"}
            </button>
            <button
              className="settings-about-btn"
              onClick={dismissUpdate}
              disabled={updateStatus.installing}
            >
              Dismiss
            </button>
          </div>
        </div>
      )}

      {/* About */}
      <div className="settings-section">
        <button
          className="settings-about-btn"
          onClick={() => setShowAbout((v) => !v)}
        >
          {showAbout ? "Hide" : "About ShadowIDE"}
        </button>
        {showAbout && (
          <div className="settings-about">
            <p><strong>ShadowIDE</strong> v0.84.0</p>
            <p>Rust-based IDE with Tauri v2</p>
            <p style={{ color: "var(--text-muted)", marginTop: 8, fontSize: 11 }}>
              React 19 + TypeScript + Monaco Editor
            </p>
          </div>
        )}
      </div>
    </div>
  );
}
