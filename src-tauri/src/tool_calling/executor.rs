use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use super::security::{parse_command_argv, resolve_path, ALLOWED_COMMANDS};
use super::OutputCallback;

/// Main tool dispatcher — routes tool name to the appropriate implementation.
pub(crate) fn dispatch_tool(
    name: &str,
    args: &serde_json::Value,
    root_path: &str,
    rag_state: Option<&Arc<crate::rag_index::RagState>>,
    on_output: Option<&OutputCallback>,
) -> Result<String, String> {
    // Run pre-tool hooks; block if a hook returns non-zero and is configured to block
    let args_str = serde_json::to_string(args).unwrap_or_default();
    super::security::run_pre_tool_hooks(name, &args_str, root_path)?;

    match name {
        "read_file" => exec_read_file(args, root_path),
        "write_file" => exec_write_file(args, root_path),
        "patch_file" | "edit_file" => exec_patch_file(args, root_path),
        "list_dir" | "list_directory" => exec_list_dir(args, root_path),
        "shell_exec" | "run_command" => exec_shell(args, root_path, on_output),
        "grep_search" => exec_grep_search(args, root_path),
        "git_op" => exec_git_op(args, root_path),
        "git_status" => exec_git_op(&serde_json::json!({"operation": "status"}), root_path),
        "git_log" => {
            let limit = args["limit"].as_u64().unwrap_or(5);
            exec_git_op(
                &serde_json::json!({"operation": "log", "args": ["-n", limit.to_string()]}),
                root_path,
            )
        }
        "git_commit" => {
            let msg = args["message"].as_str().unwrap_or("Update");
            exec_git_op(
                &serde_json::json!({"operation": "commit", "args": ["-m", msg]}),
                root_path,
            )
        }
        "rag_query" | "rag_search" => exec_rag_search(args, rag_state),
        "http_request" => exec_http_request(args),
        "calculator" => exec_calculator(args),
        "web_fetch" => exec_web_fetch(args),
        "web_search" => exec_web_search(args),
        "browse_url" => exec_browse_url(args),
        "run_tests" => exec_run_tests(args),
        "docker_exec" => exec_docker_exec(args),
        "notify" => exec_notify(args),
        "memory_store" => exec_memory_store(args, root_path),
        "memory_recall" => exec_memory_recall(args, root_path),
        "env_read" => exec_env_read(args),
        "cargo_run" => exec_cargo_run(args, root_path),
        "code_diagnostics" => exec_code_diagnostics(args),
        "process_list" => exec_process_list(args),
        "rag_index" => exec_rag_index(args, root_path, rag_state),
        "rag_list_sources" => exec_rag_list_sources(rag_state),
        "docs_search" => exec_docs_search(args),
        "json_query" => exec_json_query(args, root_path),
        "symbol_lookup" => exec_symbol_lookup(args, root_path),
        "cache_lookup" => exec_cache_lookup(args),
        "task_schedule" => exec_task_schedule(args, root_path),
        "read_image" => {
            let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
            exec_read_image(path)
        }
        "database_query" => {
            let dsn = args.get("dsn").and_then(|v| v.as_str()).unwrap_or("");
            let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");
            exec_database_query(dsn, query)
        }
        "deploy" => {
            let target = args
                .get("target")
                .and_then(|v| v.as_str())
                .unwrap_or("staging");
            let project_path = args
                .get("project_path")
                .and_then(|v| v.as_str())
                .unwrap_or(".");
            exec_deploy(target, project_path)
        }
        other => Err(format!(
            "Unknown tool '{}'. Available: shell_exec, read_file, write_file, patch_file, \
             list_dir, grep_search, git_op, rag_query, http_request, calculator, web_fetch, \
             web_search, browse_url, run_tests, docker_exec, notify, memory_store, memory_recall, \
             env_read, cargo_run, code_diagnostics, process_list, rag_index, rag_list_sources, \
             docs_search, json_query, symbol_lookup, cache_lookup, task_schedule, \
             read_image, database_query, deploy",
            other
        )),
    }
}

// ===== File Operations =====

fn exec_read_file(args: &serde_json::Value, root: &str) -> Result<String, String> {
    let p = resolve_path(args["path"].as_str().unwrap_or(""), root)?;
    let content =
        std::fs::read_to_string(&p).map_err(|e| format!("Failed to read '{}': {}", p, e))?;

    let start_line = args["start_line"].as_u64().unwrap_or(0) as usize;
    let end_line = args["end_line"].as_u64().unwrap_or(0) as usize;

    if start_line > 0 || end_line > 0 {
        let lines: Vec<&str> = content.lines().collect();
        let total = lines.len();
        if start_line < 1 || start_line > total || end_line < start_line {
            return Err(format!(
                "Invalid line range: {}-{} (file has {} lines)",
                start_line, end_line, total
            ));
        }
        let start = start_line - 1;
        let end = if end_line > 0 {
            end_line.min(total)
        } else {
            total
        };
        let selected: Vec<String> = lines[start..end]
            .iter()
            .enumerate()
            .map(|(i, l)| format!("{:4}: {}", start + i + 1, l))
            .collect();
        Ok(format!(
            "File: {} (lines {}-{} of {})\n{}",
            p,
            start + 1,
            end,
            lines.len(),
            selected.join("\n")
        ))
    } else {
        let line_count = content.lines().count();
        Ok(format!("File: {} ({} lines)\n{}", p, line_count, content))
    }
}

fn exec_write_file(args: &serde_json::Value, root: &str) -> Result<String, String> {
    let p = resolve_path(args["path"].as_str().unwrap_or(""), root)?;
    let c = args["content"].as_str().unwrap_or("");
    let existed = Path::new(&p).exists();
    let old_content = if existed {
        std::fs::read_to_string(&p).ok()
    } else {
        None
    };
    if let Some(parent) = Path::new(&p).parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&p, c).map_err(|e| format!("Failed to write '{}': {}", p, e))?;
    let lines = c.lines().count();
    let action = if existed { "updated" } else { "created" };

    let mut result = format!(
        "File {}: {} ({} lines, {} bytes)",
        action,
        p,
        lines,
        c.len()
    );

    if let Some(old) = old_content {
        let old_lines: Vec<&str> = old.lines().collect();
        let new_lines: Vec<&str> = c.lines().collect();
        let added = new_lines.len().saturating_sub(old_lines.len());
        let removed = old_lines.len().saturating_sub(new_lines.len());
        result.push_str(&format!("\n+{} -{} lines changed", added.max(1), removed));

        let diff_preview = build_mini_diff(&old, c, 12);
        if !diff_preview.is_empty() {
            result.push_str(&format!("\n{}", diff_preview));
        }
    } else {
        let preview: String = c.lines().take(15).collect::<Vec<_>>().join("\n");
        if lines > 15 {
            result.push_str(&format!("\n{}\n... ({} more lines)", preview, lines - 15));
        } else {
            result.push_str(&format!("\n{}", preview));
        }
    }

    Ok(result)
}

fn build_mini_diff(old: &str, new: &str, max_lines: usize) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let mut out = Vec::new();

    let max_i = old_lines.len().max(new_lines.len());
    let mut i_old = 0usize;
    let mut i_new = 0usize;

    while i_old < old_lines.len() || i_new < new_lines.len() {
        if out.len() >= max_lines {
            out.push("...".to_string());
            break;
        }

        let ol = old_lines.get(i_old).copied();
        let nl = new_lines.get(i_new).copied();

        match (ol, nl) {
            (Some(o), Some(n)) if o == n => {
                i_old += 1;
                i_new += 1;
            }
            (Some(o), Some(n)) => {
                out.push(format!("- {}", truncate_line(o, 100)));
                out.push(format!("+ {}", truncate_line(n, 100)));
                i_old += 1;
                i_new += 1;
            }
            (Some(o), None) => {
                out.push(format!("- {}", truncate_line(o, 100)));
                i_old += 1;
            }
            (None, Some(n)) => {
                out.push(format!("+ {}", truncate_line(n, 100)));
                i_new += 1;
            }
            (None, None) => break,
        }

        if i_old >= max_i && i_new >= max_i {
            break;
        }
    }

    out.join("\n")
}

fn truncate_line(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn format_patch_diff(old: &str, new: &str, max_lines: usize) -> String {
    let mut out = Vec::new();
    for line in old.lines().take(max_lines / 2) {
        out.push(format!("- {}", truncate_line(line, 100)));
    }
    if old.lines().count() > max_lines / 2 {
        out.push(format!(
            "  ... ({} more removed)",
            old.lines().count() - max_lines / 2
        ));
    }
    for line in new.lines().take(max_lines / 2) {
        out.push(format!("+ {}", truncate_line(line, 100)));
    }
    if new.lines().count() > max_lines / 2 {
        out.push(format!(
            "  ... ({} more added)",
            new.lines().count() - max_lines / 2
        ));
    }
    out.join("\n")
}

fn exec_patch_file(args: &serde_json::Value, root: &str) -> Result<String, String> {
    let p = resolve_path(args["path"].as_str().unwrap_or(""), root)?;
    let find = args
        .get("old_str")
        .or_else(|| args.get("find"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let replace = args
        .get("new_str")
        .or_else(|| args.get("replace"))
        .and_then(|v| v.as_str())
        .unwrap_or("");

    let content = std::fs::read_to_string(&p).map_err(|e| e.to_string())?;

    // Strategy 1: exact match
    if content.contains(find) {
        let count = content.matches(find).count();
        if count > 1 {
            return Err(format!(
                "old_str found {} times in {}. It must be unique. Add more surrounding context to disambiguate.",
                count, p
            ));
        }
        let new_content = content.replacen(find, replace, 1);
        std::fs::write(&p, &new_content).map_err(|e| e.to_string())?;
        let find_lines = find.lines().count();
        let replace_lines = replace.lines().count();
        let mut result = format!(
            "Patched: {} (exact match, {} → {} lines)",
            p, find_lines, replace_lines
        );
        result.push_str(&format!("\n{}", format_patch_diff(find, replace, 10)));
        return Ok(result);
    }

    // Strategy 2: fuzzy line-trimmed match
    let find_lines: Vec<&str> = find
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty())
        .collect();
    if find_lines.is_empty() {
        return Err("old_str/find string is empty".into());
    }
    let content_lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();

    for i in 0..content_lines.len() {
        if i + find_lines.len() > content_lines.len() {
            break;
        }
        let mut match_ok = true;
        for j in 0..find_lines.len() {
            if content_lines[i + j].trim() != find_lines[j] {
                match_ok = false;
                break;
            }
        }
        if match_ok {
            let mut next = content_lines.clone();
            let replace_lines_vec: Vec<String> = replace.lines().map(|l| l.to_string()).collect();
            next.splice(i..i + find_lines.len(), replace_lines_vec);
            std::fs::write(&p, next.join("\n")).map_err(|e| e.to_string())?;
            let mut result = format!("Patched: {} (fuzzy match at line {})", p, i + 1);
            result.push_str(&format!("\n{}", format_patch_diff(find, replace, 10)));
            return Ok(result);
        }
    }

    Err(format!(
        "old_str not found in '{}'. Read the file first to verify the exact content and indentation.",
        p
    ))
}

fn exec_list_dir(args: &serde_json::Value, root: &str) -> Result<String, String> {
    let p = resolve_path(args["path"].as_str().unwrap_or("."), root)?;
    let depth = args["depth"].as_u64().unwrap_or(1) as usize;
    let show_hidden = args["show_hidden"].as_bool().unwrap_or(false);

    let mut result = Vec::new();
    list_dir_recursive(&p, root, depth, 0, show_hidden, &mut result)?;
    Ok(result.join("\n"))
}

fn list_dir_recursive(
    path: &str,
    root: &str,
    max_depth: usize,
    current_depth: usize,
    show_hidden: bool,
    result: &mut Vec<String>,
) -> Result<(), String> {
    if current_depth > max_depth {
        return Ok(());
    }
    let entries = std::fs::read_dir(path).map_err(|e| format!("Cannot read '{}': {}", path, e))?;

    let indent = "  ".repeat(current_depth);
    let mut sorted: Vec<_> = entries.flatten().collect();
    sorted.sort_by_key(|e| e.file_name());

    for entry in sorted {
        let name = entry.file_name().to_string_lossy().to_string();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        if current_depth == 0
            && (name == "node_modules"
                || name == "target"
                || name == ".git"
                || name == "__pycache__"
                || name == ".next"
                || name == "dist"
                || name == "build")
        {
            continue;
        }

        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let tag = if is_dir { "[DIR]" } else { "[FILE]" };
        result.push(format!("{}{} {}", indent, tag, name));

        if is_dir && current_depth < max_depth {
            let sub_path = format!("{}/{}", path, name);
            list_dir_recursive(
                &sub_path,
                root,
                max_depth,
                current_depth + 1,
                show_hidden,
                result,
            )?;
        }
    }
    Ok(())
}

// ===== Shell Execution =====

fn exec_shell(
    args: &serde_json::Value,
    root: &str,
    on_output: Option<&OutputCallback>,
) -> Result<String, String> {
    let cmd = args["command"].as_str().unwrap_or("");
    let working_dir = args["working_dir"]
        .as_str()
        .map(|d| resolve_path(d, root).unwrap_or_else(|_| root.to_string()))
        .unwrap_or_else(|| root.to_string());
    let timeout_secs = args["timeout_secs"].as_u64().unwrap_or(30);

    if cmd.trim().is_empty() {
        return Err("Empty command".to_string());
    }

    let argv = parse_command_argv(cmd)?;

    let base_name = Path::new(&argv[0])
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_else(|| argv[0].clone());
    if !ALLOWED_COMMANDS.contains(&base_name.as_str()) {
        return Err(format!(
            "Command '{}' is not allowed. Allowed: {}",
            base_name,
            ALLOWED_COMMANDS.join(", ")
        ));
    }

    let dangerous_patterns = ["--exec", "-exec"];
    let cmd_lower = cmd.to_lowercase();
    for pattern in &dangerous_patterns {
        if cmd_lower.contains(pattern) {
            return Err(format!("Blocked dangerous pattern: '{}'", pattern));
        }
    }

    let mut proc = Command::new(&argv[0]);
    proc.args(&argv[1..])
        .current_dir(&working_dir)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    crate::platform::hide_window(&mut proc);
    let mut child = proc
        .spawn()
        .map_err(|e| format!("Failed to run '{}': {}", cmd, e))?;

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();

    let (stdout_tx, stdout_rx) = std::sync::mpsc::channel::<String>();
    let (stderr_tx, stderr_rx) = std::sync::mpsc::channel::<String>();

    let stdout_handle = if let Some(pipe) = stdout_pipe {
        let tx = stdout_tx;
        Some(std::thread::spawn(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(pipe);
            for line in reader.lines().flatten() {
                let _ = tx.send(line);
            }
        }))
    } else {
        drop(stdout_tx);
        None
    };

    let stderr_handle = if let Some(pipe) = stderr_pipe {
        let tx = stderr_tx;
        Some(std::thread::spawn(move || {
            use std::io::BufRead;
            let reader = std::io::BufReader::new(pipe);
            for line in reader.lines().flatten() {
                let _ = tx.send(line);
            }
        }))
    } else {
        drop(stderr_tx);
        None
    };

    let timeout_dur = std::time::Duration::from_secs(timeout_secs);
    let start = std::time::Instant::now();
    let mut stdout_full = String::new();
    let mut stderr_full = String::new();

    loop {
        while let Ok(line) = stdout_rx.try_recv() {
            stdout_full.push_str(&line);
            stdout_full.push('\n');
            if let Some(cb) = on_output {
                cb(&line);
                cb("\n");
            }
        }

        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if start.elapsed() > timeout_dur {
                    let _ = child.kill();
                    return Err(format!(
                        "Command '{}' timed out after {}s and was killed",
                        cmd, timeout_secs
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => return Err(format!("Error waiting for command: {}", e)),
        }
    }

    if let Some(h) = stdout_handle {
        let _ = h.join();
    }
    if let Some(h) = stderr_handle {
        let _ = h.join();
    }

    while let Ok(line) = stdout_rx.try_recv() {
        stdout_full.push_str(&line);
        stdout_full.push('\n');
        if let Some(cb) = on_output {
            cb(&line);
            cb("\n");
        }
    }
    while let Ok(line) = stderr_rx.try_recv() {
        stderr_full.push_str(&line);
        stderr_full.push('\n');
    }
    if let Some(cb) = on_output {
        if !stderr_full.is_empty() {
            cb(&format!("[stderr]\n{}", stderr_full));
        }
    }

    let stdout = stdout_full;
    let stderr = stderr_full;
    let exit_code = child.wait().map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);

    let mut result = String::new();
    result.push_str(&format!("Exit code: {}\n", exit_code));
    if !stdout.is_empty() {
        result.push_str(&format!("Stdout:\n{}\n", stdout));
    }
    if !stderr.is_empty() {
        result.push_str(&format!("Stderr:\n{}\n", stderr));
    }
    if stdout.is_empty() && stderr.is_empty() {
        result.push_str("(no output)\n");
    }

    Ok(result)
}

// ===== Code Search & Git =====

fn exec_grep_search(args: &serde_json::Value, root: &str) -> Result<String, String> {
    let pattern = args["pattern"].as_str().unwrap_or("");
    if pattern.is_empty() {
        return Err("Search pattern is required".to_string());
    }

    let search_path = resolve_path(args["path"].as_str().unwrap_or("."), root)?;
    let context = args["context_lines"].as_u64().unwrap_or(2);
    let case_sensitive = args["case_sensitive"].as_bool().unwrap_or(false);
    let max_results = args["max_results"].as_u64().unwrap_or(50) as usize;
    let file_glob = args["file_glob"].as_str();

    let mut cmd_args = vec![
        "--color=never".to_string(),
        "-n".to_string(),
        format!("-C{}", context),
    ];
    if !case_sensitive {
        cmd_args.push("-i".to_string());
    }
    if let Some(glob) = file_glob {
        cmd_args.push("-g".to_string());
        cmd_args.push(glob.to_string());
    }
    cmd_args.push(format!("-m{}", max_results));
    cmd_args.push("--".to_string());
    cmd_args.push(pattern.to_string());
    cmd_args.push(search_path.clone());

    let rg_result = {
        let mut cmd = Command::new("rg");
        cmd.args(&cmd_args);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
    };

    let out = match rg_result {
        Ok(o) => o,
        Err(_) => {
            let mut grep_args = vec!["-rn".to_string(), format!("-C{}", context)];
            if !case_sensitive {
                grep_args.push("-i".to_string());
            }
            grep_args.push("--".to_string());
            grep_args.push(pattern.to_string());
            grep_args.push(search_path);
            let mut cmd = Command::new("grep");
            cmd.args(&grep_args);
            crate::platform::hide_window(&mut cmd);
            cmd.output()
                .map_err(|e| format!("Neither rg nor grep available: {}", e))?
        }
    };

    let stdout = String::from_utf8_lossy(&out.stdout);
    if stdout.trim().is_empty() {
        return Ok(format!("No matches found for pattern '{}'", pattern));
    }

    let lines: Vec<&str> = stdout.lines().collect();
    if lines.len() > max_results * 5 {
        Ok(format!(
            "{}\n... [{} more lines, use max_results to see more]",
            lines[..max_results * 5].join("\n"),
            lines.len() - max_results * 5
        ))
    } else {
        Ok(stdout.to_string())
    }
}

fn exec_git_op(args: &serde_json::Value, root: &str) -> Result<String, String> {
    let operation = args["operation"].as_str().unwrap_or("status");
    let extra_args: Vec<String> = args["args"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let git_path = args["path"]
        .as_str()
        .map(|p| resolve_path(p, root).unwrap_or_else(|_| root.to_string()))
        .unwrap_or_else(|| root.to_string());

    let mut cmd_args: Vec<String> = match operation {
        "status" => vec!["status".into(), "--short".into()],
        "diff" => {
            let mut a = vec!["diff".into()];
            if extra_args.is_empty() {
                a.push("--stat".into());
            }
            a
        }
        "log" => {
            let mut a = vec!["log".into(), "--oneline".into()];
            if extra_args.is_empty() {
                a.push("-n".into());
                a.push("10".into());
            }
            a
        }
        "add" => {
            let mut a = vec!["add".into()];
            if extra_args.is_empty() {
                a.push("-u".into());
            }
            a
        }
        "commit" => vec!["commit".into()],
        "checkout" => vec!["checkout".into()],
        "branch" => vec!["branch".into()],
        "stash" => vec!["stash".into()],
        "blame" => vec!["blame".into()],
        other => return Err(format!("Unknown git operation: {}", other)),
    };

    cmd_args.extend(extra_args);

    if operation == "commit" {
        let mut add_cmd = Command::new("git");
        add_cmd.args(["add", "-u"]).current_dir(&git_path);
        crate::platform::hide_window(&mut add_cmd);
        add_cmd.status().ok();
    }

    let out = {
        let mut cmd = Command::new("git");
        cmd.args(&cmd_args).current_dir(&git_path);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
            .map_err(|e| format!("git {} failed: {}", operation, e))?
    };

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    if !out.status.success() && !stderr.is_empty() {
        return Err(format!("git {}: {}", operation, stderr));
    }

    Ok(format!(
        "{}{}",
        stdout,
        if stderr.is_empty() {
            "".into()
        } else {
            format!("\n{}", stderr)
        }
    ))
}

// ===== External Services =====

fn exec_rag_search(
    args: &serde_json::Value,
    state: Option<&Arc<crate::rag_index::RagState>>,
) -> Result<String, String> {
    let query = args["query"].as_str().unwrap_or("");
    let top_k = args["top_k"].as_u64().unwrap_or(5) as usize;
    if let Some(st) = state {
        let results = st.search(query, top_k);
        if results.is_empty() {
            return Ok("No relevant results found in the index. Try indexing the codebase first with the Index RAG button.".to_string());
        }
        let mut out = Vec::new();
        for r in results {
            out.push(format!(
                "--- {} (lines {}-{}) ---\n{}",
                r.file_path, r.line_start, r.line_end, r.content
            ));
        }
        Ok(out.join("\n\n"))
    } else {
        Err("RAG index not initialized. Index the codebase first.".into())
    }
}

fn exec_http_request(args: &serde_json::Value) -> Result<String, String> {
    let url = args["url"].as_str().unwrap_or("");
    let method = args["method"].as_str().unwrap_or("GET");
    let timeout = args["timeout_secs"].as_u64().unwrap_or(15);

    if url.is_empty() {
        return Err("URL is required".to_string());
    }

    let mut cmd_args = vec![
        "-s".to_string(),
        "-S".to_string(),
        "-w".to_string(),
        "\n---HTTP_STATUS:%{http_code}---".to_string(),
        "-X".to_string(),
        method.to_string(),
        "--max-time".to_string(),
        timeout.to_string(),
    ];

    if let Some(headers) = args["headers"].as_object() {
        for (k, v) in headers {
            if let Some(val) = v.as_str() {
                cmd_args.push("-H".to_string());
                cmd_args.push(format!("{}: {}", k, val));
            }
        }
    }

    if let Some(body) = args["body"].as_str() {
        cmd_args.push("-d".to_string());
        cmd_args.push(body.to_string());
    }

    cmd_args.push(url.to_string());

    let out = {
        let mut cmd = Command::new("curl");
        cmd.args(&cmd_args);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
            .map_err(|e| format!("HTTP request failed: {}", e))?
    };

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    if !out.status.success() {
        return Err(format!("HTTP request failed: {}", stderr));
    }

    Ok(stdout.to_string())
}

fn exec_calculator(args: &serde_json::Value) -> Result<String, String> {
    let expr = args["expression"].as_str().unwrap_or("");
    if expr.is_empty() {
        return Err("Expression is required".to_string());
    }

    let cleaned = expr
        .replace("^", " ^ ")
        .replace("pi", "3.141592653589793")
        .replace("PI", "3.141592653589793")
        .replace("e ", "2.718281828459045 ")
        .replace("E ", "2.718281828459045 ");

    match meval::eval_str(&cleaned) {
        Ok(result) => Ok(format!("{} = {}", expr, result)),
        Err(e) => Err(format!("Math error evaluating '{}': {}", expr, e)),
    }
}

// ===== Web Tools =====

fn exec_web_search(args: &serde_json::Value) -> Result<String, String> {
    let query = args["query"].as_str().unwrap_or("");
    let max_results = args["max_results"].as_u64().unwrap_or(5) as usize;

    if query.is_empty() {
        return Err("Search query is required".to_string());
    }

    let encoded = query.replace(' ', "+");
    let url = format!("https://lite.duckduckgo.com/lite/?q={}", encoded);

    let out = {
        let mut cmd = Command::new("curl");
        cmd.args([
            "-sS",
            "-L",
            "--max-time",
            "10",
            "-H",
            "User-Agent: Mozilla/5.0 (X11; Linux x86_64) ShadowIDE/1.0",
            &url,
        ]);
        crate::platform::hide_window(&mut cmd);
        cmd.output().map_err(|e| format!("Search failed: {}", e))?
    };

    if !out.status.success() {
        return Err(format!(
            "Search HTTP error: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    let body = String::from_utf8_lossy(&out.stdout).to_string();

    let mut results = Vec::new();
    let body_lower = body.to_lowercase();
    let mut search_pos = 0;
    while let Some(pos) = body_lower[search_pos..].find("class=\"result-link\"") {
        let abs_pos = search_pos + pos;
        let before = &body[abs_pos.saturating_sub(200)..abs_pos];
        if let Some(href_pos) = before.rfind("href=\"") {
            let href_str = &before[href_pos + 6..];
            if let Some(href_end) = href_str.find('"') {
                let href = &href_str[..href_end];

                let after = &body[abs_pos..];
                if let Some(gt) = after.find('>') {
                    if let Some(end_a) = after[gt..].find("</a>") {
                        let title = strip_html_tags(&after[gt + 1..gt + end_a])
                            .trim()
                            .to_string();

                        if let Some(snippet_pos) =
                            body_lower[abs_pos..].find("class=\"result-snippet\"")
                        {
                            let snippet_abs = abs_pos + snippet_pos;
                            let snippet_after = &body[snippet_abs..];
                            if let Some(sgt) = snippet_after.find('>') {
                                if let Some(std) = snippet_after[sgt..].find("</td>") {
                                    let snippet =
                                        strip_html_tags(&snippet_after[sgt + 1..sgt + std])
                                            .trim()
                                            .to_string();
                                    if !title.is_empty() && !href.is_empty() {
                                        results.push(format!(
                                            "{}. {}\n   {}\n   {}",
                                            results.len() + 1,
                                            title,
                                            href,
                                            snippet
                                        ));
                                    }
                                }
                            }
                        } else if !title.is_empty() && !href.is_empty() {
                            results.push(format!("{}. {}\n   {}", results.len() + 1, title, href));
                        }
                    }
                }
            }
        }
        search_pos = abs_pos + 20;
        if results.len() >= max_results {
            break;
        }
    }

    if results.is_empty() {
        let mut link_pos = 0;
        while let Some(pos) = body_lower[link_pos..].find("<a rel=\"nofollow\" href=\"") {
            let abs = link_pos + pos + 23;
            if let Some(end) = body[abs..].find('"') {
                let href = &body[abs..abs + end];
                if href.starts_with("http") && !href.contains("duckduckgo.com") {
                    if let Some(gt) = body[abs + end..].find('>') {
                        if let Some(ea) = body[abs + end + gt..].find("</a>") {
                            let title =
                                strip_html_tags(&body[abs + end + gt + 1..abs + end + gt + ea])
                                    .trim()
                                    .to_string();
                            if !title.is_empty() {
                                results.push(format!(
                                    "{}. {}\n   {}",
                                    results.len() + 1,
                                    title,
                                    href
                                ));
                            }
                        }
                    }
                }
            }
            link_pos = abs + 1;
            if results.len() >= max_results {
                break;
            }
        }
    }

    if results.is_empty() {
        Ok(format!(
            "No search results found for '{}'. Try different keywords.",
            query
        ))
    } else {
        Ok(format!(
            "Search results for '{}':\n\n{}",
            query,
            results.join("\n\n")
        ))
    }
}

fn exec_web_fetch(args: &serde_json::Value) -> Result<String, String> {
    let url = args["url"].as_str().unwrap_or("");
    let max_chars = args["max_chars"].as_u64().unwrap_or(8000) as usize;

    if url.is_empty() {
        return Err("URL is required".to_string());
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("URL must start with http:// or https://".to_string());
    }

    let out = {
        let mut cmd = Command::new("curl");
        cmd.args([
            "-sS",
            "-L",
            "--max-time",
            "15",
            "-H",
            "User-Agent: ShadowIDE/1.0",
            url,
        ]);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
            .map_err(|e| format!("Failed to fetch URL: {}", e))?
    };

    if !out.status.success() {
        return Err(format!(
            "HTTP error: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    let body = String::from_utf8_lossy(&out.stdout).to_string();

    let readable = strip_html_tags(&body);
    let trimmed = if readable.len() > max_chars {
        format!(
            "{}...\n\n[Truncated at {} chars, total {}]",
            truncate_line(&readable, max_chars),
            max_chars,
            readable.len()
        )
    } else {
        readable
    };
    Ok(trimmed)
}

fn simple_url_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len() * 3);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(byte as char);
            }
            b' ' => result.push('+'),
            _ => {
                result.push('%');
                result.push_str(&format!("{:02X}", byte));
            }
        }
    }
    result
}

fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut in_script = false;
    let mut in_style = false;
    let mut last_was_space = false;

    let lower = html.to_lowercase();
    let chars: Vec<char> = html.chars().collect();
    let lower_chars: Vec<char> = lower.chars().collect();

    let mut i = 0;
    while i < chars.len() {
        if !in_tag && i + 7 < lower_chars.len() {
            let peek: String = lower_chars[i..i + 7].iter().collect();
            if peek == "<script" {
                in_script = true;
            }
            if peek.starts_with("<style") {
                in_style = true;
            }
        }
        if chars[i] == '<' {
            in_tag = true;
            if i + 9 < lower_chars.len() {
                let peek: String = lower_chars[i..i + 9].iter().collect();
                if peek == "</script>" {
                    in_script = false;
                }
            }
            if i + 8 < lower_chars.len() {
                let peek: String = lower_chars[i..i + 8].iter().collect();
                if peek == "</style>" {
                    in_style = false;
                }
            }
            i += 1;
            continue;
        }
        if chars[i] == '>' {
            in_tag = false;
            i += 1;
            continue;
        }
        if !in_tag && !in_script && !in_style {
            let ch = chars[i];
            if ch.is_whitespace() {
                if !last_was_space {
                    result.push(' ');
                    last_was_space = true;
                }
            } else {
                result.push(ch);
                last_was_space = false;
            }
        }
        i += 1;
    }
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

// ===== Memory Tools =====

fn memory_dir(root: &str) -> PathBuf {
    Path::new(root).join(".shadow-memory")
}

fn exec_memory_store(args: &serde_json::Value, root: &str) -> Result<String, String> {
    let key = args["key"].as_str().unwrap_or("");
    let value = args["value"].as_str().unwrap_or("");
    let category = args["category"].as_str().unwrap_or("fact");

    if key.is_empty() || value.is_empty() {
        return Err("Both 'key' and 'value' are required".to_string());
    }
    let safe_key: String = key
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();

    let dir = memory_dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create memory dir: {}", e))?;

    let entry = serde_json::json!({
        "key": key,
        "value": value,
        "category": category,
        "timestamp": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0),
    });

    let path = dir.join(format!("{}.json", safe_key));
    let json = serde_json::to_string_pretty(&entry)
        .map_err(|e| format!("Failed to serialize memory: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("Failed to write memory: {}", e))?;

    Ok(format!(
        "Stored memory '{}' in category '{}'",
        key, category
    ))
}

fn exec_memory_recall(args: &serde_json::Value, root: &str) -> Result<String, String> {
    let query = args["query"].as_str().unwrap_or("").to_lowercase();
    if query.is_empty() {
        return Err("Query is required".to_string());
    }

    let dir = memory_dir(root);
    if !dir.exists() {
        return Ok("No memories stored yet.".to_string());
    }

    let mut results = Vec::new();
    let entries =
        std::fs::read_dir(&dir).map_err(|e| format!("Failed to read memory dir: {}", e))?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }

        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(mem) = serde_json::from_str::<serde_json::Value>(&content) {
                let key = mem["key"].as_str().unwrap_or("").to_lowercase();
                let value = mem["value"].as_str().unwrap_or("").to_lowercase();
                let category = mem["category"].as_str().unwrap_or("");

                if key.contains(&query) || value.contains(&query) || query.contains(&key) {
                    results.push(format!(
                        "[{}] {}: {}",
                        category,
                        mem["key"].as_str().unwrap_or(""),
                        mem["value"].as_str().unwrap_or("")
                    ));
                }
            }
        }
    }

    if results.is_empty() {
        Ok(format!(
            "No memories matching '{}'. Try a broader search.",
            query
        ))
    } else {
        Ok(format!(
            "Found {} memories:\n{}",
            results.len(),
            results.join("\n")
        ))
    }
}

// ===== Build & Diagnostics =====

fn exec_cargo_run(args: &serde_json::Value, root: &str) -> Result<String, String> {
    let subcmd = args["subcommand"].as_str().unwrap_or("check");
    let extra_args: Vec<String> = args["args"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let working_dir = args["working_dir"]
        .as_str()
        .map(|d| resolve_path(d, root).unwrap_or_else(|_| root.to_string()))
        .unwrap_or_else(|| root.to_string());

    let allowed = [
        "build", "test", "check", "clippy", "fmt", "run", "doc", "bench",
    ];
    if !allowed.contains(&subcmd) {
        return Err(format!(
            "Cargo subcommand '{}' not allowed. Use: {}",
            subcmd,
            allowed.join(", ")
        ));
    }

    let mut cmd_args = vec![subcmd.to_string(), "--color=never".to_string()];
    cmd_args.extend(extra_args);

    let mut child = {
        let mut cmd = Command::new("cargo");
        cmd.args(&cmd_args)
            .current_dir(&working_dir)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        crate::platform::hide_window(&mut cmd);
        cmd.spawn()
            .map_err(|e| format!("Failed to run cargo {}: {}", subcmd, e))?
    };

    let timeout = std::time::Duration::from_secs(120);
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    return Err(format!("cargo {} timed out after 120s", subcmd));
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            Err(e) => return Err(format!("Error waiting for cargo: {}", e)),
        }
    }

    let output = child.wait_with_output().map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let success = output.status.success();

    let diagnostics = parse_rust_diagnostics(&stderr);
    let diag_summary = if diagnostics.is_empty() {
        String::new()
    } else {
        format!(
            "\n\n--- Diagnostics ({} issues) ---\n{}",
            diagnostics.len(),
            diagnostics
                .iter()
                .map(|d| format!(
                    "{}:{}:{}: [{}] {}",
                    d.file, d.line, d.col, d.severity, d.message
                ))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };

    let status_line = if success { "SUCCESS" } else { "FAILED" };
    Ok(format!(
        "[cargo {} — {}]\n{}{}{}",
        subcmd,
        status_line,
        stdout,
        if stderr.is_empty() {
            "".into()
        } else {
            format!("\n{}", stderr)
        },
        diag_summary
    ))
}

#[derive(Debug)]
struct Diagnostic {
    file: String,
    line: u32,
    col: u32,
    severity: String,
    message: String,
}

fn parse_rust_diagnostics(output: &str) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    for line in output.lines() {
        if let Some(stripped) = line.strip_prefix("error") {
            if let Some(msg) = stripped.split("]: ").nth(1).or(stripped.strip_prefix(": ")) {
                diagnostics.push(Diagnostic {
                    file: String::new(),
                    line: 0,
                    col: 0,
                    severity: "error".to_string(),
                    message: msg.trim().to_string(),
                });
            }
        } else if let Some(stripped) = line.strip_prefix("warning") {
            if let Some(msg) = stripped.split("]: ").nth(1).or(stripped.strip_prefix(": ")) {
                diagnostics.push(Diagnostic {
                    file: String::new(),
                    line: 0,
                    col: 0,
                    severity: "warning".to_string(),
                    message: msg.trim().to_string(),
                });
            }
        } else if line.trim().starts_with("--> ") {
            let loc = line.trim().trim_start_matches("--> ");
            let parts: Vec<&str> = loc.rsplitn(3, ':').collect();
            if parts.len() >= 3 {
                if let Some(last) = diagnostics.last_mut() {
                    last.col = parts[0].parse().unwrap_or(0);
                    last.line = parts[1].parse().unwrap_or(0);
                    last.file = parts[2].to_string();
                }
            }
        }
    }
    diagnostics
}

fn exec_code_diagnostics(args: &serde_json::Value) -> Result<String, String> {
    let output = args["output"].as_str().unwrap_or("");
    let language = args["language"].as_str().unwrap_or("auto");

    if output.is_empty() {
        return Err("output is required".to_string());
    }

    let diagnostics = match language {
        "rust" | "auto" if output.contains("error[E") || output.contains("--> ") => {
            parse_rust_diagnostics(output)
        }
        _ => {
            let mut diags = Vec::new();
            for line in output.lines() {
                let parts: Vec<&str> = line.splitn(4, ':').collect();
                if parts.len() >= 4 {
                    let file = parts[0].trim();
                    let line_num: u32 = parts[1].trim().parse().unwrap_or(0);
                    let col: u32 = parts[2].trim().parse().unwrap_or(0);
                    let rest = parts[3].trim();
                    let (severity, msg) =
                        if rest.starts_with(" error") || rest.starts_with(" Error") {
                            (
                                "error",
                                rest.trim_start_matches(" error")
                                    .trim_start_matches(" Error")
                                    .trim_start_matches(':')
                                    .trim(),
                            )
                        } else if rest.starts_with(" warning") || rest.starts_with(" Warning") {
                            (
                                "warning",
                                rest.trim_start_matches(" warning")
                                    .trim_start_matches(" Warning")
                                    .trim_start_matches(':')
                                    .trim(),
                            )
                        } else {
                            ("info", rest)
                        };
                    if line_num > 0 {
                        diags.push(Diagnostic {
                            file: file.to_string(),
                            line: line_num,
                            col,
                            severity: severity.to_string(),
                            message: msg.to_string(),
                        });
                    }
                }
            }
            diags
        }
    };

    if diagnostics.is_empty() {
        return Ok("No structured diagnostics found in output.".to_string());
    }

    let json_diags: Vec<serde_json::Value> = diagnostics
        .iter()
        .map(|d| {
            serde_json::json!({
                "file": d.file,
                "line": d.line,
                "col": d.col,
                "severity": d.severity,
                "message": d.message,
            })
        })
        .collect();

    Ok(serde_json::to_string_pretty(&json_diags)
        .unwrap_or_else(|_| format!("{} diagnostics found", diagnostics.len())))
}

fn exec_process_list(args: &serde_json::Value) -> Result<String, String> {
    let filter = args["filter"].as_str().unwrap_or("");
    let limit = args["limit"].as_u64().unwrap_or(20) as usize;

    let mut sys = sysinfo::System::new();
    sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);

    let mut procs: Vec<(u32, String, f32, u64)> = sys
        .processes()
        .iter()
        .map(|(pid, p)| {
            (
                pid.as_u32(),
                p.name().to_string_lossy().to_string(),
                p.cpu_usage(),
                p.memory(),
            )
        })
        .collect();

    if !filter.is_empty() {
        let f = filter.to_lowercase();
        procs.retain(|(_, name, _, _)| name.to_lowercase().contains(&f));
    }

    procs.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
    procs.truncate(limit);

    let mut lines = vec![format!(
        "{:<8} {:<30} {:>8} {:>10}",
        "PID", "NAME", "CPU%", "MEM(MB)"
    )];
    for (pid, name, cpu, mem) in &procs {
        lines.push(format!(
            "{:<8} {:<30} {:>7.1}% {:>9.1}",
            pid,
            name,
            cpu,
            *mem as f64 / 1_048_576.0
        ));
    }

    Ok(lines.join("\n"))
}

// ===== RAG Tools =====

fn exec_rag_index(
    _args: &serde_json::Value,
    _root: &str,
    state: Option<&Arc<crate::rag_index::RagState>>,
) -> Result<String, String> {
    if let Some(st) = state {
        let stats = st.get_stats();
        if stats.total_chunks > 0 {
            Ok(format!("RAG index already built: {} chunks from {} files (last indexed: {}). Use the Index RAG button in the UI to rebuild.", stats.total_chunks, stats.files_indexed, if stats.last_index_time.is_empty() { "unknown" } else { &stats.last_index_time }))
        } else {
            Ok("RAG index is empty. Please use the 'Index RAG' button in the UI to build the index.".to_string())
        }
    } else {
        Err("RAG state not available".to_string())
    }
}

fn exec_rag_list_sources(
    state: Option<&Arc<crate::rag_index::RagState>>,
) -> Result<String, String> {
    if let Some(st) = state {
        let stats = st.get_stats();
        if stats.total_chunks == 0 {
            return Ok("RAG index is empty. No files indexed yet.".to_string());
        }
        let index = st.index.lock().map_err(|e| format!("Lock error: {}", e))?;
        let mut file_chunks: std::collections::HashMap<&str, usize> =
            std::collections::HashMap::new();
        for chunk in &index.chunks {
            *file_chunks.entry(&chunk.file_path).or_insert(0) += 1;
        }
        let mut files: Vec<_> = file_chunks.into_iter().collect();
        files.sort_by(|a, b| b.1.cmp(&a.1));

        let mut lines = vec![format!(
            "RAG Index: {} files, {} chunks, last indexed: {}",
            stats.files_indexed,
            stats.total_chunks,
            if stats.last_index_time.is_empty() {
                "never"
            } else {
                &stats.last_index_time
            }
        )];

        for (file, count) in files.iter().take(30) {
            lines.push(format!("  {} ({} chunks)", file, count));
        }
        if files.len() > 30 {
            lines.push(format!("  ... and {} more files", files.len() - 30));
        }
        Ok(lines.join("\n"))
    } else {
        Err("RAG state not available".to_string())
    }
}

// ===== Environment Info =====

fn exec_env_read(args: &serde_json::Value) -> Result<String, String> {
    let info_type = args["info_type"].as_str().unwrap_or("all");
    let var_name = args["var_name"].as_str().unwrap_or("");

    match info_type {
        "env_var" => {
            if var_name.is_empty() {
                return Err("var_name is required when info_type='env_var'".to_string());
            }
            match std::env::var(var_name) {
                Ok(val) => Ok(format!("{}={}", var_name, val)),
                Err(_) => Ok(format!("{} is not set", var_name)),
            }
        }
        "system" | "all" => {
            let mut info = Vec::new();
            info.push(format!(
                "OS: {} {}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ));

            if let Ok(hostname) = {
                let mut cmd = Command::new("hostname");
                crate::platform::hide_window(&mut cmd);
                cmd.output()
            } {
                info.push(format!(
                    "Hostname: {}",
                    String::from_utf8_lossy(&hostname.stdout).trim()
                ));
            }
            if let Ok(user) = std::env::var("USER").or_else(|_| std::env::var("USERNAME")) {
                info.push(format!("User: {}", user));
            }
            if let Ok(pwd) = std::env::current_dir() {
                info.push(format!("CWD: {}", pwd.display()));
            }
            if let Some(home) = dirs_next::home_dir() {
                info.push(format!("HOME: {}", home.display()));
            }

            let mut sys = sysinfo::System::new();
            sys.refresh_memory();
            sys.refresh_cpu_all();
            info.push(format!(
                "RAM: {:.1} GB total, {:.1} GB available",
                sys.total_memory() as f64 / 1_073_741_824.0,
                sys.available_memory() as f64 / 1_073_741_824.0
            ));
            info.push(format!("CPUs: {}", sys.cpus().len()));

            if info_type == "all" && !var_name.is_empty() {
                match std::env::var(var_name) {
                    Ok(val) => info.push(format!("{}={}", var_name, val)),
                    Err(_) => info.push(format!("{} is not set", var_name)),
                }
            }

            Ok(info.join("\n"))
        }
        other => Err(format!(
            "Unknown info_type '{}'. Use 'env_var', 'system', or 'all'",
            other
        )),
    }
}

// ===== Docs Search =====

fn exec_docs_search(args: &serde_json::Value) -> Result<String, String> {
    let query = args["query"].as_str().unwrap_or("");
    if query.is_empty() {
        return Err("'query' is required".to_string());
    }

    let source = args["source"].as_str().unwrap_or("auto");

    let effective_source = if source == "auto" {
        if query.contains("::")
            || query.contains("crate")
            || query.contains("tokio")
            || query.contains("serde")
            || query.contains("rust")
            || query.contains("cargo")
        {
            "rust"
        } else if query.contains("document.")
            || query.contains("Array.")
            || query.contains("Promise")
            || query.contains("fetch")
            || query.contains("DOM")
            || query.contains("CSS")
            || query.contains("HTML")
            || query.contains("javascript")
            || query.contains("js ")
        {
            "mdn"
        } else if query.contains("import ")
            || query.contains("def ")
            || query.contains("python")
            || query.contains("pip")
            || query.contains("__")
        {
            "python"
        } else if query.contains("npm") || query.contains("package.json") || query.contains("node")
        {
            "npm"
        } else {
            "rust"
        }
    } else {
        source
    };

    let search_url = match effective_source {
        "rust" => format!(
            "https://docs.rs/releases/search?query={}",
            simple_url_encode(query)
        ),
        "mdn" => format!(
            "https://developer.mozilla.org/en-US/search?q={}",
            simple_url_encode(query)
        ),
        "python" => format!(
            "https://docs.python.org/3/search.html?q={}",
            simple_url_encode(query)
        ),
        "npm" => format!(
            "https://www.npmjs.com/search?q={}",
            simple_url_encode(query)
        ),
        _ => format!(
            "https://docs.rs/releases/search?query={}",
            simple_url_encode(query)
        ),
    };

    let output = {
        let mut cmd = Command::new("curl");
        cmd.args(&[
            "-s",
            "-L",
            "--max-time",
            "10",
            "-H",
            "User-Agent: Mozilla/5.0 (X11; Linux x86_64) ShadowIDE/0.84",
            &search_url,
        ]);
        crate::platform::hide_window(&mut cmd);
        cmd.output().map_err(|e| format!("curl failed: {}", e))?
    };

    let body = String::from_utf8_lossy(&output.stdout).to_string();

    let mut results = Vec::new();

    match effective_source {
        "rust" => {
            for line in body.lines() {
                if results.len() >= 8 {
                    break;
                }
                if line.contains("class=\"release\"") || line.contains("/latest/") {
                    if let Some(href_start) = line.find("href=\"") {
                        let rest = &line[href_start + 6..];
                        if let Some(href_end) = rest.find('"') {
                            let href = &rest[..href_end];
                            let url = if href.starts_with('/') {
                                format!("https://docs.rs{}", href)
                            } else {
                                href.to_string()
                            };
                            let text = strip_html_tags(rest);
                            if !text.is_empty() && text.len() < 200 {
                                results.push(format!("- [{}]({})", text.trim(), url));
                            }
                        }
                    }
                }
            }
        }
        "mdn" => {
            let api_url = format!(
                "https://developer.mozilla.org/api/v1/search?q={}&locale=en-US&size=8",
                simple_url_encode(query)
            );
            if let Ok(mdn_out) = {
                let mut cmd = Command::new("curl");
                cmd.args(&["-s", "-L", "--max-time", "10", &api_url]);
                crate::platform::hide_window(&mut cmd);
                cmd.output()
            } {
                let text = String::from_utf8_lossy(&mdn_out.stdout);
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) {
                    if let Some(docs) = json["documents"].as_array() {
                        for doc in docs.iter().take(8) {
                            let title = doc["title"].as_str().unwrap_or("");
                            let slug = doc["mdn_url"].as_str().unwrap_or("");
                            let summary = doc["summary"].as_str().unwrap_or("");
                            let url = format!("https://developer.mozilla.org{}", slug);
                            results.push(format!(
                                "- **{}** — {}\n  {}",
                                title,
                                summary.chars().take(120).collect::<String>(),
                                url
                            ));
                        }
                    }
                }
            }
        }
        _ => {
            for line in body.lines() {
                if results.len() >= 8 {
                    break;
                }
                if line.contains("<a ") && line.contains("href=\"") {
                    let text = strip_html_tags(line);
                    if text.len() > 10 && text.len() < 200 {
                        results.push(format!("- {}", text.trim()));
                    }
                }
            }
        }
    }

    if results.is_empty() {
        Ok(format!("No results found for '{}' in {} docs. Try web_search for broader results.\nSearch URL: {}", query, effective_source, search_url))
    } else {
        Ok(format!(
            "## {} Documentation Results for '{}'\n\n{}\n\nSource: {}",
            effective_source,
            query,
            results.join("\n"),
            search_url
        ))
    }
}

// ===== JSON Query =====

fn exec_json_query(args: &serde_json::Value, root_path: &str) -> Result<String, String> {
    let data_str = args["data"].as_str().unwrap_or("");
    let query = args["query"].as_str().unwrap_or("");

    if data_str.is_empty() || query.is_empty() {
        return Err("Both 'data' and 'query' are required".to_string());
    }

    let json_data: serde_json::Value =
        if data_str.starts_with('{') || data_str.starts_with('[') || data_str.starts_with('"') {
            serde_json::from_str(data_str).map_err(|e| format!("Invalid JSON: {}", e))?
        } else {
            let path = resolve_path(data_str, root_path)?;
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("Failed to read file '{}': {}", path, e))?;
            serde_json::from_str(&content).map_err(|e| format!("Invalid JSON in file: {}", e))?
        };

    let result = json_path_query(&json_data, query)?;

    Ok(serde_json::to_string_pretty(&result).unwrap_or_else(|_| format!("{:?}", result)))
}

fn json_path_query(data: &serde_json::Value, path: &str) -> Result<serde_json::Value, String> {
    let segments = parse_json_path(path);
    let mut current = vec![data.clone()];

    for seg in &segments {
        let mut next = Vec::new();
        for val in &current {
            match seg.as_str() {
                "[*]" => {
                    if let Some(arr) = val.as_array() {
                        next.extend(arr.iter().cloned());
                    }
                }
                s if s.starts_with('[') && s.ends_with(']') => {
                    let idx_str = &s[1..s.len() - 1];
                    if let Ok(idx) = idx_str.parse::<usize>() {
                        if let Some(elem) = val.get(idx) {
                            next.push(elem.clone());
                        }
                    }
                }
                key => {
                    if let Some(v) = val.get(key) {
                        next.push(v.clone());
                    }
                }
            }
        }
        current = next;
        if current.is_empty() {
            return Err(format!("Path '{}' not found at segment '{}'", path, seg));
        }
    }

    if current.len() == 1 {
        Ok(current.into_iter().next().unwrap())
    } else {
        Ok(serde_json::Value::Array(current))
    }
}

fn parse_json_path(path: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_bracket = false;

    for ch in path.chars() {
        match ch {
            '.' if !in_bracket => {
                if !current.is_empty() {
                    segments.push(current.clone());
                    current.clear();
                }
            }
            '[' => {
                if !current.is_empty() {
                    segments.push(current.clone());
                    current.clear();
                }
                in_bracket = true;
                current.push(ch);
            }
            ']' => {
                in_bracket = false;
                current.push(ch);
                segments.push(current.clone());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments
}

// ===== Symbol Lookup =====

fn exec_symbol_lookup(args: &serde_json::Value, root_path: &str) -> Result<String, String> {
    let symbol = args["symbol"]
        .as_str()
        .ok_or("Missing 'symbol' parameter")?;
    let kind = args["kind"].as_str().unwrap_or("all");
    let search_path = args["path"].as_str().unwrap_or(".");
    let language = args["language"].as_str().unwrap_or("auto");

    let resolved = resolve_path(search_path, root_path)?;

    let patterns = build_symbol_patterns(symbol, kind, language);
    if patterns.is_empty() {
        return Err(format!(
            "No patterns for kind='{}' language='{}'",
            kind, language
        ));
    }

    let mut results: Vec<String> = Vec::new();

    let rg_available = {
        let mut cmd = std::process::Command::new("rg");
        cmd.arg("--version");
        crate::platform::hide_window(&mut cmd);
        cmd.output().is_ok()
    };

    for (pat_kind, pattern) in &patterns {
        let output = if rg_available {
            let mut cmd = std::process::Command::new("rg");
            cmd.args(&["-n", "--no-heading", "-e", pattern, &resolved]);
            crate::platform::hide_window(&mut cmd);
            cmd.output()
        } else {
            let mut cmd = std::process::Command::new("grep");
            cmd.args(&["-rn", "-E", pattern, &resolved]);
            crate::platform::hide_window(&mut cmd);
            cmd.output()
        };

        match output {
            Ok(o) => {
                let text = String::from_utf8_lossy(&o.stdout);
                for line in text.lines().take(50) {
                    results.push(format!("[{}] {}", pat_kind, line));
                }
            }
            Err(e) => {
                results.push(format!("[{}] search error: {}", pat_kind, e));
            }
        }
    }

    if results.is_empty() {
        Ok(format!(
            "No symbols matching '{}' found in {}",
            symbol, resolved
        ))
    } else {
        Ok(format!(
            "Found {} matches:\n{}",
            results.len(),
            results.join("\n")
        ))
    }
}

fn build_symbol_patterns(symbol: &str, kind: &str, language: &str) -> Vec<(String, String)> {
    let sym = regex_escape(symbol);
    let mut patterns = Vec::new();

    let is_rust = matches!(language, "rust" | "auto");
    let is_py = matches!(language, "python" | "auto");
    let is_ts = matches!(language, "typescript" | "javascript" | "auto");
    let is_go = matches!(language, "go" | "auto");
    let is_java = matches!(language, "java" | "auto");
    let is_c = matches!(language, "c" | "cpp" | "auto");

    let want = |k: &str| kind == "all" || kind == k;

    if want("function") {
        if is_rust {
            patterns.push((
                "fn".into(),
                format!(r"(pub\s+)?(async\s+)?fn\s+{}\s*[\(<]", sym),
            ));
        }
        if is_py {
            patterns.push(("def".into(), format!(r"def\s+{}\s*\(", sym)));
        }
        if is_ts {
            patterns.push((
                "function".into(),
                format!(r"(export\s+)?(async\s+)?function\s+{}\s*[\(<]", sym),
            ));
        }
        if is_go {
            patterns.push((
                "func".into(),
                format!(r"func\s+(\([^)]*\)\s*)?{}\s*\(", sym),
            ));
        }
        if is_java {
            patterns.push((
                "method".into(),
                format!(r"(public|private|protected)?\s+\w+\s+{}\s*\(", sym),
            ));
        }
        if is_c {
            patterns.push(("func".into(), format!(r"\w[\w*\s]+{}\s*\(", sym)));
        }
    }
    if want("struct") {
        if is_rust {
            patterns.push((
                "struct".into(),
                format!(r"(pub\s+)?struct\s+{}\s*[\{{<]", sym),
            ));
        }
        if is_go {
            patterns.push(("struct".into(), format!(r"type\s+{}\s+struct", sym)));
        }
        if is_c {
            patterns.push(("struct".into(), format!(r"struct\s+{}\s*\{{", sym)));
        }
    }
    if want("enum") {
        if is_rust {
            patterns.push(("enum".into(), format!(r"(pub\s+)?enum\s+{}\s*[\{{<]", sym)));
        }
        if is_ts {
            patterns.push(("enum".into(), format!(r"(export\s+)?enum\s+{}\s*\{{", sym)));
        }
        if is_java {
            patterns.push(("enum".into(), format!(r"enum\s+{}\s*\{{", sym)));
        }
    }
    if want("trait") {
        if is_rust {
            patterns.push((
                "trait".into(),
                format!(r"(pub\s+)?trait\s+{}\s*[\{{<:]", sym),
            ));
        }
    }
    if want("impl") {
        if is_rust {
            patterns.push(("impl".into(), format!(r"impl\s+(<[^>]*>\s*)?{}", sym)));
        }
    }
    if want("class") {
        if is_py {
            patterns.push(("class".into(), format!(r"class\s+{}\s*[\(:]", sym)));
        }
        if is_ts {
            patterns.push((
                "class".into(),
                format!(r"(export\s+)?class\s+{}\s*[\{{<]", sym),
            ));
        }
        if is_java {
            patterns.push((
                "class".into(),
                format!(r"(public\s+)?class\s+{}\s*[\{{<]", sym),
            ));
        }
        if is_c {
            patterns.push(("class".into(), format!(r"class\s+{}\s*[\{{:]", sym)));
        }
    }
    if want("method") {
        if is_rust {
            patterns.push((
                "method".into(),
                format!(r"(pub\s+)?(async\s+)?fn\s+{}\s*\(", sym),
            ));
        }
        if is_py {
            patterns.push(("method".into(), format!(r"def\s+{}\s*\(self", sym)));
        }
        if is_ts {
            patterns.push(("method".into(), format!(r"(async\s+)?{}\s*\(", sym)));
        }
    }

    patterns
}

fn regex_escape(s: &str) -> String {
    let special = [
        '\\', '.', '+', '*', '?', '(', ')', '[', ']', '{', '}', '|', '^', '$',
    ];
    let mut out = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        if special.contains(&c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

// ===== Cache Lookup =====

fn exec_cache_lookup(args: &serde_json::Value) -> Result<String, String> {
    let query = args["query"].as_str().ok_or("Missing 'query' parameter")?;

    let cache_dir = dirs_next::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("shadow-ide")
        .join("llm-cache");

    if !cache_dir.exists() {
        return Ok("Cache is empty (no cache directory found).".to_string());
    }

    let query_hash = simple_hash(query);
    let cache_file = cache_dir.join(format!("{:016x}.json", query_hash));

    if cache_file.exists() {
        match std::fs::read_to_string(&cache_file) {
            Ok(content) => {
                let parsed: serde_json::Value = serde_json::from_str(&content)
                    .unwrap_or(serde_json::json!({"response": content}));
                let response = parsed["response"].as_str().unwrap_or(&content);
                Ok(format!(
                    "Cache HIT (hash {:016x}):\n{}",
                    query_hash, response
                ))
            }
            Err(e) => Err(format!("Cache file exists but read failed: {}", e)),
        }
    } else {
        let mut near_matches = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&cache_dir) {
            for entry in entries.flatten() {
                if let Ok(content) = std::fs::read_to_string(entry.path()) {
                    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                        if let Some(cached_query) = parsed["query"].as_str() {
                            let q_lower = query.to_lowercase();
                            let c_lower = cached_query.to_lowercase();
                            if c_lower.contains(&q_lower) || q_lower.contains(&c_lower) {
                                let resp = parsed["response"].as_str().unwrap_or("(no response)");
                                near_matches.push(format!(
                                    "- Query: {}\n  Response: {}",
                                    cached_query,
                                    truncate_line(resp, 200)
                                ));
                            }
                        }
                    }
                }
                if near_matches.len() >= 5 {
                    break;
                }
            }
        }

        if near_matches.is_empty() {
            Ok(format!("Cache MISS for query: {}", query))
        } else {
            Ok(format!(
                "No exact cache match, but found {} similar entries:\n{}",
                near_matches.len(),
                near_matches.join("\n")
            ))
        }
    }
}

fn simple_hash(s: &str) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in s.bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

// ===== Task Scheduling =====

fn exec_task_schedule(args: &serde_json::Value, root_path: &str) -> Result<String, String> {
    let command = args["command"]
        .as_str()
        .ok_or("Missing 'command' parameter")?;
    let label = args["label"].as_str().unwrap_or("scheduled-task");
    let delay_secs = args["delay_secs"].as_u64();
    let at_time = args["at"].as_str();

    let delay = if let Some(secs) = delay_secs {
        std::time::Duration::from_secs(secs)
    } else if let Some(at_str) = at_time {
        parse_delay_from_iso(at_str)?
    } else {
        std::time::Duration::from_secs(0)
    };

    let task_id = format!(
        "{:08x}",
        simple_hash(&format!(
            "{}{}{:?}",
            command,
            label,
            std::time::SystemTime::now()
        )) as u32
    );
    let cmd = command.to_string();
    let root = root_path.to_string();
    let tid = task_id.clone();
    let lbl = label.to_string();

    let log_dir = dirs_next::cache_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
        .join("shadow-ide")
        .join("scheduled-tasks");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_file = log_dir.join(format!("{}.log", tid));
    let log_file_clone = log_file.clone();

    std::thread::spawn(move || {
        if delay.as_secs() > 0 {
            std::thread::sleep(delay);
        }

        let output = {
            let mut proc = crate::platform::shell_command(&cmd);
            proc.current_dir(&root);
            crate::platform::hide_window(&mut proc);
            proc.output()
        };

        let log_content = match output {
            Ok(o) => {
                format!(
                    "Task: {} ({})\nExit: {}\n--- stdout ---\n{}\n--- stderr ---\n{}",
                    lbl,
                    tid,
                    o.status.code().unwrap_or(-1),
                    String::from_utf8_lossy(&o.stdout),
                    String::from_utf8_lossy(&o.stderr),
                )
            }
            Err(e) => format!("Task {} failed to start: {}", tid, e),
        };

        let _ = std::fs::write(&log_file_clone, &log_content);
    });

    let delay_desc = if delay.as_secs() == 0 {
        "immediately".to_string()
    } else {
        format!("in {} seconds", delay.as_secs())
    };

    Ok(format!(
        "Scheduled task '{}' (id: {}) to run {}.\nCommand: {}\nLog: {}",
        label,
        task_id,
        delay_desc,
        command,
        log_file.display()
    ))
}

fn parse_delay_from_iso(iso: &str) -> Result<std::time::Duration, String> {
    let parts: Vec<&str> = iso.split('T').collect();
    if parts.len() != 2 {
        return Err(format!(
            "Invalid ISO 8601 format: '{}'. Expected YYYY-MM-DDTHH:MM:SS",
            iso
        ));
    }

    let date_parts: Vec<u32> = parts[0].split('-').filter_map(|s| s.parse().ok()).collect();
    let time_parts: Vec<u32> = parts[1].split(':').filter_map(|s| s.parse().ok()).collect();

    if date_parts.len() != 3 || time_parts.len() < 2 {
        return Err(format!("Cannot parse date/time from '{}'", iso));
    }

    let target_secs = date_to_epoch_approx(
        date_parts[0],
        date_parts[1],
        date_parts[2],
        time_parts[0],
        time_parts[1],
        *time_parts.get(2).unwrap_or(&0),
    );

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_secs();

    if target_secs <= now {
        Ok(std::time::Duration::from_secs(0))
    } else {
        Ok(std::time::Duration::from_secs(target_secs - now))
    }
}

fn date_to_epoch_approx(year: u32, month: u32, day: u32, hour: u32, min: u32, sec: u32) -> u64 {
    let y = year as u64;
    let m = month as u64;
    let d = day as u64;

    let month_days: [u64; 12] = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];

    let mut days: u64 = 0;
    for yr in 1970..y {
        days += if yr % 4 == 0 && (yr % 100 != 0 || yr % 400 == 0) {
            366
        } else {
            365
        };
    }
    for mo in 0..(m.saturating_sub(1) as usize) {
        days += month_days.get(mo).copied().unwrap_or(30);
    }
    if m > 2 && y % 4 == 0 && (y % 100 != 0 || y % 400 == 0) {
        days += 1;
    }
    days += d.saturating_sub(1);

    days * 86400 + (hour as u64) * 3600 + (min as u64) * 60 + sec as u64
}

// ===== Extended Web & Utility Tools =====

fn exec_browse_url(args: &serde_json::Value) -> Result<String, String> {
    let url = args["url"].as_str().unwrap_or("");
    if url.is_empty() {
        return Err("URL is required".to_string());
    }
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return Err("URL must start with http:// or https://".to_string());
    }

    let out = {
        let mut cmd = Command::new("curl");
        cmd.args([
            "-sS",
            "-L",
            "--max-time",
            "15",
            "-H",
            "User-Agent: Mozilla/5.0 (X11; Linux x86_64) ShadowIDE/1.0",
            url,
        ]);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
            .map_err(|e| format!("Failed to fetch URL: {}", e))?
    };

    if !out.status.success() {
        return Err(format!(
            "HTTP error fetching '{}': {}",
            url,
            String::from_utf8_lossy(&out.stderr)
        ));
    }

    let body = String::from_utf8_lossy(&out.stdout).to_string();
    let readable = strip_html_tags(&body);

    let max_chars = 3000usize;
    let trimmed = if readable.len() > max_chars {
        format!(
            "{}...\n\n[Truncated at {} chars]",
            &readable[..max_chars],
            max_chars
        )
    } else {
        readable
    };
    Ok(trimmed)
}

fn exec_run_tests(args: &serde_json::Value) -> Result<String, String> {
    let project_path = args["project_path"].as_str().unwrap_or(".");
    let filter = args["filter"].as_str().unwrap_or("");

    let cargo_toml = std::path::Path::new(project_path).join("Cargo.toml");
    let package_json = std::path::Path::new(project_path).join("package.json");

    if cargo_toml.exists() {
        // Rust project
        let mut cmd_args = vec!["test".to_string(), "--color=never".to_string()];
        if !filter.is_empty() {
            cmd_args.push(filter.to_string());
        }

        let out = {
            let mut cmd = Command::new("cargo");
            cmd.args(&cmd_args)
                .current_dir(project_path)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            crate::platform::hide_window(&mut cmd);
            cmd.output()
                .map_err(|e| format!("Failed to run cargo test: {}", e))?
        };

        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let status = if out.status.success() {
            "PASSED"
        } else {
            "FAILED"
        };
        Ok(format!(
            "[cargo test — {}]\n{}{}",
            status,
            stdout,
            if stderr.is_empty() {
                String::new()
            } else {
                format!("\n{}", stderr)
            }
        ))
    } else if package_json.exists() {
        // Node.js project
        let mut cmd_args = vec!["test".to_string(), "--".to_string()];
        if !filter.is_empty() {
            cmd_args.push(filter.to_string());
        }

        let out = {
            let mut cmd = Command::new("npm");
            cmd.args(&cmd_args)
                .current_dir(project_path)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped());
            crate::platform::hide_window(&mut cmd);
            cmd.output()
                .map_err(|e| format!("Failed to run npm test: {}", e))?
        };

        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let status = if out.status.success() {
            "PASSED"
        } else {
            "FAILED"
        };
        Ok(format!(
            "[npm test — {}]\n{}{}",
            status,
            stdout,
            if stderr.is_empty() {
                String::new()
            } else {
                format!("\n{}", stderr)
            }
        ))
    } else {
        Err(format!(
            "No Cargo.toml or package.json found in '{}'",
            project_path
        ))
    }
}

fn exec_docker_exec(args: &serde_json::Value) -> Result<String, String> {
    let container = args["container"].as_str().unwrap_or("");
    let command = args["command"].as_str().unwrap_or("");

    if container.is_empty() {
        return Err("Container name or ID is required".to_string());
    }
    if command.is_empty() {
        return Err("Command is required".to_string());
    }

    let out = {
        let mut cmd = Command::new("docker");
        cmd.args(["exec", container, "sh", "-c", command]);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
            .map_err(|e| format!("Failed to run docker exec: {}", e))?
    };

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let exit_code = out.status.code().unwrap_or(-1);

    Ok(format!(
        "[docker exec {} — exit {}]\n{}{}",
        container,
        exit_code,
        stdout,
        if stderr.is_empty() {
            String::new()
        } else {
            format!("\nstderr: {}", stderr)
        }
    ))
}

fn exec_notify(args: &serde_json::Value) -> Result<String, String> {
    let title = args["title"].as_str().unwrap_or("ShadowIDE");
    let message = args["message"].as_str().unwrap_or("");
    let channel = args["channel"].as_str().unwrap_or("general");

    if message.is_empty() {
        return Err("Notification message is required".to_string());
    }

    // Try platform-specific notification
    #[cfg(target_os = "linux")]
    {
        let result = Command::new("notify-send")
            .args(["-a", "ShadowIDE", "-c", channel, title, message])
            .output();
        if let Ok(out) = result {
            if out.status.success() {
                return Ok(format!("Notification sent: [{}] {}", title, message));
            }
        }
    }

    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display notification \"{}\" with title \"{}\" subtitle \"{}\"",
            message.replace('"', "\\\""),
            title.replace('"', "\\\""),
            channel
        );
        let result = Command::new("osascript").args(["-e", &script]).output();
        if let Ok(out) = result {
            if out.status.success() {
                return Ok(format!("Notification sent: [{}] {}", title, message));
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let script = format!(
            "[Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null; \
             $template = [Windows.UI.Notifications.ToastNotificationManager]::GetTemplateContent([Windows.UI.Notifications.ToastTemplateType]::ToastText02); \
             $template.GetElementsByTagName('text').Item(0).InnerText = '{}'; \
             $template.GetElementsByTagName('text').Item(1).InnerText = '{}'; \
             $toast = [Windows.UI.Notifications.ToastNotification]::new($template); \
             [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('ShadowIDE').Show($toast)",
            title.replace('\'', "''"),
            message.replace('\'', "''")
        );
        let _ = Command::new("powershell")
            .args(["-NonInteractive", "-Command", &script])
            .output();
        return Ok(format!("Notification sent: [{}] {}", title, message));
    }

    // Fallback: log to stderr
    log::info!("NOTIFY [{}] {}: {}", channel, title, message);
    Ok(format!(
        "Notification logged (no desktop notification available): [{}] {}",
        title, message
    ))
}

// ===== Image Reading =====

fn exec_read_image(path: &str) -> Result<String, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("Failed to read image '{}': {}", path, e))?;
    let b64 = base64_encode_bytes(&bytes);
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("png");
    let mime = match ext.to_lowercase().as_str() {
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "image/png",
    };
    Ok(format!("data:{};base64,{}", mime, b64))
}

fn base64_encode_bytes(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[((n >> 18) & 63) as usize] as char);
        out.push(CHARS[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            CHARS[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            CHARS[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

// ===== Database Query =====

fn exec_database_query(dsn: &str, query: &str) -> Result<String, String> {
    let q_upper = query.trim().to_uppercase();
    if !q_upper.starts_with("SELECT")
        && !q_upper.starts_with("SHOW")
        && !q_upper.starts_with("DESCRIBE")
        && !q_upper.starts_with("EXPLAIN")
    {
        return Err("Only SELECT/SHOW/DESCRIBE/EXPLAIN queries are allowed".to_string());
    }

    if dsn.starts_with("sqlite:") || dsn.ends_with(".db") || dsn.ends_with(".sqlite") {
        let path = dsn
            .trim_start_matches("sqlite://")
            .trim_start_matches("sqlite:");
        let out = Command::new("sqlite3")
            .args(["-json", path, query])
            .output();
        match out {
            Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).to_string()),
            Ok(o) => Err(String::from_utf8_lossy(&o.stderr).to_string()),
            Err(e) => Err(format!("sqlite3 not found: {}", e)),
        }
    } else if dsn.starts_with("postgres") || dsn.starts_with("postgresql") {
        let out = Command::new("psql")
            .args([dsn, "-c", query, "--csv"])
            .output();
        match out {
            Ok(o) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).to_string()),
            Ok(o) => Err(String::from_utf8_lossy(&o.stderr).to_string()),
            Err(e) => Err(format!("psql not found: {}", e)),
        }
    } else {
        Err(format!("Unsupported DSN scheme: {}", dsn))
    }
}

// ===== Deploy Tool =====

fn exec_deploy(target: &str, project_path: &str) -> Result<String, String> {
    let path = std::path::Path::new(project_path);

    // GitHub Actions
    if path.join(".github/workflows").exists() {
        let out = {
            let mut cmd = Command::new("gh");
            cmd.args(["workflow", "run", "--ref", "HEAD"])
                .current_dir(path);
            crate::platform::hide_window(&mut cmd);
            cmd.output()
        };
        if let Ok(o) = out {
            if o.status.success() {
                return Ok(format!(
                    "GitHub Actions triggered: {}",
                    String::from_utf8_lossy(&o.stdout)
                ));
            } else {
                return Err(format!(
                    "GitHub Actions failed: {}",
                    String::from_utf8_lossy(&o.stderr)
                ));
            }
        }
    }

    // Makefile deploy target
    if path.join("Makefile").exists() {
        let target_rule = format!("deploy-{}", target);
        let out = {
            let mut cmd = Command::new("make");
            cmd.arg(&target_rule).current_dir(path);
            crate::platform::hide_window(&mut cmd);
            cmd.output()
        };
        let out = match out {
            Ok(o) if o.status.success() => o,
            _ => {
                let mut cmd = Command::new("make");
                cmd.arg("deploy").current_dir(path);
                crate::platform::hide_window(&mut cmd);
                cmd.output()
                    .map_err(|e| format!("make deploy failed: {}", e))?
            }
        };
        return if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).to_string())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).to_string())
        };
    }

    // npm deploy script
    if path.join("package.json").exists() {
        let script = format!("deploy:{}", target);
        let out = {
            let mut cmd = Command::new("npm");
            cmd.args(["run", &script]).current_dir(path);
            crate::platform::hide_window(&mut cmd);
            cmd.output()
        };
        let out = match out {
            Ok(o) if o.status.success() => o,
            _ => {
                let mut cmd = Command::new("npm");
                cmd.args(["run", "deploy"]).current_dir(path);
                crate::platform::hide_window(&mut cmd);
                cmd.output()
                    .map_err(|e| format!("npm run deploy failed: {}", e))?
            }
        };
        return if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).to_string())
        } else {
            Err(String::from_utf8_lossy(&out.stderr).to_string())
        };
    }

    Err("No deploy configuration found (tried GitHub Actions, Makefile, npm scripts)".to_string())
}
