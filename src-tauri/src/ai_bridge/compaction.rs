use tauri::AppHandle;

use super::types::CompactionStats;
use super::{emit_ai_event, safe_truncate};
use crate::token_optimizer;

/// Extract key facts from the current session and save as a compaction memory file.
/// Called at the END of every agent session (after the loop exits), regardless of
/// whether context-threshold compaction was triggered. This ensures that even short
/// sessions or sessions that crash due to API errors still produce shadow-memory.
pub(crate) fn extract_and_save_session_memory(messages: &[serde_json::Value], root_path: &str) {
    if root_path.is_empty() {
        return;
    }
    let mut facts: Vec<String> = Vec::new();
    for msg in messages.iter() {
        let role = msg["role"].as_str().unwrap_or("");
        if role == "system" || role == "tool" {
            continue;
        }
        if let Some(content) = msg["content"].as_str() {
            if content.len() < 30 {
                continue;
            }
            if content.starts_with("[Summarized]") || content.starts_with("[KeyFacts]") {
                continue;
            }
            let extracted = extract_keyfacts(content, role);
            facts.extend(extracted);
        }
    }
    // Deduplicate
    facts.dedup();
    if facts.is_empty() {
        return;
    }

    let memory_dir = std::path::Path::new(root_path).join(".shadow-memory");
    if let Err(e) = std::fs::create_dir_all(&memory_dir) {
        log::warn!("[ai] Failed to create memory dir {:?}: {}", memory_dir, e);
        return;
    }
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let entry = serde_json::json!({
        "key": format!("compaction_{}", timestamp),
        "value": facts.join("\n"),
        "category": "compaction",
        "timestamp": timestamp,
    });
    let path = memory_dir.join(format!("compaction_{}.json", timestamp));
    if let Err(e) = std::fs::write(
        &path,
        serde_json::to_string_pretty(&entry).unwrap_or_default(),
    ) {
        log::warn!("[ai] Failed to write session memory to {:?}: {}", path, e);
    } else {
        log::info!(
            "[ai] Session memory saved: {} facts → {:?}",
            facts.len(),
            path
        );
    }

    // Regenerate memory.md from all compaction files
    generate_memory_md(root_path);
}

/// Aggregate all compaction JSON files from .shadow-memory/ into .shadowai/memory.md
/// memory.md lives in .shadowai/ (the project AI folder), compaction files stay in .shadow-memory/
pub(crate) fn generate_memory_md(root_path: &str) {
    let compaction_dir = std::path::Path::new(root_path).join(".shadow-memory");
    if !compaction_dir.exists() {
        return;
    }

    let mut entries: Vec<(u64, String, String)> = Vec::new(); // (timestamp, key, value)
    if let Ok(dir) = std::fs::read_dir(&compaction_dir) {
        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    let ts = json["timestamp"].as_u64().unwrap_or(0);
                    let key = json["key"].as_str().unwrap_or("unknown").to_string();
                    let value = json["value"].as_str().unwrap_or("").to_string();
                    if !value.is_empty() {
                        entries.push((ts, key, value));
                    }
                }
            }
        }
    }

    if entries.is_empty() {
        return;
    }

    // Sort by timestamp descending (newest first)
    entries.sort_by(|a, b| b.0.cmp(&a.0));

    let mut md = String::from("# Shadow Memory\n\n");
    md.push_str("> Auto-generated from compaction records. Do not edit manually.\n\n");

    // Only include last 20 entries to keep memory.md manageable
    for (ts, key, value) in entries.iter().take(20) {
        let datetime = format!("{}", ts);
        md.push_str(&format!("## {} ({})\n\n", key, datetime));
        for line in value.lines() {
            md.push_str(&format!("- {}\n", line));
        }
        md.push('\n');
    }

    // Write memory.md to .shadowai/ (NOT .shadow-memory/)
    let shadowai_dir = std::path::Path::new(root_path).join(".shadowai");
    if let Err(e) = std::fs::create_dir_all(&shadowai_dir) {
        log::warn!("[ai] Failed to create .shadowai dir: {}", e);
        return;
    }
    let md_path = shadowai_dir.join("memory.md");
    if let Err(e) = std::fs::write(&md_path, &md) {
        log::warn!("[ai] Failed to write memory.md: {}", e);
    } else {
        log::info!(
            "[ai] .shadowai/memory.md updated with {} entries",
            entries.len().min(20)
        );
    }
}

/// Detect if an API error response is a context overflow error.
/// Returns Some((n_prompt_tokens, n_ctx)) if it is, None otherwise.
pub(crate) fn detect_context_overflow(err_text: &str) -> Option<(usize, usize)> {
    // Check for the specific error type string
    if !err_text.contains("exceed_context_size_error")
        && !err_text.contains("exceeds the available context size")
    {
        return None;
    }
    // Try to parse n_prompt_tokens and n_ctx from the error
    let n_prompt = extract_number_after(err_text, "n_prompt_tokens\":")
        .or_else(|| extract_number_after(err_text, "request ("))
        .or_else(|| extract_number_after(err_text, "n_prompt_tokens\":"));
    let n_ctx = extract_number_after(err_text, "n_ctx\":")
        .or_else(|| extract_number_after(err_text, "context size ("))
        .or_else(|| extract_number_after(err_text, "n_ctx\":"));

    match (n_prompt, n_ctx) {
        (Some(p), Some(c)) => Some((p, c)),
        // If we can't parse numbers but detected the error type, return defaults
        _ if err_text.contains("exceed_context_size_error") => Some((0, 0)),
        _ if err_text.contains("exceeds the available context size") => Some((0, 0)),
        _ => None,
    }
}

fn extract_number_after(text: &str, marker: &str) -> Option<usize> {
    let pos = text.find(marker)?;
    let after = &text[pos + marker.len()..];
    let num_str: String = after
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect();
    num_str.parse().ok()
}

/// Save compaction memory: extract facts from messages, write compaction JSON to .shadow-memory/,
/// and regenerate .shadowai/memory.md. Called every time compaction runs.
fn save_compaction_memory(messages: &[serde_json::Value], root: &str, min_keep: usize) {
    let cutoff = messages.len().saturating_sub(min_keep);
    let mut facts: Vec<String> = Vec::new();
    for msg in messages.iter().take(cutoff) {
        let role = msg["role"].as_str().unwrap_or("");
        if role == "system" || role == "tool" {
            continue;
        }
        if let Some(content) = msg["content"].as_str() {
            if content.len() < 30 {
                continue;
            }
            if content.starts_with("[Summarized]") || content.starts_with("[KeyFacts]") {
                continue;
            }
            let extracted = extract_keyfacts(content, role);
            facts.extend(extracted);
        }
    }
    facts.dedup();
    if facts.is_empty() {
        return;
    }

    // Write compaction JSON to .shadow-memory/
    let memory_dir = std::path::Path::new(root).join(".shadow-memory");
    if let Err(e) = std::fs::create_dir_all(&memory_dir) {
        log::warn!(
            "[ai] Failed to create .shadow-memory dir {:?}: {}",
            memory_dir,
            e
        );
        return;
    }
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let entry = serde_json::json!({
        "key": format!("compaction_{}", timestamp),
        "value": facts.join("\n"),
        "category": "compaction",
        "timestamp": timestamp,
    });
    let path = memory_dir.join(format!("compaction_{}.json", timestamp));
    if let Err(e) = std::fs::write(
        &path,
        serde_json::to_string_pretty(&entry).unwrap_or_default(),
    ) {
        log::warn!(
            "[ai] Failed to write compaction memory to {:?}: {}",
            path,
            e
        );
    } else {
        log::info!(
            "[ai] Compaction memory saved: {} facts → {:?}",
            facts.len(),
            path
        );
    }

    // Regenerate .shadowai/memory.md from all compaction files
    generate_memory_md(root);
}

/// Extract key facts from a message as bullet points
pub(crate) fn extract_keyfacts(content: &str, role: &str) -> Vec<String> {
    let mut facts = Vec::new();
    let content_trimmed = content.trim();
    if content_trimmed.len() < 20 {
        return facts;
    }

    match role {
        "user" => {
            if content_trimmed.starts_with("[TOOL") {
                return facts;
            }
            let first_sentence = content_trimmed
                .split(". ")
                .next()
                .unwrap_or(content_trimmed);
            let preview = safe_truncate(first_sentence, 150);
            facts.push(format!("User: {}", preview));
        }
        "assistant" => {
            for line in content_trimmed.lines().take(20) {
                let line = line.trim();
                if line.contains("created")
                    || line.contains("modified")
                    || line.contains("deleted")
                    || line.contains("wrote to")
                    || line.contains("updated")
                {
                    let preview = safe_truncate(line, 120);
                    facts.push(format!("Action: {}", preview));
                }
                if (line.contains('/') || line.contains('\\'))
                    && (line.contains(".rs")
                        || line.contains(".ts")
                        || line.contains(".py")
                        || line.contains(".js")
                        || line.contains(".tsx")
                        || line.contains(".go"))
                {
                    let preview = safe_truncate(line, 120);
                    facts.push(format!("File: {}", preview));
                }
                if line.to_lowercase().contains("error") || line.to_lowercase().contains("fixed") {
                    let preview = safe_truncate(line, 120);
                    facts.push(format!("Note: {}", preview));
                }
            }
            if facts.is_empty() {
                let first = content_trimmed
                    .split(". ")
                    .next()
                    .unwrap_or(content_trimmed);
                let preview = safe_truncate(first, 150);
                facts.push(format!("Assistant: {}", preview));
            }
        }
        _ => {
            if content_trimmed.starts_with("[TOOL RESULT:") {
                let end = content_trimmed.find(']').unwrap_or(50).min(80);
                facts.push(content_trimmed[..end + 1].to_string());
            }
        }
    }
    facts
}

/// Score message importance (0.0 = least important, 1.0 = most important)
fn score_message_importance(content: &str, role: &str) -> f32 {
    let mut score: f32 = 0.5;

    match role {
        "assistant" => score += 0.1,
        "user" => score += 0.2,
        _ => score -= 0.1,
    }

    let len = content.len();
    if len < 30 {
        score -= 0.2;
    }
    if content.starts_with("[TOOL RESULT:") && len < 100 {
        score -= 0.3;
    }
    if content.starts_with("[Summarized]") || content.starts_with("[KeyFacts]") {
        score -= 0.3;
    }
    if content.contains("```") {
        score += 0.15;
    }

    let lower = content.to_lowercase();
    if lower.contains("error") || lower.contains("fix") || lower.contains("bug") {
        score += 0.1;
    }
    if content.contains(".rs") || content.contains(".ts") || content.contains(".py") {
        score += 0.05;
    }
    if lower.contains("must") || lower.contains("should") || lower.contains("requirement") {
        score += 0.1;
    }

    score.clamp(0.0, 1.0)
}

#[allow(dead_code)]
pub(crate) fn compact_messages(messages: &mut Vec<serde_json::Value>, max_context: usize) {
    compact_messages_with_memory(messages, max_context, None);
}

pub(crate) fn compact_messages_with_memory(
    messages: &mut Vec<serde_json::Value>,
    max_context: usize,
    root_path: Option<&str>,
) -> Option<CompactionStats> {
    compact_messages_configured(messages, max_context, root_path, 8, 200, true, "summarize")
}

pub(crate) fn emit_compaction_event(app: &AppHandle, stream_id: &str, stats: &CompactionStats) {
    let tokens_freed = stats.tokens_before.saturating_sub(stats.tokens_after);
    let turns_removed = stats.turns_before.saturating_sub(stats.turns_after);
    emit_ai_event(
        app,
        &format!("ai-compaction-{}", stream_id),
        serde_json::json!({
            "turns_compacted": turns_removed,
            "tokens_freed": tokens_freed,
            "tokens_before": stats.tokens_before,
            "tokens_after": stats.tokens_after,
            "strategy": stats.strategy,
        }),
    );
    emit_ai_event(
        app,
        &format!("ai-chat-stream-{}", stream_id),
        serde_json::json!({
            "content": format!(
                "\n📦 Context compacted: {} turns removed, {} tokens freed ({}→{}, strategy: {})\n",
                turns_removed, tokens_freed, stats.tokens_before, stats.tokens_after, stats.strategy
            )
        }),
    );
}

pub(crate) fn compact_messages_configured(
    messages: &mut Vec<serde_json::Value>,
    max_context: usize,
    root_path: Option<&str>,
    min_keep: usize,
    inline_max: usize,
    extract_memories: bool,
    strategy: &str,
) -> Option<CompactionStats> {
    let tokens_before = token_optimizer::count_message_tokens(messages);
    if tokens_before <= max_context {
        return None;
    }
    let turns_before = messages.len();

    let tool_result_max = (max_context / 6).max(1000);
    let system_max = (max_context / 4).max(2000);

    // 1. Truncate large tool results first
    for msg in messages.iter_mut() {
        if msg["role"] == "tool"
            || (msg["role"] == "user"
                && msg["content"]
                    .as_str()
                    .map(|s| s.starts_with("[TOOL RESULT:"))
                    .unwrap_or(false))
        {
            if let Some(content) = msg["content"].as_str() {
                if content.len() > tool_result_max {
                    msg["content"] = serde_json::Value::String(format!(
                        "{}... [truncated {} chars]",
                        safe_truncate(content, tool_result_max),
                        content.len() - tool_result_max
                    ));
                }
            }
        }
    }

    if token_optimizer::count_message_tokens(messages) <= max_context {
        return None;
    }

    // 2. Truncate code blocks in system messages
    for msg in messages.iter_mut() {
        if msg["role"] == "system" {
            if let Some(content) = msg["content"].as_str() {
                if content.contains("```") && content.len() > system_max {
                    msg["content"] = serde_json::Value::String(token_optimizer::truncate_smart(
                        content, system_max,
                    ));
                }
            }
        }
    }

    // 2.5 Strategy-based compaction of old messages
    let verbatim_start = messages.len().saturating_sub(min_keep);
    match strategy {
        "keyfacts" => {
            let mut keyfacts: Vec<String> = Vec::new();
            let mut remove_indices: Vec<usize> = Vec::new();
            for (i, msg) in messages.iter().enumerate() {
                if i >= verbatim_start {
                    break;
                }
                let role = msg["role"].as_str().unwrap_or("");
                if role == "system" {
                    continue;
                }
                if let Some(content) = msg["content"].as_str() {
                    if content.starts_with("[KeyFacts]") {
                        continue;
                    }
                    let facts = extract_keyfacts(content, role);
                    keyfacts.extend(facts);
                    remove_indices.push(i);
                }
            }
            if !keyfacts.is_empty() && !remove_indices.is_empty() {
                keyfacts.dedup();
                let summary = format!(
                    "[KeyFacts] Prior context:\n{}",
                    keyfacts
                        .iter()
                        .take(30)
                        .map(|f| format!("• {}", f))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
                for &idx in remove_indices.iter().rev() {
                    if idx < messages.len() {
                        messages.remove(idx);
                    }
                }
                let insert_pos = messages
                    .iter()
                    .position(|m| m["role"] != "system")
                    .unwrap_or(0);
                messages.insert(
                    insert_pos,
                    serde_json::json!({
                        "role": "user",
                        "content": summary
                    }),
                );
            }
        }
        "selective" => {
            let mut scored: Vec<(usize, f32)> = Vec::new();
            for (i, msg) in messages.iter().enumerate() {
                if i >= verbatim_start {
                    continue;
                }
                let role = msg["role"].as_str().unwrap_or("");
                if role == "system" {
                    continue;
                }
                let content = msg["content"].as_str().unwrap_or("");
                let score = score_message_importance(content, role);
                scored.push((i, score));
            }
            scored.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            let mut removed = 0;
            for (idx, _score) in &scored {
                if token_optimizer::count_message_tokens(messages) <= max_context {
                    break;
                }
                let adjusted_idx = idx - removed;
                if adjusted_idx < messages.len() {
                    messages.remove(adjusted_idx);
                    removed += 1;
                }
            }
        }
        _ => {
            // "summarize" (default): inline turn summarization
            for (i, msg) in messages.iter_mut().enumerate() {
                if i >= verbatim_start {
                    break;
                }
                let role = msg["role"].as_str().unwrap_or("");
                if role == "system" {
                    continue;
                }
                if let Some(content) = msg["content"].as_str() {
                    if content.len() > inline_max && !content.starts_with("[Summarized]") {
                        let summary = if let Some(period_pos) =
                            content[..inline_max.min(content.len())].find(". ")
                        {
                            &content[..period_pos + 1]
                        } else {
                            &content[..inline_max.min(content.len())]
                        };
                        msg["content"] = serde_json::Value::String(format!(
                            "[Summarized] {}... [{}→{} chars]",
                            summary.trim(),
                            content.len(),
                            summary.len()
                        ));
                    }
                }
            }
        }
    }

    // 3. Extract key facts and save compaction JSON + memory.md BEFORE any early return.
    // This runs every time compaction is triggered, even if the strategy already
    // reduced tokens below max_context. Previously the early return at this point
    // skipped memory extraction entirely — that's why no compaction files were created.
    if extract_memories {
        if let Some(root) = root_path {
            save_compaction_memory(messages, root, min_keep);
        }
    }

    if token_optimizer::count_message_tokens(messages) <= max_context {
        // Strategy already freed enough — still report stats since we did work
        let tokens_after = token_optimizer::count_message_tokens(messages);
        if turns_before != messages.len() || tokens_before != tokens_after {
            return Some(CompactionStats {
                turns_before,
                turns_after: messages.len(),
                tokens_before,
                tokens_after,
                strategy: strategy.to_string(),
            });
        }
        return None;
    }

    // 4. Still over limit — remove oldest non-system messages
    let mut i = 0;
    while token_optimizer::count_message_tokens(messages) > max_context
        && messages.len() > min_keep + 2
    {
        if i >= messages.len().saturating_sub(min_keep) {
            break;
        }
        if messages[i]["role"] != "system" {
            messages.remove(i);
        } else {
            i += 1;
        }
    }

    let tokens_after = token_optimizer::count_message_tokens(messages);
    if turns_before != messages.len() || tokens_before != tokens_after {
        Some(CompactionStats {
            turns_before,
            turns_after: messages.len(),
            tokens_before,
            tokens_after,
            strategy: strategy.to_string(),
        })
    } else {
        None
    }
}
