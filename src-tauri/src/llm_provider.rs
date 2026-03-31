//! Universal LLM Provider Abstraction
//!
//! Every LLM — local or remote — is normalized through a single trait.
//! Tool calling schemas are translated per-provider so the rest of the
//! system never cares which backend is active.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub type ProviderRegistryState = Arc<Mutex<ProviderRegistry>>;

// ===== Core Types =====

/// The format a provider uses for tool call schemas
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ToolSchemaFormat {
    OpenAI,          // OpenAI, Ollama, LM Studio, Mistral, Groq
    Anthropic,       // Claude API — tools array with input_schema
    Gemini,          // functionDeclarations format
    PromptInjection, // system-prompt-injected JSON schema (no native tool API)
}

/// Normalized provider capabilities
#[derive(Debug, Clone, Serialize)]
pub struct ProviderCapabilities {
    pub name: String,
    pub supports_tools: bool,
    pub supports_streaming: bool,
    pub supports_vision: bool,
    pub tool_schema_format: ToolSchemaFormat,
    pub max_context_tokens: u32,
    pub api_style: ApiStyle,
}

/// Which API format to use for requests
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ApiStyle {
    OpenAICompat, // /v1/chat/completions with OpenAI JSON
    Anthropic,    // /v1/messages with Anthropic JSON
    Gemini,       // /v1beta/models/*/generateContent
}

/// A normalized request that can be sent to any provider
#[derive(Debug, Clone, Serialize)]
pub struct NormalizedRequest {
    pub messages: Vec<serde_json::Value>,
    pub model: String,
    pub temperature: f64,
    pub max_tokens: Option<i32>,
    pub stream: bool,
    pub tools: Option<Vec<serde_json::Value>>,
    pub system_prompt: Option<String>,
}

/// Provider configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub name: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub api_style: ApiStyle,
    pub tool_schema_format: ToolSchemaFormat,
    pub supports_tools: bool,
    pub supports_streaming: bool,
    pub supports_vision: bool,
    pub max_context_tokens: u32,
}

impl ProviderConfig {
    pub fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            name: self.name.clone(),
            supports_tools: self.supports_tools,
            supports_streaming: self.supports_streaming,
            supports_vision: self.supports_vision,
            tool_schema_format: self.tool_schema_format.clone(),
            max_context_tokens: self.max_context_tokens,
            api_style: self.api_style.clone(),
        }
    }
}

// ===== Provider Registry =====

pub struct ProviderRegistry {
    providers: HashMap<String, ProviderConfig>,
    active: Option<String>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            providers: HashMap::new(),
            active: None,
        };
        registry.register_defaults();
        registry.load_custom_providers();
        registry
    }

    /// Persistence path for custom providers
    fn custom_providers_path() -> std::path::PathBuf {
        dirs_next::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("shadow-ide")
            .join("providers.json")
    }

    /// Load user-added custom providers from disk (merges with defaults)
    fn load_custom_providers(&mut self) {
        let path = Self::custom_providers_path();
        if !path.exists() {
            return;
        }
        let raw = match std::fs::read_to_string(&path) {
            Ok(r) => r,
            Err(_) => return,
        };
        let custom: Vec<ProviderConfig> = match serde_json::from_str(&raw) {
            Ok(c) => c,
            Err(_) => return,
        };
        let active_name = custom
            .iter()
            .find(|c| c.name == "__active__")
            .and_then(|c| Some(c.base_url.clone()));
        for config in custom {
            if config.name == "__active__" {
                continue;
            }
            self.providers.insert(config.name.clone(), config);
        }
        if let Some(name) = active_name {
            if self.providers.contains_key(&name) {
                self.active = Some(name);
            }
        }
    }

    /// Save custom (non-default) providers to disk
    pub fn save_custom_providers(&self) {
        let defaults = [
            "lmstudio",
            "ollama",
            "openai",
            "anthropic",
            "gemini",
            "groq",
            "mistral",
            "llamacpp",
        ];
        let mut custom: Vec<&ProviderConfig> = self
            .providers
            .values()
            .filter(|p| !defaults.contains(&p.name.as_str()))
            .collect();
        custom.sort_by(|a, b| a.name.cmp(&b.name));

        let path = Self::custom_providers_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        // Also store the active provider name
        let mut to_save: Vec<serde_json::Value> = custom
            .iter()
            .map(|p| serde_json::to_value(p).unwrap_or_default())
            .collect();
        if let Some(ref active) = self.active {
            to_save.push(serde_json::json!({"name": "__active__", "base_url": active}));
        }
        let _ = std::fs::write(
            &path,
            serde_json::to_string_pretty(&to_save).unwrap_or_default(),
        );
    }

    fn register_defaults(&mut self) {
        // OpenAI-compatible providers (LM Studio, Ollama, vLLM, etc.)
        self.register(ProviderConfig {
            name: "lmstudio".into(),
            base_url: "http://localhost:1234/v1".into(),
            api_key: None,
            api_style: ApiStyle::OpenAICompat,
            tool_schema_format: ToolSchemaFormat::OpenAI,
            supports_tools: true,
            supports_streaming: true,
            supports_vision: false,
            max_context_tokens: 32768,
        });

        self.register(ProviderConfig {
            name: "ollama".into(),
            base_url: "http://localhost:11434/v1".into(),
            api_key: None,
            api_style: ApiStyle::OpenAICompat,
            tool_schema_format: ToolSchemaFormat::OpenAI,
            supports_tools: true,
            supports_streaming: true,
            supports_vision: false,
            max_context_tokens: 32768,
        });

        self.register(ProviderConfig {
            name: "openai".into(),
            base_url: "https://api.openai.com/v1".into(),
            api_key: None,
            api_style: ApiStyle::OpenAICompat,
            tool_schema_format: ToolSchemaFormat::OpenAI,
            supports_tools: true,
            supports_streaming: true,
            supports_vision: true,
            max_context_tokens: 128000,
        });

        self.register(ProviderConfig {
            name: "anthropic".into(),
            base_url: "https://api.anthropic.com/v1".into(),
            api_key: None,
            api_style: ApiStyle::Anthropic,
            tool_schema_format: ToolSchemaFormat::Anthropic,
            supports_tools: true,
            supports_streaming: true,
            supports_vision: true,
            max_context_tokens: 200000,
        });

        self.register(ProviderConfig {
            name: "gemini".into(),
            base_url: "https://generativelanguage.googleapis.com/v1beta".into(),
            api_key: None,
            api_style: ApiStyle::Gemini,
            tool_schema_format: ToolSchemaFormat::Gemini,
            supports_tools: true,
            supports_streaming: true,
            supports_vision: true,
            max_context_tokens: 1000000,
        });

        self.register(ProviderConfig {
            name: "groq".into(),
            base_url: "https://api.groq.com/openai/v1".into(),
            api_key: None,
            api_style: ApiStyle::OpenAICompat,
            tool_schema_format: ToolSchemaFormat::OpenAI,
            supports_tools: true,
            supports_streaming: true,
            supports_vision: false,
            max_context_tokens: 131072,
        });

        self.register(ProviderConfig {
            name: "mistral".into(),
            base_url: "https://api.mistral.ai/v1".into(),
            api_key: None,
            api_style: ApiStyle::OpenAICompat,
            tool_schema_format: ToolSchemaFormat::OpenAI,
            supports_tools: true,
            supports_streaming: true,
            supports_vision: false,
            max_context_tokens: 128000,
        });

        self.register(ProviderConfig {
            name: "llamacpp".into(),
            base_url: "http://localhost:8080/v1".into(),
            api_key: None,
            api_style: ApiStyle::OpenAICompat,
            tool_schema_format: ToolSchemaFormat::PromptInjection,
            supports_tools: false,
            supports_streaming: true,
            supports_vision: false,
            max_context_tokens: 16384,
        });
    }

    pub fn register(&mut self, config: ProviderConfig) {
        let name = config.name.clone();
        self.providers.insert(name.clone(), config);
        if self.active.is_none() {
            self.active = Some(name);
        }
    }

    pub fn get(&self, name: &str) -> Option<&ProviderConfig> {
        self.providers.get(name)
    }

    pub fn get_active(&self) -> Option<&ProviderConfig> {
        self.active.as_ref().and_then(|n| self.providers.get(n))
    }

    pub fn set_active(&mut self, name: &str) -> bool {
        if self.providers.contains_key(name) {
            self.active = Some(name.to_string());
            true
        } else {
            false
        }
    }

    pub fn list(&self) -> Vec<&ProviderConfig> {
        self.providers.values().collect()
    }

    /// Detect which providers are available by URL
    pub fn detect_from_url(&self, url: &str) -> Option<&ProviderConfig> {
        fn strip_v1(s: &str) -> &str {
            let s = s.trim_end_matches('/');
            let s = s.strip_suffix("/v1").unwrap_or(s);
            s.strip_suffix("/v1beta").unwrap_or(s)
        }
        let url_base = strip_v1(url);
        for config in self.providers.values() {
            let config_base = strip_v1(&config.base_url);
            // Either the URL starts with the config base, or they share the same host:port base
            if url.starts_with(&config.base_url) || url_base == config_base {
                return Some(config);
            }
        }
        None
    }

    /// Get or create a provider config for a given base URL
    pub fn get_or_infer(&self, base_url: &str) -> ProviderConfig {
        // Check if any registered provider matches
        if let Some(config) = self.detect_from_url(base_url) {
            return config.clone();
        }

        // Infer from URL patterns
        let url_lower = base_url.to_lowercase();
        let (api_style, tool_format, supports_tools, max_ctx) = if url_lower.contains("anthropic") {
            (
                ApiStyle::Anthropic,
                ToolSchemaFormat::Anthropic,
                true,
                200000,
            )
        } else if url_lower.contains("generativelanguage.googleapis") {
            (ApiStyle::Gemini, ToolSchemaFormat::Gemini, true, 1000000)
        } else {
            // Default: assume OpenAI-compatible
            (
                ApiStyle::OpenAICompat,
                ToolSchemaFormat::OpenAI,
                true,
                32768,
            )
        };

        ProviderConfig {
            name: "custom".into(),
            base_url: base_url.to_string(),
            api_key: None,
            api_style,
            tool_schema_format: tool_format,
            supports_tools,
            supports_streaming: true,
            supports_vision: false,
            max_context_tokens: max_ctx,
        }
    }
}

// ===== Tool Schema Translation =====

/// Translate OpenAI tool definitions to the target provider format
pub fn translate_tools(
    tools: &[crate::tool_calling::ToolDefinition],
    format: &ToolSchemaFormat,
) -> Vec<serde_json::Value> {
    match format {
        ToolSchemaFormat::OpenAI => {
            // Already in OpenAI format
            tools
                .iter()
                .map(|t| serde_json::to_value(t).unwrap_or_default())
                .collect()
        }
        ToolSchemaFormat::Anthropic => {
            // Anthropic: { name, description, input_schema }
            tools
                .iter()
                .map(|t| {
                    serde_json::json!({
                        "name": t.function.name,
                        "description": t.function.description,
                        "input_schema": t.function.parameters,
                    })
                })
                .collect()
        }
        ToolSchemaFormat::Gemini => tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.function.name,
                    "description": t.function.description,
                    "parameters": t.function.parameters,
                })
            })
            .collect(),
        ToolSchemaFormat::PromptInjection => tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.function.name,
                    "description": t.function.description,
                    "parameters": t.function.parameters,
                })
            })
            .collect(),
    }
}

/// Build the tool injection system prompt for models that don't support native tool calling
pub fn build_tool_injection_prompt(tools: &[crate::tool_calling::ToolDefinition]) -> String {
    let mut tool_descriptions = String::new();
    for tool in tools {
        tool_descriptions.push_str(&format!(
            "- {}: {}\n  Parameters: {}\n\n",
            tool.function.name,
            tool.function.description,
            serde_json::to_string_pretty(&tool.function.parameters).unwrap_or_default()
        ));
    }

    format!(
        "You have access to these tools. To call one, respond with ONLY valid JSON:\n\
         {{\"tool\": \"<name>\", \"args\": {{<args>}}}}\n\n\
         Available tools:\n{}\n\
         After a tool result is shown, continue your response normally.",
        tool_descriptions
    )
}

/// Extract a tool call from plain text response (for prompt-injection fallback)
pub fn extract_tool_call_from_text(text: &str) -> Option<crate::tool_calling::ToolCall> {
    // Try to find JSON blocks in the text
    let text = text.trim();

    // Strategy 1: Look for ```json ... ``` blocks
    if let Some(start) = text.find("```json") {
        if let Some(end) = text[start + 7..].find("```") {
            let json_str = text[start + 7..start + 7 + end].trim();
            if let Some(call) = parse_tool_json(json_str) {
                return Some(call);
            }
        }
    }

    // Strategy 2: Look for ``` ... ``` blocks
    if let Some(start) = text.find("```") {
        if let Some(end) = text[start + 3..].find("```") {
            let json_str = text[start + 3..start + 3 + end].trim();
            if let Some(call) = parse_tool_json(json_str) {
                return Some(call);
            }
        }
    }

    // Strategy 3: Look for {"tool": ... } patterns directly
    for i in 0..text.len() {
        if text[i..].starts_with("{\"tool\"") || text[i..].starts_with("{ \"tool\"") {
            // Find matching closing brace
            let mut depth = 0;
            let mut end = i;
            for (j, ch) in text[i..].char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i + j + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            if end > i {
                let json_str = &text[i..end];
                if let Some(call) = parse_tool_json(json_str) {
                    return Some(call);
                }
            }
        }
    }

    None
}

fn parse_tool_json(json_str: &str) -> Option<crate::tool_calling::ToolCall> {
    let v: serde_json::Value = serde_json::from_str(json_str).ok()?;

    let tool_name = v["tool"].as_str().or_else(|| v["name"].as_str())?;
    let args = v
        .get("args")
        .or_else(|| v.get("arguments"))
        .or_else(|| v.get("parameters"))
        .cloned()
        .unwrap_or(serde_json::json!({}));

    Some(crate::tool_calling::ToolCall {
        id: format!("fallback_{}", tool_name),
        call_type: Some("function".to_string()),
        function: crate::tool_calling::FunctionCallData {
            name: tool_name.to_string(),
            arguments: serde_json::to_string(&args).unwrap_or_default(),
        },
    })
}

// ===== Anthropic Request/Response Translation =====

/// Build an Anthropic API request from normalized messages
pub fn build_anthropic_request(
    req: &NormalizedRequest,
    tools: Option<&[serde_json::Value]>,
) -> serde_json::Value {
    // Anthropic expects system as a top-level field, not in messages
    let mut messages = Vec::new();
    let mut system_prompt = req.system_prompt.clone().unwrap_or_default();

    for msg in &req.messages {
        let role = msg["role"].as_str().unwrap_or("user");
        match role {
            "system" => {
                if let Some(content) = msg["content"].as_str() {
                    if !system_prompt.is_empty() {
                        system_prompt.push_str("\n\n");
                    }
                    system_prompt.push_str(content);
                }
            }
            "tool" => {
                // Anthropic: tool results are role "user" with tool_result content block
                let tool_use_id = msg["tool_call_id"].as_str().unwrap_or("unknown");
                let content = msg["content"].as_str().unwrap_or("");
                messages.push(serde_json::json!({
                    "role": "user",
                    "content": [{
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": content,
                    }]
                }));
            }
            "assistant" => {
                // Check if assistant message has tool_calls → convert to tool_use blocks
                if let Some(tool_calls) = msg.get("tool_calls").and_then(|tc| tc.as_array()) {
                    let mut content_blocks: Vec<serde_json::Value> = Vec::new();
                    // Add text content if any
                    if let Some(text) = msg["content"].as_str() {
                        if !text.is_empty() {
                            content_blocks.push(serde_json::json!({
                                "type": "text",
                                "text": text,
                            }));
                        }
                    }
                    // Add tool_use blocks
                    for tc in tool_calls {
                        let args: serde_json::Value = tc["function"]["arguments"]
                            .as_str()
                            .and_then(|s| serde_json::from_str(s).ok())
                            .unwrap_or(serde_json::json!({}));
                        content_blocks.push(serde_json::json!({
                            "type": "tool_use",
                            "id": tc["id"],
                            "name": tc["function"]["name"],
                            "input": args,
                        }));
                    }
                    messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": content_blocks,
                    }));
                } else {
                    messages.push(serde_json::json!({
                        "role": "assistant",
                        "content": msg["content"],
                    }));
                }
            }
            _ => {
                messages.push(serde_json::json!({
                    "role": role,
                    "content": msg["content"],
                }));
            }
        }
    }

    let mut request = serde_json::json!({
        "model": req.model,
        "messages": messages,
        "max_tokens": req.max_tokens.unwrap_or(4096),
        "stream": req.stream,
    });

    if !system_prompt.is_empty() {
        request["system"] = serde_json::Value::String(system_prompt);
    }

    if req.temperature > 0.0 {
        request["temperature"] = serde_json::json!(req.temperature);
    }

    if let Some(tools) = tools {
        if !tools.is_empty() {
            request["tools"] = serde_json::Value::Array(tools.to_vec());
        }
    }

    request
}

/// Parse an Anthropic streaming event into the OpenAI-compatible format
/// used by ai_bridge.rs
pub fn parse_anthropic_stream_event(event_type: &str, data: &serde_json::Value) -> AnthropicEvent {
    match event_type {
        "content_block_start" => {
            let block = &data["content_block"];
            if block["type"] == "tool_use" {
                AnthropicEvent::ToolCallStart {
                    id: block["id"].as_str().unwrap_or("").to_string(),
                    name: block["name"].as_str().unwrap_or("").to_string(),
                }
            } else {
                AnthropicEvent::None
            }
        }
        "content_block_delta" => {
            let delta = &data["delta"];
            if delta["type"] == "text_delta" {
                AnthropicEvent::TextDelta(delta["text"].as_str().unwrap_or("").to_string())
            } else if delta["type"] == "input_json_delta" {
                AnthropicEvent::ToolArgsDelta(
                    delta["partial_json"].as_str().unwrap_or("").to_string(),
                )
            } else {
                AnthropicEvent::None
            }
        }
        "content_block_stop" => AnthropicEvent::BlockStop,
        "message_stop" => AnthropicEvent::MessageStop,
        "message_delta" => {
            let stop = data["delta"]["stop_reason"].as_str().unwrap_or("");
            AnthropicEvent::MessageDelta {
                stop_reason: if stop.is_empty() {
                    None
                } else {
                    Some(stop.to_string())
                },
            }
        }
        _ => AnthropicEvent::None,
    }
}

#[derive(Debug)]
pub enum AnthropicEvent {
    TextDelta(String),
    ToolCallStart { id: String, name: String },
    ToolArgsDelta(String),
    BlockStop,
    MessageStop,
    MessageDelta { stop_reason: Option<String> },
    None,
}

// ===== Gemini Streaming Response Parser =====

/// Parse a Gemini streaming event into normalized events.
/// Gemini SSE data format:
///   {"candidates":[{"content":{"parts":[{"text":"..."}]},...}]}
///   {"candidates":[{"content":{"parts":[{"functionCall":{"name":"...","args":{...}}}]},...}]}
pub fn parse_gemini_stream_event(data: &serde_json::Value) -> GeminiEvent {
    let candidates = match data["candidates"].as_array() {
        Some(c) => c,
        None => return GeminiEvent::None,
    };
    let candidate = match candidates.first() {
        Some(c) => c,
        None => return GeminiEvent::None,
    };

    // Check finish reason
    if let Some(reason) = candidate["finishReason"].as_str() {
        if reason == "STOP" || reason == "MAX_TOKENS" {
            return GeminiEvent::Done {
                stop_reason: Some(reason.to_string()),
            };
        }
    }

    let parts = match candidate["content"]["parts"].as_array() {
        Some(p) => p,
        None => return GeminiEvent::None,
    };

    for part in parts {
        // Text delta
        if let Some(text) = part["text"].as_str() {
            return GeminiEvent::TextDelta(text.to_string());
        }
        // Function call
        if let Some(fc) = part.get("functionCall") {
            let name = fc["name"].as_str().unwrap_or("").to_string();
            let args = fc.get("args").cloned().unwrap_or(serde_json::json!({}));
            return GeminiEvent::ToolCall { name, args };
        }
    }

    GeminiEvent::None
}

#[derive(Debug)]
pub enum GeminiEvent {
    TextDelta(String),
    ToolCall {
        name: String,
        args: serde_json::Value,
    },
    Done {
        stop_reason: Option<String>,
    },
    None,
}

// ===== Gemini Request Translation =====

/// Build a Gemini API request from normalized messages
pub fn build_gemini_request(
    req: &NormalizedRequest,
    tools: Option<&[serde_json::Value]>,
) -> serde_json::Value {
    // Gemini uses "contents" with "parts" and "role" being "user" or "model"
    let mut contents: Vec<serde_json::Value> = Vec::new();
    let mut system_instruction = req.system_prompt.clone().unwrap_or_default();

    for msg in &req.messages {
        let role = msg["role"].as_str().unwrap_or("user");
        let content = msg["content"].as_str().unwrap_or("");

        match role {
            "system" => {
                if !system_instruction.is_empty() {
                    system_instruction.push_str("\n\n");
                }
                system_instruction.push_str(content);
            }
            "assistant" => {
                contents.push(serde_json::json!({
                    "role": "model",
                    "parts": [{ "text": content }]
                }));
            }
            "tool" => {
                // Gemini: function response
                let name = msg["name"].as_str().unwrap_or("tool");
                let response: serde_json::Value = serde_json::from_str(content)
                    .unwrap_or(serde_json::json!({ "result": content }));
                contents.push(serde_json::json!({
                    "role": "function",
                    "parts": [{
                        "functionResponse": {
                            "name": name,
                            "response": response,
                        }
                    }]
                }));
            }
            _ => {
                contents.push(serde_json::json!({
                    "role": "user",
                    "parts": [{ "text": content }]
                }));
            }
        }
    }

    let mut request = serde_json::json!({
        "contents": contents,
        "generationConfig": {
            "temperature": req.temperature,
            "maxOutputTokens": req.max_tokens.unwrap_or(4096),
        }
    });

    if !system_instruction.is_empty() {
        request["systemInstruction"] = serde_json::json!({
            "parts": [{ "text": system_instruction }]
        });
    }

    if let Some(tools) = tools {
        if !tools.is_empty() {
            request["tools"] = serde_json::json!([{
                "functionDeclarations": tools
            }]);
        }
    }

    request
}

// ===== Token Budget Optimizer =====

/// Pre-request token budget optimizer
pub struct TokenBudgetOptimizer {
    pub max_context: usize,
    pub system_reserve: usize,
    pub tool_schema_cap: usize,
    pub rag_budget: usize,
    pub history_min_turns: usize,
    pub response_reserve: usize,
}

impl Default for TokenBudgetOptimizer {
    fn default() -> Self {
        Self {
            max_context: 120000,
            system_reserve: 4096,
            tool_schema_cap: 8192,
            rag_budget: 16384,
            history_min_turns: 6,
            response_reserve: 8192,
        }
    }
}

impl TokenBudgetOptimizer {
    pub fn new(max_context: usize) -> Self {
        Self {
            max_context,
            ..Default::default()
        }
    }

    /// Optimize a request to fit within the token budget.
    /// Returns true if modifications were made.
    pub fn optimize(
        &self,
        messages: &mut Vec<serde_json::Value>,
        tools: &mut Option<Vec<crate::tool_calling::ToolDefinition>>,
        recent_tool_names: &[String],
        core_tool_names: &[String],
    ) -> bool {
        let budget = self.max_context.saturating_sub(self.response_reserve);
        let current = crate::token_optimizer::count_message_tokens(messages);

        if current <= budget {
            return false;
        }

        let mut modified = false;

        // Step 1: Drop tool schemas for tools not recently used
        if let Some(ref mut tool_list) = tools {
            let before = tool_list.len();
            tool_list.retain(|t| {
                core_tool_names.contains(&t.function.name)
                    || recent_tool_names.contains(&t.function.name)
            });
            if tool_list.len() < before {
                modified = true;
            }
        }

        // Step 2: Trim RAG context blocks (messages containing <rag_context>)
        let rag_budget_chars = self.rag_budget * 4; // rough chars-to-tokens
        for msg in messages.iter_mut() {
            if let Some(content) = msg["content"].as_str() {
                if content.contains("<rag_context>") && content.len() > rag_budget_chars {
                    // Find safe UTF-8 cut point (don't slice mid-character)
                    let safe_end = content
                        .char_indices()
                        .take_while(|(i, _)| *i < rag_budget_chars)
                        .last()
                        .map(|(i, c)| i + c.len_utf8())
                        .unwrap_or(rag_budget_chars.min(content.len()));
                    msg["content"] = serde_json::Value::String(content[..safe_end].to_string());
                    modified = true;
                }
            }
        }

        // Step 3: Drop oldest non-system messages if still over budget
        let current_after_trim = crate::token_optimizer::count_message_tokens(messages);
        if current_after_trim > budget {
            // Keep at least history_min_turns recent messages + system messages
            let total = messages.len();
            if total > self.history_min_turns {
                let mut system_count = 0;
                // Count system messages at the start
                for (i, msg) in messages.iter().enumerate() {
                    if msg["role"].as_str() == Some("system") {
                        system_count = i + 1;
                    } else {
                        break;
                    }
                }
                // Drop messages between system prefix and the last history_min_turns
                let keep_tail = self.history_min_turns;
                let first_keep = if total > keep_tail {
                    total - keep_tail
                } else {
                    0
                };
                let keep_from = first_keep.max(system_count);

                if keep_from > system_count {
                    // Remove messages from system_count..keep_from
                    messages.drain(system_count..keep_from);
                    modified = true;
                }
            }
        }

        modified
    }
}

// ===== Tauri Commands =====

#[tauri::command]
pub fn provider_list(
    state: tauri::State<'_, ProviderRegistryState>,
) -> Result<Vec<serde_json::Value>, String> {
    let reg = state.lock().map_err(|e| e.to_string())?;
    Ok(reg
        .list()
        .iter()
        .map(|p| {
            serde_json::json!({
                "name": p.name,
                "base_url": p.base_url,
                "api_style": p.api_style,
                "supports_tools": p.supports_tools,
                "supports_streaming": p.supports_streaming,
                "supports_vision": p.supports_vision,
                "max_context_tokens": p.max_context_tokens,
            })
        })
        .collect())
}

#[tauri::command]
pub fn provider_get_active(
    state: tauri::State<'_, ProviderRegistryState>,
) -> Result<Option<serde_json::Value>, String> {
    let reg = state.lock().map_err(|e| e.to_string())?;
    Ok(reg.get_active().map(|p| {
        serde_json::json!({
            "name": p.name,
            "base_url": p.base_url,
            "api_style": p.api_style,
            "tool_schema_format": p.tool_schema_format,
            "supports_tools": p.supports_tools,
            "supports_vision": p.supports_vision,
            "max_context_tokens": p.max_context_tokens,
        })
    }))
}

#[tauri::command]
pub fn provider_set_active(
    name: String,
    state: tauri::State<'_, ProviderRegistryState>,
) -> Result<bool, String> {
    let mut reg = state.lock().map_err(|e| e.to_string())?;
    let result = reg.set_active(&name);
    if result {
        reg.save_custom_providers();
    }
    Ok(result)
}

#[tauri::command]
pub fn provider_add(
    name: String,
    base_url: String,
    api_key: Option<String>,
    api_style: Option<String>,
    state: tauri::State<'_, ProviderRegistryState>,
) -> Result<(), String> {
    let style = match api_style.as_deref() {
        Some("anthropic") => ApiStyle::Anthropic,
        Some("gemini") => ApiStyle::Gemini,
        _ => ApiStyle::OpenAICompat,
    };
    let tool_format = match &style {
        ApiStyle::Anthropic => ToolSchemaFormat::Anthropic,
        ApiStyle::Gemini => ToolSchemaFormat::Gemini,
        ApiStyle::OpenAICompat => ToolSchemaFormat::OpenAI,
    };
    let mut reg = state.lock().map_err(|e| e.to_string())?;
    reg.register(ProviderConfig {
        name,
        base_url,
        api_key,
        api_style: style,
        tool_schema_format: tool_format,
        supports_tools: true,
        supports_streaming: true,
        supports_vision: false,
        max_context_tokens: 120000,
    });
    reg.save_custom_providers();
    Ok(())
}

#[tauri::command]
pub fn provider_infer(
    base_url: String,
    state: tauri::State<'_, ProviderRegistryState>,
) -> Result<serde_json::Value, String> {
    let reg = state.lock().map_err(|e| e.to_string())?;
    let config = reg.get_or_infer(&base_url);
    Ok(serde_json::json!({
        "name": config.name,
        "api_style": config.api_style,
        "tool_schema_format": config.tool_schema_format,
        "supports_tools": config.supports_tools,
        "max_context_tokens": config.max_context_tokens,
    }))
}

/// Probe local providers to detect which are available.
/// Returns a list of provider names with their online/offline status.
#[tauri::command]
pub async fn provider_detect_available(
    state: tauri::State<'_, ProviderRegistryState>,
) -> Result<Vec<serde_json::Value>, String> {
    let providers: Vec<(String, String)> = {
        let reg = state.lock().map_err(|e| e.to_string())?;
        reg.list()
            .iter()
            .filter(|p| p.base_url.contains("localhost") || p.base_url.contains("127.0.0.1"))
            .map(|p| (p.name.clone(), p.base_url.clone()))
            .collect()
    };

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .map_err(|e| e.to_string())?;

    let mut results = Vec::new();
    for (name, base_url) in providers {
        let url = format!("{}/models", base_url);
        let online = match client.get(&url).send().await {
            Ok(resp) => resp.status().is_success(),
            Err(_) => false,
        };
        results.push(serde_json::json!({
            "name": name,
            "base_url": base_url,
            "online": online,
        }));
    }
    Ok(results)
}

// ===== Ollama Integration =====

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct OllamaModel {
    pub name: String,
    pub size: u64,
    pub modified_at: String,
    pub digest: String,
}

#[tauri::command]
pub async fn detect_ollama() -> bool {
    reqwest::get("http://localhost:11434/api/tags")
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

#[tauri::command]
pub async fn list_ollama_models() -> Result<Vec<OllamaModel>, String> {
    let resp = reqwest::get("http://localhost:11434/api/tags")
        .await
        .map_err(|e| e.to_string())?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| e.to_string())?;

    let models = resp["models"]
        .as_array()
        .ok_or("No models field in Ollama response")?
        .iter()
        .filter_map(|m| {
            Some(OllamaModel {
                name: m["name"].as_str()?.to_string(),
                size: m["size"].as_u64().unwrap_or(0),
                modified_at: m["modified_at"].as_str().unwrap_or("").to_string(),
                digest: m["digest"].as_str().unwrap_or("").to_string(),
            })
        })
        .collect();
    Ok(models)
}

#[tauri::command]
pub async fn ollama_pull_model(
    model_name: String,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    use tauri::Emitter;
    let client = reqwest::Client::new();
    let mut resp = client
        .post("http://localhost:11434/api/pull")
        .json(&serde_json::json!({ "name": model_name }))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        if let Ok(text) = std::str::from_utf8(&chunk) {
            for line in text.lines() {
                if !line.is_empty() {
                    let _ = app_handle.emit("ollama-pull-progress", line);
                }
            }
        }
    }
    Ok(format!("Model {} pulled successfully", model_name))
}

// ===== Multi-Model Routing =====

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct RoutingRule {
    pub skill_pattern: String,
    pub model: String,
    pub priority: u32,
}

#[tauri::command]
pub fn get_default_routing_rules() -> Vec<RoutingRule> {
    vec![
        RoutingRule {
            skill_pattern: "code|refactor|debug".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            priority: 10,
        },
        RoutingRule {
            skill_pattern: "docs|explain|summarize".to_string(),
            model: "claude-haiku-4-5".to_string(),
            priority: 10,
        },
        RoutingRule {
            skill_pattern: ".*".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            priority: 0,
        },
    ]
}

// ===== Model Metadata =====

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct ModelMetadata {
    pub id: String,
    pub name: String,
    pub provider: String, // "ollama", "lmstudio", "openai", "anthropic", "llamacpp"
    pub context_window: u32,
    pub quantization: Option<String>,
    pub vram_mb: Option<u32>,
    pub license: Option<String>,
    pub capabilities: Vec<String>, // "chat", "fim", "vision", "embed"
}

#[tauri::command]
pub async fn get_model_metadata(
    provider: String,
    model_id: String,
) -> Result<ModelMetadata, String> {
    match provider.as_str() {
        "ollama" => {
            let resp = reqwest::Client::new()
                .post("http://localhost:11434/api/show")
                .json(&serde_json::json!({ "name": model_id }))
                .send()
                .await
                .map_err(|e| e.to_string())?
                .json::<serde_json::Value>()
                .await
                .map_err(|e| e.to_string())?;

            let ctx = resp["model_info"]["llama.context_length"]
                .as_u64()
                .or_else(|| {
                    resp["parameters"].as_str().and_then(|p| {
                        p.lines()
                            .find(|l| l.contains("num_ctx"))
                            .and_then(|l| l.split_whitespace().last()?.parse().ok())
                    })
                })
                .unwrap_or(4096) as u32;

            Ok(ModelMetadata {
                id: model_id.clone(),
                name: model_id,
                provider: "ollama".to_string(),
                context_window: ctx,
                quantization: resp["details"]["quantization_level"]
                    .as_str()
                    .map(str::to_string),
                vram_mb: None,
                license: resp["license"].as_str().map(str::to_string),
                capabilities: vec!["chat".to_string()],
            })
        }
        "lmstudio" => {
            let resp = reqwest::get("http://localhost:1234/v1/models")
                .await
                .map_err(|e| e.to_string())?
                .json::<serde_json::Value>()
                .await
                .map_err(|e| e.to_string())?;
            let model = resp["data"]
                .as_array()
                .and_then(|arr| arr.iter().find(|m| m["id"].as_str() == Some(&model_id)))
                .cloned()
                .unwrap_or(serde_json::json!({}));
            Ok(ModelMetadata {
                id: model_id.clone(),
                name: model["id"].as_str().unwrap_or(&model_id).to_string(),
                provider: "lmstudio".to_string(),
                context_window: model["max_context_length"].as_u64().unwrap_or(4096) as u32,
                quantization: None,
                vram_mb: None,
                license: None,
                capabilities: vec!["chat".to_string()],
            })
        }
        _ => Err(format!("Unknown provider: {}", provider)),
    }
}

/// Detect LM Studio at localhost:1234
#[tauri::command]
pub async fn detect_lm_studio() -> bool {
    reqwest::get("http://localhost:1234/v1/models")
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// List LM Studio models
#[tauri::command]
pub async fn list_lm_studio_models() -> Result<Vec<OllamaModel>, String> {
    let resp = reqwest::get("http://localhost:1234/v1/models")
        .await
        .map_err(|e| e.to_string())?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| e.to_string())?;
    Ok(resp["data"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|m| OllamaModel {
            name: m["id"].as_str().unwrap_or("").to_string(),
            size: 0,
            modified_at: "".to_string(),
            digest: "".to_string(),
        })
        .collect())
}

/// Detect vLLM at localhost:8000
#[tauri::command]
pub async fn detect_vllm() -> bool {
    reqwest::get("http://localhost:8000/v1/models")
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// List vLLM models
#[tauri::command]
pub async fn list_vllm_models() -> Result<Vec<OllamaModel>, String> {
    let resp = reqwest::get("http://localhost:8000/v1/models")
        .await
        .map_err(|e| e.to_string())?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| e.to_string())?;
    Ok(resp["data"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .map(|m| OllamaModel {
            name: m["id"].as_str().unwrap_or("").to_string(),
            size: 0,
            modified_at: "".to_string(),
            digest: "".to_string(),
        })
        .collect())
}

/// Start a llama.cpp server for a model file
#[tauri::command]
pub async fn start_llama_server(
    model_path: String,
    port: Option<u16>,
    context_size: Option<u32>,
) -> Result<String, String> {
    let port = port.unwrap_or(8080);
    let ctx = context_size.unwrap_or(4096);

    let child = std::process::Command::new("llama-server")
        .args([
            "-m",
            &model_path,
            "--port",
            &port.to_string(),
            "-c",
            &ctx.to_string(),
            "--host",
            "127.0.0.1",
        ])
        .spawn()
        .map_err(|e| format!("llama-server not found or failed to start: {}", e))?;

    // Store PID for later stop
    let pid = child.id();
    // Detach — intentionally leak to keep process running
    std::mem::forget(child);

    Ok(format!(
        "llama-server started on port {} (pid {})",
        port, pid
    ))
}

/// Stop llama.cpp server by port (kill process listening on that port)
#[tauri::command]
pub async fn stop_llama_server(port: u16) -> Result<String, String> {
    // Use fuser or lsof to find and kill the process
    #[cfg(target_os = "linux")]
    {
        let out = std::process::Command::new("fuser")
            .args(["-k", &format!("{}/tcp", port)])
            .output()
            .map_err(|e| e.to_string())?;
        return Ok(format!(
            "Killed process on port {} (exit: {})",
            port, out.status
        ));
    }
    #[cfg(target_os = "macos")]
    {
        let find = std::process::Command::new("lsof")
            .args(["-ti", &format!(":{}", port)])
            .output()
            .map_err(|e| e.to_string())?;
        let pid = String::from_utf8_lossy(&find.stdout).trim().to_string();
        if !pid.is_empty() {
            let _ = std::process::Command::new("kill")
                .args(["-9", &pid])
                .output();
        }
        return Ok(format!("Killed process on port {}", port));
    }
    #[allow(unreachable_code)]
    Err("stop_llama_server not implemented on this platform".to_string())
}

/// Download a GGUF model from HuggingFace
#[tauri::command]
pub async fn download_gguf_model(
    repo_id: String,
    filename: String,
    dest_dir: String,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    use tauri::Emitter;

    let url = format!(
        "https://huggingface.co/{}/resolve/main/{}",
        repo_id, filename
    );
    let dest = std::path::Path::new(&dest_dir).join(&filename);

    let client = reqwest::Client::new();
    let mut resp = client.get(&url).send().await.map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}: {}", resp.status(), url));
    }

    let total = resp.content_length().unwrap_or(0);
    let mut file = tokio::fs::File::create(&dest)
        .await
        .map_err(|e| e.to_string())?;
    let mut downloaded: u64 = 0;

    use tokio::io::AsyncWriteExt;
    while let Some(chunk) = resp.chunk().await.map_err(|e| e.to_string())? {
        file.write_all(&chunk).await.map_err(|e| e.to_string())?;
        downloaded += chunk.len() as u64;
        if total > 0 {
            let pct = downloaded * 100 / total;
            let _ = app_handle.emit(
                "gguf-download-progress",
                serde_json::json!({
                    "filename": filename,
                    "downloaded": downloaded,
                    "total": total,
                    "percent": pct,
                }),
            );
        }
    }

    Ok(dest.to_string_lossy().to_string())
}

/// Model benchmarking — run a standard prompt through a model, measure tokens/s
#[tauri::command]
pub async fn benchmark_model(
    provider: String,
    model: String,
    prompt: Option<String>,
) -> Result<BenchmarkResult, String> {
    let test_prompt =
        prompt.unwrap_or_else(|| "Write a bubble sort in Python. Be concise.".to_string());
    let start = std::time::Instant::now();

    let base_url = match provider.as_str() {
        "ollama" => "http://localhost:11434/v1",
        "lmstudio" => "http://localhost:1234/v1",
        "vllm" => "http://localhost:8000/v1",
        _ => return Err(format!("Unknown provider: {}", provider)),
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/chat/completions", base_url))
        .json(&serde_json::json!({
            "model": model,
            "messages": [{ "role": "user", "content": test_prompt }],
            "stream": false,
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json::<serde_json::Value>()
        .await
        .map_err(|e| e.to_string())?;

    let elapsed = start.elapsed().as_secs_f64();
    let completion_tokens = resp["usage"]["completion_tokens"].as_u64().unwrap_or(0);
    let tokens_per_sec = if elapsed > 0.0 {
        completion_tokens as f64 / elapsed
    } else {
        0.0
    };

    Ok(BenchmarkResult {
        provider,
        model,
        tokens_per_second: tokens_per_sec,
        latency_ms: (elapsed * 1000.0) as u64,
        completion_tokens,
        response_preview: resp["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(200)
            .collect(),
    })
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct BenchmarkResult {
    pub provider: String,
    pub model: String,
    pub tokens_per_second: f64,
    pub latency_ms: u64,
    pub completion_tokens: u64,
    pub response_preview: String,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct ModelFallbackResult {
    pub text: String,
    pub provider_used: String,
    pub model_used: String,
    pub fell_back: bool,
}

/// Try to call a local model; if it fails or is too slow, fall back to a cloud provider.
/// Returns the completion text and which provider was used.
#[tauri::command]
pub async fn model_with_fallback(
    prompt: String,
    preferred_provider: String,
    preferred_model: String,
    fallback_model: String,
    api_key: Option<String>,
    timeout_secs: Option<u64>,
) -> Result<ModelFallbackResult, String> {
    let timeout = tokio::time::Duration::from_secs(timeout_secs.unwrap_or(15));

    let local_url = match preferred_provider.as_str() {
        "ollama" => "http://localhost:11434/v1/chat/completions",
        "lmstudio" => "http://localhost:1234/v1/chat/completions",
        "llamacpp" => "http://localhost:8080/v1/chat/completions",
        _ => return Err(format!("Unknown provider: {}", preferred_provider)),
    };

    // Try local first
    let local_result = tokio::time::timeout(timeout, async {
        reqwest::Client::new()
            .post(local_url)
            .json(&serde_json::json!({
                "model": preferred_model,
                "messages": [{ "role": "user", "content": prompt }],
                "stream": false,
            }))
            .send()
            .await?
            .json::<serde_json::Value>()
            .await
    })
    .await;

    match local_result {
        Ok(Ok(resp)) if resp["choices"][0]["message"]["content"].is_string() => {
            Ok(ModelFallbackResult {
                text: resp["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                provider_used: preferred_provider,
                model_used: preferred_model,
                fell_back: false,
            })
        }
        _ => {
            // Fall back to Anthropic
            let key = api_key
                .or_else(|| std::env::var("ANTHROPIC_API_KEY").ok())
                .ok_or("Local model unavailable and no ANTHROPIC_API_KEY for fallback")?;
            let resp = reqwest::Client::new()
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", &key)
                .header("anthropic-version", "2023-06-01")
                .json(&serde_json::json!({
                    "model": fallback_model,
                    "max_tokens": 2048,
                    "messages": [{ "role": "user", "content": prompt }],
                }))
                .send()
                .await
                .map_err(|e| e.to_string())?
                .json::<serde_json::Value>()
                .await
                .map_err(|e| e.to_string())?;
            Ok(ModelFallbackResult {
                text: resp["content"][0]["text"]
                    .as_str()
                    .unwrap_or("")
                    .to_string(),
                provider_used: "anthropic".to_string(),
                model_used: fallback_model,
                fell_back: true,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_defaults() {
        let reg = ProviderRegistry::new();
        assert!(reg.get("lmstudio").is_some());
        assert!(reg.get("ollama").is_some());
        assert!(reg.get("openai").is_some());
        assert!(reg.get("anthropic").is_some());
        assert!(reg.get("gemini").is_some());
    }

    #[test]
    fn test_registry_set_active() {
        let mut reg = ProviderRegistry::new();
        assert!(reg.set_active("anthropic"));
        assert_eq!(reg.get_active().unwrap().name, "anthropic");
        assert!(!reg.set_active("nonexistent"));
    }

    #[test]
    fn test_get_or_infer_anthropic() {
        let reg = ProviderRegistry::new();
        let config = reg.get_or_infer("https://api.anthropic.com/v1");
        assert_eq!(config.api_style, ApiStyle::Anthropic);
    }

    #[test]
    fn test_get_or_infer_openai_compat() {
        let reg = ProviderRegistry::new();
        let config = reg.get_or_infer("http://some-custom-server:8080/v1");
        assert_eq!(config.api_style, ApiStyle::OpenAICompat);
    }

    #[test]
    fn test_extract_tool_call_json_block() {
        let text = "Let me read the file:\n```json\n{\"tool\": \"read_file\", \"args\": {\"path\": \"test.rs\"}}\n```";
        let call = extract_tool_call_from_text(text).unwrap();
        assert_eq!(call.function.name, "read_file");
    }

    #[test]
    fn test_extract_tool_call_inline() {
        let text = "I'll execute: {\"tool\": \"shell_exec\", \"args\": {\"command\": \"ls\"}}";
        let call = extract_tool_call_from_text(text).unwrap();
        assert_eq!(call.function.name, "shell_exec");
    }

    #[test]
    fn test_extract_tool_call_no_match() {
        let text = "This is just a regular response with no tool calls.";
        assert!(extract_tool_call_from_text(text).is_none());
    }

    #[test]
    fn test_translate_tools_anthropic() {
        let tools = vec![crate::tool_calling::ToolDefinition {
            tool_type: "function".into(),
            function: crate::tool_calling::FunctionDefinition {
                name: "test".into(),
                description: "A test tool".into(),
                parameters: serde_json::json!({"type": "object", "properties": {}}),
            },
        }];
        let translated = translate_tools(&tools, &ToolSchemaFormat::Anthropic);
        assert_eq!(translated.len(), 1);
        assert_eq!(translated[0]["name"], "test");
        assert!(translated[0].get("input_schema").is_some());
    }

    #[test]
    fn test_build_anthropic_request() {
        let req = NormalizedRequest {
            messages: vec![
                serde_json::json!({"role": "system", "content": "You are helpful"}),
                serde_json::json!({"role": "user", "content": "Hello"}),
            ],
            model: "claude-3-5-sonnet-20241022".into(),
            temperature: 0.7,
            max_tokens: Some(4096),
            stream: true,
            tools: None,
            system_prompt: None,
        };
        let result = build_anthropic_request(&req, None);
        assert_eq!(result["system"], "You are helpful");
        assert_eq!(result["messages"].as_array().unwrap().len(), 1); // system extracted
        assert_eq!(result["model"], "claude-3-5-sonnet-20241022");
    }

    #[test]
    fn test_parse_gemini_text_delta() -> Result<(), String> {
        let data = serde_json::json!({"candidates":[{"content":{"parts":[{"text":"Hello"}]}}]});
        match parse_gemini_stream_event(&data) {
            GeminiEvent::TextDelta(t) => {
                assert_eq!(t, "Hello");
                Ok(())
            }
            other => Err(format!("Expected TextDelta, got {:?}", other)),
        }
    }

    #[test]
    fn test_parse_gemini_tool_call() -> Result<(), String> {
        let data = serde_json::json!({"candidates":[{"content":{"parts":[{"functionCall":{"name":"search","args":{"q":"test"}}}]}}]});
        match parse_gemini_stream_event(&data) {
            GeminiEvent::ToolCall { name, args } => {
                assert_eq!(name, "search");
                assert_eq!(args["q"], "test");
                Ok(())
            }
            other => Err(format!("Expected ToolCall, got {:?}", other)),
        }
    }

    #[test]
    fn test_parse_gemini_done() -> Result<(), String> {
        let data = serde_json::json!({"candidates":[{"finishReason":"STOP"}]});
        match parse_gemini_stream_event(&data) {
            GeminiEvent::Done { stop_reason } => {
                assert_eq!(stop_reason, Some("STOP".to_string()));
                Ok(())
            }
            other => Err(format!("Expected Done, got {:?}", other)),
        }
    }
}
