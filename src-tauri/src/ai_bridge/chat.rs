use futures_util::StreamExt;
use std::collections::HashMap;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use tauri::{AppHandle, Emitter, Listener};

use super::compaction::{
    compact_messages_configured, detect_context_overflow, emit_compaction_event,
    extract_and_save_session_memory,
};
use super::emit_ai_event;
use super::safe_truncate;
use super::types::*;
use super::AiConfig;
use crate::llm_provider::{self, ApiStyle, ToolSchemaFormat};
use crate::token_budget::{BudgetTracker, StateWriter, TaskBudget};
use crate::token_optimizer::{self, TokenCache, TokenSettings, WarmCache};
use crate::tool_calling::{self, FunctionCallData, ToolCall};

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum ConversationState {
    Idle,
    Thinking,
    Streaming,
    ToolExecuting { tool_name: String, attempt: usize },
    HealLoop { error_type: String, attempt: usize },
    Paused,
    Error { message: String },
}

static CONVERSATION_STATE: std::sync::OnceLock<
    std::sync::Arc<std::sync::Mutex<ConversationState>>,
> = std::sync::OnceLock::new();

pub fn conversation_state() -> &'static std::sync::Arc<std::sync::Mutex<ConversationState>> {
    CONVERSATION_STATE
        .get_or_init(|| std::sync::Arc::new(std::sync::Mutex::new(ConversationState::Idle)))
}

fn set_state(state: ConversationState) {
    if let Ok(mut guard) = conversation_state().lock() {
        *guard = state;
    }
}

#[tauri::command]
pub fn get_conversation_state() -> ConversationState {
    conversation_state()
        .lock()
        .map(|g| g.clone())
        .unwrap_or(ConversationState::Idle)
}

/// Legacy wrapper — redirects to ai_chat_with_tools for backward compatibility (BT/remote)
#[tauri::command]
pub async fn ai_chat_stream(
    stream_id: String,
    messages: Vec<ChatMessage>,
    model: Option<String>,
    temperature: Option<f64>,
    max_tokens: Option<i32>,
    app: AppHandle,
    state: tauri::State<'_, AiConfig>,
    cache: tauri::State<'_, TokenCache>,
    warm_cache: tauri::State<'_, WarmCache>,
    settings: tauri::State<'_, TokenSettings>,
    rag_state: tauri::State<'_, std::sync::Arc<crate::rag_index::RagState>>,
    shadow_config: tauri::State<'_, crate::config::ConfigState>,
) -> Result<(), String> {
    ai_chat_with_tools(
        stream_id,
        messages,
        model,
        None,
        None,
        temperature,
        max_tokens,
        false,
        "plan".to_string(),
        "".to_string(),
        app,
        state,
        cache,
        warm_cache,
        settings,
        rag_state,
        shadow_config,
    )
    .await
}

#[tauri::command]
pub async fn ai_chat_with_tools(
    stream_id: String,
    messages: Vec<ChatMessage>,
    model: Option<String>,
    base_url_override: Option<String>,
    api_key: Option<String>,
    temperature: Option<f64>,
    max_tokens: Option<i32>,
    tools_enabled: bool,
    chat_mode: String,
    root_path: String,
    app: AppHandle,
    state: tauri::State<'_, AiConfig>,
    cache: tauri::State<'_, TokenCache>,
    warm_cache: tauri::State<'_, WarmCache>,
    settings: tauri::State<'_, TokenSettings>,
    rag_state: tauri::State<'_, std::sync::Arc<crate::rag_index::RagState>>,
    shadow_config: tauri::State<'_, crate::config::ConfigState>,
) -> Result<(), String> {
    // Load project config from state
    let cfg = shadow_config.lock().map(|c| c.clone()).unwrap_or_default();

    let base_url = if let Some(url) = base_url_override {
        url
    } else {
        state.base_url.lock().map_err(|e| e.to_string())?.clone()
    };

    // Air-gap mode: reject non-local providers
    if cfg.security.air_gap || cfg.privacy_mode {
        let url_lower = base_url.to_lowercase();
        let is_local = url_lower.contains("localhost")
            || url_lower.contains("127.0.0.1")
            || url_lower.contains("0.0.0.0")
            || url_lower.contains("::1");
        if !is_local {
            return Err(
                "Air-gap mode is enabled. Only local providers (Ollama, llama.cpp) are allowed. \
                 Configure openai_base_url to a local endpoint."
                    .to_string(),
            );
        }
    }
    let model_name = model.unwrap_or_else(|| "default".to_string());
    let temp = temperature.unwrap_or(cfg.ai.default_temperature);

    // Local provider health check: if base_url points to a local endpoint that isn't running,
    // automatically fall back to Anthropic API (if key is configured) instead of showing
    // repeated connection errors on every request.
    let (base_url, api_key) = {
        let url_lower = base_url.to_lowercase();
        let is_local = url_lower.contains("localhost")
            || url_lower.contains("127.0.0.1")
            || url_lower.contains("0.0.0.0");
        if is_local {
            // Quick 500ms health check on the /models endpoint
            let health_ok = state
                .client
                .get(format!("{}/models", base_url))
                .timeout(std::time::Duration::from_millis(500))
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false);
            if !health_ok {
                // Local provider unavailable — check for Anthropic fallback
                let anthropic_key = api_key
                    .as_deref()
                    .filter(|k| !k.is_empty())
                    .map(String::from)
                    .or_else(|| {
                        std::env::var("ANTHROPIC_API_KEY")
                            .ok()
                            .filter(|k| !k.is_empty())
                    });
                if let Some(key) = anthropic_key {
                    log::warn!(
                        "[shadow-ai] Local provider at {} is unavailable, falling back to Anthropic API.",
                        base_url
                    );
                    ("https://api.anthropic.com/v1".to_string(), Some(key))
                } else {
                    (base_url, api_key)
                }
            } else {
                (base_url, api_key)
            }
        } else {
            (base_url, api_key)
        }
    };

    let (clean_mode, cache_enabled, configured_max_context) = if let Ok(st) = settings.state.lock()
    {
        (
            st.clean_mode.clone(),
            st.cache_enabled,
            st.max_context_tokens,
        )
    } else {
        (
            token_optimizer::CleanMode::Trim,
            cfg.lmcache.enabled,
            cfg.ai.default_max_context,
        )
    };

    // Query the server's actual n_ctx to prevent exceed_context_size errors.
    // The configured max_context may be larger than what the server supports.
    let server_n_ctx = query_server_context_size(&base_url, &state.client).await;
    let max_context = if let Some(n_ctx) = server_n_ctx {
        if configured_max_context > n_ctx {
            log::info!(
                "[token_budget] Clamping max_context from {} to server n_ctx {}",
                configured_max_context,
                n_ctx
            );
        }
        configured_max_context.min(n_ctx)
    } else {
        configured_max_context
    };

    let abort_flag = Arc::new(AtomicBool::new(false));
    if let Ok(mut signals) = state.abort_signals.lock() {
        signals.insert(stream_id.clone(), abort_flag.clone());
    }

    let mode_instructions = match chat_mode.as_str() {
        "plan" => "\nMODE: PLANNING. Focus on high-level strategy, architecture, and discussion. Do NOT use tools to edit or create files unless the user explicitly asks for a file modification. Discuss your plan before implementing.",
        "auto" => "\nMODE: AUTOMATION. You have full autonomy to complete the user's request. Execute tools in sequence to accomplish the task. If a tool call fails, read the error, investigate, and try an alternative approach. DO NOT STOP until the entire task is finished. Always verify that your changes are correct by reading files after editing. CRITICAL: You MUST keep using tools until everything is done. When you are truly finished with ALL tasks, end your final message with ##DONE## on its own line. Never stop without ##DONE## unless you need user input.",
        _ => "\nMODE: BUILDING. Focus on implementation and code changes. Use tools to read, write, and edit files as needed to fulfill the request. Read files before editing. Verify changes after making them.",
    }.to_string();

    let mut api_messages: Vec<serde_json::Value> = Vec::new();
    let mut sys_added = false;
    for msg in &messages {
        let mut content = msg.content.clone();
        if !sys_added && msg.role == "system" {
            content.push_str(&mode_instructions);
            sys_added = true;
        }
        api_messages.push(serde_json::json!({ "role": msg.role, "content": token_optimizer::clean_context(&content, &clean_mode) }));
    }
    if !sys_added {
        let system = format!(
            "You are ShadowAI, an advanced coding assistant integrated into Shadow IDE.\n\n\
             IMPORTANT RULES:\n\
             1. Always read a file before editing it to see the current content\n\
             2. Use patch_file for small changes, write_file only for new files or complete rewrites\n\
             3. After making changes, verify by reading the file or running a build command\n\
             4. If a tool call fails, read the error and try a different approach\n\
             5. Use grep_search and list_dir to explore the codebase before making assumptions\n\
             6. Provide clear explanations of what you're doing and why\n\n\
             Project root: {}\n\
             {}",
            root_path, mode_instructions
        );
        api_messages.insert(0, serde_json::json!({"role": "system", "content": system}));
    }

    let cfg_min_keep = cfg.compaction.keep_last_turns;
    let cfg_inline_max = cfg.compaction.inline_summary_max_chars;
    let cfg_extract_mem = cfg.compaction.extract_memories;
    let cfg_strategy = cfg.compaction.strategy.clone();

    compact_messages_configured(
        &mut api_messages,
        max_context,
        Some(&root_path),
        cfg_min_keep,
        cfg_inline_max,
        cfg_extract_mem,
        &cfg_strategy,
    );

    // SessionResume trigger: if context is >75% full on first call, compact more aggressively
    let current_tokens = token_optimizer::count_message_tokens(&api_messages);
    if current_tokens > (max_context * 3) / 4 && api_messages.len() > 10 {
        let resume_limit = (max_context * 4) / 5;
        compact_messages_configured(
            &mut api_messages,
            resume_limit,
            Some(&root_path),
            cfg_min_keep,
            cfg_inline_max,
            cfg_extract_mem,
            &cfg_strategy,
        );
    }

    let cache_tools = cfg.lmcache.cache_tool_results;
    let min_tokens_cache = cfg.lmcache.min_tokens_to_cache;
    let force_refresh = chat_mode == "refresh"; // special mode to bypass cache

    if cache_enabled && !force_refresh && (!tools_enabled || cache_tools) {
        let key = token_optimizer::cache_key(
            &serde_json::to_string(&api_messages).unwrap_or_default(),
            &model_name,
            temp,
        );
        // Level 1: hot in-memory cache
        if let Some(c) = cache.get(&key) {
            emit_ai_event(
                &app,
                &format!("ai-chat-stream-{}", stream_id),
                serde_json::json!({"content": c}),
            );
            emit_ai_event(
                &app,
                &format!("ai-chat-done-{}", stream_id),
                serde_json::json!({}),
            );
            return Ok(());
        }
        // Level 2: warm SQLite cache (exact key match)
        if let Some(c) = warm_cache.get(&key) {
            // Promote to hot cache
            cache.put(key, c.clone());
            emit_ai_event(
                &app,
                &format!("ai-chat-stream-{}", stream_id),
                serde_json::json!({"content": c}),
            );
            emit_ai_event(
                &app,
                &format!("ai-chat-done-{}", stream_id),
                serde_json::json!({}),
            );
            return Ok(());
        }
    }

    // Pre-request token budget optimization
    let budget_optimizer = llm_provider::TokenBudgetOptimizer::new(max_context);
    let core_tool_names = tool_calling::get_core_tool_names();

    // Check if the server supports native tool calling
    let tool_defs = if tools_enabled {
        Some(tool_calling::get_tool_definitions())
    } else {
        None
    };

    // Try native tool calling first; if the server rejects tools, fall back to prompt injection
    let mut use_native_tools = tools_enabled;
    let mut fallback_tools_injected = false;

    let mut current_messages = api_messages.clone();
    let max_iterations = if chat_mode == "auto" {
        cfg.ai.max_iterations_auto
    } else {
        cfg.ai.max_iterations_other
    };
    let mut total_output_tokens = 0;
    let mut consecutive_empty = 0;
    let mut heal_attempts = 0u32;
    let max_heal_attempts = cfg.self_healing.max_attempts;
    let mut overflow_retries = 0u32;

    // ── Dynamic token budgeting ──
    // Classify the last user message to determine how many tokens to allocate.
    let last_user_content = messages
        .iter()
        .rev()
        .find(|m| m.role == "user")
        .map(|m| m.content.as_str())
        .unwrap_or("");

    // Load auto-scaling tracker from history, then create a scaled budget
    let state_writer = StateWriter::new(None);
    let tracker = BudgetTracker::load_from_file(&state_writer.state_dir());
    let mut task_budget = TaskBudget::from_prompt_scaled(last_user_content, &tracker);
    let task_id = format!(
        "task-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
    );

    // If caller didn't specify max_tokens, use the dynamic budget
    let max_tokens = max_tokens.or_else(|| Some(task_budget.n_predict as i32));
    if let Err(e) = state_writer.log_task_start(&task_id, &task_budget.task_type) {
        log::warn!("[token_budget] Failed to log task start: {}", e);
    }

    let compact_turn_interval = cfg.compaction.turn_interval;
    for iteration in 0..max_iterations {
        if abort_flag.load(Ordering::SeqCst) {
            break;
        }

        // Emit live token stats so the frontend bar updates during streaming
        {
            let live_tokens = token_optimizer::count_message_tokens(&current_messages);
            emit_ai_event(
                &app,
                &format!("ai-token-stats-{}", stream_id),
                TokenStatsEvent {
                    input_tokens: live_tokens,
                    output_tokens: total_output_tokens,
                    cached: false,
                    cache_stats: cache.stats(),
                    breakdown: None,
                },
            );
        }

        // Token-threshold compaction (always)
        if let Some(stats) = compact_messages_configured(
            &mut current_messages,
            max_context,
            Some(&root_path),
            cfg_min_keep,
            cfg_inline_max,
            cfg_extract_mem,
            &cfg_strategy,
        ) {
            emit_compaction_event(&app, &stream_id, &stats);
        }

        // Turn-count compaction: only if context is actually getting full (>70%)
        if iteration > 0 && iteration % compact_turn_interval == 0 {
            let current_tokens = token_optimizer::count_message_tokens(&current_messages);
            if current_tokens > (max_context * 7) / 10 {
                let aggressive_limit = (max_context * 4) / 5;
                if let Some(stats) = compact_messages_configured(
                    &mut current_messages,
                    aggressive_limit,
                    Some(&root_path),
                    cfg_min_keep,
                    cfg_inline_max,
                    cfg_extract_mem,
                    &cfg_strategy,
                ) {
                    emit_compaction_event(&app, &stream_id, &stats);
                }
            }
        }

        // PreToolCall compaction: if we're about to execute tools and context is >85%, compact
        if iteration > 0 {
            let current_tokens = token_optimizer::count_message_tokens(&current_messages);
            if current_tokens > (max_context * 85) / 100 {
                let pre_tool_limit = (max_context * 3) / 4;
                if let Some(stats) = compact_messages_configured(
                    &mut current_messages,
                    pre_tool_limit,
                    Some(&root_path),
                    cfg_min_keep,
                    cfg_inline_max,
                    cfg_extract_mem,
                    &cfg_strategy,
                ) {
                    emit_compaction_event(&app, &stream_id, &stats);
                }
            }
        }

        // RAG injection on first iteration: query the last user message with structured file blocks
        if iteration == 0 && tools_enabled {
            if let Some(last_user) = current_messages.iter().rev().find(|m| m["role"] == "user") {
                let query = last_user["content"].as_str().unwrap_or("").to_string();
                if !query.is_empty() && query.len() < 2000 {
                    let rag_results = rag_state.search(&query, 5);
                    if !rag_results.is_empty() {
                        let rag_context: Vec<String> = rag_results
                            .iter()
                            .map(|r| {
                                format!(
                                    "<file path=\"{}\" lines=\"{}-{}\">\n{}\n</file>",
                                    r.file_path, r.line_start, r.line_end, r.content
                                )
                            })
                            .collect();
                        // Inject structured RAG context into system message
                        if let Some(sys) = current_messages.first_mut() {
                            if sys["role"] == "system" {
                                let existing = sys["content"].as_str().unwrap_or("");
                                sys["content"] = serde_json::Value::String(format!(
                                    "{}\n\n<codebase-context>\n{}\n</codebase-context>",
                                    existing,
                                    rag_context.join("\n\n")
                                ));
                            }
                        }
                    }
                }
            }
        }

        // ── Hard safety check: prevent exceed_context_size errors ──
        // The configured max_context may exceed the server's actual n_ctx.
        // Compact aggressively if we're within 5% of the limit, and leave
        // room for the response (n_predict / max_tokens).
        {
            let current_tokens = token_optimizer::count_message_tokens(&current_messages);
            let response_reserve = max_tokens.unwrap_or(4096) as usize;
            let hard_limit = max_context.saturating_sub(response_reserve);
            if current_tokens > (hard_limit * 95) / 100 {
                let target = (hard_limit * 3) / 4;
                log::warn!(
                    "[token_budget] Context near limit: {} tokens / {} hard limit (reserve {}). Compacting to {}.",
                    current_tokens, hard_limit, response_reserve, target
                );
                if let Some(stats) = compact_messages_configured(
                    &mut current_messages,
                    target,
                    Some(&root_path),
                    cfg_min_keep,
                    cfg_inline_max,
                    cfg_extract_mem,
                    &cfg_strategy,
                ) {
                    emit_compaction_event(&app, &stream_id, &stats);
                }
            }
        }

        // Build the request — use token budget optimizer to trim cold tools
        let native_tools = if use_native_tools && !fallback_tools_injected {
            let mut tools = tool_defs.clone();
            // Collect recently used tool names from conversation
            let recent_tools: Vec<String> = current_messages
                .iter()
                .filter_map(|m| m.get("tool_calls"))
                .filter_map(|tc| tc.as_array())
                .flatten()
                .filter_map(|tc| tc["function"]["name"].as_str())
                .map(|s| s.to_string())
                .collect();
            // Optimize: drop unused tools when over budget
            if tools.is_some() {
                budget_optimizer.optimize(
                    &mut current_messages,
                    &mut tools,
                    &recent_tools,
                    &core_tool_names,
                );
            }
            tools
        } else {
            None
        };

        // Detect provider API style from base URL
        let provider_config = {
            let url_lower = base_url.to_lowercase();
            if url_lower.contains("anthropic") {
                ApiStyle::Anthropic
            } else if url_lower.contains("generativelanguage.googleapis") {
                ApiStyle::Gemini
            } else {
                ApiStyle::OpenAICompat
            }
        };

        let mut req_builder = match &provider_config {
            ApiStyle::Anthropic => {
                // Build Anthropic-format request
                let tool_schemas = native_tools
                    .as_ref()
                    .map(|t| llm_provider::translate_tools(t, &ToolSchemaFormat::Anthropic));
                let norm_req = llm_provider::NormalizedRequest {
                    messages: current_messages.clone(),
                    model: model_name.clone(),
                    temperature: temp,
                    max_tokens,
                    stream: true,
                    tools: None,
                    system_prompt: None,
                };
                let body =
                    llm_provider::build_anthropic_request(&norm_req, tool_schemas.as_deref());
                state
                    .client
                    .post(format!("{}/messages", base_url))
                    .header("anthropic-version", "2023-06-01")
                    .header("content-type", "application/json")
                    .json(&body)
            }
            ApiStyle::Gemini => {
                // Build Gemini-format request
                let tool_schemas = native_tools
                    .as_ref()
                    .map(|t| llm_provider::translate_tools(t, &ToolSchemaFormat::Gemini));
                let norm_req = llm_provider::NormalizedRequest {
                    messages: current_messages.clone(),
                    model: model_name.clone(),
                    temperature: temp,
                    max_tokens,
                    stream: true,
                    tools: None,
                    system_prompt: None,
                };
                let body = llm_provider::build_gemini_request(&norm_req, tool_schemas.as_deref());
                let method = if true {
                    "streamGenerateContent"
                } else {
                    "generateContent"
                };
                state
                    .client
                    .post(format!(
                        "{}/models/{}:{}?alt=sse",
                        base_url, model_name, method
                    ))
                    .header("content-type", "application/json")
                    .json(&body)
            }
            ApiStyle::OpenAICompat => {
                // Standard OpenAI-compatible request
                let request = ToolChatRequest {
                    model: model_name.clone(),
                    messages: current_messages.clone(),
                    tools: native_tools,
                    stream: true,
                    temperature: Some(temp),
                    max_tokens,
                    stop: Some(vec![
                        "<|eot_id|>".to_string(),
                        "<|im_end|>".to_string(),
                        "<|endoftext|>".to_string(),
                        "</s>".to_string(),
                    ]),
                };
                state
                    .client
                    .post(format!("{}/chat/completions", base_url))
                    .json(&request)
            }
        };

        if let Some(ref key) = api_key {
            if !key.is_empty() {
                if matches!(provider_config, ApiStyle::Anthropic) {
                    req_builder = req_builder.header("x-api-key", key.as_str());
                } else if matches!(provider_config, ApiStyle::Gemini) {
                    // Gemini uses query param, already in URL via api_key query param
                    // But also support header
                    req_builder = req_builder.header("x-goog-api-key", key.as_str());
                } else {
                    req_builder = req_builder.header("Authorization", format!("Bearer {}", key));
                }
            }
        }
        set_state(ConversationState::Thinking);
        let _ = app.emit("conversation-state", get_conversation_state());
        let response = match req_builder.send().await {
            Ok(r) => r,
            Err(e) => {
                set_state(ConversationState::Error {
                    message: e.to_string(),
                });
                let _ = app.emit("conversation-state", get_conversation_state());
                emit_ai_event(
                    &app,
                    &format!("ai-chat-stream-{}", stream_id),
                    serde_json::json!({"content": format!("\n[Connection error: {}]\n", e)}),
                );
                break;
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let err_text = response.text().await.unwrap_or_default();

            // If we got a 400/422 and we were using native tools, the server doesn't support them
            // Fall back to prompt injection
            if use_native_tools
                && !fallback_tools_injected
                && (status.as_u16() == 400 || status.as_u16() == 422 || status.as_u16() == 500)
                && (err_text.contains("tools")
                    || err_text.contains("tool")
                    || err_text.contains("function"))
            {
                use_native_tools = false;
                fallback_tools_injected = true;

                // Inject core tool schemas only into the system prompt (smaller = more reliable in fallback)
                let core_tools = tool_calling::get_core_tool_definitions();
                let tool_prompt = tool_calling::build_tool_injection_prompt(&core_tools);
                // Prepend to system message
                if let Some(sys) = current_messages.first_mut() {
                    if sys["role"] == "system" {
                        let existing = sys["content"].as_str().unwrap_or("");
                        sys["content"] =
                            serde_json::Value::String(format!("{}\n\n{}", tool_prompt, existing));
                    }
                }
                // Retry this iteration with prompt injection
                continue;
            }

            // ── Context overflow recovery ──
            // Detect exceed_context_size_error (400/413) and compact aggressively instead of crashing
            if let Some((n_prompt, n_ctx_reported)) = detect_context_overflow(&err_text) {
                overflow_retries += 1;

                if overflow_retries > 3 {
                    // Exhausted all overflow recovery attempts — break instead of looping forever
                    emit_ai_event(
                        &app,
                        &format!("ai-chat-stream-{}", stream_id),
                        serde_json::json!({"content": "\n[Context overflow: recovery failed after 3 attempts. Ending session.]\n"}),
                    );
                    break;
                }

                let overage = if n_prompt > 0 && n_ctx_reported > 0 {
                    n_prompt.saturating_sub(n_ctx_reported)
                } else {
                    max_context / 4
                };
                log::warn!(
                    "[shadow-ai] Context overflow attempt {}/3 (prompt={}, ctx={}, overage={})",
                    overflow_retries,
                    n_prompt,
                    n_ctx_reported,
                    overage
                );

                // Increasingly aggressive compaction per retry
                match overflow_retries {
                    1 => {
                        // Attempt 1: compact to 50%
                        emit_ai_event(
                            &app,
                            &format!("ai-chat-stream-{}", stream_id),
                            serde_json::json!({"content": format!(
                                "\n[Context overflow: {} over — compacting to 50% (attempt 1/3)...]\n", overage
                            )}),
                        );
                        let target = max_context / 2;
                        if let Some(stats) = compact_messages_configured(
                            &mut current_messages,
                            target,
                            Some(&root_path),
                            cfg_min_keep,
                            cfg_inline_max,
                            cfg_extract_mem,
                            &cfg_strategy,
                        ) {
                            emit_compaction_event(&app, &stream_id, &stats);
                        }
                    }
                    2 => {
                        // Attempt 2: compact to 25%, keep only last 4 messages
                        emit_ai_event(
                            &app,
                            &format!("ai-chat-stream-{}", stream_id),
                            serde_json::json!({"content": "\n[Context overflow: aggressive compaction to 25% (attempt 2/3)...]\n"}),
                        );
                        let target = max_context / 4;
                        if let Some(stats) = compact_messages_configured(
                            &mut current_messages,
                            target,
                            Some(&root_path),
                            4,
                            cfg_inline_max,
                            cfg_extract_mem,
                            &cfg_strategy,
                        ) {
                            emit_compaction_event(&app, &stream_id, &stats);
                        }
                    }
                    _ => {
                        // Attempt 3: hard reset — keep only system + last user message
                        emit_ai_event(
                            &app,
                            &format!("ai-chat-stream-{}", stream_id),
                            serde_json::json!({"content": "\n[Context overflow: hard reset — keeping only system prompt + last message (attempt 3/3)...]\n"}),
                        );
                        // Save everything before nuking
                        extract_and_save_session_memory(&current_messages, &root_path);
                        // Keep system message (index 0) and last user message only
                        let last_msg = current_messages.last().cloned();
                        current_messages.retain(|m| m["role"] == "system");
                        if let Some(msg) = last_msg {
                            if msg["role"] != "system" {
                                current_messages.push(msg);
                            }
                        }
                    }
                }
                continue;
            }

            // ── Tool call JSON parse error recovery ──
            // Uses a strategy selector instead of blindly retrying the same broken output
            let is_tool_json_error = status.as_u16() == 500
                && (err_text.contains("parse tool call arguments")
                    || err_text.contains("parse_error")
                    || err_text.contains("missing closing quote"));

            if is_tool_json_error && heal_attempts < max_heal_attempts {
                heal_attempts += 1;

                // Strategy selector based on attempt number
                let (strategy_name, heal_prompt) = match heal_attempts {
                    1 => ("RegenerateWithConstraint",
                        "Your last tool call had malformed JSON — the content string was too long and got \
                        truncated mid-response (the server saw a missing closing quote at ~16k chars). \
                        RULES FOR THIS RETRY:\n\
                        1. Do NOT try to write the entire file in one tool call\n\
                        2. Max ~120 lines of content per write_file or patch_file call\n\
                        3. If the path you used was a directory (e.g. '/home/user/MyProject'), \
                           add a filename: '/home/user/MyProject/PLAN.md'\n\
                        4. Write a skeleton/outline first, then patch in sections one at a time".to_string()),
                    2 => ("FallbackToChunkedWrite",
                        "Your tool call JSON is still failing (content too large). Mandatory chunked strategy:\n\
                        1. FIRST: write_file with ONLY the file header + section headings (< 30 lines)\n\
                        2. THEN: use patch_file to append each section, one at a time (max 80 lines each)\n\
                        3. Check the path — if it points to a directory, append a filename like '/PLAN.md'\n\
                        4. Never put more than ~10,000 characters in a single JSON string argument".to_string()),
                    _ => ("SkipAndContinue",
                        "The file write has failed too many times due to JSON size limits. \
                        Instead of writing the file now:\n\
                        1. Tell the user: 'I need to write PLAN.md in chunks — the content is too large for a single call'\n\
                        2. Ask: 'Shall I proceed writing it section by section?'\n\
                        3. Do NOT attempt another large write_file call".to_string()),
                };

                let _ = app.emit(
                    &format!("ai-chat-stream-{}", stream_id),
                    serde_json::json!({"content": format!(
                        "\n[Tool call JSON error — healing {}/{} (strategy: {})]\n",
                        heal_attempts, max_heal_attempts, strategy_name
                    )}),
                );
                current_messages.push(serde_json::json!({
                    "role": "assistant",
                    "content": "(attempted tool call but JSON was too large and got truncated)"
                }));
                current_messages.push(serde_json::json!({
                    "role": "user",
                    "content": heal_prompt
                }));
                continue;
            }

            let _ = app.emit(
                &format!("ai-chat-stream-{}", stream_id),
                serde_json::json!({"content": format!("\n[API Error ({}): {}]\n", status, safe_truncate(&err_text, 500))}),
            );
            break;
        }

        // Success — reset overflow retry counter
        overflow_retries = 0;

        // Stream the response
        set_state(ConversationState::Streaming);
        let _ = app.emit("conversation-state", get_conversation_state());
        let mut stream = response.bytes_stream();
        let mut tool_call_acc: HashMap<usize, ToolCallAccumulator> = HashMap::new();
        let mut content_acc = String::new();
        let mut partial_line = String::new();
        let mut in_think = false;
        let mut think_buf = String::new();
        let mut finish_reason: Option<String> = None;

        while let Some(chunk_res) = stream.next().await {
            if abort_flag.load(Ordering::SeqCst) {
                break;
            }
            let chunk = match chunk_res {
                Ok(c) => c,
                Err(_) => break,
            };
            partial_line.push_str(&String::from_utf8_lossy(&chunk));
            let mut lines: Vec<String> = partial_line.split('\n').map(|s| s.to_string()).collect();
            partial_line = if !chunk.ends_with(&[b'\n']) {
                lines.pop().unwrap_or_default()
            } else {
                String::new()
            };

            for line in lines {
                if !line.starts_with("data: ") {
                    continue;
                }
                let data = &line[6..];
                if data == "[DONE]" {
                    break;
                }
                if let Ok(c) = serde_json::from_str::<StreamChunk>(data) {
                    for choice in c.choices {
                        if let Some(fr) = choice.finish_reason {
                            finish_reason = Some(fr);
                        }
                        if let Some(content) = choice.delta.content {
                            content_acc.push_str(&content);
                            let combined = format!("{}{}", think_buf, content);
                            think_buf.clear();

                            if !in_think && combined.contains("<think>") {
                                if let Some(p) = combined.find("<think>") {
                                    let b = &combined[..p];
                                    if !b.is_empty() {
                                        let _ = app.emit(
                                            &format!("ai-chat-stream-{}", stream_id),
                                            serde_json::json!({"content": b}),
                                        );
                                    }
                                    in_think = true;
                                }
                            }
                            if in_think {
                                if let Some(p) = combined.find("</think>") {
                                    let t = &combined[..p];
                                    emit_ai_event(&app, &format!("ai-chat-think-{}", stream_id), t);
                                    in_think = false;
                                    let a = &combined[p + 8..];
                                    if !a.is_empty() {
                                        let _ = app.emit(
                                            &format!("ai-chat-stream-{}", stream_id),
                                            serde_json::json!({"content": a}),
                                        );
                                    }
                                } else {
                                    emit_ai_event(
                                        &app,
                                        &format!("ai-chat-think-{}", stream_id),
                                        combined,
                                    );
                                }
                            } else {
                                emit_ai_event(
                                    &app,
                                    &format!("ai-chat-stream-{}", stream_id),
                                    serde_json::json!({"content": combined}),
                                );
                            }
                            total_output_tokens += 1;
                        }
                        if let Some(tc) = choice.delta.tool_calls {
                            for t in tc {
                                let a = tool_call_acc.entry(t.index).or_insert_with(|| {
                                    ToolCallAccumulator {
                                        id: String::new(),
                                        name: String::new(),
                                        arguments: String::new(),
                                    }
                                });
                                if let Some(id) = t.id {
                                    a.id = id;
                                }
                                if let Some(f) = t.function {
                                    if let Some(n) = f.name {
                                        a.name.push_str(&n);
                                    }
                                    if let Some(arg) = f.arguments {
                                        a.arguments.push_str(&arg);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Collect tool calls from native API
        let mut calls: Vec<ToolCall> = tool_call_acc
            .into_iter()
            .filter(|(_, a)| !a.name.is_empty())
            .map(|(_, a)| ToolCall {
                id: if a.id.is_empty() {
                    format!("call_{}", iteration)
                } else {
                    a.id
                },
                call_type: Some("function".to_string()),
                function: FunctionCallData {
                    name: a.name,
                    arguments: a.arguments,
                },
            })
            .collect();

        // If no native tool calls found, try extracting from text (fallback mode)
        if calls.is_empty() && tools_enabled && !content_acc.is_empty() {
            let text_calls = tool_calling::extract_tool_calls_from_text(&content_acc);
            if !text_calls.is_empty() {
                calls = text_calls;
                // Mark that we're using fallback mode for future iterations
                if !fallback_tools_injected {
                    fallback_tools_injected = true;
                    use_native_tools = false;
                }
            }
        }

        if calls.is_empty() {
            // No tool calls — cache the response if conditions are met
            if cache_enabled && !content_acc.is_empty() && (!tools_enabled || cache_tools) {
                // Check min_tokens_to_cache: estimate tokens as chars/4
                let approx_tokens = content_acc.len() / 4;
                if approx_tokens >= min_tokens_cache {
                    let ck = token_optimizer::cache_key(
                        &serde_json::to_string(&api_messages).unwrap_or_default(),
                        &model_name,
                        temp,
                    );
                    // Level 1: hot cache
                    cache.put(ck.clone(), content_acc.clone());
                    // Level 2: warm SQLite cache (write-through)
                    let summary = token_optimizer::prompt_summary(
                        &serde_json::to_string(&api_messages).unwrap_or_default(),
                    );
                    warm_cache.put(
                        ck,
                        content_acc.clone(),
                        &model_name,
                        &summary,
                        None, // embedding generated separately via rag_embed_chunks
                        approx_tokens,
                    );
                }
            }

            // AUTOMATION MODE: prompt to continue if needed
            if chat_mode == "auto" && iteration < max_iterations - 1 {
                if finish_reason == Some("length".to_string()) {
                    // Hit token limit — prompt to continue
                    consecutive_empty = 0;
                    current_messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": if content_acc.is_empty() { "..." } else { &content_acc }
                    }));
                    current_messages.push(serde_json::json!({
                        "role": "user",
                        "content": "Continue. If you're done, say so."
                    }));
                    continue;
                }

                if content_acc.trim().is_empty() {
                    consecutive_empty += 1;
                    if consecutive_empty >= 3 {
                        // Model is stuck — break the loop
                        emit_ai_event(
                            &app,
                            &format!("ai-chat-stream-{}", stream_id),
                            serde_json::json!({"content": "\n[Auto mode: stopped after 3 empty responses]\n"}),
                        );
                        break;
                    }
                    current_messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": "..."
                    }));
                    current_messages.push(serde_json::json!({
                        "role": "user",
                        "content": "You stopped without completing the task. Continue using tools to accomplish the goal, or explain what's blocking you."
                    }));
                    continue;
                }

                // Non-empty response with no tool calls in auto mode:
                // Only stop if the AI uses an explicit magic done token.
                // Check only the very last 100 chars to avoid false positives from echoed content.
                let tail: String = content_acc
                    .chars()
                    .rev()
                    .take(100)
                    .collect::<String>()
                    .chars()
                    .rev()
                    .collect();
                let lower = tail.to_lowercase();
                let is_done = lower.contains("##done##") || lower.contains("agentdone");

                if !is_done {
                    // AI stopped mid-task without using tools — nudge it to continue
                    consecutive_empty = 0;
                    current_messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": &content_acc
                    }));
                    current_messages.push(serde_json::json!({
                        "role": "user",
                        "content": "Continue. Use tools to accomplish the task. Do not stop until the task is fully complete. When you are truly done with ALL tasks, end your message with ##DONE##"
                    }));
                    continue;
                }
                // AI explicitly said ##DONE## — break
            }
            break;
        }

        // Reset empty counter when we get tool calls
        consecutive_empty = 0;

        // Add assistant message with tool calls to history
        let assistant_content = if content_acc.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::Value::String(content_acc.clone())
        };

        if use_native_tools && !fallback_tools_injected {
            // Native mode: include tool_calls in the assistant message
            current_messages.push(serde_json::json!({
                "role": "assistant",
                "content": assistant_content,
                "tool_calls": calls
            }));
        } else {
            // Fallback mode: just add the assistant text as-is
            current_messages.push(serde_json::json!({
                "role": "assistant",
                "content": if content_acc.is_empty() { "Calling tools...".to_string() } else { content_acc.clone() }
            }));
        }

        // Execute tool calls — parallel for safe tools, sequential with confirmation for risky tools
        let mut heal_needed = false;

        // Update state to ToolExecuting for the first tool call
        if let Some(first_call) = calls.first() {
            set_state(ConversationState::ToolExecuting {
                tool_name: first_call.function.name.clone(),
                attempt: heal_attempts as usize,
            });
            let _ = app.emit("conversation-state", get_conversation_state());
        }

        // Emit all tool-call events first
        for call in &calls {
            emit_ai_event(
                &app,
                &format!("ai-tool-call-{}", stream_id),
                ToolCallEvent {
                    name: call.function.name.clone(),
                    arguments: call.function.arguments.clone(),
                },
            );
        }

        // Separate into safe (auto-execute) and risky (needs confirmation) tools
        let mut safe_calls: Vec<(usize, &tool_calling::ToolCall)> = Vec::new();
        let mut risky_calls: Vec<(usize, &tool_calling::ToolCall)> = Vec::new();
        for (i, call) in calls.iter().enumerate() {
            let risk = tool_calling::get_tool_risk_level(&call.function.name);
            match risk {
                tool_calling::RiskLevel::High | tool_calling::RiskLevel::Medium => {
                    risky_calls.push((i, call));
                }
                _ => {
                    safe_calls.push((i, call));
                }
            }
        }

        // Execute safe tools in parallel
        let semaphore = std::sync::Arc::new(tokio::sync::Semaphore::new(4));
        let mut all_results: Vec<(usize, tool_calling::ToolExecution)> = Vec::new();

        let mut handles = Vec::new();
        for (idx, call) in &safe_calls {
            let call_clone = (*call).clone();
            let root_clone = root_path.clone();
            let rag_clone = rag_state.inner().clone();
            let sem = semaphore.clone();
            let abort = abort_flag.clone();
            let i = *idx;
            let app_clone = app.clone();
            let sid = stream_id.clone();
            handles.push(tokio::spawn(async move {
                if abort.load(Ordering::SeqCst) {
                    return (
                        i,
                        tool_calling::ToolExecution {
                            name: call_clone.function.name.clone(),
                            result: "Aborted".to_string(),
                            success: false,
                            duration_ms: 0,
                        },
                    );
                }
                let Ok(_permit) = sem.acquire().await else {
                    return (
                        i,
                        tool_calling::ToolExecution {
                            name: call_clone.function.name.clone(),
                            result: "Failed to acquire semaphore".to_string(),
                            success: false,
                            duration_ms: 0,
                        },
                    );
                };
                let tool_name = call_clone.function.name.clone();
                let res = tokio::task::spawn_blocking(move || {
                    let cb: tool_calling::OutputCallback = Box::new(move |chunk: &str| {
                        emit_ai_event(
                            &app_clone,
                            &format!("ai-tool-stream-{}", sid),
                            serde_json::json!({ "tool": tool_name, "chunk": chunk }),
                        );
                    });
                    tool_calling::execute_tool_with_rag_streaming(
                        &call_clone,
                        &root_clone,
                        Some(&rag_clone),
                        Some(&cb),
                    )
                })
                .await
                .unwrap_or_else(|e| tool_calling::ToolExecution {
                    name: "unknown".to_string(),
                    result: format!("Error: task panicked: {}", e),
                    success: false,
                    duration_ms: 0,
                });
                (i, res)
            }));
        }
        let safe_results = futures_util::future::join_all(handles).await;
        for jr in safe_results {
            if let Ok(r) = jr {
                all_results.push(r);
            }
        }

        // Execute risky tools sequentially with confirmation
        for (idx, call) in &risky_calls {
            if abort_flag.load(Ordering::SeqCst) {
                break;
            }

            let risk = tool_calling::get_tool_risk_level(&call.function.name);

            // In AUTO mode, skip confirmation and auto-approve all tools
            let approved = if chat_mode == "auto" {
                // Still emit the tool call info for UI display (no confirmation needed)
                emit_ai_event(
                    &app,
                    &format!("ai-tool-confirm-{}", stream_id),
                    serde_json::json!({
                        "name": call.function.name,
                        "arguments": call.function.arguments,
                        "risk": format!("{:?}", risk),
                        "auto_approved": true,
                    }),
                );
                true
            } else {
                // Emit confirmation request
                emit_ai_event(
                    &app,
                    &format!("ai-tool-confirm-{}", stream_id),
                    serde_json::json!({
                        "name": call.function.name,
                        "arguments": call.function.arguments,
                        "risk": format!("{:?}", risk),
                    }),
                );

                // Wait for confirmation response (with 30s timeout)
                let confirm_event = format!("ai-tool-confirm-response-{}", stream_id);
                let (tx, rx) = tokio::sync::oneshot::channel::<bool>();
                let tx = std::sync::Arc::new(std::sync::Mutex::new(Some(tx)));
                let tx_clone = tx.clone();

                let _unlisten = app.listen(&confirm_event, move |event| {
                    let approved = serde_json::from_str::<serde_json::Value>(event.payload())
                        .ok()
                        .and_then(|v| v["approved"].as_bool())
                        .unwrap_or(true); // default to approved if parse fails
                    if let Some(sender) = tx_clone.lock().ok().and_then(|mut g| g.take()) {
                        let _ = sender.send(approved);
                    }
                });

                tokio::select! {
                    result = rx => result.unwrap_or(true),
                    _ = tokio::time::sleep(std::time::Duration::from_secs(30)) => {
                        // Auto-approve on timeout (user might not have confirmation UI)
                        true
                    }
                }
            };

            if !approved {
                all_results.push((
                    *idx,
                    tool_calling::ToolExecution {
                        name: call.function.name.clone(),
                        result: "Tool execution denied by user".to_string(),
                        success: false,
                        duration_ms: 0,
                    },
                ));
                continue;
            }

            let call_clone = (*call).clone();
            let root_clone = root_path.clone();
            let rag_clone = rag_state.inner().clone();
            let app_clone = app.clone();
            let sid = stream_id.clone();
            let tool_name = call.function.name.clone();
            let res = tokio::task::spawn_blocking(move || {
                let cb: tool_calling::OutputCallback = Box::new(move |chunk: &str| {
                    emit_ai_event(
                        &app_clone,
                        &format!("ai-tool-stream-{}", sid),
                        serde_json::json!({ "tool": tool_name, "chunk": chunk }),
                    );
                });
                tool_calling::execute_tool_with_rag_streaming(
                    &call_clone,
                    &root_clone,
                    Some(&rag_clone),
                    Some(&cb),
                )
            })
            .await
            .unwrap_or_else(|e| tool_calling::ToolExecution {
                name: call.function.name.clone(),
                result: format!("Error: task panicked: {}", e),
                success: false,
                duration_ms: 0,
            });
            all_results.push((*idx, res));
        }

        // Sort results by original index to maintain order
        all_results.sort_by_key(|(i, _)| *i);
        let results: Vec<tool_calling::ToolExecution> =
            all_results.into_iter().map(|(_, r)| r).collect();
        for (i, res) in results.into_iter().enumerate() {
            let call = &calls[i];

            let display_result = if res.result.chars().count() > 2000 {
                let s: String = res.result.chars().take(2000).collect();
                format!("{}...", s)
            } else {
                res.result.clone()
            };

            emit_ai_event(
                &app,
                &format!("ai-tool-result-{}", stream_id),
                ToolResultEvent {
                    name: res.name.clone(),
                    result: display_result,
                    success: res.success,
                    duration_ms: res.duration_ms,
                },
            );

            // Emit structured file-change event for file-modifying tools
            if res.success {
                let parsed_args: serde_json::Value = serde_json::from_str(&call.function.arguments)
                    .unwrap_or(serde_json::Value::Null);
                let file_tool_action = match res.name.as_str() {
                    "write_file" | "create_file" => {
                        let path = parsed_args
                            .get("path")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        let action = if res.result.starts_with("File created") {
                            "created"
                        } else {
                            "updated"
                        };
                        Some((path, action.to_string()))
                    }
                    "patch_file" => {
                        let path = parsed_args
                            .get("path")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        Some((path, "patched".to_string()))
                    }
                    "delete_file" => {
                        let path = parsed_args
                            .get("path")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown")
                            .to_string();
                        Some((path, "deleted".to_string()))
                    }
                    _ => None,
                };

                if let Some((path, action)) = file_tool_action {
                    // Extract the diff/preview portion (everything after first newline)
                    let preview = res
                        .result
                        .find('\n')
                        .map(|idx| res.result[idx + 1..].to_string())
                        .unwrap_or_default();
                    emit_ai_event(
                        &app,
                        &format!("ai-file-change-{}", stream_id),
                        FileChangeEvent {
                            tool: res.name.clone(),
                            path: path.clone(),
                            action: action.clone(),
                            preview: if preview.len() > 3000 {
                                format!("{}...", safe_truncate(&preview, 3000))
                            } else {
                                preview
                            },
                        },
                    );

                    // Emit ghost diff event for live preview overlay in editor
                    if action == "patched" || action == "updated" || action == "created" {
                        // Read the file's new content after the tool wrote it
                        if let Ok(new_content) = crate::fs_commands::read_file_content(path.clone())
                        {
                            emit_ai_event(
                                &app,
                                "ai-ghost-diff",
                                serde_json::json!({
                                    "path": path,
                                    "action": action,
                                    "newContent": new_content,
                                }),
                            );
                        }
                    }
                }
            }

            // Inject result into conversation
            if use_native_tools && !fallback_tools_injected {
                current_messages.push(serde_json::json!({
                    "role": "tool",
                    "tool_call_id": call.id,
                    "content": res.result
                }));
            } else {
                current_messages.push(serde_json::json!({
                    "role": "user",
                    "content": format!(
                        "[TOOL RESULT: {}]\n{}\n[END TOOL RESULT]",
                        res.name, res.result
                    )
                }));
            }

            if !res.success && tool_calling::is_fixable_error(&res) {
                heal_needed = true;
            }
        }

        // Active self-healing: inject heal prompt when fixable errors detected
        if heal_needed && heal_attempts < max_heal_attempts {
            heal_attempts += 1;
            // Find the last failed tool result for context
            let failed_tool = calls
                .iter()
                .rev()
                .find(|_c| true)
                .map(|c| c.function.name.clone())
                .unwrap_or_default();
            let last_error = current_messages
                .last()
                .and_then(|m| m["content"].as_str())
                .unwrap_or("Unknown error")
                .to_string();
            let heal_prompt = tool_calling::build_heal_prompt(
                "Fix the error and try again",
                &failed_tool,
                &last_error,
                heal_attempts as usize,
                max_heal_attempts as usize,
            );
            current_messages.push(serde_json::json!({
                "role": "user",
                "content": heal_prompt
            }));
            let strategy_name =
                tool_calling::heal_strategy_name(&last_error, &failed_tool, heal_attempts as usize);
            let _ = app.emit(
                &format!("ai-chat-stream-{}", stream_id),
                serde_json::json!({ "content": format!("\n🔧 Self-healing attempt {}/{} (strategy: {})\n", heal_attempts, max_heal_attempts, strategy_name) }),
            );
        } else if heal_needed && heal_attempts >= max_heal_attempts {
            // Healing exhausted — reset counter and tell AI to try a different approach
            heal_attempts = 0;
            current_messages.push(serde_json::json!({
                "role": "user",
                "content": "The previous approach failed after multiple attempts. Try a completely different approach to accomplish the task. Do not repeat the same failing strategy. Move on to the next step if this step cannot be completed, and continue working on the remaining tasks."
            }));
            let _ = app.emit(
                &format!("ai-chat-stream-{}", stream_id),
                serde_json::json!({ "content": "\n⚡ Healing exhausted — switching strategy\n" }),
            );
        }

        // content_acc must be reset for next iteration
    }

    set_state(ConversationState::Idle);
    let _ = app.emit("conversation-state", get_conversation_state());
    emit_ai_event(
        &app,
        &format!("ai-chat-done-{}", stream_id),
        serde_json::json!({}),
    );

    // Global notification for remote/mobile clients
    let _ = app.emit("ai-chat-complete-notify", serde_json::json!({
        "title": "ShadowAI",
        "body": "AI response complete",
        "stream_id": stream_id,
        "timestamp": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    }));

    // Desktop notification (Linux: notify-send, macOS: osascript, Windows: PowerShell toast)
    #[cfg(target_os = "linux")]
    {
        let _ = std::process::Command::new("notify-send")
            .args([
                "--app-name=ShadowIDE",
                "--icon=dialog-information",
                "ShadowAI",
                "AI response complete",
            ])
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("osascript")
            .args([
                "-e",
                "display notification \"AI response complete\" with title \"ShadowAI\"",
            ])
            .spawn();
    }
    #[cfg(target_os = "windows")]
    {
        let mut cmd = std::process::Command::new("powershell");
        cmd.args([
            "-WindowStyle", "Hidden", "-Command",
            "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null; \
             $template = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
             $textNodes = $template.GetElementsByTagName('text'); \
             $textNodes.Item(0).AppendChild($template.CreateTextNode('ShadowAI')) > $null; \
             $textNodes.Item(1).AppendChild($template.CreateTextNode('AI response complete')) > $null; \
             $toast = [Windows.UI.Notifications.ToastNotification]::new($template); \
             [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('ShadowIDE').Show($toast)",
        ]);
        crate::platform::hide_window(&mut cmd);
        let _ = cmd.spawn();
    }

    // Calculate per-category token breakdown
    let mut sys_tokens = 0usize;
    let mut tool_tokens = 0usize;
    let mut hist_tokens = 0usize;
    for msg in &current_messages {
        let role = msg["role"].as_str().unwrap_or("");
        let count = token_optimizer::count_message_tokens(&[msg.clone()]);
        match role {
            "system" => sys_tokens += count,
            "tool" => tool_tokens += count,
            _ => hist_tokens += count,
        }
    }
    // Also count tool schema tokens if tools were injected
    if tools_enabled {
        let schema_estimate = if use_native_tools && !fallback_tools_injected {
            // Native tools: estimate ~50 tokens per tool definition
            tool_calling::get_tool_definitions().len() * 50
        } else if fallback_tools_injected {
            // Prompt-injected tools are in the system message already
            0
        } else {
            0
        };
        tool_tokens += schema_estimate;
    }

    let stats = TokenStatsEvent {
        input_tokens: sys_tokens + tool_tokens + hist_tokens,
        output_tokens: total_output_tokens,
        cached: false,
        cache_stats: cache.stats(),
        breakdown: Some(TokenBreakdown {
            system: sys_tokens,
            tools: tool_tokens,
            history: hist_tokens,
            response: total_output_tokens,
        }),
    };
    emit_ai_event(&app, &format!("ai-token-stats-{}", stream_id), stats);
    if let Ok(mut signals) = state.abort_signals.lock() {
        signals.remove(&stream_id);
    }

    // ── Token cost tracking ──
    let prompt_tokens = sys_tokens + tool_tokens + hist_tokens;
    let completion_tokens = total_output_tokens;
    let model_lower = model_name.to_lowercase();
    let (input_price_per_1k, output_price_per_1k) = if model_lower.contains("claude-sonnet") {
        (0.003f64, 0.015f64)
    } else if model_lower.contains("claude-opus") {
        (0.015f64, 0.075f64)
    } else if model_lower.contains("claude-haiku") {
        (0.00025f64, 0.00125f64)
    } else if model_lower.contains("gpt-4o") {
        (0.0025f64, 0.01f64)
    } else {
        (0.001f64, 0.001f64)
    };
    let cost_usd = prompt_tokens as f64 * input_price_per_1k / 1000.0
        + completion_tokens as f64 * output_price_per_1k / 1000.0;

    // Append to spend.json
    if !root_path.is_empty() {
        let spend_dir = std::path::Path::new(&root_path).join(".shadowai");
        let _ = std::fs::create_dir_all(&spend_dir);
        let spend_file = spend_dir.join("spend.json");
        let entry = serde_json::json!({
            "date": "2026-03-22",
            "model": model_name,
            "prompt_tokens": prompt_tokens,
            "completion_tokens": completion_tokens,
            "cost_usd": cost_usd,
        });
        // Read existing, append, write back
        let mut entries: Vec<serde_json::Value> = std::fs::read_to_string(&spend_file)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        entries.push(entry);
        if let Ok(serialized) = serde_json::to_string_pretty(&entries) {
            let _ = std::fs::write(&spend_file, serialized);
        }

        // Calculate session and daily totals
        let today = "2026-03-22";
        let daily_cost: f64 = entries
            .iter()
            .filter(|e| e["date"].as_str() == Some(today))
            .filter_map(|e| e["cost_usd"].as_f64())
            .sum();
        let total_tokens_all: u64 = entries
            .iter()
            .filter_map(|e| {
                let p = e["prompt_tokens"].as_u64().unwrap_or(0);
                let c = e["completion_tokens"].as_u64().unwrap_or(0);
                Some(p + c)
            })
            .sum();

        let _ = app.emit(
            "ai-cost-update",
            serde_json::json!({
                "session_cost": cost_usd,
                "daily_cost": daily_cost,
                "total_tokens": total_tokens_all,
            }),
        );
    }

    // ── Log task completion + flush budget ──
    task_budget.record_usage(total_output_tokens as u32);
    if let Err(e) = state_writer.log_task_complete(&task_id, &task_budget.task_type, &task_budget) {
        log::warn!("[token_budget] Failed to log task complete: {}", e);
    }

    // Log overruns
    if task_budget.overran() {
        if let Err(e) = state_writer.log_overrun(&task_id, &task_budget) {
            log::warn!("[token_budget] Failed to log overrun: {}", e);
        }
    }

    // Save to auto-scaling history
    if let Err(e) = BudgetTracker::save_record(
        state_writer.state_dir(),
        task_budget.task_type,
        task_budget.n_predict,
        task_budget.actual_tokens_used,
    ) {
        log::warn!("[token_budget] Failed to save budget history: {}", e);
    }

    // Mark memory as idle
    if let Err(e) = state_writer.mark_idle() {
        log::warn!("[token_budget] Failed to mark idle: {}", e);
    }

    // ── Post-session compaction: ALWAYS save shadow-memory ──
    // This is the critical fix: even if context never hit the compaction threshold,
    // even if the session crashed with API errors, we extract key facts from whatever
    // messages we have and persist them to .shadow-memory/compaction_<ts>.json + memory.md
    if !root_path.is_empty() && current_messages.len() > 2 {
        extract_and_save_session_memory(&current_messages, &root_path);
    }

    // Flush KV cache (best-effort, async, non-blocking)
    let flush_url = base_url.clone();
    tokio::spawn(async move {
        if let Err(e) = crate::token_budget::flush_kv_cache(&flush_url, 0).await {
            log::warn!("[token_budget] KV cache flush failed: {}", e);
        }
    });

    Ok(())
}

#[tauri::command]
pub fn get_spend_stats(root: String) -> serde_json::Value {
    let spend_file = std::path::Path::new(&root)
        .join(".shadowai")
        .join("spend.json");
    let entries: Vec<serde_json::Value> = std::fs::read_to_string(&spend_file)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();

    let today = "2026-03-22";
    let today_cost: f64 = entries
        .iter()
        .filter(|e| e["date"].as_str() == Some(today))
        .filter_map(|e| e["cost_usd"].as_f64())
        .sum();

    // Week: last 7 days (simplified: just count all for now since date is hardcoded)
    let this_week_cost: f64 = entries.iter().filter_map(|e| e["cost_usd"].as_f64()).sum();

    let total_cost: f64 = entries.iter().filter_map(|e| e["cost_usd"].as_f64()).sum();

    let session_count = entries.len();

    let total_tokens: u64 = entries
        .iter()
        .map(|e| {
            e["prompt_tokens"].as_u64().unwrap_or(0) + e["completion_tokens"].as_u64().unwrap_or(0)
        })
        .sum();

    serde_json::json!({
        "today_cost": today_cost,
        "this_week_cost": this_week_cost,
        "total_cost": total_cost,
        "session_count": session_count,
        "total_tokens": total_tokens,
    })
}

/// Ask AI about current debug state — send locals, call stack to AI
#[tauri::command]
pub async fn ai_debug_explain(
    locals: Vec<serde_json::Value>,
    call_stack: Vec<String>,
    source_context: String,
    question: Option<String>,
    api_key: Option<String>,
) -> Result<String, String> {
    let q = question.unwrap_or_else(|| {
        "Explain the current program state and suggest what might be wrong.".to_string()
    });

    let locals_str = serde_json::to_string_pretty(&locals).unwrap_or_default();
    let stack_str = call_stack.join("\n  → ");

    let prompt = format!(
        "You are debugging a program. Here is the current state at a breakpoint:\n\n\
         **Call Stack:**\n  → {}\n\n\
         **Local Variables:**\n```json\n{}\n```\n\n\
         **Source Context:**\n```\n{}\n```\n\n\
         **Question:** {}",
        stack_str, locals_str, source_context, q
    );

    let key = api_key
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
        .ok_or("No API key")?;

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &key)
        .header("anthropic-version", "2023-06-01")
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": prompt }],
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| e.to_string())?;

    resp["content"][0]["text"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("Unexpected response: {:?}", resp))
}

/// Query the llama.cpp server's actual context size (n_ctx) via `/props` or `/v1/models`.
/// Returns `None` if the server doesn't respond or doesn't expose this info.
async fn query_server_context_size(base_url: &str, client: &reqwest::Client) -> Option<usize> {
    // Try /props first (llama.cpp native endpoint)
    if let Ok(resp) = client
        .get(format!("{}/props", base_url.trim_end_matches("/v1")))
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(n_ctx) = json
                    .get("default_generation_settings")
                    .and_then(|s| s.get("n_ctx"))
                    .and_then(|v| v.as_u64())
                {
                    return Some(n_ctx as usize);
                }
                // Some versions put it at top level
                if let Some(n_ctx) = json.get("n_ctx").and_then(|v| v.as_u64()) {
                    return Some(n_ctx as usize);
                }
            }
        }
    }

    // Try /slots (alternative endpoint)
    if let Ok(resp) = client
        .get(format!("{}/slots", base_url.trim_end_matches("/v1")))
        .timeout(std::time::Duration::from_secs(2))
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(arr) = json.as_array() {
                    if let Some(slot) = arr.first() {
                        if let Some(n_ctx) = slot.get("n_ctx").and_then(|v| v.as_u64()) {
                            return Some(n_ctx as usize);
                        }
                    }
                }
            }
        }
    }

    None
}

// ===== AI Docgen / SQL / README / Architecture =====

/// Generate a doc comment for a function/type
#[tauri::command]
pub async fn ai_generate_docstring(
    file_path: String,
    line: u32,
    language: String,
    api_key: Option<String>,
    model: Option<String>,
) -> Result<String, String> {
    let content = std::fs::read_to_string(&file_path).map_err(|e| e.to_string())?;
    let lines: Vec<&str> = content.lines().collect();
    let start = (line as usize).saturating_sub(5);
    let end = (line as usize + 45).min(lines.len());
    let context = lines[start..end].join("\n");

    let prompt = format!(
        "Generate a concise doc comment for the code below. \
         Use the idiomatic format for {} (e.g. /// for Rust, \"\"\" for Python, /** */ for JS/TS). \
         Return ONLY the doc comment, nothing else.\n\n```{}\n{}\n```",
        language, language, context
    );

    let key = api_key
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
        .ok_or("No API key")?;
    let model_id = model.unwrap_or_else(|| "claude-haiku-4-5".to_string());

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &key)
        .header("anthropic-version", "2023-06-01")
        .json(&serde_json::json!({
            "model": model_id,
            "max_tokens": 512,
            "messages": [{ "role": "user", "content": prompt }],
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| e.to_string())?;

    resp["content"][0]["text"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("Unexpected response: {:?}", resp))
}

/// Generate README.md for a project
#[tauri::command]
pub async fn ai_generate_readme(
    project_path: String,
    api_key: Option<String>,
    model: Option<String>,
) -> Result<String, String> {
    let path = std::path::Path::new(&project_path);

    let mut info = String::new();
    for filename in &["Cargo.toml", "package.json", "pyproject.toml", "go.mod"] {
        let f = path.join(filename);
        if f.exists() {
            if let Ok(content) = std::fs::read_to_string(&f) {
                info.push_str(&format!(
                    "=== {} ===\n{}\n\n",
                    filename,
                    &content[..content.len().min(1000)]
                ));
            }
        }
    }
    if info.is_empty() {
        info = "Unknown project structure".to_string();
    }

    let prompt = format!(
        "Generate a comprehensive README.md for this project based on its configuration files. \
         Include: project name/description, features, installation, usage, contributing, license.\n\n{}",
        info
    );

    let key = api_key
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
        .ok_or("No API key")?;
    let model_id = model.unwrap_or_else(|| "claude-sonnet-4-6".to_string());

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &key)
        .header("anthropic-version", "2023-06-01")
        .json(&serde_json::json!({
            "model": model_id,
            "max_tokens": 2048,
            "messages": [{ "role": "user", "content": prompt }],
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| e.to_string())?;

    resp["content"][0]["text"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("Unexpected response: {:?}", resp))
}

/// Generate an architecture Mermaid diagram for a project
#[tauri::command]
pub async fn ai_generate_architecture_diagram(
    project_path: String,
    api_key: Option<String>,
) -> Result<String, String> {
    let path = std::path::Path::new(&project_path);

    let mut file_list = Vec::new();
    fn collect_files(dir: &std::path::Path, list: &mut Vec<String>, depth: usize) {
        if depth > 4 || list.len() > 100 {
            return;
        }
        if let Ok(rd) = std::fs::read_dir(dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if !["node_modules", "target", ".git", "dist", "build"].contains(&name) {
                        collect_files(&p, list, depth + 1);
                    }
                } else {
                    list.push(p.to_string_lossy().to_string());
                }
            }
        }
    }
    collect_files(path, &mut file_list, 0);

    let prompt = format!(
        "Given this project file structure, generate a Mermaid architecture diagram (graph TD) \
         showing the main components and their relationships. Return ONLY the Mermaid code block.\n\n{}",
        file_list.join("\n")
    );

    let key = api_key
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
        .ok_or("No API key")?;

    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &key)
        .header("anthropic-version", "2023-06-01")
        .json(&serde_json::json!({
            "model": "claude-haiku-4-5",
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": prompt }],
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| e.to_string())?;

    resp["content"][0]["text"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("Unexpected response: {:?}", resp))
}

/// AI SQL assistant — generate a query from natural language
#[tauri::command]
pub async fn ai_sql_query(
    natural_language: String,
    schema: String,
    api_key: Option<String>,
) -> Result<String, String> {
    let prompt = format!(
        "Generate a SQL query for the following request. Return ONLY the SQL, no explanation.\n\nSchema:\n{}\n\nRequest: {}",
        schema, natural_language
    );
    let key = api_key
        .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
        .ok_or("No API key")?;
    let client = reqwest::Client::new();
    let resp = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &key)
        .header("anthropic-version", "2023-06-01")
        .json(&serde_json::json!({
            "model": "claude-haiku-4-5",
            "max_tokens": 512,
            "messages": [{ "role": "user", "content": prompt }],
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| e.to_string())?;
    resp["content"][0]["text"]
        .as_str()
        .map(str::to_string)
        .ok_or_else(|| format!("Unexpected response: {:?}", resp))
}
