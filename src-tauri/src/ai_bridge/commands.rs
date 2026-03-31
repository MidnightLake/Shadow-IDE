use tauri::{AppHandle, Emitter, Manager};

use super::chat::ai_chat_with_tools;
use super::types::{ChatCompletionRequest, ChatCompletionResponse, ChatMessage};
use super::AiConfig;
use crate::token_optimizer::{self, TokenCache, TokenSettings};

#[tauri::command]
pub async fn ai_complete_code(
    prefix: String,
    suffix: String,
    language: String,
    model: Option<String>,
    state: tauri::State<'_, AiConfig>,
    cache: tauri::State<'_, TokenCache>,
    settings: tauri::State<'_, TokenSettings>,
) -> Result<String, String> {
    let url = state
        .base_url
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?
        .clone();
    let name = model.unwrap_or_else(|| "default".to_string());
    let clean_mode = if let Ok(st) = settings.state.lock() {
        st.clean_mode.clone()
    } else {
        token_optimizer::CleanMode::Trim
    };
    let (p, s) = (
        token_optimizer::clean_context(&prefix, &clean_mode),
        token_optimizer::clean_context(&suffix, &clean_mode),
    );
    let key = token_optimizer::cache_key(&format!("code:{}:{}:{}", language, p, s), &name, 0.2);
    if let Some(c) = cache.get(&key) {
        return Ok(c);
    }
    let req = ChatCompletionRequest {
        model: name.clone(),
        messages: vec![
            serde_json::json!({ "role": "system", "content": format!("Code completion for {}.", language) }),
            serde_json::json!({ "role": "user", "content": format!("{}<CURSOR>{}", p, s) }),
        ],
        stream: false,
        temperature: Some(0.2),
        max_tokens: Some(256),
        stop: Some(vec!["\n\n".to_string()]),
    };
    let response = state
        .client
        .post(format!("{}/chat/completions", url))
        .json(&req)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let resp: ChatCompletionResponse = response.json().await.map_err(|e| e.to_string())?;
    let res = resp
        .choices
        .first()
        .and_then(|c| c.message.content.clone())
        .ok_or_else(|| "No result".to_string())?;
    cache.put(key, res.clone());
    Ok(res)
}

#[tauri::command]
pub async fn ai_explain_error(
    text: String,
    ctx: Option<String>,
    model: Option<String>,
    app: AppHandle,
    state: tauri::State<'_, AiConfig>,
) -> Result<(), String> {
    let mut content = format!("Explain error:\n\n```\n{}\n```", text);
    if let Some(c) = ctx {
        content.push_str(&format!("\n\nContext:\n{}", c));
    }
    let msgs = vec![
        ChatMessage {
            role: "system".to_string(),
            content: "Explain terminal error concisely.".to_string(),
        },
        ChatMessage {
            role: "user".to_string(),
            content,
        },
    ];
    let sid = format!("explain-{}", uuid::Uuid::new_v4());
    let _ = app.emit("ai-explain-stream-id", sid.clone());
    ai_chat_with_tools(
        sid,
        msgs,
        model,
        None,
        None, // api_key
        Some(0.3),
        Some(1024),
        false,
        "plan".to_string(),
        "".to_string(),
        app.clone(),
        state,
        app.state(),
        app.state(),
        app.state(),
        app.state(),
        app.state(),
    )
    .await
}

// ===== Memory Panel Commands =====

#[tauri::command]
pub async fn ai_list_memories(root_path: String) -> Result<Vec<serde_json::Value>, String> {
    let dir = std::path::Path::new(&root_path).join(".shadow-memory");
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut memories = Vec::new();
    let entries =
        std::fs::read_dir(&dir).map_err(|e| format!("Failed to read memory dir: {}", e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(mut mem) = serde_json::from_str::<serde_json::Value>(&content) {
                mem["_filename"] = serde_json::Value::String(
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                );
                memories.push(mem);
            }
        }
    }
    // Sort by timestamp descending
    memories.sort_by(|a, b| {
        let ta = a["timestamp"].as_u64().unwrap_or(0);
        let tb = b["timestamp"].as_u64().unwrap_or(0);
        tb.cmp(&ta)
    });
    Ok(memories)
}

#[tauri::command]
pub async fn ai_delete_memory(root_path: String, filename: String) -> Result<(), String> {
    // Sanitize filename to prevent path traversal
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return Err("Invalid filename".to_string());
    }
    let path = std::path::Path::new(&root_path)
        .join(".shadow-memory")
        .join(&filename);
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| format!("Failed to delete: {}", e))?;
    }
    Ok(())
}

#[tauri::command]
pub async fn ai_regenerate_memory(root_path: String) -> Result<String, String> {
    super::compaction::generate_memory_md(&root_path);
    let md_path = std::path::Path::new(&root_path)
        .join(".shadowai")
        .join("memory.md");
    if md_path.exists() {
        Ok(format!("memory.md regenerated at {:?}", md_path))
    } else {
        Ok("No compaction records found to generate memory.md from".to_string())
    }
}

// Old session system — kept as stubs for remote/bluetooth compatibility
// Real sessions are in ferrum_bridge::SessionStore
pub fn chat_save_sessions(_json: String) -> Result<(), String> {
    Ok(())
}
pub fn chat_load_sessions_raw() -> String {
    "[]".to_string()
}
