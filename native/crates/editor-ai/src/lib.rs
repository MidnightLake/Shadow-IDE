use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

// ── Chat types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub content: String,
    pub model: String,
    pub usage_tokens: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct AiSuggestion {
    pub title: String,
    pub rationale: String,
    pub action: String,
}

// ── Provider trait ───────────────────────────────────────────────────

pub trait AiProvider: Send {
    fn name(&self) -> &str;
    fn chat(&self, messages: &[ChatMessage], system: Option<&str>) -> Result<ChatResponse>;
}

// ── LLM Loader provider (OpenAI-compatible, localhost:8080) ─────────

pub struct LlmLoaderProvider {
    pub base_url: String,
    pub model: String,
}

impl Default for LlmLoaderProvider {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:8080/v1".into(),
            model: "default".into(),
        }
    }
}

impl AiProvider for LlmLoaderProvider {
    fn name(&self) -> &str {
        "LLM Loader"
    }

    fn chat(&self, messages: &[ChatMessage], system: Option<&str>) -> Result<ChatResponse> {
        let mut msgs: Vec<serde_json::Value> = Vec::new();
        if let Some(sys) = system {
            msgs.push(serde_json::json!({"role": "system", "content": sys}));
        }
        for m in messages {
            msgs.push(serde_json::json!({"role": &m.role, "content": &m.content}));
        }

        let body = serde_json::json!({
            "model": &self.model,
            "messages": msgs,
            "temperature": 0.7,
            "max_tokens": 2048,
        });

        let url = format!("{}/chat/completions", self.base_url);
        let resp = reqwest::blocking::Client::new()
            .post(&url)
            .json(&body)
            .send()
            .context("failed to reach LLM Loader")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            bail!("LLM Loader returned {}: {}", status, text);
        }

        let json: serde_json::Value = resp.json().context("invalid JSON from LLM Loader")?;
        let content = json["choices"][0]["message"]["content"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let model = json["model"].as_str().unwrap_or("unknown").to_string();
        let usage = json["usage"]["total_tokens"].as_u64().map(|v| v as u32);

        Ok(ChatResponse {
            content,
            model,
            usage_tokens: usage,
        })
    }
}

// ── Anthropic provider ──────────────────────────────────────────────

pub struct AnthropicProvider {
    pub api_key: String,
    pub model: String,
}

impl AnthropicProvider {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            model: "claude-sonnet-4-20250514".into(),
        }
    }

    pub fn from_env() -> Result<Self> {
        let key = std::env::var("ANTHROPIC_API_KEY")
            .context("ANTHROPIC_API_KEY not set")?;
        Ok(Self::new(key))
    }
}

impl AiProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "Anthropic"
    }

    fn chat(&self, messages: &[ChatMessage], system: Option<&str>) -> Result<ChatResponse> {
        let msgs: Vec<serde_json::Value> = messages
            .iter()
            .map(|m| serde_json::json!({"role": &m.role, "content": &m.content}))
            .collect();

        let mut body = serde_json::json!({
            "model": &self.model,
            "messages": msgs,
            "max_tokens": 4096,
        });

        if let Some(sys) = system {
            body["system"] = serde_json::Value::String(sys.to_string());
        }

        let resp = reqwest::blocking::Client::new()
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .context("failed to reach Anthropic API")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().unwrap_or_default();
            bail!("Anthropic API returned {}: {}", status, text);
        }

        let json: serde_json::Value = resp.json().context("invalid JSON from Anthropic")?;
        let content = json["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let model = json["model"].as_str().unwrap_or("unknown").to_string();
        let usage = json["usage"]["output_tokens"].as_u64().map(|v| v as u32);

        Ok(ChatResponse {
            content,
            model,
            usage_tokens: usage,
        })
    }
}

// ── AI Service (public API) ─────────────────────────────────────────

pub struct AiService {
    pub provider_name: String,
    pub autocomplete_model: String,
    pub chat_model: String,
    provider: Option<Box<dyn AiProvider>>,
    pub history: Vec<ChatMessage>,
    suggestions: Vec<AiSuggestion>,
}

impl Default for AiService {
    fn default() -> Self {
        Self {
            provider_name: "LLM Loader".into(),
            autocomplete_model: "deepseek-coder-v2:16b".into(),
            chat_model: "default".into(),
            provider: None,
            history: Vec::new(),
            suggestions: vec![
                AiSuggestion {
                    title: "Add RigidBody".into(),
                    rationale: "Selected entity has geometry but no physics component.".into(),
                    action: "Would call set_component via the stable C ABI bridge.".into(),
                },
                AiSuggestion {
                    title: "Generate Stamina Component".into(),
                    rationale: "PlayerController matches a typical stamina-driven loop.".into(),
                    action: "Would create .h/.cpp files and queue an incremental rebuild.".into(),
                },
                AiSuggestion {
                    title: "Fix Missing compile_commands".into(),
                    rationale: "clangd quality depends on compile flags from editor-build.".into(),
                    action: "Would trigger compile_commands.json generation.".into(),
                },
            ],
        }
    }
}

impl AiService {
    pub fn with_llm_loader() -> Self {
        let mut svc = Self::default();
        svc.provider = Some(Box::new(LlmLoaderProvider::default()));
        svc
    }

    pub fn with_anthropic(api_key: String) -> Self {
        let provider = AnthropicProvider::new(api_key.clone());
        Self {
            provider_name: "Anthropic".into(),
            autocomplete_model: String::new(),
            chat_model: provider.model.clone(),
            provider: Some(Box::new(provider)),
            history: Vec::new(),
            suggestions: Vec::new(),
        }
    }

    pub fn set_provider(&mut self, provider: Box<dyn AiProvider>) {
        self.provider_name = provider.name().to_string();
        self.provider = Some(provider);
    }

    pub fn chat(&mut self, user_message: &str) -> Result<String> {
        self.history.push(ChatMessage {
            role: "user".into(),
            content: user_message.into(),
        });

        let provider = self.provider.as_ref()
            .context("no AI provider configured")?;

        let system = "You are an AI assistant embedded in ShadowEditor, a C++23 game editor. \
            Help the user with game development tasks: writing components, debugging builds, \
            setting up scenes, and optimizing performance. Be concise and code-focused.";

        let response = provider.chat(&self.history, Some(system))?;

        self.history.push(ChatMessage {
            role: "assistant".into(),
            content: response.content.clone(),
        });

        Ok(response.content)
    }

    pub fn explain_code(&self, code: &str, language: &str) -> Result<String> {
        let provider = self.provider.as_ref()
            .context("no AI provider configured")?;

        let msg = ChatMessage {
            role: "user".into(),
            content: format!("Explain this {} code concisely:\n```{}\n{}\n```", language, language, code),
        };

        let response = provider.chat(&[msg], None)?;
        Ok(response.content)
    }

    pub fn suggest_fix(&self, error: &str, code_context: &str) -> Result<String> {
        let provider = self.provider.as_ref()
            .context("no AI provider configured")?;

        let msg = ChatMessage {
            role: "user".into(),
            content: format!(
                "Fix this build error. Return only the corrected code.\n\nError:\n{}\n\nCode:\n{}",
                error, code_context
            ),
        };

        let system = "You are a C++23 expert. Return only corrected code, no explanation.";
        let response = provider.chat(&[msg], Some(system))?;
        Ok(response.content)
    }

    /// Like `chat()` but injects game-project context into the system prompt.
    ///
    /// * `context_snippet` – a pre-built string assembled by the caller from
    ///   scene entities, reflection JSON, active C++ buffer, and recent console
    ///   output.  Pass an empty string if no context is available.
    pub fn chat_with_context(&mut self, user_message: &str, context_snippet: &str) -> Result<String> {
        let system = if context_snippet.is_empty() {
            "You are an AI assistant embedded in ShadowEditor, a C++23 game editor. \
             Help the user with game development tasks: writing components, debugging builds, \
             setting up scenes, and optimizing performance. Be concise and code-focused.".to_string()
        } else {
            format!(
                "You are an AI assistant embedded in ShadowEditor, a C++23 game editor. \
                 Help the user with game development tasks. Be concise and code-focused.\n\n\
                 == Current project context ==\n{context_snippet}"
            )
        };

        self.history.push(ChatMessage {
            role: "user".into(),
            content: user_message.into(),
        });

        let provider = self.provider.as_ref().context("no AI provider configured")?;
        let response = provider.chat(&self.history, Some(&system))?;

        self.history.push(ChatMessage {
            role: "assistant".into(),
            content: response.content.clone(),
        });

        Ok(response.content)
    }

    pub fn clear_history(&mut self) {
        self.history.clear();
    }

    pub fn routing_summary(&self) -> String {
        let status = if self.provider.is_some() { "connected" } else { "disconnected" };
        format!(
            "{} ({}) | autocomplete: {} | chat: {}",
            self.provider_name, status, self.autocomplete_model, self.chat_model
        )
    }

    pub fn inspector_suggestions(&self) -> &[AiSuggestion] {
        &self.suggestions
    }

    pub fn is_connected(&self) -> bool {
        self.provider.is_some()
    }
}
