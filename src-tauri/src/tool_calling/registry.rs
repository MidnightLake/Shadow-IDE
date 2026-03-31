use super::{FunctionDefinition, RiskLevel, ToolDefinition};

pub(crate) struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: serde_json::Value,
    #[allow(dead_code)]
    pub risk_level: RiskLevel,
    pub always_include: bool,
}

pub(crate) fn all_tool_defs() -> Vec<ToolDef> {
    vec![
        // --- Coding Tools ---
        ToolDef {
            name: "shell_exec",
            description: "Execute a shell command and return stdout, stderr, and exit code. \
                          Use for running scripts, build commands, or system operations. \
                          Commands are sandboxed with an allowlist.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command":      { "type": "string", "description": "Shell command to execute" },
                    "working_dir":  { "type": "string", "description": "Working directory (optional, defaults to project root)" },
                    "timeout_secs": { "type": "integer", "description": "Timeout in seconds (default 30)", "default": 30 }
                },
                "required": ["command"]
            }),
            risk_level: RiskLevel::High,
            always_include: true,
        },
        ToolDef {
            name: "read_file",
            description: "Read the contents of a file. Supports partial reads with line ranges. \
                          Always read a file before editing it.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path":       { "type": "string", "description": "File path (relative to project root or absolute)" },
                    "start_line": { "type": "integer", "description": "1-indexed start line (optional)" },
                    "end_line":   { "type": "integer", "description": "1-indexed end line (optional)" }
                },
                "required": ["path"]
            }),
            risk_level: RiskLevel::None,
            always_include: true,
        },
        ToolDef {
            name: "write_file",
            description: "Write or overwrite a file completely. Creates parent directories if needed. \
                          Use patch_file for surgical edits instead of rewriting entire files.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path":    { "type": "string", "description": "File path" },
                    "content": { "type": "string", "description": "Full file content to write" }
                },
                "required": ["path", "content"]
            }),
            risk_level: RiskLevel::Medium,
            always_include: true,
        },
        ToolDef {
            name: "patch_file",
            description: "Apply a targeted search-and-replace patch to a file. \
                          Use instead of write_file when only a small section changes. \
                          old_str must match exactly (including whitespace/indentation). \
                          If exact match fails, a fuzzy line-trimmed match is attempted.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path":    { "type": "string", "description": "File path" },
                    "old_str": { "type": "string", "description": "Exact text to find (must be unique in the file)" },
                    "new_str": { "type": "string", "description": "Replacement text" }
                },
                "required": ["path", "old_str", "new_str"]
            }),
            risk_level: RiskLevel::Medium,
            always_include: true,
        },
        ToolDef {
            name: "list_dir",
            description: "List directory contents. Shows files and directories with [FILE] and [DIR] tags. \
                          Use to explore project structure before reading files.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path":        { "type": "string", "description": "Directory path (default: project root)", "default": "." },
                    "depth":       { "type": "integer", "description": "Recursion depth (default: 1, max: 5)", "default": 1 },
                    "show_hidden": { "type": "boolean", "description": "Show hidden files (default: false)", "default": false }
                }
            }),
            risk_level: RiskLevel::None,
            always_include: true,
        },
        ToolDef {
            name: "grep_search",
            description: "Search file contents using grep/ripgrep. Returns file path, line number, and matching line. \
                          Use to find code patterns, function definitions, or specific text in the project.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern":        { "type": "string", "description": "Search pattern (regex or literal)" },
                    "path":           { "type": "string", "description": "Directory or file to search (default: project root)", "default": "." },
                    "file_glob":      { "type": "string", "description": "File glob filter, e.g. '*.rs' or '*.tsx'" },
                    "context_lines":  { "type": "integer", "description": "Lines of context around each match (default: 2)", "default": 2 },
                    "case_sensitive": { "type": "boolean", "description": "Case-sensitive search (default: false)", "default": false },
                    "max_results":    { "type": "integer", "description": "Max number of results (default: 50)", "default": 50 }
                },
                "required": ["pattern"]
            }),
            risk_level: RiskLevel::None,
            always_include: true,
        },
        ToolDef {
            name: "git_op",
            description: "Perform git operations: status, diff, log, add, commit, branch. \
                          Use for version control operations.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "operation": { "type": "string",
                                   "enum": ["status", "diff", "log", "add", "commit", "checkout",
                                            "branch", "stash", "blame"],
                                   "description": "Git operation to perform" },
                    "args":      { "type": "array", "items": { "type": "string" },
                                   "description": "Additional arguments for the operation" },
                    "path":      { "type": "string", "description": "Repository path (default: project root)", "default": "." }
                },
                "required": ["operation"]
            }),
            risk_level: RiskLevel::Medium,
            always_include: false,
        },
        ToolDef {
            name: "rag_query",
            description: "Search the indexed codebase for semantically similar content. \
                          Use when you need information from the project that isn't in the current context.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language search query" },
                    "top_k": { "type": "integer", "description": "Number of results (default: 5)", "default": 5 }
                },
                "required": ["query"]
            }),
            risk_level: RiskLevel::None,
            always_include: true,
        },
        ToolDef {
            name: "http_request",
            description: "Make HTTP requests (GET/POST/PUT/DELETE). \
                          Use for API calls, service testing, or fetching web content.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "method":       { "type": "string", "enum": ["GET", "POST", "PUT", "DELETE", "PATCH"], "default": "GET" },
                    "url":          { "type": "string", "description": "URL to request" },
                    "headers":      { "type": "object", "additionalProperties": { "type": "string" },
                                      "description": "Request headers" },
                    "body":         { "type": "string", "description": "Request body (for POST/PUT/PATCH)" },
                    "timeout_secs": { "type": "integer", "description": "Timeout in seconds (default: 15)", "default": 15 }
                },
                "required": ["url"]
            }),
            risk_level: RiskLevel::Medium,
            always_include: false,
        },
        ToolDef {
            name: "calculator",
            description: "Evaluate mathematical expressions. Supports +, -, *, /, %, ^, sqrt, sin, cos, pi, e. \
                          Use for any numeric computation.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "expression": { "type": "string", "description": "Math expression to evaluate, e.g. 'sqrt(2^10 + 44) * 3.14'" }
                },
                "required": ["expression"]
            }),
            risk_level: RiskLevel::None,
            always_include: false,
        },
        // --- Web Tools ---
        ToolDef {
            name: "web_fetch",
            description: "Fetch content from a URL and return readable text. Strips HTML tags. \
                          Use for reading documentation, APIs, or web pages.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "url":          { "type": "string", "description": "URL to fetch" },
                    "max_chars":    { "type": "integer", "description": "Max characters to return (default: 8000)", "default": 8000 }
                },
                "required": ["url"]
            }),
            risk_level: RiskLevel::Low,
            always_include: false,
        },
        ToolDef {
            name: "web_search",
            description: "Search the web using DuckDuckGo. Returns titles, URLs, and snippets. \
                          Use when you need current information, documentation links, or answers not in the codebase.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query":       { "type": "string", "description": "Search query" },
                    "max_results": { "type": "integer", "description": "Max results to return (default: 5)", "default": 5 }
                },
                "required": ["query"]
            }),
            risk_level: RiskLevel::Low,
            always_include: false,
        },
        // --- Extended Web & Utility Tools ---
        ToolDef {
            name: "browse_url",
            description: "Fetch a URL and return its readable text content. Strips HTML tags and returns \
                          up to 3000 characters. Use for reading documentation pages, blog posts, or any web content.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL to browse" }
                },
                "required": ["url"]
            }),
            risk_level: RiskLevel::Low,
            always_include: false,
        },
        ToolDef {
            name: "run_tests",
            description: "Run the project's test suite. Detects Cargo.toml (runs `cargo test`) or \
                          package.json (runs `npm test`). Optionally filter to a specific test name.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "project_path": { "type": "string", "description": "Project root directory" },
                    "filter": { "type": "string", "description": "Optional test name filter/pattern" }
                },
                "required": ["project_path"]
            }),
            risk_level: RiskLevel::Medium,
            always_include: false,
        },
        ToolDef {
            name: "docker_exec",
            description: "Execute a shell command inside a running Docker container. \
                          Equivalent to `docker exec <container> sh -c <command>`.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "container": { "type": "string", "description": "Container name or ID" },
                    "command":   { "type": "string", "description": "Shell command to run inside the container" }
                },
                "required": ["container", "command"]
            }),
            risk_level: RiskLevel::High,
            always_include: false,
        },
        ToolDef {
            name: "notify",
            description: "Send a desktop notification to the user. Uses notify-send on Linux, \
                          osascript on macOS. Optionally specify a channel/category.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "title":   { "type": "string", "description": "Notification title" },
                    "message": { "type": "string", "description": "Notification body text" },
                    "channel": { "type": "string", "description": "Optional channel/category (e.g. 'build', 'alert')" }
                },
                "required": ["title", "message"]
            }),
            risk_level: RiskLevel::Low,
            always_include: false,
        },
        // --- Memory Tools ---
        ToolDef {
            name: "memory_store",
            description: "Store a key-value fact in persistent memory. Survives session boundaries and compaction. \
                          Use for remembering important context, decisions, or user preferences.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "key":   { "type": "string", "description": "Memory key (e.g. 'user_preference_theme', 'project_db_type')" },
                    "value": { "type": "string", "description": "Value to store" },
                    "category": { "type": "string", "description": "Category: 'fact', 'preference', 'decision', 'context'", "default": "fact" }
                },
                "required": ["key", "value"]
            }),
            risk_level: RiskLevel::None,
            always_include: false,
        },
        ToolDef {
            name: "memory_recall",
            description: "Retrieve stored memories by key or search query. Returns matching facts from persistent memory.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Key or search term to find memories" }
                },
                "required": ["query"]
            }),
            risk_level: RiskLevel::None,
            always_include: false,
        },
        // --- Build & Diagnostics Tools ---
        ToolDef {
            name: "cargo_run",
            description: "Run cargo subcommands (build, test, check, clippy, fmt, run, doc). \
                          Returns structured output with errors parsed into file:line:message format.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "subcommand": { "type": "string", "enum": ["build", "test", "check", "clippy", "fmt", "run", "doc", "bench"],
                                    "description": "Cargo subcommand to run" },
                    "args":       { "type": "array", "items": { "type": "string" },
                                    "description": "Additional arguments (e.g. ['--release', '-p', 'mypackage'])" },
                    "working_dir": { "type": "string", "description": "Working directory (default: project root)" }
                },
                "required": ["subcommand"]
            }),
            risk_level: RiskLevel::Medium,
            always_include: false,
        },
        ToolDef {
            name: "code_diagnostics",
            description: "Parse compiler/linter output into structured diagnostics: [{file, line, col, severity, message}]. \
                          Pass raw output from cargo, tsc, eslint, gcc, etc.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "output":   { "type": "string", "description": "Raw compiler/linter output to parse" },
                    "language": { "type": "string", "description": "Language hint: 'rust', 'typescript', 'python', 'c'", "default": "auto" }
                },
                "required": ["output"]
            }),
            risk_level: RiskLevel::None,
            always_include: false,
        },
        ToolDef {
            name: "process_list",
            description: "List running processes with CPU and memory usage. \
                          Use to check if servers are running or find resource-heavy processes.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "filter": { "type": "string", "description": "Filter processes by name (optional)" },
                    "limit":  { "type": "integer", "description": "Max processes to return (default: 20)", "default": 20 }
                }
            }),
            risk_level: RiskLevel::None,
            always_include: false,
        },
        // --- RAG Tools ---
        ToolDef {
            name: "rag_index",
            description: "Trigger RAG indexing of files or directories. Use when the codebase index is stale or unbuilt.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Path to index (default: project root)", "default": "." }
                }
            }),
            risk_level: RiskLevel::Low,
            always_include: false,
        },
        ToolDef {
            name: "rag_list_sources",
            description: "List all indexed sources in the RAG knowledge base with chunk counts and stats.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
            risk_level: RiskLevel::None,
            always_include: false,
        },
        // --- Documentation & Data Tools ---
        ToolDef {
            name: "docs_search",
            description: "Search official documentation for a library or API. \
                          Faster and more precise than general web search for coding questions. \
                          Searches docs.rs (Rust), MDN (Web), or Python docs.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query":  { "type": "string", "description": "Search query (e.g. 'tokio spawn_blocking', 'Array.prototype.map')" },
                    "source": { "type": "string", "enum": ["rust", "mdn", "python", "npm", "auto"],
                                "description": "Documentation source (default: auto-detect from query)", "default": "auto" }
                },
                "required": ["query"]
            }),
            risk_level: RiskLevel::None,
            always_include: false,
        },
        ToolDef {
            name: "json_query",
            description: "Query JSON data with path expressions. Use dot notation for nested access, \
                          [N] for array indexing, [*] for iterating arrays. \
                          Input can be a JSON string or a file path to read.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "data":  { "type": "string", "description": "JSON string or file path to a .json file" },
                    "query": { "type": "string", "description": "Query path, e.g. 'users[0].name' or 'items[*].id'" }
                },
                "required": ["data", "query"]
            }),
            risk_level: RiskLevel::None,
            always_include: false,
        },
        ToolDef {
            name: "symbol_lookup",
            description: "Find function, struct, class, or method definitions in source code files. \
                          Uses regex-based extraction. Returns symbol name, kind, file path, and line number.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "symbol": { "type": "string", "description": "Symbol name to search for (e.g. 'MyStruct', 'handle_request')" },
                    "kind":   { "type": "string", "enum": ["function", "struct", "enum", "trait", "impl", "class", "method", "all"],
                                "description": "Kind of symbol to find (default: all)", "default": "all" },
                    "path":   { "type": "string", "description": "Directory to search in (default: project root)", "default": "." },
                    "language": { "type": "string", "enum": ["rust", "python", "typescript", "javascript", "go", "java", "c", "cpp", "auto"],
                                  "description": "Language hint (default: auto-detect from file extension)", "default": "auto" }
                },
                "required": ["symbol"]
            }),
            risk_level: RiskLevel::None,
            always_include: false,
        },
        ToolDef {
            name: "cache_lookup",
            description: "Check if a query has a cached LLM response. Returns the cached answer if found. \
                          Use to avoid redundant LLM calls for previously answered questions.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "The question or prompt to check cache for" }
                },
                "required": ["query"]
            }),
            risk_level: RiskLevel::None,
            always_include: false,
        },
        ToolDef {
            name: "task_schedule",
            description: "Schedule a shell command to run after a delay or at a specific time. \
                          Returns a task ID. The command runs in the background.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "command":   { "type": "string", "description": "Shell command to execute" },
                    "delay_secs": { "type": "integer", "description": "Delay in seconds before executing (mutually exclusive with 'at')" },
                    "at":        { "type": "string", "description": "ISO 8601 datetime to execute at (e.g. '2026-03-13T14:30:00'). Mutually exclusive with 'delay_secs'" },
                    "label":     { "type": "string", "description": "Optional human-readable label for the task" }
                },
                "required": ["command"]
            }),
            risk_level: RiskLevel::High,
            always_include: false,
        },
        // --- Image & Database Tools ---
        ToolDef {
            name: "read_image",
            description: "Read an image file and return it as a base64-encoded data URI. \
                          Supports PNG, JPEG, GIF, and WebP formats.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute or project-relative path to the image file" }
                },
                "required": ["path"]
            }),
            risk_level: RiskLevel::None,
            always_include: false,
        },
        ToolDef {
            name: "database_query",
            description: "Execute a read-only SQL query (SELECT/SHOW/DESCRIBE/EXPLAIN) against a \
                          SQLite or PostgreSQL database. SQLite uses the sqlite3 CLI; PostgreSQL \
                          uses psql. Only read-only queries are permitted.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "dsn":   { "type": "string", "description": "Data source name. For SQLite: file path or 'sqlite:/path/to/db'. For PostgreSQL: 'postgresql://user:pass@host/db'" },
                    "query": { "type": "string", "description": "SQL query to execute (SELECT/SHOW/DESCRIBE/EXPLAIN only)" }
                },
                "required": ["dsn", "query"]
            }),
            risk_level: RiskLevel::Low,
            always_include: false,
        },
        // --- Deploy Tool ---
        ToolDef {
            name: "deploy",
            description: "Deploy the project to a target environment. Detects GitHub Actions, Makefile, \
                          or npm deploy scripts automatically.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "target": { "type": "string", "description": "Deployment target (e.g. 'staging', 'production'). Default: 'staging'", "default": "staging" },
                    "project_path": { "type": "string", "description": "Path to the project root. Default: current directory", "default": "." }
                }
            }),
            risk_level: RiskLevel::High,
            always_include: false,
        },
        // --- System Tools ---
        ToolDef {
            name: "env_read",
            description: "Read system environment information: env vars, OS, CPU, RAM, disk. \
                          Use for understanding the development environment.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "info_type": { "type": "string", "description": "'env_var', 'system', or 'all'", "default": "all" },
                    "var_name":  { "type": "string", "description": "Environment variable name (when info_type='env_var')" }
                },
                "required": []
            }),
            risk_level: RiskLevel::None,
            always_include: false,
        },
    ]
}

/// Get the risk level for a tool by name
pub fn get_tool_risk_level(name: &str) -> RiskLevel {
    all_tool_defs()
        .into_iter()
        .find(|d| d.name == name)
        .map(|d| d.risk_level)
        .unwrap_or(RiskLevel::Medium)
}

pub fn get_tool_definitions() -> Vec<ToolDefinition> {
    all_tool_defs()
        .into_iter()
        .map(|d| ToolDefinition {
            tool_type: "function".to_string(),
            function: FunctionDefinition {
                name: d.name.to_string(),
                description: d.description.to_string(),
                parameters: d.parameters,
            },
        })
        .collect()
}

/// Get only "always_include" tools for context-constrained situations
#[allow(dead_code)]
pub fn get_core_tool_definitions() -> Vec<ToolDefinition> {
    all_tool_defs()
        .into_iter()
        .filter(|d| d.always_include)
        .map(|d| ToolDefinition {
            tool_type: "function".to_string(),
            function: FunctionDefinition {
                name: d.name.to_string(),
                description: d.description.to_string(),
                parameters: d.parameters,
            },
        })
        .collect()
}

/// Get names of core (always_include) tools
pub fn get_core_tool_names() -> Vec<String> {
    all_tool_defs()
        .into_iter()
        .filter(|d| d.always_include)
        .map(|d| d.name.to_string())
        .collect()
}

/// Generate a system prompt injection for models without native tool calling support.
pub fn build_tool_injection_prompt(tools: &[ToolDefinition]) -> String {
    let mut schemas = String::new();
    for tool in tools {
        schemas.push_str(&format!(
            "\n### {}\n{}\nParameters: {}\n",
            tool.function.name,
            tool.function.description,
            serde_json::to_string_pretty(&tool.function.parameters).unwrap_or_default()
        ));
    }

    format!(
        r#"You have access to tools. To call a tool, output a JSON block in this exact format:
```tool_call
{{"tool": "<tool_name>", "args": {{<arguments>}}}}
```

You can call multiple tools by outputting multiple ```tool_call blocks.

After a tool result is shown to you in a [TOOL RESULT] block, continue your response normally.
You may call more tools if needed to complete the task.

When you are done and have no more tools to call, just respond with your final answer text.

Available tools:
{schemas}
"#
    )
}
