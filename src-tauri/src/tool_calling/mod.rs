mod executor;
mod healing;
mod parsing;
mod registry;
pub mod security;

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Instant;

// Re-export submodule items that form the public API
pub use healing::{build_heal_prompt, heal_strategy_name, is_fixable_error};
pub use parsing::{extract_tool_calls_from_text, repair_json};
pub use registry::{
    build_tool_injection_prompt, get_core_tool_definitions, get_core_tool_names,
    get_tool_definitions, get_tool_risk_level,
};

// ===== Risk Levels =====

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RiskLevel {
    None,
    Low,
    Medium,
    High,
}

// ===== OpenAI-compatible Tool Types =====

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub function: FunctionDefinition,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: Option<String>,
    pub function: FunctionCallData,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FunctionCallData {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct ToolExecution {
    pub name: String,
    pub result: String,
    pub success: bool,
    pub duration_ms: u64,
}

// ===== Execution Logic =====

/// Optional callback for streaming tool output line-by-line
pub type OutputCallback = Box<dyn Fn(&str) + Send + Sync>;

#[allow(dead_code)]
pub fn execute_tool_with_rag(
    call: &ToolCall,
    root_path: &str,
    rag_state: Option<&Arc<crate::rag_index::RagState>>,
) -> ToolExecution {
    execute_tool_with_rag_streaming(call, root_path, rag_state, None)
}

pub fn execute_tool_with_rag_streaming(
    call: &ToolCall,
    root_path: &str,
    rag_state: Option<&Arc<crate::rag_index::RagState>>,
    on_output: Option<&OutputCallback>,
) -> ToolExecution {
    let start = Instant::now();
    let repaired = repair_json(&call.function.arguments);
    let args: serde_json::Value = match serde_json::from_str(&repaired) {
        Ok(v) => v,
        Err(e) => {
            return ToolExecution {
                name: call.function.name.clone(),
                result: format!(
                    "Error: Bad JSON arguments: {}. Arguments received: {}",
                    e, call.function.arguments
                ),
                success: false,
                duration_ms: start.elapsed().as_millis() as u64,
            }
        }
    };

    let result =
        executor::dispatch_tool(&call.function.name, &args, root_path, rag_state, on_output);

    let (res_text, ok) = match result {
        Ok(t) => (trim_tool_result(&t, 12000), true),
        Err(e) => (format!("Error: {}", e), false),
    };

    ToolExecution {
        name: call.function.name.clone(),
        result: res_text,
        success: ok,
        duration_ms: start.elapsed().as_millis() as u64,
    }
}

/// Trim tool results that are too large, keeping start and end
fn trim_tool_result(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let keep_start = max_chars * 2 / 5;
    let keep_end = max_chars * 2 / 5;
    let start_part = &text[..keep_start];
    let end_part = &text[text.len() - keep_end..];
    let omitted = text.len() - keep_start - keep_end;
    format!(
        "{}\n\n... [{} characters omitted] ...\n\n{}",
        start_part, omitted, end_part
    )
}
