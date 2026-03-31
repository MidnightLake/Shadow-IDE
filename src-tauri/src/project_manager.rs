use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const MAX_RECENT: usize = 20;
const RECENT_FILE: &str = "recent_projects.json";
const PROJECTS_DIR: &str = "projects";

pub struct ProjectManagerState {
    pub data_dir: PathBuf,
    pub recent: Mutex<Vec<RecentProject>>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RecentProject {
    pub path: String,
    pub name: String,
    pub last_opened: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct WorkspaceSettings {
    pub tab_size: Option<u32>,
    pub use_tabs: Option<bool>,
    pub font_size: Option<u32>,
    pub minimap_enabled: Option<bool>,
    pub ai_model: Option<String>,
    pub ai_temperature: Option<f64>,
    pub tools_enabled: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SavedTerminalSession {
    pub name: String,
    pub shell: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProjectState {
    pub root_path: String,
    pub open_files: Vec<String>,
    pub active_file_index: usize,
    pub sidebar_view: String,
    pub sidebar_width: u32,
    pub terminal_visible: bool,
    pub terminal_height: u32,
    pub ai_completion_enabled: bool,
    pub timestamp: u64,
    #[serde(default)]
    pub workspace_settings: Option<WorkspaceSettings>,
    #[serde(default)]
    pub terminal_sessions: Vec<SavedTerminalSession>,
}

impl ProjectManagerState {
    pub fn new(data_dir: PathBuf) -> Self {
        let recent = load_recent_from_disk(&data_dir);
        Self {
            data_dir,
            recent: Mutex::new(recent),
        }
    }

    fn projects_dir(&self) -> PathBuf {
        self.data_dir.join(PROJECTS_DIR)
    }

    fn project_state_path(&self, root_path: &str) -> PathBuf {
        let hash = simple_hash(root_path);
        self.projects_dir().join(format!("{}.json", hash))
    }
}

fn simple_hash(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())[..16].to_string()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn load_recent_from_disk(data_dir: &Path) -> Vec<RecentProject> {
    let path = data_dir.join(RECENT_FILE);
    if let Ok(content) = std::fs::read_to_string(&path) {
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        Vec::new()
    }
}

fn save_recent_to_disk(data_dir: &Path, recent: &[RecentProject]) {
    let path = data_dir.join(RECENT_FILE);
    let _ = std::fs::create_dir_all(data_dir);
    if let Ok(json) = serde_json::to_string_pretty(recent) {
        let _ = std::fs::write(&path, json);
    }
}

// ===== Tests =====

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_simple_hash_deterministic() {
        let h1 = simple_hash("/home/user/project");
        let h2 = simple_hash("/home/user/project");
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16);
    }

    #[test]
    fn test_simple_hash_different_inputs() {
        let h1 = simple_hash("/home/user/project-a");
        let h2 = simple_hash("/home/user/project-b");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_load_recent_empty() {
        let dir = tempfile::tempdir().unwrap();
        let recent = load_recent_from_disk(dir.path());
        assert!(recent.is_empty());
    }

    #[test]
    fn test_save_and_load_recent() {
        let dir = tempfile::tempdir().unwrap();
        let projects = vec![
            RecentProject {
                path: "/home/user/proj1".to_string(),
                name: "proj1".to_string(),
                last_opened: 1000,
            },
            RecentProject {
                path: "/home/user/proj2".to_string(),
                name: "proj2".to_string(),
                last_opened: 2000,
            },
        ];

        save_recent_to_disk(dir.path(), &projects);
        let loaded = load_recent_from_disk(dir.path());

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].path, "/home/user/proj1");
        assert_eq!(loaded[1].name, "proj2");
    }

    #[test]
    fn test_project_state_path_uses_hash() {
        let dir = tempfile::tempdir().unwrap();
        let state = ProjectManagerState::new(dir.path().to_path_buf());
        let path = state.project_state_path("/home/user/my-project");
        let hash = simple_hash("/home/user/my-project");
        assert!(path.to_string_lossy().contains(&hash));
        assert!(path.to_string_lossy().ends_with(".json"));
    }

    #[test]
    fn test_now_secs_nonzero() {
        let ts = now_secs();
        assert!(ts > 0);
    }

    #[test]
    fn test_workspace_settings_defaults() {
        let settings = WorkspaceSettings::default();
        assert!(settings.tab_size.is_none());
        assert!(settings.use_tabs.is_none());
        assert!(settings.font_size.is_none());
    }

    #[test]
    fn test_workspace_settings_serialization() {
        let settings = WorkspaceSettings {
            tab_size: Some(4),
            use_tabs: Some(false),
            font_size: Some(16),
            minimap_enabled: Some(true),
            ai_model: Some("codellama".to_string()),
            ai_temperature: Some(0.7),
            tools_enabled: Some(true),
        };

        let json = serde_json::to_string(&settings).unwrap();
        let deserialized: WorkspaceSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.tab_size, Some(4));
        assert_eq!(deserialized.ai_model, Some("codellama".to_string()));
    }

    #[test]
    fn test_project_state_serialization() {
        let state = ProjectState {
            root_path: "/home/user/project".to_string(),
            open_files: vec!["main.rs".to_string(), "lib.rs".to_string()],
            active_file_index: 0,
            sidebar_view: "explorer".to_string(),
            sidebar_width: 250,
            terminal_visible: true,
            terminal_height: 200,
            ai_completion_enabled: false,
            timestamp: 12345,
            workspace_settings: None,
            terminal_sessions: vec![],
        };

        let json = serde_json::to_string(&state).unwrap();
        let deserialized: ProjectState = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.root_path, "/home/user/project");
        assert_eq!(deserialized.open_files.len(), 2);
        assert_eq!(deserialized.sidebar_width, 250);
    }

    #[test]
    fn test_recent_project_max_cap() {
        let dir = tempfile::tempdir().unwrap();
        let mut projects = Vec::new();
        for i in 0..25 {
            projects.push(RecentProject {
                path: format!("/home/user/proj{}", i),
                name: format!("proj{}", i),
                last_opened: i as u64,
            });
        }
        projects.truncate(MAX_RECENT);
        assert_eq!(projects.len(), MAX_RECENT);
        save_recent_to_disk(dir.path(), &projects);
        let loaded = load_recent_from_disk(dir.path());
        assert_eq!(loaded.len(), MAX_RECENT);
    }

    #[test]
    fn test_load_corrupt_json_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(RECENT_FILE), "not valid json").unwrap();
        let recent = load_recent_from_disk(dir.path());
        assert!(recent.is_empty());
    }
}

// ===== Tauri Commands =====

#[tauri::command]
pub fn project_open(
    path: String,
    state: tauri::State<'_, ProjectManagerState>,
) -> Result<RecentProject, String> {
    let name = Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| path.clone());

    let project = RecentProject {
        path: path.clone(),
        name,
        last_opened: now_secs(),
    };

    let mut recent = state
        .recent
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    // Remove existing entry for this path
    recent.retain(|r| r.path != path);

    // Add at front
    recent.insert(0, project.clone());

    // Cap to max
    recent.truncate(MAX_RECENT);

    save_recent_to_disk(&state.data_dir, &recent);

    Ok(project)
}

#[tauri::command]
pub fn project_list_recent(
    state: tauri::State<'_, ProjectManagerState>,
) -> Result<Vec<RecentProject>, String> {
    let recent = state
        .recent
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    Ok(recent.clone())
}

#[tauri::command]
pub fn project_remove_recent(
    path: String,
    state: tauri::State<'_, ProjectManagerState>,
) -> Result<(), String> {
    let mut recent = state
        .recent
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    recent.retain(|r| r.path != path);
    save_recent_to_disk(&state.data_dir, &recent);
    Ok(())
}

#[tauri::command]
pub fn project_save_state(
    project_state: ProjectState,
    state: tauri::State<'_, ProjectManagerState>,
) -> Result<(), String> {
    let projects_dir = state.projects_dir();
    std::fs::create_dir_all(&projects_dir)
        .map_err(|e| format!("Failed to create projects dir: {}", e))?;

    let file_path = state.project_state_path(&project_state.root_path);
    let json = serde_json::to_string_pretty(&project_state)
        .map_err(|e| format!("Failed to serialize project state: {}", e))?;

    std::fs::write(&file_path, json)
        .map_err(|e| format!("Failed to write project state: {}", e))?;

    Ok(())
}

#[tauri::command]
pub fn project_load_state(
    root_path: String,
    state: tauri::State<'_, ProjectManagerState>,
) -> Result<Option<ProjectState>, String> {
    let file_path = state.project_state_path(&root_path);

    if !file_path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&file_path)
        .map_err(|e| format!("Failed to read project state: {}", e))?;

    let project_state: ProjectState = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse project state: {}", e))?;

    Ok(Some(project_state))
}

#[tauri::command]
pub fn project_clear_recent(state: tauri::State<'_, ProjectManagerState>) -> Result<(), String> {
    let mut recent = state
        .recent
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    recent.clear();
    save_recent_to_disk(&state.data_dir, &recent);
    Ok(())
}

#[tauri::command]
pub fn project_load_config(root_path: String) -> Result<Option<WorkspaceSettings>, String> {
    let config_path = std::path::Path::new(&root_path)
        .join(".shadowide")
        .join("config.json");
    if !config_path.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Failed to read config: {}", e))?;
    let settings: WorkspaceSettings =
        serde_json::from_str(&content).map_err(|e| format!("Failed to parse config: {}", e))?;
    Ok(Some(settings))
}

#[tauri::command]
pub fn project_save_config(root_path: String, config: WorkspaceSettings) -> Result<(), String> {
    let config_dir = std::path::Path::new(&root_path).join(".shadowide");
    std::fs::create_dir_all(&config_dir)
        .map_err(|e| format!("Failed to create config dir: {}", e))?;
    let json = serde_json::to_string_pretty(&config)
        .map_err(|e| format!("Failed to serialize config: {}", e))?;
    std::fs::write(config_dir.join("config.json"), json)
        .map_err(|e| format!("Failed to write config: {}", e))?;
    Ok(())
}
