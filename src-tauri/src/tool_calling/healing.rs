use super::ToolExecution;

/// Check if a tool result indicates a build/compile error that could be auto-fixed.
pub fn is_fixable_error(result: &ToolExecution) -> bool {
    if result.success {
        return false;
    }
    let r = result.result.to_lowercase();
    r.contains("error[e")
        || r.contains("error:")
        || r.contains("syntaxerror")
        || r.contains("typeerror")
        || r.contains("cannot find")
        || r.contains("not found in")
        || r.contains("expected ")
        || r.contains("undefined reference")
        || r.contains("no such file")
        || r.contains("block not found")
        || r.contains("old_str not found")
        || r.contains("is a directory")
        || r.contains("os error 21")
        || r.contains("specify a filename")
}

/// Self-healing strategies
#[derive(Debug, Clone)]
pub enum HealStrategy {
    RetryWithDiagnostics,
    SimplifyToolCall,
    FallbackTool,
    IncreaseTimeout,
    FixDirectoryPath,
    RequestHumanClarification,
}

/// Select the best healing strategy based on error type and attempt number
pub fn select_heal_strategy(error_output: &str, _tool_name: &str, attempt: usize) -> HealStrategy {
    let err_lower = error_output.to_lowercase();

    // "Is a directory" — path was a directory, not a file
    if err_lower.contains("is a directory")
        || err_lower.contains("os error 21")
        || err_lower.contains("specify a filename")
    {
        return HealStrategy::FixDirectoryPath;
    }

    if err_lower.contains("timed out")
        || err_lower.contains("timeout")
        || err_lower.contains("deadline exceeded")
    {
        return HealStrategy::IncreaseTimeout;
    }

    if err_lower.contains("no such file")
        || err_lower.contains("file not found")
        || err_lower.contains("not found in")
    {
        if attempt >= 2 {
            return HealStrategy::RequestHumanClarification;
        }
        return HealStrategy::FallbackTool;
    }

    if err_lower.contains("old_str not found") || err_lower.contains("block not found") {
        return HealStrategy::SimplifyToolCall;
    }

    if err_lower.contains("error[e")
        || err_lower.contains("error:")
        || err_lower.contains("syntaxerror")
    {
        if attempt >= 3 {
            return HealStrategy::RequestHumanClarification;
        }
        return HealStrategy::RetryWithDiagnostics;
    }

    if attempt >= 3 {
        return HealStrategy::RequestHumanClarification;
    }

    HealStrategy::RetryWithDiagnostics
}

/// Build a diagnostic message for the self-healing loop
pub fn build_heal_prompt(
    task_context: &str,
    tool_name: &str,
    error_output: &str,
    attempt: usize,
    max_attempts: usize,
) -> String {
    let strategy = select_heal_strategy(error_output, tool_name, attempt);

    match strategy {
        HealStrategy::RetryWithDiagnostics => format!(
            "Your previous tool call '{}' failed (attempt {}/{}).\n\n\
             Error output:\n```\n{}\n```\n\n\
             Task context: {}\n\n\
             Fix the issue and try again. Rules:\n\
             1. Read the error carefully — fix ONLY what the error indicates\n\
             2. If the error is about a file path, read the file or list the directory first\n\
             3. If old_str was not found, read the file to see the actual content\n\
             4. Prefer minimal targeted patches over full rewrites\n\
             5. If you cannot fix this after {} attempts, explain what went wrong",
            tool_name, attempt, max_attempts, error_output, task_context, max_attempts
        ),
        HealStrategy::SimplifyToolCall => format!(
            "Your previous '{}' call failed because the target text wasn't found (attempt {}/{}).\n\n\
             Error: {}\n\n\
             Strategy: SIMPLIFY your approach:\n\
             1. First, use read_file to see the actual current content of the file\n\
             2. Then use a smaller, more precise old_str that matches exactly\n\
             3. If the file structure changed, re-read it before patching\n\
             4. Consider using write_file instead if the patch is too complex",
            tool_name, attempt, max_attempts, error_output
        ),
        HealStrategy::FallbackTool => format!(
            "Your previous '{}' call failed — file or path not found (attempt {}/{}).\n\n\
             Error: {}\n\n\
             Strategy: USE ALTERNATIVE TOOLS to find the correct path:\n\
             1. Use list_dir to explore the directory structure\n\
             2. Use grep_search to find the file by content\n\
             3. Check if the path is relative to the project root\n\
             4. Then retry with the correct path",
            tool_name, attempt, max_attempts, error_output
        ),
        HealStrategy::FixDirectoryPath => format!(
            "Your previous '{}' call failed because you gave a **directory path** instead of a file path (attempt {}/{}).\n\n\
             Error: {}\n\n\
             Strategy: ADD A FILENAME to the path:\n\
             1. The path you used points to a directory, not a file\n\
             2. Append a filename — e.g. if you used '/home/user/MyProject', use '/home/user/MyProject/PLAN.md'\n\
             3. For a game engine plan, write to '{{project_dir}}/PLAN.md' or '{{project_dir}}/docs/PLAN.md'\n\
             4. IMPORTANT: If the file content is large (> 200 lines), split it across multiple write_file or patch_file calls — never generate more than ~150 lines of content in a single tool call JSON argument, as longer strings get truncated",
            tool_name, attempt, max_attempts, error_output
        ),
        HealStrategy::IncreaseTimeout => format!(
            "Your previous '{}' call timed out (attempt {}/{}).\n\n\
             Error: {}\n\n\
             Strategy: The operation took too long. Options:\n\
             1. If using shell_exec, add timeout_secs with a larger value (e.g. 120)\n\
             2. Break the command into smaller parts\n\
             3. Check if the process is stuck (use process_list to verify)\n\
             4. Try a simpler alternative command",
            tool_name, attempt, max_attempts, error_output
        ),
        HealStrategy::RequestHumanClarification => format!(
            "Multiple attempts to fix this have failed ({}/{}).\n\n\
             Tool: {}\nLast error:\n```\n{}\n```\n\n\
             Task context: {}\n\n\
             Please explain to the user:\n\
             1. What you were trying to do\n\
             2. What errors you encountered\n\
             3. What you think the root cause is\n\
             4. Ask the user for guidance on how to proceed",
            attempt, max_attempts, tool_name, error_output, task_context
        ),
    }
}

/// Get the strategy name for display
pub fn heal_strategy_name(error_output: &str, tool_name: &str, attempt: usize) -> &'static str {
    match select_heal_strategy(error_output, tool_name, attempt) {
        HealStrategy::RetryWithDiagnostics => "RetryWithDiagnostics",
        HealStrategy::SimplifyToolCall => "SimplifyToolCall",
        HealStrategy::FallbackTool => "FallbackTool",
        HealStrategy::FixDirectoryPath => "FixDirectoryPath",
        HealStrategy::IncreaseTimeout => "IncreaseTimeout",
        HealStrategy::RequestHumanClarification => "RequestHumanClarification",
    }
}
