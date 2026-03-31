use super::{FunctionCallData, ToolCall};

/// Extract tool calls from model text output (for models without native tool calling).
/// Looks for ```tool_call JSON blocks or raw {"tool": ...} patterns.
pub fn extract_tool_calls_from_text(text: &str) -> Vec<ToolCall> {
    let mut calls = Vec::new();
    let mut counter = 0;

    // Strategy 1: Look for ```tool_call blocks
    let parts: Vec<&str> = text.split("```tool_call").collect();
    for part in parts.iter().skip(1) {
        if let Some(end) = part.find("```") {
            let json_str = part[..end].trim();
            if let Some(call) = parse_tool_call_json(json_str, &mut counter) {
                calls.push(call);
            }
        }
    }

    if !calls.is_empty() {
        return calls;
    }

    // Strategy 2: Look for ```json blocks with tool/args pattern
    let json_parts: Vec<&str> = text.split("```json").collect();
    for part in json_parts.iter().skip(1) {
        if let Some(end) = part.find("```") {
            let json_str = part[..end].trim();
            if let Some(call) = parse_tool_call_json(json_str, &mut counter) {
                calls.push(call);
            }
        }
    }

    if !calls.is_empty() {
        return calls;
    }

    // Strategy 3: Look for raw {"tool": ...} patterns
    let mut search_from = 0;
    while let Some(start) = text[search_from..].find("{\"tool\"") {
        let abs_start = search_from + start;
        if let Some(end) = find_matching_brace(&text[abs_start..]) {
            let json_str = &text[abs_start..abs_start + end + 1];
            if let Some(call) = parse_tool_call_json(json_str, &mut counter) {
                calls.push(call);
            }
            search_from = abs_start + end + 1;
        } else {
            break;
        }
    }

    calls
}

fn parse_tool_call_json(json_str: &str, counter: &mut usize) -> Option<ToolCall> {
    let repaired = repair_json(json_str);
    let v: serde_json::Value = serde_json::from_str(&repaired).ok()?;

    let tool_name = v.get("tool")?.as_str()?.to_string();
    let args = v.get("args").cloned().unwrap_or(serde_json::json!({}));

    *counter += 1;
    Some(ToolCall {
        id: format!("fallback_{}", counter),
        call_type: Some("function".to_string()),
        function: FunctionCallData {
            name: tool_name,
            arguments: serde_json::to_string(&args).unwrap_or_default(),
        },
    })
}

fn find_matching_brace(text: &str) -> Option<usize> {
    let mut depth = 0;
    let mut in_string = false;
    let mut escape_next = false;
    for (i, ch) in text.char_indices() {
        if escape_next {
            escape_next = false;
            continue;
        }
        match ch {
            '\\' if in_string => escape_next = true,
            '"' => in_string = !in_string,
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

pub fn repair_json(input: &str) -> String {
    let trimmed = input.trim();
    if serde_json::from_str::<serde_json::Value>(trimmed).is_ok() {
        return trimmed.to_string();
    }
    // Try extracting the first complete JSON object using brace matching.
    if let Some(json) = extract_first_json_object(trimmed) {
        if serde_json::from_str::<serde_json::Value>(&json).is_ok() {
            return json;
        }
    }
    // Fallback: first { to last }
    if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            let candidate = &trimmed[start..=end];
            if serde_json::from_str::<serde_json::Value>(candidate).is_ok() {
                return candidate.to_string();
            }
        }
    }
    // Try fixing common issues: trailing commas
    let mut fixed = trimmed.to_string();
    fixed = fixed.replace(",}", "}").replace(",]", "]");
    if serde_json::from_str::<serde_json::Value>(&fixed).is_ok() {
        return fixed;
    }

    // Strategy: Fix truncated JSON strings (missing closing quote + braces)
    // This handles cases where the LLM output was cut off mid-string
    if let Some(repaired) = repair_truncated_json(trimmed) {
        if serde_json::from_str::<serde_json::Value>(&repaired).is_ok() {
            return repaired;
        }
    }

    trimmed.to_string()
}

/// Attempt to repair truncated JSON by closing open strings and braces.
/// Handles cases where the LLM output was cut off mid-string value.
fn repair_truncated_json(input: &str) -> Option<String> {
    // Must start with {
    let start = input.find('{')?;
    let text = &input[start..];

    let mut result = String::with_capacity(text.len() + 10);
    let mut in_string = false;
    let mut escape_next = false;
    let mut brace_depth = 0i32;
    let mut bracket_depth = 0i32;

    for ch in text.chars() {
        result.push(ch);
        if escape_next {
            escape_next = false;
            continue;
        }
        match ch {
            '\\' if in_string => escape_next = true,
            '"' => in_string = !in_string,
            '{' if !in_string => brace_depth += 1,
            '}' if !in_string => brace_depth -= 1,
            '[' if !in_string => bracket_depth += 1,
            ']' if !in_string => bracket_depth -= 1,
            _ => {}
        }
    }

    // If we ended mid-escape, drop the trailing backslash
    if escape_next {
        result.pop();
    }

    // Close open string
    if in_string {
        result.push('"');
    }

    // Close open brackets and braces
    for _ in 0..bracket_depth {
        result.push(']');
    }
    for _ in 0..brace_depth {
        result.push('}');
    }

    Some(result)
}

/// Extract the first balanced JSON object from text using brace-depth tracking.
/// Respects string literals (skips braces inside "...").
fn extract_first_json_object(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut start = None;
    let mut depth = 0i32;
    let mut in_string = false;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if in_string {
            if b == b'\\' {
                i += 1; // skip escaped char
            } else if b == b'"' {
                in_string = false;
            }
        } else {
            match b {
                b'"' => in_string = true,
                b'{' => {
                    if start.is_none() {
                        start = Some(i);
                    }
                    depth += 1;
                }
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Some(s) = start {
                            return Some(text[s..=i].to_string());
                        }
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    None
}
