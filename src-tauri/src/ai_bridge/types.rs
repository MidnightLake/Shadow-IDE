use crate::token_optimizer::CacheStats;
use serde::{Deserialize, Serialize};

// ===== API Types (OpenAI-compatible) =====

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ChatCompletionRequest {
    pub model: String,
    pub messages: Vec<serde_json::Value>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub(crate) struct ToolChatRequest {
    pub model: String,
    pub messages: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<crate::tool_calling::ToolDefinition>>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatCompletionResponse {
    pub choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatChoice {
    pub message: ChatChoiceMessage,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ChatChoiceMessage {
    #[serde(default)]
    pub content: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamChunk {
    pub choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamChoice {
    pub delta: StreamDelta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamDelta {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<StreamToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamToolCallDelta {
    pub index: usize,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<StreamFunctionDelta>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct StreamFunctionDelta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ModelsResponse {
    pub data: Vec<ModelInfo>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AiProvider {
    pub name: String,
    pub base_url: String,
    pub available: bool,
    pub model_count: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModelProfile {
    pub category: String,
    pub recommended_temp: f64,
    pub recommended_max_tokens: i32,
    pub recommended_context: usize,
    pub supports_tools: bool,
    pub description: String,
}

pub(crate) struct ToolCallAccumulator {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct ToolCallEvent {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct ToolResultEvent {
    pub name: String,
    pub result: String,
    pub success: bool,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct FileChangeEvent {
    pub tool: String,
    pub path: String,
    pub action: String,
    pub preview: String,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct TokenStatsEvent {
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub cached: bool,
    pub cache_stats: CacheStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub breakdown: Option<TokenBreakdown>,
}

#[derive(Debug, Serialize, Clone)]
pub(crate) struct TokenBreakdown {
    pub system: usize,
    pub tools: usize,
    pub history: usize,
    pub response: usize,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CompactionStats {
    pub turns_before: usize,
    pub turns_after: usize,
    pub tokens_before: usize,
    pub tokens_after: usize,
    pub strategy: String,
}
