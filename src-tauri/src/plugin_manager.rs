use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::Emitter;

const PLUGIN_STATE_FILE: &str = "plugins.json";
const PLUGINS_DIR: &str = "plugins";
const SUPPORTED_PLUGIN_PERMISSIONS: &[&str] = &[
    "workspace:read",
    "workspace:write",
    "workspace:watch",
    "lsp:spawn",
    "terminal:cargo",
    "terminal:go",
    "terminal:npm",
    "python:venv",
    "shell:docker",
    "terminal:docker",
    "analytics:project",
    "theme:install",
    "ai:chat",
    "diagnostics:write",
    "network:http",
    "ui:panel",
];

pub struct PluginManagerState {
    data_dir: PathBuf,
    plugin_states: Mutex<HashMap<String, PluginState>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    Language,
    Tool,
    Panel,
    Agent,
    Theme,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum PluginRuntime {
    RustCrate,
    Wasm,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub api_version: String,
    pub author: String,
    pub description: String,
    #[serde(rename = "type")]
    pub plugin_type: PluginKind,
    #[serde(default = "default_plugin_runtime")]
    pub runtime: PluginRuntime,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub entry_points: Vec<String>,
    #[serde(default = "default_plugin_source")]
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginState {
    pub installed: bool,
    pub enabled: bool,
    #[serde(default)]
    pub installed_version: Option<String>,
    #[serde(default)]
    pub granted_permissions: Vec<String>,
    #[serde(default)]
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginRecord {
    pub id: String,
    pub name: String,
    pub description: String,
    pub version: String,
    pub installed_version: Option<String>,
    pub author: String,
    #[serde(rename = "type")]
    pub plugin_type: PluginKind,
    pub runtime: PluginRuntime,
    pub api_version: String,
    pub permissions: Vec<String>,
    pub granted_permissions: Vec<String>,
    pub missing_permissions: Vec<String>,
    pub all_permissions_granted: bool,
    pub entry_points: Vec<String>,
    pub installed: bool,
    pub enabled: bool,
    pub can_enable: bool,
    pub update_available: bool,
    pub source: String,
    pub manifest_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginApiInfo {
    pub api_version: String,
    pub supported_runtimes: Vec<PluginRuntime>,
    pub permission_model: String,
    pub supported_permissions: Vec<String>,
    pub manifest_fields: Vec<String>,
    pub execution_isolation: String,
    pub hot_reload: bool,
    pub installation_root: String,
}

#[derive(Debug, Clone)]
struct InstalledManifest {
    manifest: PluginManifest,
    path: PathBuf,
}

impl PluginManagerState {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            plugin_states: Mutex::new(load_plugin_states_from_disk(&data_dir)),
            data_dir,
        }
    }

    fn state_path(&self) -> PathBuf {
        self.data_dir.join(PLUGIN_STATE_FILE)
    }

    fn plugins_dir(&self) -> PathBuf {
        self.data_dir.join(PLUGINS_DIR)
    }

    fn persist_plugin_states(&self, states: &HashMap<String, PluginState>) -> Result<(), String> {
        let path = self.state_path();
        std::fs::create_dir_all(&self.data_dir)
            .map_err(|e| format!("Failed to create plugin data directory: {}", e))?;
        let json = serde_json::to_string_pretty(states)
            .map_err(|e| format!("Failed to serialize plugin state: {}", e))?;
        std::fs::write(&path, json)
            .map_err(|e| format!("Failed to write plugin state to {}: {}", path.display(), e))?;
        Ok(())
    }

    fn load_installed_manifests(&self) -> HashMap<String, InstalledManifest> {
        let mut manifests = HashMap::new();
        let entries = match std::fs::read_dir(self.plugins_dir()) {
            Ok(entries) => entries,
            Err(_) => return manifests,
        };

        for entry in entries.flatten() {
            let plugin_dir = entry.path();
            if !plugin_dir.is_dir() {
                continue;
            }

            let manifest_path = plugin_dir.join("plugin.toml");
            let content = match std::fs::read_to_string(&manifest_path) {
                Ok(content) => content,
                Err(_) => continue,
            };

            let manifest = match toml::from_str::<PluginManifest>(&content) {
                Ok(manifest) => manifest,
                Err(err) => {
                    log::warn!(
                        "Skipping invalid plugin manifest at {}: {}",
                        manifest_path.display(),
                        err
                    );
                    continue;
                }
            };

            if let Err(err) = validate_manifest(&manifest) {
                log::warn!(
                    "Skipping unsupported plugin manifest at {}: {}",
                    manifest_path.display(),
                    err
                );
                continue;
            }

            manifests.insert(
                manifest.id.clone(),
                InstalledManifest {
                    manifest,
                    path: manifest_path,
                },
            );
        }

        manifests
    }

    fn registry_snapshot(&self) -> Result<Vec<PluginRecord>, String> {
        let catalog = builtin_catalog();
        let installed_manifests = self.load_installed_manifests();
        let states = self
            .plugin_states
            .lock()
            .map_err(|_| "Plugin manager lock poisoned".to_string())?;

        let mut ids = BTreeSet::new();
        ids.extend(catalog.keys().cloned());
        ids.extend(installed_manifests.keys().cloned());
        ids.extend(states.keys().cloned());

        let mut plugins = ids
            .into_iter()
            .filter_map(|id| {
                let available_manifest = catalog.get(&id).cloned().or_else(|| {
                    installed_manifests
                        .get(&id)
                        .map(|item| item.manifest.clone())
                })?;
                let installed_manifest = installed_manifests.get(&id);
                let state = states
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| default_plugin_state(&available_manifest));
                let granted_permissions = sanitize_granted_permissions(
                    state.granted_permissions.clone(),
                    &available_manifest,
                );
                let missing_permissions =
                    missing_permissions_for(&available_manifest.permissions, &granted_permissions);
                let all_permissions_granted = missing_permissions.is_empty();
                let installed = installed_manifest.is_some() || state.installed;
                let installed_version = if installed {
                    installed_manifest
                        .map(|item| item.manifest.version.clone())
                        .or(state.installed_version.clone())
                        .or_else(|| Some(available_manifest.version.clone()))
                } else {
                    None
                };
                let update_available = installed
                    && installed_version
                        .as_ref()
                        .map(|version| version_is_newer(&available_manifest.version, version))
                        .unwrap_or(false);
                let can_enable = installed && all_permissions_granted;
                let enabled = installed && state.enabled && all_permissions_granted;

                Some(PluginRecord {
                    id: available_manifest.id.clone(),
                    name: available_manifest.name.clone(),
                    description: available_manifest.description.clone(),
                    version: available_manifest.version.clone(),
                    installed_version,
                    author: available_manifest.author.clone(),
                    plugin_type: available_manifest.plugin_type.clone(),
                    runtime: available_manifest.runtime.clone(),
                    api_version: available_manifest.api_version.clone(),
                    permissions: available_manifest.permissions.clone(),
                    granted_permissions,
                    missing_permissions,
                    all_permissions_granted,
                    entry_points: available_manifest.entry_points.clone(),
                    installed,
                    enabled,
                    can_enable,
                    update_available,
                    source: available_manifest.source.clone(),
                    manifest_path: installed_manifest
                        .map(|item| item.path.to_string_lossy().to_string()),
                })
            })
            .collect::<Vec<_>>();

        plugins.sort_by(|a, b| match (a.installed, b.installed) {
            (true, false) => Ordering::Less,
            (false, true) => Ordering::Greater,
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        });

        Ok(plugins)
    }

    fn lookup_manifest(&self, plugin_id: &str) -> Result<PluginManifest, String> {
        validate_plugin_id(plugin_id)?;

        let catalog = builtin_catalog();
        if let Some(manifest) = catalog.get(plugin_id).cloned() {
            return Ok(manifest);
        }

        let installed_manifests = self.load_installed_manifests();
        if let Some(installed_manifest) = installed_manifests.get(plugin_id) {
            return Ok(installed_manifest.manifest.clone());
        }

        Err(format!(
            "Plugin {} was not found in the registry",
            plugin_id
        ))
    }

    fn plugin_record(&self, plugin_id: &str) -> Result<PluginRecord, String> {
        self.registry_snapshot()?
            .into_iter()
            .find(|plugin| plugin.id == plugin_id)
            .ok_or_else(|| format!("Plugin {} was not found in the registry", plugin_id))
    }

    fn write_manifest(&self, manifest: &PluginManifest) -> Result<PathBuf, String> {
        validate_manifest(manifest)?;

        let dir = self.plugins_dir().join(&manifest.id);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("Failed to create plugin directory {}: {}", dir.display(), e))?;
        let manifest_path = dir.join("plugin.toml");
        let body = toml::to_string_pretty(manifest)
            .map_err(|e| format!("Failed to serialize plugin manifest: {}", e))?;
        std::fs::write(&manifest_path, body)
            .map_err(|e| format!("Failed to write {}: {}", manifest_path.display(), e))?;
        Ok(manifest_path)
    }

    fn remove_plugin_dir(&self, plugin_id: &str) -> Result<(), String> {
        validate_plugin_id(plugin_id)?;

        let dir = self.plugins_dir().join(plugin_id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| {
                format!("Failed to remove plugin directory {}: {}", dir.display(), e)
            })?;
        }
        Ok(())
    }
}

fn default_plugin_runtime() -> PluginRuntime {
    PluginRuntime::RustCrate
}

fn default_plugin_source() -> String {
    "community".to_string()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn validate_plugin_id(plugin_id: &str) -> Result<(), String> {
    if plugin_id.is_empty() {
        return Err("Plugin id cannot be empty".to_string());
    }

    if plugin_id
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        Ok(())
    } else {
        Err(format!(
            "Plugin id {} contains unsupported characters",
            plugin_id
        ))
    }
}

fn supported_permissions() -> Vec<String> {
    SUPPORTED_PLUGIN_PERMISSIONS
        .iter()
        .map(|permission| (*permission).to_string())
        .collect()
}

fn validate_manifest(manifest: &PluginManifest) -> Result<(), String> {
    validate_plugin_id(&manifest.id)?;
    let unsupported = manifest
        .permissions
        .iter()
        .filter(|permission| !SUPPORTED_PLUGIN_PERMISSIONS.contains(&permission.as_str()))
        .cloned()
        .collect::<Vec<_>>();

    if unsupported.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Plugin {} requests unsupported permissions: {}",
            manifest.id,
            unsupported.join(", ")
        ))
    }
}

fn sorted_unique_strings(list: &[String]) -> Vec<String> {
    list.iter()
        .filter(|value| !value.is_empty())
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn sanitize_granted_permissions(
    granted_permissions: Vec<String>,
    manifest: &PluginManifest,
) -> Vec<String> {
    sorted_unique_strings(&granted_permissions)
        .into_iter()
        .filter(|permission| {
            SUPPORTED_PLUGIN_PERMISSIONS.contains(&permission.as_str())
                && manifest.permissions.contains(permission)
        })
        .collect()
}

fn missing_permissions_for(
    requested_permissions: &[String],
    granted_permissions: &[String],
) -> Vec<String> {
    requested_permissions
        .iter()
        .filter(|permission| !granted_permissions.contains(permission))
        .cloned()
        .collect()
}

fn sanitize_requested_permissions(
    manifest: &PluginManifest,
    permissions: Vec<String>,
) -> Result<Vec<String>, String> {
    let permissions = sorted_unique_strings(&permissions);
    if permissions.is_empty() {
        return Err(format!(
            "Plugin {} did not receive any permissions to update",
            manifest.id
        ));
    }

    let unsupported = permissions
        .iter()
        .filter(|permission| !SUPPORTED_PLUGIN_PERMISSIONS.contains(&permission.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unsupported.is_empty() {
        return Err(format!(
            "Unsupported permissions requested: {}",
            unsupported.join(", ")
        ));
    }

    let unrequested = permissions
        .iter()
        .filter(|permission| !manifest.permissions.contains(permission))
        .cloned()
        .collect::<Vec<_>>();
    if !unrequested.is_empty() {
        return Err(format!(
            "Plugin {} does not request permissions: {}",
            manifest.id,
            unrequested.join(", ")
        ));
    }

    Ok(permissions)
}

fn is_core_default_plugin(plugin_id: &str) -> bool {
    matches!(plugin_id, "rust-lang" | "python-lang")
}

fn default_plugin_state(manifest: &PluginManifest) -> PluginState {
    if is_core_default_plugin(&manifest.id) {
        PluginState {
            installed: true,
            enabled: true,
            installed_version: Some(manifest.version.clone()),
            granted_permissions: manifest.permissions.clone(),
            updated_at: 0,
        }
    } else {
        PluginState::default()
    }
}

fn default_plugin_states() -> HashMap<String, PluginState> {
    builtin_catalog()
        .into_values()
        .filter(|manifest| is_core_default_plugin(&manifest.id))
        .map(|manifest| (manifest.id.clone(), default_plugin_state(&manifest)))
        .collect()
}

fn load_plugin_states_from_disk(data_dir: &Path) -> HashMap<String, PluginState> {
    let path = data_dir.join(PLUGIN_STATE_FILE);
    let mut states = default_plugin_states();

    if let Ok(content) = std::fs::read_to_string(&path) {
        if let Ok(saved) = serde_json::from_str::<HashMap<String, PluginState>>(&content) {
            states.extend(saved);
        }
    }

    states
}

fn version_is_newer(latest: &str, installed: &str) -> bool {
    compare_versions(latest, installed) == Ordering::Greater
}

fn compare_versions(left: &str, right: &str) -> Ordering {
    let parse = |value: &str| {
        value
            .split('.')
            .map(|part| part.parse::<u64>().unwrap_or(0))
            .collect::<Vec<_>>()
    };

    let left_parts = parse(left);
    let right_parts = parse(right);
    let max_len = left_parts.len().max(right_parts.len());

    for index in 0..max_len {
        let left_value = *left_parts.get(index).unwrap_or(&0);
        let right_value = *right_parts.get(index).unwrap_or(&0);
        match left_value.cmp(&right_value) {
            Ordering::Equal => continue,
            ordering => return ordering,
        }
    }

    Ordering::Equal
}

fn builtin_catalog() -> HashMap<String, PluginManifest> {
    [
        PluginManifest {
            id: "rust-lang".to_string(),
            name: "Rust Language Support".to_string(),
            version: "1.4.0".to_string(),
            api_version: "1".to_string(),
            author: "Shadow IDE".to_string(),
            description:
                "Syntax highlighting, rust-analyzer wiring, cargo tasks, and Rust snippets."
                    .to_string(),
            plugin_type: PluginKind::Language,
            runtime: PluginRuntime::RustCrate,
            permissions: vec![
                "workspace:read".to_string(),
                "workspace:write".to_string(),
                "lsp:spawn".to_string(),
                "terminal:cargo".to_string(),
            ],
            entry_points: vec![
                "frontend:languages/rust".to_string(),
                "backend:lsp/rust-analyzer".to_string(),
            ],
            source: "core".to_string(),
        },
        PluginManifest {
            id: "python-lang".to_string(),
            name: "Python Language Support".to_string(),
            version: "2.1.0".to_string(),
            api_version: "1".to_string(),
            author: "Shadow IDE".to_string(),
            description:
                "Python syntax, linting, formatting, notebook helpers, and environment tooling."
                    .to_string(),
            plugin_type: PluginKind::Language,
            runtime: PluginRuntime::RustCrate,
            permissions: vec![
                "workspace:read".to_string(),
                "workspace:write".to_string(),
                "lsp:spawn".to_string(),
                "python:venv".to_string(),
            ],
            entry_points: vec![
                "frontend:languages/python".to_string(),
                "backend:lsp/pyright".to_string(),
            ],
            source: "core".to_string(),
        },
        PluginManifest {
            id: "go-lang".to_string(),
            name: "Go Language Support".to_string(),
            version: "1.2.0".to_string(),
            api_version: "1".to_string(),
            author: "Shadow IDE".to_string(),
            description: "Go syntax, gopls integration, gofmt hooks, and module helpers."
                .to_string(),
            plugin_type: PluginKind::Language,
            runtime: PluginRuntime::RustCrate,
            permissions: vec![
                "workspace:read".to_string(),
                "workspace:write".to_string(),
                "lsp:spawn".to_string(),
                "terminal:go".to_string(),
            ],
            entry_points: vec![
                "frontend:languages/go".to_string(),
                "backend:lsp/gopls".to_string(),
            ],
            source: "core".to_string(),
        },
        PluginManifest {
            id: "docker-tools".to_string(),
            name: "Docker Tools".to_string(),
            version: "0.9.1".to_string(),
            api_version: "1".to_string(),
            author: "Shadow IDE".to_string(),
            description:
                "Dockerfile editing, compose support, image inspection, and container tasks."
                    .to_string(),
            plugin_type: PluginKind::Tool,
            runtime: PluginRuntime::RustCrate,
            permissions: vec![
                "workspace:read".to_string(),
                "shell:docker".to_string(),
                "terminal:docker".to_string(),
            ],
            entry_points: vec![
                "frontend:tools/docker".to_string(),
                "backend:tooling/docker".to_string(),
            ],
            source: "core".to_string(),
        },
        PluginManifest {
            id: "workspace-insights".to_string(),
            name: "Workspace Insights".to_string(),
            version: "0.8.0".to_string(),
            api_version: "1".to_string(),
            author: "Shadow IDE Labs".to_string(),
            description:
                "Adds a project health panel with dependency drift, churn, and ownership views."
                    .to_string(),
            plugin_type: PluginKind::Panel,
            runtime: PluginRuntime::Wasm,
            permissions: vec![
                "workspace:read".to_string(),
                "analytics:project".to_string(),
                "ui:panel".to_string(),
            ],
            entry_points: vec!["frontend:panels/workspace-insights".to_string()],
            source: "labs".to_string(),
        },
        PluginManifest {
            id: "dracula-ext".to_string(),
            name: "Dracula Theme Extended".to_string(),
            version: "3.0.0".to_string(),
            api_version: "1".to_string(),
            author: "Community".to_string(),
            description: "Extended Dracula color theme with additional file type colorization."
                .to_string(),
            plugin_type: PluginKind::Theme,
            runtime: PluginRuntime::Wasm,
            permissions: vec!["theme:install".to_string()],
            entry_points: vec!["frontend:themes/dracula-extended".to_string()],
            source: "community".to_string(),
        },
        PluginManifest {
            id: "ai-code-review".to_string(),
            name: "AI Code Review".to_string(),
            version: "0.5.0".to_string(),
            api_version: "1".to_string(),
            author: "Shadow IDE".to_string(),
            description:
                "Automated AI review with inline comments, summaries, and fix suggestions."
                    .to_string(),
            plugin_type: PluginKind::Agent,
            runtime: PluginRuntime::RustCrate,
            permissions: vec![
                "workspace:read".to_string(),
                "ai:chat".to_string(),
                "diagnostics:write".to_string(),
            ],
            entry_points: vec![
                "frontend:agents/code-review".to_string(),
                "backend:agents/code-review".to_string(),
            ],
            source: "core".to_string(),
        },
    ]
    .into_iter()
    .map(|manifest| (manifest.id.clone(), manifest))
    .collect()
}

fn emit_registry_update(app: &tauri::AppHandle, plugins: &[PluginRecord]) {
    let _ = app.emit("plugin-registry-updated", plugins);
}

#[tauri::command]
pub fn plugin_api_info(state: tauri::State<'_, PluginManagerState>) -> PluginApiInfo {
    PluginApiInfo {
        api_version: "1".to_string(),
        supported_runtimes: vec![PluginRuntime::RustCrate, PluginRuntime::Wasm],
        permission_model: "explicit-grant".to_string(),
        supported_permissions: supported_permissions(),
        manifest_fields: vec![
            "id".to_string(),
            "name".to_string(),
            "version".to_string(),
            "api_version".to_string(),
            "author".to_string(),
            "description".to_string(),
            "type".to_string(),
            "runtime".to_string(),
            "permissions".to_string(),
            "entry_points".to_string(),
            "source".to_string(),
        ],
        execution_isolation: "manifest-validated runtime with explicit permission grants"
            .to_string(),
        hot_reload: true,
        installation_root: state.plugins_dir().to_string_lossy().to_string(),
    }
}

#[tauri::command]
pub fn plugin_list(
    state: tauri::State<'_, PluginManagerState>,
) -> Result<Vec<PluginRecord>, String> {
    state.registry_snapshot()
}

#[tauri::command]
pub fn plugin_reload(
    state: tauri::State<'_, PluginManagerState>,
    app: tauri::AppHandle,
) -> Result<Vec<PluginRecord>, String> {
    let plugins = state.registry_snapshot()?;
    emit_registry_update(&app, &plugins);
    Ok(plugins)
}

#[tauri::command]
pub fn plugin_install(
    plugin_id: String,
    state: tauri::State<'_, PluginManagerState>,
    app: tauri::AppHandle,
) -> Result<PluginRecord, String> {
    let manifest = state.lookup_manifest(&plugin_id)?;
    state.write_manifest(&manifest)?;

    {
        let mut plugin_states = state
            .plugin_states
            .lock()
            .map_err(|_| "Plugin manager lock poisoned".to_string())?;
        let entry = plugin_states.entry(plugin_id.clone()).or_default();
        entry.installed = true;
        entry.installed_version = Some(manifest.version.clone());
        entry.granted_permissions =
            sanitize_granted_permissions(entry.granted_permissions.clone(), &manifest);
        let missing_permissions =
            missing_permissions_for(&manifest.permissions, &entry.granted_permissions);
        entry.enabled = missing_permissions.is_empty();
        entry.updated_at = now_secs();
        state.persist_plugin_states(&plugin_states)?;
    }

    let plugin = state.plugin_record(&plugin_id)?;
    emit_registry_update(&app, &state.registry_snapshot()?);
    Ok(plugin)
}

#[tauri::command]
pub fn plugin_update(
    plugin_id: String,
    state: tauri::State<'_, PluginManagerState>,
    app: tauri::AppHandle,
) -> Result<PluginRecord, String> {
    let manifest = state.lookup_manifest(&plugin_id)?;

    {
        let plugin_states = state
            .plugin_states
            .lock()
            .map_err(|_| "Plugin manager lock poisoned".to_string())?;
        let installed = plugin_states
            .get(&plugin_id)
            .map(|plugin| plugin.installed)
            .unwrap_or(false);
        if !installed {
            return Err(format!("Plugin {} is not installed", plugin_id));
        }
    }

    state.write_manifest(&manifest)?;

    {
        let mut plugin_states = state
            .plugin_states
            .lock()
            .map_err(|_| "Plugin manager lock poisoned".to_string())?;
        let entry = plugin_states.entry(plugin_id.clone()).or_default();
        entry.installed = true;
        entry.installed_version = Some(manifest.version.clone());
        entry.granted_permissions =
            sanitize_granted_permissions(entry.granted_permissions.clone(), &manifest);
        if !missing_permissions_for(&manifest.permissions, &entry.granted_permissions).is_empty() {
            entry.enabled = false;
        }
        entry.updated_at = now_secs();
        state.persist_plugin_states(&plugin_states)?;
    }

    let plugin = state.plugin_record(&plugin_id)?;
    emit_registry_update(&app, &state.registry_snapshot()?);
    Ok(plugin)
}

#[tauri::command]
pub fn plugin_grant_permissions(
    plugin_id: String,
    permissions: Vec<String>,
    state: tauri::State<'_, PluginManagerState>,
    app: tauri::AppHandle,
) -> Result<PluginRecord, String> {
    let manifest = state.lookup_manifest(&plugin_id)?;
    let permissions = sanitize_requested_permissions(&manifest, permissions)?;

    {
        let mut plugin_states = state
            .plugin_states
            .lock()
            .map_err(|_| "Plugin manager lock poisoned".to_string())?;
        let entry = plugin_states.entry(plugin_id.clone()).or_default();
        if !entry.installed {
            return Err(format!("Plugin {} is not installed", plugin_id));
        }
        let mut granted_permissions = entry.granted_permissions.clone();
        granted_permissions.extend(permissions);
        entry.granted_permissions = sanitize_granted_permissions(granted_permissions, &manifest);
        entry.installed_version = entry
            .installed_version
            .clone()
            .or_else(|| Some(manifest.version.clone()));
        entry.updated_at = now_secs();
        state.persist_plugin_states(&plugin_states)?;
    }

    let plugin = state.plugin_record(&plugin_id)?;
    emit_registry_update(&app, &state.registry_snapshot()?);
    Ok(plugin)
}

#[tauri::command]
pub fn plugin_revoke_permissions(
    plugin_id: String,
    permissions: Vec<String>,
    state: tauri::State<'_, PluginManagerState>,
    app: tauri::AppHandle,
) -> Result<PluginRecord, String> {
    let manifest = state.lookup_manifest(&plugin_id)?;
    let permissions = sanitize_requested_permissions(&manifest, permissions)?;

    {
        let mut plugin_states = state
            .plugin_states
            .lock()
            .map_err(|_| "Plugin manager lock poisoned".to_string())?;
        let entry = plugin_states.entry(plugin_id.clone()).or_default();
        if !entry.installed {
            return Err(format!("Plugin {} is not installed", plugin_id));
        }
        entry.granted_permissions = entry
            .granted_permissions
            .iter()
            .filter(|permission| !permissions.contains(permission))
            .cloned()
            .collect();
        entry.granted_permissions =
            sanitize_granted_permissions(entry.granted_permissions.clone(), &manifest);
        if !missing_permissions_for(&manifest.permissions, &entry.granted_permissions).is_empty() {
            entry.enabled = false;
        }
        entry.installed_version = entry
            .installed_version
            .clone()
            .or_else(|| Some(manifest.version.clone()));
        entry.updated_at = now_secs();
        state.persist_plugin_states(&plugin_states)?;
    }

    let plugin = state.plugin_record(&plugin_id)?;
    emit_registry_update(&app, &state.registry_snapshot()?);
    Ok(plugin)
}

#[tauri::command]
pub fn plugin_uninstall(
    plugin_id: String,
    state: tauri::State<'_, PluginManagerState>,
    app: tauri::AppHandle,
) -> Result<Option<PluginRecord>, String> {
    state.remove_plugin_dir(&plugin_id)?;

    {
        let mut plugin_states = state
            .plugin_states
            .lock()
            .map_err(|_| "Plugin manager lock poisoned".to_string())?;
        let entry = plugin_states.entry(plugin_id.clone()).or_default();
        entry.installed = false;
        entry.enabled = false;
        entry.installed_version = None;
        entry.granted_permissions.clear();
        entry.updated_at = now_secs();
        state.persist_plugin_states(&plugin_states)?;
    }

    let plugin = state.plugin_record(&plugin_id).ok().map(|mut record| {
        record.installed = false;
        record.enabled = false;
        record.can_enable = false;
        record.installed_version = None;
        record.granted_permissions.clear();
        record.missing_permissions = record.permissions.clone();
        record.all_permissions_granted = false;
        record
    });
    emit_registry_update(&app, &state.registry_snapshot()?);
    Ok(plugin)
}

#[tauri::command]
pub fn plugin_enable(
    plugin_id: String,
    state: tauri::State<'_, PluginManagerState>,
    app: tauri::AppHandle,
) -> Result<PluginRecord, String> {
    let manifest = state.lookup_manifest(&plugin_id)?;

    {
        let mut plugin_states = state
            .plugin_states
            .lock()
            .map_err(|_| "Plugin manager lock poisoned".to_string())?;
        let entry = plugin_states.entry(plugin_id.clone()).or_default();
        if !entry.installed {
            return Err(format!("Plugin {} is not installed", plugin_id));
        }
        let granted_permissions =
            sanitize_granted_permissions(entry.granted_permissions.clone(), &manifest);
        let missing_permissions =
            missing_permissions_for(&manifest.permissions, &granted_permissions);
        if !missing_permissions.is_empty() {
            return Err(format!(
                "Plugin {} requires permission grants before enabling: {}",
                plugin_id,
                missing_permissions.join(", ")
            ));
        }
        entry.granted_permissions = granted_permissions;
        entry.enabled = true;
        entry.installed_version = entry
            .installed_version
            .clone()
            .or_else(|| Some(manifest.version.clone()));
        entry.updated_at = now_secs();
        state.persist_plugin_states(&plugin_states)?;
    }

    let plugin = state.plugin_record(&plugin_id)?;
    emit_registry_update(&app, &state.registry_snapshot()?);
    Ok(plugin)
}

#[tauri::command]
pub fn plugin_disable(
    plugin_id: String,
    state: tauri::State<'_, PluginManagerState>,
    app: tauri::AppHandle,
) -> Result<PluginRecord, String> {
    let manifest = state.lookup_manifest(&plugin_id)?;

    {
        let mut plugin_states = state
            .plugin_states
            .lock()
            .map_err(|_| "Plugin manager lock poisoned".to_string())?;
        let entry = plugin_states.entry(plugin_id.clone()).or_default();
        if !entry.installed {
            return Err(format!("Plugin {} is not installed", plugin_id));
        }
        entry.granted_permissions =
            sanitize_granted_permissions(entry.granted_permissions.clone(), &manifest);
        entry.enabled = false;
        entry.installed_version = entry
            .installed_version
            .clone()
            .or_else(|| Some(manifest.version.clone()));
        entry.updated_at = now_secs();
        state.persist_plugin_states(&plugin_states)?;
    }

    let plugin = state.plugin_record(&plugin_id)?;
    emit_registry_update(&app, &state.registry_snapshot()?);
    Ok(plugin)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_manifest_roundtrip_is_valid_toml() {
        let catalog = builtin_catalog();
        let manifest = catalog.get("rust-lang").unwrap();
        let body = toml::to_string_pretty(manifest).unwrap();
        let parsed: PluginManifest = toml::from_str(&body).unwrap();
        assert_eq!(parsed.id, "rust-lang");
        assert_eq!(parsed.plugin_type, PluginKind::Language);
        assert_eq!(parsed.runtime, PluginRuntime::RustCrate);
    }

    #[test]
    fn load_defaults_when_state_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let states = load_plugin_states_from_disk(dir.path());
        assert_eq!(states.get("rust-lang").unwrap().installed, true);
        assert_eq!(
            states.get("python-lang").unwrap().granted_permissions.len(),
            4
        );
    }

    #[test]
    fn version_compare_detects_update() {
        assert!(version_is_newer("1.4.1", "1.4.0"));
        assert!(version_is_newer("2.0.0", "1.9.9"));
        assert!(!version_is_newer("1.4.0", "1.4.0"));
        assert!(!version_is_newer("1.3.9", "1.4.0"));
    }

    #[test]
    fn installed_manifest_overrides_state_version() {
        let dir = tempfile::tempdir().unwrap();
        let plugins_dir = dir.path().join(PLUGINS_DIR).join("go-lang");
        std::fs::create_dir_all(&plugins_dir).unwrap();
        let catalog = builtin_catalog();
        let manifest = catalog.get("go-lang").unwrap();
        std::fs::write(
            plugins_dir.join("plugin.toml"),
            toml::to_string_pretty(manifest).unwrap(),
        )
        .unwrap();

        let state = PluginManagerState::new(dir.path().to_path_buf());
        {
            let mut plugin_states = state.plugin_states.lock().unwrap();
            plugin_states.insert(
                "go-lang".to_string(),
                PluginState {
                    installed: true,
                    enabled: true,
                    installed_version: Some("0.8.0".to_string()),
                    granted_permissions: vec!["workspace:read".to_string()],
                    updated_at: 1,
                },
            );
        }

        let records = state.registry_snapshot().unwrap();
        let go = records
            .iter()
            .find(|plugin| plugin.id == "go-lang")
            .unwrap();
        assert_eq!(go.installed_version.as_deref(), Some("1.2.0"));
        assert!(go.installed);
        assert_eq!(go.missing_permissions.len(), 3);
        assert!(!go.enabled);
    }

    #[test]
    fn invalid_manifest_permissions_are_rejected() {
        let manifest = PluginManifest {
            id: "bad-plugin".to_string(),
            name: "Bad".to_string(),
            version: "1.0.0".to_string(),
            api_version: "1".to_string(),
            author: "Test".to_string(),
            description: "Bad plugin".to_string(),
            plugin_type: PluginKind::Tool,
            runtime: PluginRuntime::Wasm,
            permissions: vec!["root:shell".to_string()],
            entry_points: vec!["frontend:test".to_string()],
            source: "community".to_string(),
        };

        let err = validate_manifest(&manifest).unwrap_err();
        assert!(err.contains("unsupported permissions"));
    }

    #[test]
    fn sanitize_permissions_keeps_supported_requested_only() {
        let catalog = builtin_catalog();
        let manifest = catalog.get("ai-code-review").unwrap();
        let granted = sanitize_granted_permissions(
            vec![
                "ai:chat".to_string(),
                "ai:chat".to_string(),
                "root:shell".to_string(),
                "workspace:read".to_string(),
            ],
            manifest,
        );

        assert_eq!(
            granted,
            vec!["ai:chat".to_string(), "workspace:read".to_string()]
        );
    }
}
