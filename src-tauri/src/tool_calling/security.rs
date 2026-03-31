use serde::{Deserialize, Serialize};
use std::path::Path;

// ===== PreToolUse Hook System =====

#[derive(Debug, Clone, Deserialize)]
pub struct PreToolHook {
    pub command: String,
    pub block_on_exit_nonzero: bool,
}

fn load_pre_tool_hooks(root: &str) -> Vec<PreToolHook> {
    // Try project-local hooks first, then global config
    let candidates = [
        std::path::PathBuf::from(root)
            .join(".shadowai")
            .join("hooks.toml"),
        dirs_next::config_dir()
            .map(|d| d.join("shadowai").join("hooks.toml"))
            .unwrap_or_default(),
    ];

    for path in &candidates {
        if !path.exists() {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(path) {
            // Simple TOML parse: look for [[pre_tool_use]] sections
            // We parse manually to avoid adding a TOML dep just for this
            let mut hooks = Vec::new();
            let mut in_pre_tool = false;
            let mut cur_command: Option<String> = None;
            let mut cur_block = false;

            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed == "[[pre_tool_use]]" {
                    // Save previous if complete
                    if let Some(cmd) = cur_command.take() {
                        hooks.push(PreToolHook {
                            command: cmd,
                            block_on_exit_nonzero: cur_block,
                        });
                    }
                    in_pre_tool = true;
                    cur_command = None;
                    cur_block = false;
                } else if trimmed.starts_with("[[") {
                    if let Some(cmd) = cur_command.take() {
                        hooks.push(PreToolHook {
                            command: cmd,
                            block_on_exit_nonzero: cur_block,
                        });
                    }
                    in_pre_tool = false;
                } else if in_pre_tool {
                    if let Some(rest) = trimmed.strip_prefix("command") {
                        let rest = rest.trim_start_matches(|c: char| c == ' ' || c == '=');
                        let val = rest.trim_matches('"').trim_matches('\'');
                        cur_command = Some(val.to_string());
                    } else if let Some(rest) = trimmed.strip_prefix("block_on_exit_nonzero") {
                        let rest = rest.trim_start_matches(|c: char| c == ' ' || c == '=');
                        cur_block = rest.trim() == "true";
                    }
                }
            }
            if let Some(cmd) = cur_command.take() {
                hooks.push(PreToolHook {
                    command: cmd,
                    block_on_exit_nonzero: cur_block,
                });
            }
            if !hooks.is_empty() {
                return hooks;
            }
        }
    }
    Vec::new()
}

pub fn run_pre_tool_hooks(tool_name: &str, args_json: &str, root: &str) -> Result<(), String> {
    let hooks = load_pre_tool_hooks(root);
    for hook in &hooks {
        let cmd = hook
            .command
            .replace("{tool}", tool_name)
            .replace("{args}", args_json);
        let result = std::process::Command::new("sh")
            .arg("-c")
            .arg(&cmd)
            .output();
        match result {
            Ok(output) => {
                if hook.block_on_exit_nonzero && !output.status.success() {
                    let code = output.status.code().unwrap_or(-1);
                    return Err(format!(
                        "PreToolUse hook blocked '{}': hook exited with status {}",
                        tool_name, code
                    ));
                }
            }
            Err(e) => {
                // Non-blocking error — log and continue
                log::warn!("PreToolUse hook error for '{}': {}", tool_name, e);
            }
        }
    }
    Ok(())
}

/// Commands allowed to be executed by AI tool calling.
pub(crate) const ALLOWED_COMMANDS: &[&str] = &[
    // Build tools
    "cargo", "npm", "npx", "yarn", "pnpm", "make", "cmake", "python", "python3", "pip", "pip3",
    "node", "deno", "bun", "go", "rustc", "rustup", "gcc", "g++", "clang", "javac", "java",
    // Common utilities
    "cd", "ls", "cat", "head", "tail", "wc", "sort", "uniq", "grep", "rg", "find", "which",
    "whereis", "file", "stat", "du", "df", "echo", "printf", "date", "env", "printenv", "mkdir",
    "touch", "cp", "mv", "rm", "rmdir", "sed", "awk", "xargs", "tar", "zip", "unzip", "chmod",
    "chown", "tee", "tr", "cut", "paste", "diff", "patch", "basename", "dirname", "realpath",
    // Git
    "git", // Formatters / linters
    "prettier", "eslint", "black", "ruff", "clippy", "tsc", "esbuild", "vite", "webpack",
    // Testing
    "jest", "pytest", "vitest", // System
    "ps", "top", "htop", "free", "uname", "hostname", "id", // Networking (read-only)
    "curl", "wget",
];

/// Resolve a path relative to the project root, blocking traversal outside the root.
pub(crate) fn resolve_path(p: &str, root: &str) -> Result<String, String> {
    let abs_root = std::fs::canonicalize(root)
        .map_err(|e| format!("Invalid project root '{}': {}", root, e))?;

    // Block absolute paths — force everything relative to project root
    if Path::new(p).is_absolute() {
        return Err(format!(
            "Access denied: absolute paths are not allowed (got '{}')",
            p
        ));
    }

    // Reject raw traversal components before joining
    for component in Path::new(p).components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(format!(
                "Access denied: path traversal ('..') is not allowed in '{}'",
                p
            ));
        }
    }

    let candidate = abs_root.join(p);

    // Canonicalize the full path; for new files, canonicalize the parent
    let abs = match std::fs::canonicalize(&candidate) {
        Ok(canon) => canon,
        Err(_) => {
            let parent = candidate
                .parent()
                .ok_or_else(|| format!("Invalid path: '{}'", p))?;
            let canon_parent = std::fs::canonicalize(parent).map_err(|_| {
                format!(
                    "Access denied: parent directory of '{}' does not exist within project root",
                    p
                )
            })?;
            let filename = candidate
                .file_name()
                .ok_or_else(|| format!("Invalid filename in '{}'", p))?;
            canon_parent.join(filename)
        }
    };

    if !abs.starts_with(&abs_root) {
        return Err(format!(
            "Access denied: '{}' resolves outside project root",
            p
        ));
    }
    Ok(abs.to_string_lossy().to_string())
}

// ===== Tool Whitelist =====

/// Check if a tool call is explicitly whitelisted.
pub fn is_tool_whitelisted(tool_name: &str) -> bool {
    const ALLOWED: &[&str] = &[
        "read_file",
        "write_file",
        "patch_file",
        "list_dir",
        "grep_search",
        "shell_exec",
        "web_search",
        "browse_url",
        "run_tests",
        "docker_exec",
        "notify",
        "git_status",
        "git_diff",
        "git_commit",
    ];
    ALLOWED.contains(&tool_name)
}

// ===== Secrets Detection =====

#[derive(Serialize, Deserialize, Debug)]
pub struct SecretFinding {
    pub pattern: String,
    pub line: u32,
    pub redacted_value: String,
}

/// Scan content for common secrets patterns.
/// Returns list of (pattern_name, line_number, redacted_match).
pub fn scan_content_for_secrets(content: &str) -> Vec<(String, usize, String)> {
    let mut findings = Vec::new();
    for (i, line) in content.lines().enumerate() {
        let line_no = i + 1;
        // AWS Access Key
        if let Some(pos) = find_pattern(line, "AKIA", 20) {
            // find_pattern guarantees >=20 ASCII alphanumeric chars at pos, safe to slice
            let end = (pos + 20).min(line.len());
            findings.push((
                "AWS Access Key".to_string(),
                line_no,
                redact(&line[pos..end]),
            ));
        }
        // GitHub token
        if line.contains("ghp_") || line.contains("github_pat_") {
            findings.push((
                "GitHub Token".to_string(),
                line_no,
                "[REDACTED]".to_string(),
            ));
        }
        // Generic API key pattern: key=... or apikey=... or api_key=...
        let lower = line.to_lowercase();
        if (lower.contains("api_key") || lower.contains("apikey") || lower.contains("secret_key"))
            && line.contains('=')
            && line.len() > 20
        {
            findings.push(("API Key".to_string(), line_no, "[REDACTED]".to_string()));
        }
        // Private key header
        if line.contains("-----BEGIN") && (line.contains("PRIVATE KEY") || line.contains("RSA KEY"))
        {
            findings.push(("Private Key".to_string(), line_no, "[REDACTED]".to_string()));
        }
        // JWT token pattern
        if line.contains("eyJ") && line.split('.').count() >= 3 {
            findings.push(("JWT Token".to_string(), line_no, "[REDACTED]".to_string()));
        }
    }
    findings
}

fn find_pattern(s: &str, prefix: &str, min_total_len: usize) -> Option<usize> {
    if let Some(pos) = s.find(prefix) {
        let remaining = &s[pos..];
        let alphanums: String = remaining
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        if alphanums.len() >= min_total_len {
            Some(pos)
        } else {
            None
        }
    } else {
        None
    }
}

fn redact(s: &str) -> String {
    if s.len() <= 4 {
        return "[REDACTED]".to_string();
    }
    format!("{}...{}", &s[..2], &s[s.len() - 2..])
}

/// Tauri command to scan a file for secrets.
#[tauri::command]
pub async fn scan_file_for_secrets(file_path: String) -> Result<Vec<SecretFinding>, String> {
    let content = std::fs::read_to_string(&file_path).map_err(|e| e.to_string())?;
    let raw = scan_content_for_secrets(&content);
    Ok(raw
        .into_iter()
        .map(|(pattern, line, value)| SecretFinding {
            pattern,
            line: line as u32,
            redacted_value: value,
        })
        .collect())
}

/// Parse a command string into argv tokens, rejecting shell metacharacters.
/// Supports single/double quoting and backslash escapes within double quotes.
pub(crate) fn parse_command_argv(cmd: &str) -> Result<Vec<String>, String> {
    let mut parts: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escape_next = false;

    for ch in cmd.trim().chars() {
        if escape_next {
            current.push(ch);
            escape_next = false;
            continue;
        }
        match ch {
            '\\' if !in_single => {
                escape_next = true;
            }
            '\'' if !in_double => {
                in_single = !in_single;
            }
            '"' if !in_single => {
                in_double = !in_double;
            }
            ' ' | '\t' if !in_single && !in_double => {
                if !current.is_empty() {
                    parts.push(current.clone());
                    current.clear();
                }
            }
            // Block ALL shell metacharacters — no pipes, chains, subshells, redirects
            '|' | ';' | '&' | '`' | '$' | '(' | ')' | '<' | '>' | '!' | '#'
                if !in_single && !in_double =>
            {
                return Err(format!(
                    "Shell metacharacter '{}' is not allowed. Use separate tool calls for each command.",
                    ch
                ));
            }
            _ => {
                current.push(ch);
            }
        }
    }
    if in_single || in_double {
        return Err("Unterminated quote in command".to_string());
    }
    if escape_next {
        return Err("Trailing backslash in command".to_string());
    }
    if !current.is_empty() {
        parts.push(current);
    }
    if parts.is_empty() {
        return Err("Empty command".to_string());
    }
    Ok(parts)
}
