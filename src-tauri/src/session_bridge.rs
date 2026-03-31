//! Session bridge — connects the local Tauri webview to the agent session system.
//!
//! This is the key piece that makes PC and phone share the same chat:
//!   - PC calls `session_join` on startup → gets/creates the primary session
//!   - PC calls `session_chat` → message goes through the session queue
//!   - agent_runner processes it → fires Tauri events (PC sees them) AND
//!     puts events in session buffer (phone sees them via WebSocket)
//!   - When the phone sends a message, agent_runner fires Tauri events too,
//!     and we re-emit them to the PC webview via `session-agent-event`

use crate::agent_queue::UserMessage;
use crate::manager::SessionManager;
use crate::session::Session;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

/// Holds a reference to the primary session for the local PC UI.
/// Created once, shared between all Tauri command invocations.
pub struct PrimarySession {
    pub session: Mutex<Option<Arc<Session>>>,
}

impl PrimarySession {
    pub fn new() -> Self {
        Self {
            session: Mutex::new(None),
        }
    }
}

/// Join (or create) the primary session.
/// Returns the session_id. The PC webview calls this on startup.
#[tauri::command]
pub async fn session_join(
    session_id: Option<String>,
    primary: State<'_, PrimarySession>,
    session_mgr: State<'_, Arc<SessionManager>>,
    app: AppHandle,
) -> Result<serde_json::Value, String> {
    let (session, is_new) = session_mgr.get_or_create(session_id, &app).await;
    let sid = session.id.clone();
    let current_seq = session.current_seq();

    // Store as primary session
    *primary.session.lock().await = Some(session.clone());

    // Subscribe to session events and forward them to the local webview
    // as `session-agent-event` Tauri events. This is how the PC sees
    // messages that originated from the phone.
    let mut event_rx = session.subscribe();
    let handle = app.clone();
    tokio::spawn(async move {
        while let Ok(event) = event_rx.recv().await {
            let _ = handle.emit("session-agent-event", &event);
        }
    });

    log::info!(
        "[session_bridge] PC joined session {} (new: {})",
        sid,
        is_new
    );

    Ok(serde_json::json!({
        "session_id": sid,
        "is_new": is_new,
        "current_seq": current_seq,
    }))
}

/// Send a chat message through the primary session.
/// The agent_runner processes it on the PC, events go to both PC and phone.
#[tauri::command]
pub async fn session_chat(
    stream_id: String,
    messages: serde_json::Value,
    model: Option<String>,
    temperature: Option<f64>,
    max_tokens: Option<i32>,
    tools_enabled: Option<bool>,
    chat_mode: Option<String>,
    root_path: Option<String>,
    primary: State<'_, PrimarySession>,
) -> Result<(), String> {
    let session = primary
        .session
        .lock()
        .await
        .clone()
        .ok_or_else(|| "No primary session — call session_join first".to_string())?;

    // Build the args object that agent_runner expects
    let args = serde_json::json!({
        "streamId": stream_id,
        "messages": messages,
        "model": model,
        "temperature": temperature,
        "maxTokens": max_tokens,
        "toolsEnabled": tools_enabled.unwrap_or(false),
        "chatMode": chat_mode.unwrap_or_else(|| "build".to_string()),
        "rootPath": root_path.unwrap_or_default(),
    });

    let msg = UserMessage::AiChat { stream_id, args };

    session
        .send(msg)
        .await
        .map_err(|e| format!("Failed to send to session: {}", e))
}

/// Get the current primary session info.
#[tauri::command]
pub async fn session_info(primary: State<'_, PrimarySession>) -> Result<serde_json::Value, String> {
    let session = primary.session.lock().await;
    match session.as_ref() {
        Some(s) => Ok(serde_json::json!({
            "session_id": s.id,
            "current_seq": s.current_seq(),
        })),
        None => Ok(serde_json::json!({
            "session_id": null,
            "current_seq": 0,
        })),
    }
}

/// Replay missed events from the primary session (used after reconnect/refresh).
#[tauri::command]
pub async fn session_replay(
    last_seq: u64,
    primary: State<'_, PrimarySession>,
) -> Result<Vec<serde_json::Value>, String> {
    let session = primary
        .session
        .lock()
        .await
        .clone()
        .ok_or_else(|| "No primary session".to_string())?;

    let events = session.events_since(last_seq);
    let json_events: Vec<serde_json::Value> = events
        .into_iter()
        .map(|e| serde_json::to_value(&e).unwrap_or(serde_json::Value::Null))
        .collect();

    Ok(json_events)
}

/// Abort the current AI chat stream in the primary session.
#[tauri::command]
pub async fn session_abort(
    stream_id: String,
    primary: State<'_, PrimarySession>,
) -> Result<(), String> {
    let session = primary
        .session
        .lock()
        .await
        .clone()
        .ok_or_else(|| "No primary session".to_string())?;

    session
        .send(UserMessage::Abort { stream_id })
        .await
        .map_err(|e| format!("Failed to abort: {}", e))
}
