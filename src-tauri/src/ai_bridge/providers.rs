use super::types::{AiProvider, ModelInfo, ModelProfile, ModelsResponse};
use super::AiConfig;

#[tauri::command]
pub fn ai_set_base_url(url: String, state: tauri::State<'_, AiConfig>) -> Result<(), String> {
    let mut base_url = state.base_url.lock().map_err(|e| e.to_string())?;
    *base_url = url;
    Ok(())
}

#[tauri::command]
pub async fn ai_get_models(state: tauri::State<'_, AiConfig>) -> Result<Vec<ModelInfo>, String> {
    let base_url = state.base_url.lock().map_err(|e| e.to_string())?.clone();
    let response = state
        .client
        .get(format!("{}/models", base_url))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let models: ModelsResponse = response.json().await.map_err(|e| e.to_string())?;
    Ok(models.data)
}

#[tauri::command]
pub async fn ai_check_connection(state: tauri::State<'_, AiConfig>) -> Result<bool, String> {
    let base_url = state.base_url.lock().map_err(|e| e.to_string())?.clone();
    match state
        .client
        .get(format!("{}/models", base_url))
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
    {
        Ok(resp) => Ok(resp.status().is_success()),
        Err(_) => Ok(false),
    }
}

#[tauri::command]
pub async fn ai_detect_providers(
    state: tauri::State<'_, AiConfig>,
) -> Result<Vec<AiProvider>, String> {
    let presets = vec![
        ("LM Studio", "http://localhost:1234/v1"),
        ("Ollama", "http://localhost:11434/v1"),
        ("llama.cpp", "http://localhost:8080/v1"),
    ];
    let mut results = Vec::new();
    for (name, url) in presets {
        let available = match state
            .client
            .get(format!("{}/models", url))
            .timeout(std::time::Duration::from_secs(1))
            .send()
            .await
        {
            Ok(r) => r.status().is_success(),
            Err(_) => false,
        };
        results.push(AiProvider {
            name: name.to_string(),
            base_url: url.to_string(),
            available,
            model_count: 0,
        });
    }
    Ok(results)
}

#[tauri::command]
pub fn ai_profile_model(model_id: String) -> ModelProfile {
    let id = model_id.to_lowercase();
    if id.contains("code")
        || id.contains("coder")
        || id.contains("deepseek-coder")
        || id.contains("qwen2.5-coder")
    {
        ModelProfile {
            category: "code".into(),
            recommended_temp: 0.3,
            recommended_max_tokens: 4096,
            recommended_context: 16384,
            supports_tools: true,
            description: "Optimized for coding".into(),
        }
    } else if id.contains("reason") || id.contains("think") || id.contains("r1") {
        ModelProfile {
            category: "reasoning".into(),
            recommended_temp: 0.2,
            recommended_max_tokens: 8192,
            recommended_context: 32768,
            supports_tools: true,
            description: "Chain-of-thought logic".into(),
        }
    } else {
        ModelProfile {
            category: "chat".into(),
            recommended_temp: 0.7,
            recommended_max_tokens: 2048,
            recommended_context: 16384,
            supports_tools: true,
            description: "General conversation".into(),
        }
    }
}
