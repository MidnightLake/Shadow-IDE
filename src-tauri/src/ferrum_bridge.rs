use ferrum_core::config::{Config, Profile};
use ferrum_core::types::Message;
use ferrum_sessions::compact;
use ferrum_sessions::{Session, SessionStore};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;
use tauri::State;

pub struct FerrumState {
    pub store: Mutex<SessionStore>,
    pub active_profile: Mutex<Option<String>>,
    pub last_session_id: Mutex<Option<String>>,
}

impl FerrumState {
    pub fn new() -> anyhow::Result<Self> {
        Ok(Self {
            store: Mutex::new(SessionStore::new()?),
            active_profile: Mutex::new(None),
            last_session_id: Mutex::new(None),
        })
    }

    /// Create a degraded FerrumState with an in-memory store.
    /// Chat history will not persist, but the app can still start.
    pub fn empty() -> Self {
        Self {
            store: Mutex::new(
                SessionStore::open_in_memory().expect("In-memory SQLite should never fail"),
            ),
            active_profile: Mutex::new(None),
            last_session_id: Mutex::new(None),
        }
    }
}

// ===== Config Commands =====

#[tauri::command]
pub fn ferrum_get_config() -> Result<Config, String> {
    Config::load().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ferrum_save_config(config: Config) -> Result<(), String> {
    config.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ferrum_get_profiles() -> Result<Vec<Profile>, String> {
    let config = Config::load().map_err(|e| e.to_string())?;
    Ok(config.profiles)
}

#[tauri::command]
pub fn ferrum_add_profile(profile: Profile) -> Result<(), String> {
    let mut config = Config::load().map_err(|e| e.to_string())?;
    // Replace if exists, else append
    if let Some(idx) = config.profiles.iter().position(|p| p.name == profile.name) {
        config.profiles[idx] = profile;
    } else {
        config.profiles.push(profile);
    }
    config.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ferrum_remove_profile(name: String) -> Result<(), String> {
    let mut config = Config::load().map_err(|e| e.to_string())?;
    config.profiles.retain(|p| p.name != name);
    config.save().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ferrum_set_active_profile(
    name: String,
    state: State<'_, FerrumState>,
) -> Result<Profile, String> {
    let config = Config::load().map_err(|e| e.to_string())?;
    let profile = config
        .get_profile(&name)
        .cloned()
        .ok_or_else(|| format!("Profile '{}' not found", name))?;
    if let Ok(mut active) = state.active_profile.lock() {
        *active = Some(name);
    }
    Ok(profile)
}

#[tauri::command]
pub fn ferrum_get_active_profile(state: State<'_, FerrumState>) -> Result<Option<Profile>, String> {
    let name = state
        .active_profile
        .lock()
        .map_err(|e| e.to_string())?
        .clone();
    if let Some(name) = name {
        let config = Config::load().map_err(|e| e.to_string())?;
        Ok(config.get_profile(&name).cloned())
    } else {
        let config = Config::load().map_err(|e| e.to_string())?;
        Ok(config.get_default_profile().cloned())
    }
}

// ===== Session Commands =====

#[tauri::command]
pub fn ferrum_list_sessions(state: State<'_, FerrumState>) -> Result<Vec<Session>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.list_sessions().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ferrum_create_session(
    name: String,
    profile: String,
    state: State<'_, FerrumState>,
) -> Result<Session, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let session = store
        .create_session(name, profile)
        .map_err(|e| e.to_string())?;
    if let Ok(mut last) = state.last_session_id.lock() {
        *last = Some(session.id.clone());
    }
    Ok(session)
}

#[tauri::command]
pub fn ferrum_load_messages(
    session_id: String,
    state: State<'_, FerrumState>,
) -> Result<Vec<Message>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.load_messages(&session_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ferrum_save_message(
    session_id: String,
    message: Message,
    state: State<'_, FerrumState>,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .save_message(&session_id, &message)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ferrum_delete_session(
    session_id: String,
    state: State<'_, FerrumState>,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.delete_session(&session_id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ferrum_rename_session(
    session_id: String,
    new_name: String,
    state: State<'_, FerrumState>,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .rename_session(&session_id, &new_name)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ferrum_pin_session(
    session_id: String,
    pinned: bool,
    state: State<'_, FerrumState>,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .pin_session(&session_id, pinned)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ferrum_get_latest_session(state: State<'_, FerrumState>) -> Result<Option<Session>, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store.get_latest_session().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ferrum_get_session_token_count(
    session_id: String,
    state: State<'_, FerrumState>,
) -> Result<usize, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .get_session_token_count(&session_id)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn ferrum_export_session(
    session_id: String,
    state: State<'_, FerrumState>,
) -> Result<String, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    store
        .export_session_markdown(&session_id)
        .map_err(|e| e.to_string())
}

// ===== Compaction Commands =====

#[derive(Debug, Serialize, Deserialize)]
pub struct CompactionCheck {
    pub should_compact: bool,
    pub used_tokens: u32,
    pub max_tokens: u32,
    pub percentage: f64,
}

#[tauri::command]
pub fn ferrum_check_compaction(
    session_id: String,
    max_tokens: u32,
    threshold: f64,
    state: State<'_, FerrumState>,
) -> Result<CompactionCheck, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let used = store
        .get_session_token_count(&session_id)
        .map_err(|e| e.to_string())? as u32;
    let should = compact::should_compact(used, max_tokens, threshold);
    let percentage = if max_tokens > 0 {
        used as f64 / max_tokens as f64
    } else {
        0.0
    };
    Ok(CompactionCheck {
        should_compact: should,
        used_tokens: used,
        max_tokens,
        percentage,
    })
}

#[tauri::command]
pub fn ferrum_get_compaction_prompt(
    session_id: String,
    state: State<'_, FerrumState>,
) -> Result<String, String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let messages = store
        .load_messages(&session_id)
        .map_err(|e| e.to_string())?;
    Ok(compact::compaction_prompt(&messages))
}

#[tauri::command]
pub fn ferrum_apply_compaction(
    session_id: String,
    summary: String,
    state: State<'_, FerrumState>,
) -> Result<(), String> {
    let store = state.store.lock().map_err(|e| e.to_string())?;
    let messages = store
        .load_messages(&session_id)
        .map_err(|e| e.to_string())?;
    let compacted = compact::compact_messages(&messages, &summary);

    // Clear old messages and save compacted ones
    store
        .clear_session_messages(&session_id)
        .map_err(|e| e.to_string())?;
    for msg in &compacted {
        store
            .save_message(&session_id, msg)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ===== Connection Check =====

#[tauri::command]
pub async fn ferrum_check_provider(base_url: String) -> Result<bool, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .map_err(|e| e.to_string())?;
    match client.get(format!("{}/models", base_url)).send().await {
        Ok(resp) => Ok(resp.status().is_success()),
        Err(_) => Ok(false),
    }
}

#[derive(Debug, Serialize)]
pub struct ProviderModels {
    pub models: Vec<String>,
    pub connected: bool,
}

#[tauri::command]
pub async fn ferrum_list_provider_models(base_url: String) -> Result<ProviderModels, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;

    #[derive(Deserialize)]
    struct ModelInfo {
        id: String,
    }
    #[derive(Deserialize)]
    struct ModelsResponse {
        data: Vec<ModelInfo>,
    }

    match client.get(format!("{}/models", base_url)).send().await {
        Ok(resp) if resp.status().is_success() => {
            let models: ModelsResponse = resp.json().await.map_err(|e| e.to_string())?;
            Ok(ProviderModels {
                models: models.data.into_iter().map(|m| m.id).collect(),
                connected: true,
            })
        }
        _ => Ok(ProviderModels {
            models: vec![],
            connected: false,
        }),
    }
}
