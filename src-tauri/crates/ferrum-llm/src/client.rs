use ferrum_core::config::Profile;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ModelInfo>,
}

pub struct LlmClient {
    pub client: Client,
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,
    pub max_context_tokens: u32,
}

impl LlmClient {
    pub fn from_profile(profile: &Profile) -> Self {
        let api_key = if !profile.api_key_env.is_empty() {
            std::env::var(&profile.api_key_env).ok()
        } else {
            None
        };

        let client = Client::builder()
            .timeout(Duration::from_secs(600))
            .build()
            .unwrap_or_else(|_| Client::new());

        Self {
            client,
            base_url: profile.base_url.clone(),
            model: profile.model.clone(),
            api_key,
            max_context_tokens: profile.max_context_tokens,
        }
    }

    pub async fn check_connection(&self) -> bool {
        let url = format!("{}/models", self.base_url);
        for attempt in 0..3u32 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt - 1))).await;
            }
            let mut req = self.client.get(&url).timeout(Duration::from_secs(3));
            if let Some(key) = &self.api_key {
                req = req.bearer_auth(key);
            }
            match req.send().await {
                Ok(resp) if resp.status().is_success() => return true,
                Ok(_) => return false, // Got a response but not success — don't retry
                Err(_) if attempt < 2 => continue, // Network error — retry
                Err(_) => return false,
            }
        }
        false
    }

    pub async fn list_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        let url = format!("{}/models", self.base_url);
        let mut last_err = None;
        for attempt in 0..3u32 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(500 * 2u64.pow(attempt - 1))).await;
            }
            let mut req = self.client.get(&url);
            if let Some(key) = &self.api_key {
                req = req.bearer_auth(key);
            }
            match req.send().await {
                Ok(resp) => {
                    let models: ModelsResponse = resp.json().await?;
                    return Ok(models.data);
                }
                Err(e) if attempt < 2 => { last_err = Some(e); continue; }
                Err(e) => return Err(e.into()),
            }
        }
        Err(last_err.map(|e| e.into()).unwrap_or_else(|| anyhow::anyhow!("Failed after retries")))
    }
}
