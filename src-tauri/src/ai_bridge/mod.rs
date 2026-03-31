pub mod chat;
pub mod commands;
pub(crate) mod compaction;
pub mod providers;
pub mod stream;
pub mod tools;
pub mod types;

use reqwest::Client;
use std::collections::HashMap;
use std::sync::{atomic::AtomicBool, Arc, Mutex};

// Re-export everything — glob re-exports are needed so Tauri's hidden __cmd__ symbols come through
pub use chat::*;
pub use commands::*;
pub use providers::*;
pub use types::ChatMessage;

const DEFAULT_BASE_URL: &str = "http://localhost:1234/v1";

/// Safely truncate a string to at most `max_bytes` without splitting a multi-byte char.
pub(crate) fn safe_truncate(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Emit an AI event both locally (for Tauri windows) and globally (for remote WebSocket clients).
pub fn emit_ai_event<T: serde::Serialize + Clone>(
    app: &tauri::AppHandle,
    event_name: &str,
    payload: T,
) {
    use tauri::Emitter;
    let _ = app.emit(event_name, payload.clone());
    if let Ok(value) = serde_json::to_value(&payload) {
        let _ = app.emit(
            "ai-remote-relay",
            serde_json::json!({
                "event": event_name,
                "payload": value,
            }),
        );
    }
}

pub struct AiConfig {
    pub base_url: Mutex<String>,
    pub client: Client,
    pub abort_signals: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl AiConfig {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Self {
            base_url: Mutex::new(DEFAULT_BASE_URL.to_string()),
            client,
            abort_signals: Mutex::new(HashMap::new()),
        }
    }
}

#[tauri::command]
pub fn abort_ai_chat(stream_id: String, state: tauri::State<'_, AiConfig>) -> Result<(), String> {
    if let Ok(signals) = state.abort_signals.lock() {
        if let Some(flag) = signals.get(&stream_id) {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    Ok(())
}
