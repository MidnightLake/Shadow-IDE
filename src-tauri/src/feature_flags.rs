use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeatureFlagProviderConfig {
    pub id: String,
    pub name: String,
    pub provider: String,
    #[serde(default)]
    pub base_url: Option<String>,
    pub token: String,
    pub project_key: String,
    pub environment: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeatureFlagRecord {
    pub key: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub provider: String,
    pub tags: Vec<String>,
}

pub struct FeatureFlagsState {
    file_path: PathBuf,
    configs: Mutex<Vec<FeatureFlagProviderConfig>>,
}

impl FeatureFlagsState {
    pub fn new(data_dir: PathBuf) -> Self {
        let file_path = data_dir.join("feature_flags.json");
        let configs = std::fs::read(&file_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<FeatureFlagProviderConfig>>(&bytes).ok())
            .unwrap_or_default();
        Self {
            file_path,
            configs: Mutex::new(configs),
        }
    }

    fn save(&self) -> Result<(), String> {
        let configs = self
            .configs
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        if let Some(parent) = self.file_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config dir: {}", e))?;
        }
        std::fs::write(
            &self.file_path,
            serde_json::to_vec_pretty(&*configs).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("Failed to write feature flag providers: {}", e))
    }

    fn get_config(&self, id: &str) -> Result<FeatureFlagProviderConfig, String> {
        let configs = self
            .configs
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        configs
            .iter()
            .find(|cfg| cfg.id == id)
            .cloned()
            .ok_or_else(|| format!("Feature flag provider '{}' not found", id))
    }
}

fn build_client(config: &FeatureFlagProviderConfig) -> Result<reqwest::Client, String> {
    let mut headers = reqwest::header::HeaderMap::new();
    if !config.token.trim().is_empty() {
        let header_value = if config.provider == "launchdarkly" {
            config.token.trim().to_string()
        } else if config.token.trim_start().starts_with("Bearer ") {
            config.token.trim().to_string()
        } else {
            format!("Bearer {}", config.token.trim())
        };
        headers.insert(
            reqwest::header::AUTHORIZATION,
            reqwest::header::HeaderValue::from_str(&header_value).map_err(|e| e.to_string())?,
        );
    }
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {}", e))
}

async fn list_launchdarkly_flags(
    config: &FeatureFlagProviderConfig,
) -> Result<Vec<FeatureFlagRecord>, String> {
    let client = build_client(config)?;
    let base_url = config
        .base_url
        .clone()
        .unwrap_or_else(|| "https://app.launchdarkly.com".to_string());
    let url = format!(
        "{}/api/v2/flags/{}",
        base_url.trim_end_matches('/'),
        config.project_key
    );
    let value = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("LaunchDarkly request failed: {}", e))?
        .error_for_status()
        .map_err(|e| format!("LaunchDarkly request failed: {}", e))?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("Failed to decode LaunchDarkly response: {}", e))?;
    let items = value["items"].as_array().cloned().unwrap_or_default();
    Ok(items
        .into_iter()
        .map(|item| FeatureFlagRecord {
            key: item["key"].as_str().unwrap_or_default().to_string(),
            name: item["name"]
                .as_str()
                .unwrap_or_else(|| item["key"].as_str().unwrap_or("flag"))
                .to_string(),
            description: item["description"].as_str().unwrap_or_default().to_string(),
            enabled: item["environments"][&config.environment]["on"]
                .as_bool()
                .unwrap_or(false),
            provider: "launchdarkly".to_string(),
            tags: item["tags"]
                .as_array()
                .map(|values| {
                    values
                        .iter()
                        .filter_map(|value| value.as_str().map(|v| v.to_string()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
        })
        .collect())
}

async fn list_unleash_flags(
    config: &FeatureFlagProviderConfig,
) -> Result<Vec<FeatureFlagRecord>, String> {
    let client = build_client(config)?;
    let base_url = config
        .base_url
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Unleash provider requires base_url".to_string())?;
    let url = format!(
        "{}/api/admin/projects/{}/features",
        base_url.trim_end_matches('/'),
        config.project_key
    );
    let items = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Unleash request failed: {}", e))?
        .error_for_status()
        .map_err(|e| format!("Unleash request failed: {}", e))?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| format!("Failed to decode Unleash response: {}", e))?["features"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    Ok(items
        .into_iter()
        .map(|item| {
            let enabled = item["environments"]
                .as_array()
                .and_then(|envs| {
                    envs.iter().find(|environment| {
                        environment["name"].as_str() == Some(config.environment.as_str())
                    })
                })
                .and_then(|environment| environment["enabled"].as_bool())
                .unwrap_or(false);
            FeatureFlagRecord {
                key: item["name"].as_str().unwrap_or_default().to_string(),
                name: item["name"].as_str().unwrap_or_default().to_string(),
                description: item["description"].as_str().unwrap_or_default().to_string(),
                enabled,
                provider: "unleash".to_string(),
                tags: item["tags"]
                    .as_array()
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(|value| value["value"].as_str().map(|v| v.to_string()))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default(),
            }
        })
        .collect())
}

async fn set_launchdarkly_flag(
    config: &FeatureFlagProviderConfig,
    key: &str,
    enabled: bool,
) -> Result<(), String> {
    let client = build_client(config)?;
    let base_url = config
        .base_url
        .clone()
        .unwrap_or_else(|| "https://app.launchdarkly.com".to_string());
    client
        .patch(format!(
            "{}/api/v2/flags/{}/{}",
            base_url.trim_end_matches('/'),
            config.project_key,
            key
        ))
        .header(
            "Content-Type",
            "application/json; domain-model=launchdarkly.semanticpatch",
        )
        .json(&serde_json::json!({
            "environmentKey": config.environment,
            "instructions": [
                { "kind": if enabled { "turnFlagOn" } else { "turnFlagOff" } }
            ],
            "comment": "Updated from ShadowIDE"
        }))
        .send()
        .await
        .map_err(|e| format!("LaunchDarkly request failed: {}", e))?
        .error_for_status()
        .map_err(|e| format!("LaunchDarkly request failed: {}", e))?;
    Ok(())
}

async fn set_unleash_flag(
    config: &FeatureFlagProviderConfig,
    key: &str,
    enabled: bool,
) -> Result<(), String> {
    let client = build_client(config)?;
    let base_url = config
        .base_url
        .as_ref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "Unleash provider requires base_url".to_string())?;
    client
        .post(format!(
            "{}/api/admin/projects/{}/features/{}/environments/{}/{}",
            base_url.trim_end_matches('/'),
            config.project_key,
            key,
            config.environment,
            if enabled { "on" } else { "off" }
        ))
        .send()
        .await
        .map_err(|e| format!("Unleash request failed: {}", e))?
        .error_for_status()
        .map_err(|e| format!("Unleash request failed: {}", e))?;
    Ok(())
}

#[tauri::command]
pub fn feature_flag_list_configs(
    state: tauri::State<'_, FeatureFlagsState>,
) -> Result<Vec<FeatureFlagProviderConfig>, String> {
    Ok(state
        .configs
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?
        .clone())
}

#[tauri::command]
pub fn feature_flag_save_config(
    config: FeatureFlagProviderConfig,
    state: tauri::State<'_, FeatureFlagsState>,
) -> Result<FeatureFlagProviderConfig, String> {
    let mut config = config;
    if config.id.trim().is_empty() {
        config.id = uuid::Uuid::new_v4().to_string();
    }
    let mut configs = state
        .configs
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    if let Some(existing) = configs.iter_mut().find(|existing| existing.id == config.id) {
        *existing = config.clone();
    } else {
        configs.push(config.clone());
    }
    drop(configs);
    state.save()?;
    Ok(config)
}

#[tauri::command]
pub fn feature_flag_delete_config(
    id: String,
    state: tauri::State<'_, FeatureFlagsState>,
) -> Result<(), String> {
    let mut configs = state
        .configs
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    configs.retain(|config| config.id != id);
    drop(configs);
    state.save()
}

#[tauri::command]
pub async fn feature_flag_list_flags(
    id: String,
    state: tauri::State<'_, FeatureFlagsState>,
) -> Result<Vec<FeatureFlagRecord>, String> {
    let config = state.get_config(&id)?;
    match config.provider.as_str() {
        "launchdarkly" => list_launchdarkly_flags(&config).await,
        "unleash" => list_unleash_flags(&config).await,
        _ => Err("Unsupported feature flag provider".to_string()),
    }
}

#[tauri::command]
pub async fn feature_flag_set_enabled(
    id: String,
    key: String,
    enabled: bool,
    state: tauri::State<'_, FeatureFlagsState>,
) -> Result<(), String> {
    let config = state.get_config(&id)?;
    match config.provider.as_str() {
        "launchdarkly" => set_launchdarkly_flag(&config, &key, enabled).await,
        "unleash" => set_unleash_flag(&config, &key, enabled).await,
        _ => Err("Unsupported feature flag provider".to_string()),
    }
}
