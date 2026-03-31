use ferrum_core::types::Message;

/// Generate a compaction prompt for summarizing conversation history.
pub fn compaction_prompt(messages: &[Message]) -> String {
    let mut conversation = String::new();
    let mut code_blocks = Vec::new();

    for msg in messages {
        if msg.is_compacted {
            continue;
        }
        conversation.push_str(&format!("[{}]: {}\n\n", msg.role, msg.content));

        // Extract code blocks to preserve them
        let content = &msg.content;
        let mut pos = 0;
        while let Some(start) = content[pos..].find("```") {
            let abs_start = pos + start;
            if let Some(end) = content[abs_start + 3..].find("```") {
                let block = &content[abs_start..abs_start + 3 + end + 3];
                if block.len() > 20 && block.len() < 2000 {
                    code_blocks.push(block.to_string());
                }
                pos = abs_start + 3 + end + 3;
            } else {
                break;
            }
        }
    }

    let code_section = if code_blocks.is_empty() {
        String::new()
    } else {
        let preserved: Vec<&str> = code_blocks.iter().take(5).map(|s| s.as_str()).collect();
        format!("\n\nIMPORTANT: Preserve these code blocks verbatim in your summary:\n{}", preserved.join("\n\n"))
    };

    format!(
        "Summarize the following conversation into a structured format:\n\n\
         1. **Key Decisions**: Bullet points of decisions made\n\
         2. **Current State**: What has been accomplished\n\
         3. **Open Issues**: Unresolved questions or tasks\n\
         4. **Code Changes**: Preserve important code snippets verbatim\n\n\
         Be concise but complete. Do not lose any technical details.{}\n\n\
         ---\n\n{}",
        code_section, conversation
    )
}

/// Replace all messages (except system) with a compacted summary.
pub fn compact_messages(messages: &[Message], summary: &str) -> Vec<Message> {
    let mut result = Vec::new();

    // Keep system messages
    for msg in messages {
        if msg.role == "system" {
            result.push(msg.clone());
        }
    }

    // Add compacted summary
    result.push(Message {
        role: "assistant".to_string(),
        content: format!("[COMPACTED SUMMARY]\n\n{}", summary),
        tool_calls: None,
        tool_name: None,
        token_count: 0,
        is_compacted: true,
        created_at: now_secs(),
    });

    result
}

/// Check if compaction should be triggered.
pub fn should_compact(used_tokens: u32, max_tokens: u32, threshold: f64) -> bool {
    if max_tokens == 0 {
        return false;
    }
    (used_tokens as f64 / max_tokens as f64) >= threshold
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> Message {
        Message {
            role: role.to_string(),
            content: content.to_string(),
            tool_calls: None,
            tool_name: None,
            token_count: 0,
            is_compacted: false,
            created_at: 0,
        }
    }

    #[test]
    fn should_compact_at_threshold() {
        assert!(!should_compact(700, 1000, 0.8));
        assert!(should_compact(800, 1000, 0.8));
        assert!(should_compact(900, 1000, 0.8));
    }

    #[test]
    fn should_compact_zero_max() {
        assert!(!should_compact(100, 0, 0.8));
    }

    #[test]
    fn should_compact_exact_boundary() {
        assert!(should_compact(80, 100, 0.8));
    }

    #[test]
    fn compact_messages_preserves_system() {
        let messages = vec![
            msg("system", "You are helpful."),
            msg("user", "Hello"),
            msg("assistant", "Hi there!"),
        ];
        let result = compact_messages(&messages, "Summary of conversation.");
        assert_eq!(result.len(), 2); // system + compacted
        assert_eq!(result[0].role, "system");
        assert_eq!(result[0].content, "You are helpful.");
        assert!(result[1].is_compacted);
        assert!(result[1].content.contains("[COMPACTED SUMMARY]"));
        assert!(result[1].content.contains("Summary of conversation."));
    }

    #[test]
    fn compact_messages_skips_already_compacted() {
        let mut compacted = msg("assistant", "old summary");
        compacted.is_compacted = true;
        let messages = vec![
            compacted,
            msg("user", "New question"),
            msg("assistant", "New answer"),
        ];
        let prompt = compaction_prompt(&messages);
        // The compacted message should be skipped
        assert!(!prompt.contains("old summary"));
        assert!(prompt.contains("New question"));
        assert!(prompt.contains("New answer"));
    }

    #[test]
    fn compaction_prompt_extracts_code_blocks() {
        let code_msg = msg("assistant", "Here is some code:\n```rust\nfn main() {\n    println!(\"hello\");\n}\n```\nThat's it.");
        let messages = vec![code_msg];
        let prompt = compaction_prompt(&messages);
        assert!(prompt.contains("Preserve these code blocks"));
        assert!(prompt.contains("fn main()"));
    }

    #[test]
    fn compaction_prompt_skips_tiny_code_blocks() {
        let code_msg = msg("assistant", "Use ```x``` for that.");
        let messages = vec![code_msg];
        let prompt = compaction_prompt(&messages);
        // Block is < 20 chars so should not be in "preserve" section
        assert!(!prompt.contains("Preserve these code blocks"));
    }

    #[test]
    fn compaction_prompt_formats_roles() {
        let messages = vec![
            msg("user", "What is Rust?"),
            msg("assistant", "A systems language."),
        ];
        let prompt = compaction_prompt(&messages);
        assert!(prompt.contains("[user]: What is Rust?"));
        assert!(prompt.contains("[assistant]: A systems language."));
    }

    #[test]
    fn compact_messages_empty_input() {
        let result = compact_messages(&[], "Empty summary");
        assert_eq!(result.len(), 1);
        assert!(result[0].is_compacted);
    }
}
