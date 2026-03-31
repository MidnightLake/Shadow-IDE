import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

interface Plugin {
  id: string;
  name: string;
  description: string;
  version: string;
  installedVersion?: string | null;
  author: string;
  type: "language" | "tool" | "panel" | "agent" | "theme";
  runtime: "rust-crate" | "wasm";
  apiVersion: string;
  permissions: string[];
  grantedPermissions: string[];
  missingPermissions: string[];
  allPermissionsGranted: boolean;
  entryPoints: string[];
  installed: boolean;
  enabled: boolean;
  canEnable: boolean;
  updateAvailable: boolean;
  source: string;
  manifestPath?: string | null;
}

interface PluginApiInfo {
  apiVersion: string;
  supportedRuntimes: Array<"rust-crate" | "wasm">;
  permissionModel: string;
  supportedPermissions: string[];
  manifestFields: string[];
  executionIsolation: string;
  hotReload: boolean;
  installationRoot: string;
}

const TYPE_EMOJI: Record<Plugin["type"], string> = {
  language: "🔤",
  tool: "🔧",
  panel: "🖼️",
  agent: "🤖",
  theme: "🎨",
};

const TYPE_COLOR: Record<Plugin["type"], string> = {
  language: "#89b4fa",
  tool: "#fab387",
  panel: "#a6e3a1",
  agent: "#cba6f7",
  theme: "#f9e2af",
};

const RUNTIME_COLOR: Record<Plugin["runtime"], string> = {
  "rust-crate": "#94e2d5",
  wasm: "#fab387",
};

export default function PluginsPanel() {
  const [plugins, setPlugins] = useState<Plugin[]>([]);
  const [apiInfo, setApiInfo] = useState<PluginApiInfo | null>(null);
  const [search, setSearch] = useState("");
  const [loadingIds, setLoadingIds] = useState<Set<string>>(new Set());
  const [isLoading, setIsLoading] = useState(true);
  const [isReloading, setIsReloading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadApiInfo = useCallback(async () => {
    try {
      const info = await invoke<PluginApiInfo>("plugin_api_info");
      setApiInfo(info);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const loadPlugins = useCallback(async (command: "plugin_list" | "plugin_reload" = "plugin_list") => {
    if (command === "plugin_reload") {
      setIsReloading(true);
    } else {
      setIsLoading(true);
    }
    setError(null);

    try {
      const pluginRecords = await invoke<Plugin[]>(command);
      setPlugins(pluginRecords);
    } catch (e) {
      setError(String(e));
    } finally {
      setIsLoading(false);
      setIsReloading(false);
    }
  }, []);

  useEffect(() => {
    void Promise.all([loadApiInfo(), loadPlugins()]);
  }, [loadApiInfo, loadPlugins]);

  const withLoading = async (id: string, fn: () => Promise<void>) => {
    setLoadingIds((prev) => new Set(prev).add(id));
    setError(null);
    try {
      await fn();
      await loadPlugins();
    } catch (e) {
      setError(String(e));
    } finally {
      setLoadingIds((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    }
  };

  const handleInstall = (plugin: Plugin) => {
    void withLoading(plugin.id, async () => {
      await invoke("plugin_install", { pluginId: plugin.id });
    });
  };

  const handleUpdate = (plugin: Plugin) => {
    void withLoading(plugin.id, async () => {
      await invoke("plugin_update", { pluginId: plugin.id });
    });
  };

  const handleGrantAll = (plugin: Plugin) => {
    void withLoading(plugin.id, async () => {
      await invoke("plugin_grant_permissions", {
        pluginId: plugin.id,
        permissions: plugin.missingPermissions,
      });
    });
  };

  const handleRevokePermissions = (plugin: Plugin) => {
    void withLoading(plugin.id, async () => {
      await invoke("plugin_revoke_permissions", {
        pluginId: plugin.id,
        permissions: plugin.grantedPermissions,
      });
    });
  };

  const handleEnable = (plugin: Plugin) => {
    void withLoading(plugin.id, async () => {
      await invoke("plugin_enable", { pluginId: plugin.id });
    });
  };

  const handleDisable = (plugin: Plugin) => {
    void withLoading(plugin.id, async () => {
      await invoke("plugin_disable", { pluginId: plugin.id });
    });
  };

  const handleUninstall = (plugin: Plugin) => {
    void withLoading(plugin.id, async () => {
      await invoke("plugin_uninstall", { pluginId: plugin.id });
    });
  };

  const filtered = plugins.filter((plugin) => {
    const query = search.toLowerCase();
    return (
      plugin.name.toLowerCase().includes(query) ||
      plugin.description.toLowerCase().includes(query) ||
      plugin.type.toLowerCase().includes(query) ||
      plugin.runtime.toLowerCase().includes(query) ||
      plugin.source.toLowerCase().includes(query) ||
      plugin.permissions.some((permission) => permission.toLowerCase().includes(query)) ||
      plugin.entryPoints.some((entryPoint) => entryPoint.toLowerCase().includes(query))
    );
  });

  const installed = filtered.filter((plugin) => plugin.installed);
  const available = filtered.filter((plugin) => !plugin.installed);
  const updates = installed.filter((plugin) => plugin.updateAvailable).length;
  const blocked = installed.filter((plugin) => !plugin.allPermissionsGranted).length;

  return (
    <div style={{ height: "100%", overflowY: "auto", background: "var(--bg-secondary)", color: "var(--text-primary)", fontFamily: "monospace", fontSize: 12 }}>
      <div style={{ padding: "10px 12px", borderBottom: "1px solid var(--border-color)", position: "sticky", top: 0, background: "var(--bg-secondary)", zIndex: 5 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 8 }}>
          <div style={{ fontWeight: "bold", color: "var(--accent-color)", flex: 1 }}>Extensions / Plugins</div>
          <button
            onClick={() => { void loadPlugins("plugin_reload"); }}
            style={{
              background: "transparent",
              border: "1px solid var(--border-color)",
              borderRadius: 4,
              color: "var(--accent-color)",
              padding: "2px 8px",
              fontSize: 10,
              cursor: "pointer",
            }}
          >
            {isReloading ? "Reloading..." : "Reload"}
          </button>
        </div>

        <div style={{ display: "flex", gap: 6, marginBottom: 8, flexWrap: "wrap" }}>
          <MetaBadge label={`${installed.length} installed`} color="var(--accent-color)" />
          <MetaBadge label={`${available.length} available`} color="var(--text-secondary)" />
          <MetaBadge label={`${updates} updates`} color={updates > 0 ? "#f9e2af" : "var(--text-muted)"} />
          <MetaBadge label={`${blocked} awaiting grants`} color={blocked > 0 ? "#f38ba8" : "var(--text-muted)"} />
        </div>

        {apiInfo && (
          <div style={{ marginBottom: 8, padding: "8px 10px", borderRadius: 6, background: "var(--bg-primary)", border: "1px solid var(--border-color)" }}>
            <div style={{ display: "flex", gap: 6, flexWrap: "wrap", marginBottom: 6 }}>
              <MetaBadge label={`API v${apiInfo.apiVersion}`} color="#94e2d5" />
              <MetaBadge label={apiInfo.permissionModel} color="#cba6f7" />
              <MetaBadge label={apiInfo.hotReload ? "hot reload" : "manual reload"} color="#a6e3a1" />
            </div>
            <div style={{ color: "var(--text-secondary)", fontSize: 10, lineHeight: 1.5 }}>
              Runtimes: {apiInfo.supportedRuntimes.join(" • ")}<br />
              Isolation: {apiInfo.executionIsolation}<br />
              Install root: {apiInfo.installationRoot}
            </div>
          </div>
        )}

        <input
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search extensions, runtimes, permissions, entry points..."
          style={{
            width: "100%",
            background: "var(--bg-primary)",
            border: "1px solid var(--border-color)",
            borderRadius: 4,
            color: "var(--text-primary)",
            padding: "6px 8px",
            fontSize: 12,
            outline: "none",
            boxSizing: "border-box",
          }}
        />
      </div>

      {error && (
        <div style={{ padding: "6px 12px", background: "#3b0a0a", color: "#f38ba8", fontSize: 11 }}>
          {error}
        </div>
      )}

      <div style={{ padding: "8px 12px" }}>
        {isLoading && plugins.length === 0 && (
          <>
            <PluginSkeleton />
            <PluginSkeleton />
            <PluginSkeleton />
          </>
        )}

        {installed.length > 0 && (
          <>
            <div style={{ color: "var(--text-muted)", fontSize: 10, fontWeight: "bold", marginBottom: 6, textTransform: "uppercase", letterSpacing: "0.05em" }}>
              Installed ({installed.length})
            </div>
            {installed.map((plugin) => (
              <PluginCard
                key={plugin.id}
                plugin={plugin}
                loading={loadingIds.has(plugin.id)}
                onInstall={() => handleInstall(plugin)}
                onUpdate={() => handleUpdate(plugin)}
                onGrantAll={() => handleGrantAll(plugin)}
                onRevokePermissions={() => handleRevokePermissions(plugin)}
                onEnable={() => handleEnable(plugin)}
                onDisable={() => handleDisable(plugin)}
                onUninstall={() => handleUninstall(plugin)}
              />
            ))}
          </>
        )}

        {available.length > 0 && (
          <>
            <div style={{ color: "var(--text-muted)", fontSize: 10, fontWeight: "bold", marginTop: installed.length > 0 ? 12 : 0, marginBottom: 6, textTransform: "uppercase", letterSpacing: "0.05em" }}>
              Available ({available.length})
            </div>
            {available.map((plugin) => (
              <PluginCard
                key={plugin.id}
                plugin={plugin}
                loading={loadingIds.has(plugin.id)}
                onInstall={() => handleInstall(plugin)}
                onUpdate={() => handleUpdate(plugin)}
                onGrantAll={() => handleGrantAll(plugin)}
                onRevokePermissions={() => handleRevokePermissions(plugin)}
                onEnable={() => handleEnable(plugin)}
                onDisable={() => handleDisable(plugin)}
                onUninstall={() => handleUninstall(plugin)}
              />
            ))}
          </>
        )}

        {!isLoading && filtered.length === 0 && (
          <div style={{ color: "var(--text-muted)", padding: 12 }}>No extensions found.</div>
        )}
      </div>
    </div>
  );
}

interface PluginCardProps {
  plugin: Plugin;
  loading: boolean;
  onInstall: () => void;
  onUpdate: () => void;
  onGrantAll: () => void;
  onRevokePermissions: () => void;
  onEnable: () => void;
  onDisable: () => void;
  onUninstall: () => void;
}

function PluginCard({
  plugin,
  loading,
  onInstall,
  onUpdate,
  onGrantAll,
  onRevokePermissions,
  onEnable,
  onDisable,
  onUninstall,
}: PluginCardProps) {
  return (
    <div style={{ background: "var(--bg-primary)", borderRadius: 5, marginBottom: 6, padding: "8px 10px", border: "1px solid var(--border-color)" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 4, flexWrap: "wrap" }}>
        <span style={{ fontSize: 16, flexShrink: 0 }}>{TYPE_EMOJI[plugin.type]}</span>
        <span style={{ fontWeight: "bold", color: "var(--text-primary)", flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", minWidth: 160 }}>{plugin.name}</span>
        <MetaBadge label={plugin.type} color={TYPE_COLOR[plugin.type]} />
        <MetaBadge label={plugin.runtime} color={RUNTIME_COLOR[plugin.runtime]} />
        <MetaBadge label={plugin.source} color="var(--text-secondary)" />
        {plugin.updateAvailable && <MetaBadge label="update" color="#f9e2af" />}
        {!plugin.allPermissionsGranted && plugin.installed && <MetaBadge label="blocked" color="#f38ba8" />}
      </div>

      <div style={{ color: "var(--text-secondary)", fontSize: 11, marginBottom: 6, lineHeight: 1.4 }}>
        {plugin.description}
      </div>

      <div style={{ display: "flex", flexWrap: "wrap", gap: 6, marginBottom: 6 }}>
        <MetaBadge label={`api v${plugin.apiVersion}`} color="#94e2d5" />
        <MetaBadge label={`latest ${plugin.version}`} color="var(--accent-color)" />
        {plugin.installedVersion && <MetaBadge label={`installed ${plugin.installedVersion}`} color="#a6e3a1" />}
      </div>

      {plugin.permissions.length > 0 && (
        <div style={{ marginBottom: 6 }}>
          <div style={{ color: "var(--text-muted)", fontSize: 10, marginBottom: 4 }}>Permissions</div>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>
            {plugin.permissions.map((permission) => {
              const granted = plugin.grantedPermissions.includes(permission);
              return (
                <MetaBadge
                  key={permission}
                  label={granted ? `${permission} ✓` : permission}
                  color={granted ? "#a6e3a1" : "#cba6f7"}
                />
              );
            })}
          </div>
        </div>
      )}

      {plugin.installed && plugin.missingPermissions.length > 0 && (
        <div style={{ marginBottom: 6, padding: "6px 8px", borderRadius: 4, background: "#3b0a0a", color: "#f9e2af", fontSize: 10, lineHeight: 1.5 }}>
          Enable is blocked until permissions are granted: {plugin.missingPermissions.join(", ")}
        </div>
      )}

      {plugin.entryPoints.length > 0 && (
        <div style={{ marginBottom: 6 }}>
          <div style={{ color: "var(--text-muted)", fontSize: 10, marginBottom: 4 }}>Entry Points</div>
          <div style={{ color: "var(--text-secondary)", fontSize: 10, lineHeight: 1.5 }}>
            {plugin.entryPoints.join(" • ")}
          </div>
        </div>
      )}

      {plugin.manifestPath && (
        <div style={{ color: "var(--text-muted)", fontSize: 10, marginBottom: 6 }}>
          Manifest: {plugin.manifestPath}
        </div>
      )}

      <div style={{ display: "flex", alignItems: "center", gap: 4, flexWrap: "wrap" }}>
        <span style={{ color: "var(--text-muted)", fontSize: 10 }}>{plugin.author}</span>
        <span style={{ flex: 1 }} />

        {loading ? (
          <span style={{ color: "var(--text-muted)", fontSize: 10 }}>...</span>
        ) : plugin.installed ? (
          <>
            {plugin.updateAvailable && (
              <button
                onClick={onUpdate}
                style={{ background: "#f9e2af", color: "#11111b", border: "none", borderRadius: 4, padding: "2px 8px", fontSize: 10, fontWeight: 700, cursor: "pointer" }}
              >
                Update
              </button>
            )}
            {plugin.missingPermissions.length > 0 && (
              <button
                onClick={onGrantAll}
                style={{ background: "var(--accent-hover)", color: "var(--bg-primary)", border: "none", borderRadius: 4, padding: "2px 8px", fontSize: 10, fontWeight: 700, cursor: "pointer" }}
              >
                Grant All
              </button>
            )}
            {plugin.grantedPermissions.length > 0 && (
              <button
                onClick={onRevokePermissions}
                style={{ background: "transparent", border: "1px solid var(--border-color)", borderRadius: 4, color: "#f38ba8", padding: "2px 8px", fontSize: 10, cursor: "pointer" }}
              >
                Revoke
              </button>
            )}
            {plugin.enabled ? (
              <button
                onClick={onDisable}
                style={{ background: "transparent", border: "1px solid var(--border-color)", borderRadius: 4, color: "#f9e2af", padding: "2px 8px", fontSize: 10, cursor: "pointer" }}
              >
                Disable
              </button>
            ) : (
              <button
                onClick={onEnable}
                disabled={!plugin.canEnable}
                style={{
                  background: "transparent",
                  border: "1px solid var(--border-color)",
                  borderRadius: 4,
                  color: plugin.canEnable ? "#a6e3a1" : "var(--text-muted)",
                  padding: "2px 8px",
                  fontSize: 10,
                  cursor: plugin.canEnable ? "pointer" : "not-allowed",
                  opacity: plugin.canEnable ? 1 : 0.6,
                }}
              >
                Enable
              </button>
            )}
            <button
              onClick={onUninstall}
              style={{ background: "transparent", border: "1px solid var(--border-color)", borderRadius: 4, color: "#f38ba8", padding: "2px 8px", fontSize: 10, cursor: "pointer" }}
            >
              Uninstall
            </button>
          </>
        ) : (
          <button
            onClick={onInstall}
            style={{ background: "var(--accent-hover)", color: "var(--bg-primary)", border: "none", borderRadius: 4, padding: "2px 10px", fontSize: 10, fontWeight: 700, cursor: "pointer" }}
          >
            Install
          </button>
        )}
      </div>
    </div>
  );
}

function MetaBadge({ label, color }: { label: string; color: string }) {
  return (
    <span style={{ background: `color-mix(in srgb, ${color} 18%, transparent)`, color, borderRadius: 999, padding: "2px 7px", fontSize: 10 }}>
      {label}
    </span>
  );
}

function PluginSkeleton() {
  return (
    <div style={{ background: "var(--bg-primary)", borderRadius: 5, marginBottom: 6, padding: "10px", border: "1px solid var(--border-color)" }}>
      <div className="skeleton skeleton-text wide" style={{ height: 14, marginBottom: 8 }} />
      <div className="skeleton skeleton-text medium" style={{ height: 10, marginBottom: 6 }} />
      <div className="skeleton skeleton-text short" style={{ height: 10, marginBottom: 10 }} />
      <div style={{ display: "flex", gap: 6 }}>
        <div className="skeleton" style={{ width: 72, height: 22, borderRadius: 999 }} />
        <div className="skeleton" style={{ width: 96, height: 22, borderRadius: 999 }} />
      </div>
    </div>
  );
}
