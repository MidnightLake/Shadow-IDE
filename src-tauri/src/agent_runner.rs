use crate::agent_queue::{AgentQueueRx, UserMessage};
use crate::session::Session;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Listener, Manager};

pub static AGENT_PAUSED: AtomicBool = AtomicBool::new(false);

#[tauri::command]
pub fn agent_pause() -> Result<(), String> {
    AGENT_PAUSED.store(true, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub fn agent_resume() -> Result<(), String> {
    AGENT_PAUSED.store(false, Ordering::Relaxed);
    Ok(())
}

#[tauri::command]
pub fn agent_is_paused() -> bool {
    AGENT_PAUSED.load(Ordering::Relaxed)
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct AgentTemplate {
    pub name: String,
    pub description: String,
    pub category: String,
    pub steps: Vec<String>,
    pub tools: Vec<String>,
}

#[tauri::command]
pub async fn list_agent_templates() -> Result<Vec<AgentTemplate>, String> {
    Ok(vec![
        AgentTemplate {
            name: "New Feature".to_string(),
            description: "Implement a new feature end-to-end".to_string(),
            category: "feature".to_string(),
            steps: vec![
                "Read existing code to understand patterns".to_string(),
                "Create implementation plan".to_string(),
                "Write code with tests".to_string(),
                "Update documentation".to_string(),
            ],
            tools: vec![
                "read_file".to_string(),
                "write_file".to_string(),
                "run_tests".to_string(),
            ],
        },
        AgentTemplate {
            name: "Bug Fix".to_string(),
            description: "Diagnose and fix a bug".to_string(),
            category: "bugfix".to_string(),
            steps: vec![
                "Reproduce the bug".to_string(),
                "Find root cause in code".to_string(),
                "Apply minimal fix".to_string(),
                "Verify fix with tests".to_string(),
            ],
            tools: vec![
                "read_file".to_string(),
                "grep_search".to_string(),
                "patch_file".to_string(),
                "run_tests".to_string(),
            ],
        },
        AgentTemplate {
            name: "Refactor".to_string(),
            description: "Refactor code for clarity and maintainability".to_string(),
            category: "refactor".to_string(),
            steps: vec![
                "Identify code smells".to_string(),
                "Plan refactor without changing behavior".to_string(),
                "Apply changes incrementally".to_string(),
                "Run full test suite".to_string(),
            ],
            tools: vec![
                "read_file".to_string(),
                "patch_file".to_string(),
                "run_tests".to_string(),
            ],
        },
        AgentTemplate {
            name: "Release Prep".to_string(),
            description: "Prepare a release: changelog, version bump, tag".to_string(),
            category: "release".to_string(),
            steps: vec![
                "Review git log since last tag".to_string(),
                "Update CHANGELOG.md".to_string(),
                "Bump version in Cargo.toml/package.json".to_string(),
                "Run tests and lint".to_string(),
            ],
            tools: vec![
                "read_file".to_string(),
                "write_file".to_string(),
                "shell_exec".to_string(),
            ],
        },
        AgentTemplate {
            name: "Write Tests".to_string(),
            description: "Generate comprehensive tests for existing code".to_string(),
            category: "test".to_string(),
            steps: vec![
                "Analyze functions to test".to_string(),
                "Write unit tests for each public function".to_string(),
                "Write integration tests for key flows".to_string(),
                "Run tests and fix failures".to_string(),
            ],
            tools: vec![
                "read_file".to_string(),
                "write_file".to_string(),
                "run_tests".to_string(),
            ],
        },
        AgentTemplate {
            name: "Security Audit".to_string(),
            description: "Audit codebase for security vulnerabilities".to_string(),
            category: "security".to_string(),
            steps: vec![
                "Scan for hardcoded secrets".to_string(),
                "Check dependency vulnerabilities".to_string(),
                "Review input validation".to_string(),
                "Report findings".to_string(),
            ],
            tools: vec![
                "grep_search".to_string(),
                "shell_exec".to_string(),
                "read_file".to_string(),
            ],
        },
    ])
}

/// Max tool-call iterations per single run (safety valve).
#[allow(dead_code)]
const MAX_TOOL_CALLS_PER_RUN: usize = 200;

/// If the AI finishes in under this many seconds AND produced no tool calls,
/// it likely stalled — auto-inject "Continue." in auto mode.
#[allow(dead_code)]
const STALL_TIMEOUT_SECS: u64 = 60;

/// Spawn the persistent agent loop for a session.
///
/// This runs forever (until the channel is dropped) — it does NOT stop when
/// the phone disconnects. All AI chat events are emitted through the session's
/// ring buffer, so they survive phone disconnects and get replayed on reconnect.
pub fn spawn_agent(session: Arc<Session>, mut queue_rx: AgentQueueRx, app: AppHandle) {
    let session_id = session.id.clone();
    tokio::spawn(async move {
        log::info!("[agent] Session {} — agent started", session_id);

        while let Some(msg) = queue_rx.recv().await {
            match msg {
                UserMessage::AiChat { stream_id, args } => {
                    run_ai_chat(session.clone(), stream_id, args, app.clone()).await;

                    // After AI chat completes, drain any pending messages that
                    // arrived while we were running (e.g. "keep going" / "continue").
                    // This prevents requiring the user to send a new message.
                    loop {
                        match queue_rx.try_recv() {
                            Some(UserMessage::AiChat { stream_id, args }) => {
                                // Another AI chat request came in while we were busy — run it
                                log::info!(
                                    "[agent] Session {} — absorbed pending AiChat",
                                    session_id
                                );
                                run_ai_chat(session.clone(), stream_id, args, app.clone()).await;
                            }
                            Some(UserMessage::Chat { text }) => {
                                // User sent a "keep going" text message while AI was running
                                // — absorb it, no double-start needed
                                log::info!(
                                    "[agent] Session {} — absorbed pending Chat: {}",
                                    session_id,
                                    &text[..text.len().min(50)]
                                );
                            }
                            Some(other) => {
                                // Handle other message types normally
                                handle_non_chat(session.clone(), other, app.clone()).await;
                            }
                            None => break, // Queue is empty, go back to waiting
                        }
                    }
                }
                other => {
                    handle_non_chat(session.clone(), other, app.clone()).await;
                }
            }
        }

        log::info!(
            "[agent] Session {} — agent stopped (queue closed)",
            session_id
        );
    });
}

/// Handle non-AI-chat messages.
async fn handle_non_chat(session: Arc<Session>, msg: UserMessage, app: AppHandle) {
    match msg {
        UserMessage::Chat { text } => {
            session.emit("user_message", serde_json::json!({ "text": text }));
        }
        UserMessage::Cancel => {
            session.emit(
                "agent_cancelled",
                serde_json::json!({ "reason": "user_cancel" }),
            );
        }
        UserMessage::Abort { stream_id } => {
            let ai_state = app.state::<crate::ai_bridge::AiConfig>();
            let _ = crate::ai_bridge::abort_ai_chat(stream_id.clone(), ai_state);
            session.emit(
                "agent_cancelled",
                serde_json::json!({ "stream_id": stream_id }),
            );
        }
        UserMessage::SetMode { mode } => {
            session.emit("mode_changed", serde_json::json!({ "mode": mode }));
        }
        UserMessage::IndexRag => {
            session.emit("rag_index_started", serde_json::json!({}));
        }
        UserMessage::SwitchFile { path } => {
            session.emit("file_context_switched", serde_json::json!({ "path": path }));
        }
        UserMessage::ApplyDiff { diff } => {
            session.emit("diff_applied", serde_json::json!({ "diff": diff }));
        }
        UserMessage::AiChat { .. } => {
            // Handled in spawn_agent directly
        }
    }
}

/// Run an AI chat with tools on the PC.
///
/// All stream events (tokens, tool calls, results, done) are captured via
/// Tauri event listeners and emitted through `session.emit()`. This means:
///   - Events are stored in the ring buffer (500 events)
///   - Connected phones get them via broadcast in real-time
///   - Disconnected phones get them replayed on reconnect
///   - The AI chat NEVER stops just because the phone disconnected
async fn run_ai_chat(
    session: Arc<Session>,
    stream_id: String,
    args: serde_json::Value,
    app: AppHandle,
) {
    log::info!(
        "[agent] Session {} — AI chat started (stream {})",
        session.id,
        stream_id
    );

    session.emit(
        "agent_thinking",
        serde_json::json!({ "stream_id": stream_id }),
    );

    // Parse args
    let messages: Vec<crate::ai_bridge::ChatMessage> = match serde_json::from_value(
        args.get("messages")
            .cloned()
            .unwrap_or(serde_json::Value::Array(vec![])),
    ) {
        Ok(m) => m,
        Err(e) => {
            let err_msg = format!("Invalid messages JSON: {}", e);
            log::error!("[agent] {}", err_msg);
            session.emit(
                &format!("ai-chat-stream-{}", stream_id),
                serde_json::json!({"content": err_msg}),
            );
            session.emit(
                &format!("ai-chat-done-{}", stream_id),
                serde_json::json!({}),
            );
            return;
        }
    };
    let model = args
        .get("model")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let base_url_override = args
        .get("baseUrlOverride")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let temperature = args.get("temperature").and_then(|v| v.as_f64());
    let max_tokens = args
        .get("maxTokens")
        .and_then(|v| v.as_i64())
        .map(|v| v as i32);
    let tools_enabled = args
        .get("toolsEnabled")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let chat_mode = args
        .get("chatMode")
        .and_then(|v| v.as_str())
        .unwrap_or("build")
        .to_string();
    let root_path = args
        .get("rootPath")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Set up event listeners that route ALL AI events through the session buffer.
    let event_names = vec![
        format!("ai-chat-stream-{}", stream_id),
        format!("ai-chat-done-{}", stream_id),
        format!("ai-chat-stats-{}", stream_id),
        format!("ai-tool-call-{}", stream_id),
        format!("ai-tool-result-{}", stream_id),
        format!("ai-tool-confirm-{}", stream_id),
        format!("ai-tool-stream-{}", stream_id),
        format!("ai-file-change-{}", stream_id),
    ];

    let mut listener_ids = Vec::new();

    for event_name in &event_names {
        let sess = session.clone();
        let ename = event_name.clone();
        let id = app.listen(event_name.clone(), move |event| {
            let payload = serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
            // Emit through session buffer — survives phone disconnect
            sess.emit(&ename, payload);
        });
        listener_ids.push(id);
    }

    // Run the actual AI chat on the PC — this is the heavy lifting
    let ai_state = app.state::<crate::ai_bridge::AiConfig>();
    let token_cache = app.state::<crate::token_optimizer::TokenCache>();
    let warm_cache = app.state::<crate::token_optimizer::WarmCache>();
    let token_settings = app.state::<crate::token_optimizer::TokenSettings>();
    let rag_state = app.state::<std::sync::Arc<crate::rag_index::RagState>>();
    let shadow_config = app.state::<crate::config::ConfigState>();

    let _ = crate::ai_bridge::ai_chat_with_tools(
        stream_id.clone(),
        messages,
        model,
        base_url_override,
        None, // api_key
        temperature,
        max_tokens,
        tools_enabled,
        chat_mode,
        root_path,
        app.clone(),
        ai_state,
        token_cache,
        warm_cache,
        token_settings,
        rag_state,
        shadow_config,
    )
    .await;

    // Emit done event through session buffer
    session.emit("agent_done", serde_json::json!({ "stream_id": stream_id }));

    // Cleanup listeners after a short delay (let final events flush)
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    for id in listener_ids {
        app.unlisten(id);
    }

    log::info!(
        "[agent] Session {} — AI chat done (stream {})",
        session.id,
        stream_id
    );
}
