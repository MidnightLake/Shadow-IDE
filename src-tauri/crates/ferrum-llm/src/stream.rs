use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Chunk {
    Token(String),
    Think(String),
    ToolCall(StreamToolCall),
    Done(Option<String>, Option<StreamUsage>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamToolCall {
    pub index: usize,
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

/// SSE stream chunk from OpenAI-compatible API
#[derive(Debug, Deserialize)]
pub struct SseStreamChunk {
    pub choices: Vec<SseChoice>,
    #[serde(default)]
    pub usage: Option<StreamUsage>,
}

#[derive(Debug, Deserialize)]
pub struct SseChoice {
    pub delta: SseDelta,
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SseDelta {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<SseToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
pub struct SseToolCallDelta {
    pub index: usize,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<SseFunctionDelta>,
}

#[derive(Debug, Deserialize)]
pub struct SseFunctionDelta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}
