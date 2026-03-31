use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{oneshot, Mutex};

const LSP_REQUEST_TIMEOUT_SECS: u64 = 30;

// ===== Types for Frontend =====

#[derive(Debug, Serialize, Clone)]
pub struct LspServerInfo {
    pub language: String,
    pub command: String,
    pub available: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LspDiagnosticEvent {
    pub file: String,
    pub diagnostics: Vec<LspDiagnostic>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct LspDiagnostic {
    pub line: u32,
    pub col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub severity: String,
    pub message: String,
    pub source: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct LspHoverResult {
    pub contents: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct LspCompletionItem {
    pub label: String,
    pub kind: String,
    pub detail: Option<String>,
    pub insert_text: Option<String>,
    pub documentation: Option<String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct LspLocation {
    pub file: String,
    pub line: u32,
    pub col: u32,
}

// ===== Internal State =====

pub struct LspState {
    servers: Arc<Mutex<HashMap<String, LspServer>>>,
}

impl LspState {
    pub fn new() -> Self {
        Self {
            servers: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

struct LspServerInner {
    stdin: Mutex<tokio::process::ChildStdin>,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, Value>>>>>,
    next_id: AtomicI64,
}

struct LspServer {
    inner: Arc<LspServerInner>,
    child: std::sync::Mutex<Option<tokio::process::Child>>,
    reader_handle: tokio::task::JoinHandle<()>,
}

// ===== Helpers =====

pub fn server_key_for_extension(ext: &str) -> Option<&'static str> {
    match ext {
        "rs" => Some("rust"),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Some("typescript"),
        "py" | "pyi" => Some("python"),
        "c" | "h" | "cpp" | "cxx" | "cc" | "hpp" | "hxx" => Some("cpp"),
        "go" => Some("go"),
        "zig" => Some("zig"),
        "lua" => Some("lua"),
        _ => None,
    }
}

fn language_id_for_extension(ext: &str) -> &str {
    match ext {
        "rs" => "rust",
        "ts" => "typescript",
        "tsx" => "typescriptreact",
        "js" | "mjs" | "cjs" => "javascript",
        "jsx" => "javascriptreact",
        "py" | "pyi" => "python",
        "c" | "h" => "c",
        "cpp" | "cxx" | "cc" | "hpp" | "hxx" => "cpp",
        "go" => "go",
        "zig" => "zig",
        "lua" => "lua",
        _ => "plaintext",
    }
}

fn server_command(language: &str) -> Vec<(String, Vec<String>)> {
    match language {
        "rust" => vec![("rust-analyzer".into(), vec![])],
        "typescript" => vec![("typescript-language-server".into(), vec!["--stdio".into()])],
        "python" => vec![
            ("pyright-langserver".into(), vec!["--stdio".into()]),
            ("pylsp".into(), vec![]),
        ],
        "cpp" => vec![("clangd".into(), vec![])],
        "go" => vec![("gopls".into(), vec!["serve".into()])],
        "zig" => vec![("zls".into(), vec![])],
        "lua" => vec![("lua-language-server".into(), vec![])],
        _ => vec![],
    }
}

fn is_command_available(cmd: &str) -> bool {
    crate::platform::is_command_available(cmd)
}

fn file_to_uri(path: &str) -> String {
    format!("file://{}", path)
}

fn uri_to_file(uri: &str) -> String {
    uri.strip_prefix("file://").unwrap_or(uri).to_string()
}

fn ext_from_path(path: &str) -> &str {
    path.rsplit('.').next().unwrap_or("")
}

// ===== JSON-RPC Protocol =====

async fn send_request(
    inner: &LspServerInner,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let id = inner.next_id.fetch_add(1, Ordering::SeqCst);
    let msg = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });

    let body = serde_json::to_string(&msg).map_err(|e| e.to_string())?;
    let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);

    let (tx, rx) = oneshot::channel();
    inner.pending.lock().await.insert(id, tx);

    {
        let mut stdin = inner.stdin.lock().await;
        stdin
            .write_all(frame.as_bytes())
            .await
            .map_err(|e| format!("LSP write error: {}", e))?;
        stdin
            .flush()
            .await
            .map_err(|e| format!("LSP flush error: {}", e))?;
    }

    match tokio::time::timeout(std::time::Duration::from_secs(LSP_REQUEST_TIMEOUT_SECS), rx).await {
        Ok(Ok(result)) => result.map_err(|e| format!("LSP error: {}", e)),
        Ok(Err(_)) => Err("LSP request cancelled".into()),
        Err(_) => {
            inner.pending.lock().await.remove(&id);
            Err("LSP request timed out".into())
        }
    }
}

async fn send_notification(
    inner: &LspServerInner,
    method: &str,
    params: Value,
) -> Result<(), String> {
    let msg = json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": params,
    });

    let body = serde_json::to_string(&msg).map_err(|e| e.to_string())?;
    let frame = format!("Content-Length: {}\r\n\r\n{}", body.len(), body);

    let mut stdin = inner.stdin.lock().await;
    stdin
        .write_all(frame.as_bytes())
        .await
        .map_err(|e| format!("LSP write error: {}", e))?;
    stdin
        .flush()
        .await
        .map_err(|e| format!("LSP flush error: {}", e))?;

    Ok(())
}

async fn reader_loop(
    stdout: tokio::process::ChildStdout,
    pending: Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, Value>>>>>,
    app: AppHandle,
) {
    let mut reader = BufReader::new(stdout);
    let mut line_buf = String::new();

    loop {
        let mut content_length: usize = 0;
        loop {
            line_buf.clear();
            match reader.read_line(&mut line_buf).await {
                Ok(0) => return,
                Ok(_) => {
                    let trimmed = line_buf.trim();
                    if trimmed.is_empty() {
                        break;
                    }
                    if let Some(len_str) = trimmed.strip_prefix("Content-Length: ") {
                        content_length = len_str.parse().unwrap_or(0);
                    }
                }
                Err(_) => return,
            }
        }

        if content_length == 0 {
            continue;
        }

        let mut body = vec![0u8; content_length];
        if reader.read_exact(&mut body).await.is_err() {
            return;
        }

        let msg: Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(_) => continue,
        };

        if let Some(id) = msg.get("id").and_then(|v| v.as_i64()) {
            let mut pending_map = pending.lock().await;
            if let Some(tx) = pending_map.remove(&id) {
                if let Some(error) = msg.get("error") {
                    let _ = tx.send(Err(error.clone()));
                } else {
                    let result = msg.get("result").cloned().unwrap_or(Value::Null);
                    let _ = tx.send(Ok(result));
                }
            }
        } else if let Some(method) = msg.get("method").and_then(|v| v.as_str()) {
            if method == "textDocument/publishDiagnostics" {
                if let Some(params) = msg.get("params") {
                    let event = parse_diagnostics(params);
                    let _ = app.emit("lsp-diagnostics", &event);
                }
            }
        }
    }
}

// ===== Response Parsers =====

fn parse_diagnostics(params: &Value) -> LspDiagnosticEvent {
    let file = uri_to_file(params.get("uri").and_then(|v| v.as_str()).unwrap_or(""));
    let raw = params.get("diagnostics").and_then(|v| v.as_array());

    let diagnostics = raw
        .map(|arr| {
            arr.iter()
                .map(|d| {
                    let range = d.get("range").unwrap_or(&Value::Null);
                    let start = range.get("start").unwrap_or(&Value::Null);
                    let end = range.get("end").unwrap_or(&Value::Null);
                    let severity = match d.get("severity").and_then(|v| v.as_u64()) {
                        Some(1) => "error",
                        Some(2) => "warning",
                        Some(3) => "info",
                        _ => "hint",
                    };
                    LspDiagnostic {
                        line: start.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                        col: start.get("character").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                        end_line: end.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                        end_col: end.get("character").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                        severity: severity.to_string(),
                        message: d
                            .get("message")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        source: d.get("source").and_then(|v| v.as_str()).map(String::from),
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    LspDiagnosticEvent { file, diagnostics }
}

fn extract_hover_contents(result: &Value) -> String {
    // Hover result has "contents" which can be:
    // - MarkedString (string or { language, value })
    // - MarkedString[]
    // - MarkupContent { kind, value }
    if let Some(contents) = result.get("contents") {
        if let Some(s) = contents.as_str() {
            return s.to_string();
        }
        if let Some(value) = contents.get("value").and_then(|v| v.as_str()) {
            return value.to_string();
        }
        if let Some(arr) = contents.as_array() {
            let parts: Vec<String> = arr
                .iter()
                .filter_map(|item| {
                    if let Some(s) = item.as_str() {
                        Some(s.to_string())
                    } else {
                        item.get("value").and_then(|v| v.as_str()).map(String::from)
                    }
                })
                .collect();
            return parts.join("\n\n");
        }
    }
    String::new()
}

fn completion_kind_name(kind: u64) -> String {
    match kind {
        1 => "text",
        2 => "method",
        3 => "function",
        4 => "constructor",
        5 => "field",
        6 => "variable",
        7 => "class",
        8 => "interface",
        9 => "module",
        10 => "property",
        11 => "unit",
        12 => "value",
        13 => "enum",
        14 => "keyword",
        15 => "snippet",
        16 => "color",
        17 => "file",
        18 => "reference",
        19 => "folder",
        20 => "enum_member",
        21 => "constant",
        22 => "struct",
        23 => "event",
        24 => "operator",
        25 => "type_parameter",
        _ => "text",
    }
    .to_string()
}

fn extract_documentation(item: &Value) -> Option<String> {
    let doc = item.get("documentation")?;
    if let Some(s) = doc.as_str() {
        return Some(s.to_string());
    }
    doc.get("value").and_then(|v| v.as_str()).map(String::from)
}

fn parse_locations(value: &Value) -> Vec<LspLocation> {
    if value.is_null() {
        return vec![];
    }
    if let Some(arr) = value.as_array() {
        arr.iter().filter_map(parse_single_location).collect()
    } else {
        parse_single_location(value).into_iter().collect()
    }
}

fn parse_single_location(value: &Value) -> Option<LspLocation> {
    let uri = value
        .get("uri")
        .or_else(|| value.get("targetUri"))?
        .as_str()?;
    let range = value.get("range").or_else(|| value.get("targetRange"))?;
    let start = range.get("start")?;

    Some(LspLocation {
        file: uri_to_file(uri),
        line: start.get("line")?.as_u64()? as u32,
        col: start.get("character")?.as_u64()? as u32,
    })
}

// ===== LSP Auto-Install =====

pub fn lsp_server_installed(server_name: &str) -> bool {
    crate::platform::is_command_available(server_name)
}

pub fn get_lsp_install_command(server_name: &str) -> Option<Vec<String>> {
    match server_name {
        "rust-analyzer" => Some(vec![
            "rustup".into(),
            "component".into(),
            "add".into(),
            "rust-analyzer".into(),
        ]),
        "typescript-language-server" => Some(vec![
            "npm".into(),
            "install".into(),
            "-g".into(),
            "typescript-language-server".into(),
            "typescript".into(),
        ]),
        "pyright" => Some(vec![
            "npm".into(),
            "install".into(),
            "-g".into(),
            "pyright".into(),
        ]),
        "clangd" => {
            if cfg!(target_os = "macos") {
                Some(vec!["brew".into(), "install".into(), "llvm".into()])
            } else {
                Some(vec![
                    "apt-get".into(),
                    "install".into(),
                    "-y".into(),
                    "clangd".into(),
                ])
            }
        }
        "gopls" => Some(vec![
            "go".into(),
            "install".into(),
            "golang.org/x/tools/gopls@latest".into(),
        ]),
        "lua-language-server" => {
            if cfg!(target_os = "macos") {
                Some(vec![
                    "brew".into(),
                    "install".into(),
                    "lua-language-server".into(),
                ])
            } else {
                // Linux: manual install required
                None
            }
        }
        "zls" => Some(vec!["snap".into(), "install".into(), "zls".into()]),
        _ => None,
    }
}

#[tauri::command]
pub async fn auto_install_lsp(
    server_name: String,
    app: tauri::AppHandle,
) -> Result<String, String> {
    let install_cmd = get_lsp_install_command(&server_name)
        .ok_or_else(|| format!("No install command known for '{}'", server_name))?;

    let (program, args) = install_cmd
        .split_first()
        .ok_or_else(|| "Empty install command".to_string())?;

    let mut child = tokio::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start install: {}", e))?;

    // Stream stdout
    if let Some(stdout) = child.stdout.take() {
        use tokio::io::{AsyncBufReadExt, BufReader as TokioBufReader};
        let mut reader = TokioBufReader::new(stdout).lines();
        let app2 = app.clone();
        let sname = server_name.clone();
        tokio::spawn(async move {
            while let Ok(Some(line)) = reader.next_line().await {
                let _ = app2.emit(
                    "lsp-install-progress",
                    serde_json::json!({
                        "server": sname,
                        "line": line,
                    }),
                );
            }
        });
    }

    let status = child
        .wait()
        .await
        .map_err(|e| format!("Install process error: {}", e))?;

    if status.success() {
        Ok("Installed successfully".to_string())
    } else {
        Err(format!("Install failed with status: {}", status))
    }
}

#[tauri::command]
pub fn detect_project_lsp_servers(root: String) -> Vec<String> {
    let mut needed: std::collections::HashSet<String> = std::collections::HashSet::new();
    let skip = ["node_modules", ".git", "target", "dist", "build"];

    fn walk(
        dir: &std::path::Path,
        skip: &[&str],
        needed: &mut std::collections::HashSet<String>,
        depth: usize,
    ) {
        if depth > 4 {
            return;
        }
        let rd = match std::fs::read_dir(dir) {
            Ok(r) => r,
            Err(_) => return,
        };
        for entry in rd.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if skip.contains(&name.as_str()) {
                continue;
            }
            let path = entry.path();
            if path.is_dir() {
                walk(&path, skip, needed, depth + 1);
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                match ext {
                    "rs" => {
                        needed.insert("rust-analyzer".into());
                    }
                    "ts" | "tsx" => {
                        needed.insert("typescript-language-server".into());
                    }
                    "py" => {
                        needed.insert("pyright".into());
                    }
                    "cpp" | "c" | "h" | "cc" | "cxx" => {
                        needed.insert("clangd".into());
                    }
                    "go" => {
                        needed.insert("gopls".into());
                    }
                    "lua" => {
                        needed.insert("lua-language-server".into());
                    }
                    "zig" => {
                        needed.insert("zls".into());
                    }
                    "cs" => {
                        needed.insert("omnisharp".into());
                    }
                    _ => {}
                }
            }
        }
    }

    walk(std::path::Path::new(&root), &skip, &mut needed, 0);
    needed.into_iter().collect()
}

// ===== Tauri Commands =====

#[tauri::command]
pub fn lsp_detect_servers() -> Vec<LspServerInfo> {
    let languages = ["rust", "typescript", "python", "cpp", "go", "zig", "lua"];
    let mut results = Vec::new();

    for lang in languages {
        let candidates = server_command(lang);
        let mut found = false;
        for (cmd, _) in &candidates {
            if is_command_available(cmd) {
                results.push(LspServerInfo {
                    language: lang.to_string(),
                    command: cmd.clone(),
                    available: true,
                });
                found = true;
                break;
            }
        }
        if !found {
            if let Some((cmd, _)) = candidates.first() {
                results.push(LspServerInfo {
                    language: lang.to_string(),
                    command: cmd.clone(),
                    available: false,
                });
            }
        }
    }

    results
}

#[tauri::command]
pub async fn lsp_start(
    language: String,
    root_path: String,
    app: AppHandle,
    state: tauri::State<'_, LspState>,
) -> Result<String, String> {
    let mut servers = state.servers.lock().await;
    if servers.contains_key(&language) {
        return Ok(format!("{} already running", language));
    }

    let candidates = server_command(&language);
    let (cmd, args) = candidates
        .into_iter()
        .find(|(c, _)| is_command_available(c))
        .ok_or_else(|| format!("No language server found for {}", language))?;

    let mut child = {
        let mut proc = tokio::process::Command::new(&cmd);
        proc.args(&args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .current_dir(&root_path);
        crate::platform::hide_window_async(&mut proc);
        proc.spawn()
            .map_err(|e| format!("Failed to start {}: {}", cmd, e))?
    };

    let stdin = child.stdin.take().ok_or("Failed to capture stdin")?;
    let stdout = child.stdout.take().ok_or("Failed to capture stdout")?;

    let inner = Arc::new(LspServerInner {
        stdin: Mutex::new(stdin),
        pending: Arc::new(Mutex::new(HashMap::new())),
        next_id: AtomicI64::new(1),
    });

    let reader_pending = inner.pending.clone();
    let reader_app = app.clone();
    let reader_handle = tokio::spawn(async move {
        reader_loop(stdout, reader_pending, reader_app).await;
    });

    // Initialize
    let init_result = send_request(
        &inner,
        "initialize",
        json!({
            "processId": std::process::id(),
            "rootUri": file_to_uri(&root_path),
            "capabilities": {
                "textDocument": {
                    "hover": { "contentFormat": ["markdown", "plaintext"] },
                    "completion": {
                        "completionItem": {
                            "snippetSupport": false,
                            "documentationFormat": ["markdown", "plaintext"]
                        }
                    },
                    "definition": {},
                    "publishDiagnostics": {
                        "relatedInformation": true
                    },
                    "synchronization": {
                        "didSave": true,
                        "dynamicRegistration": false
                    }
                }
            }
        }),
    )
    .await?;

    let server_name = init_result
        .get("serverInfo")
        .and_then(|s| s.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or(&cmd)
        .to_string();

    send_notification(&inner, "initialized", json!({})).await?;

    let server = LspServer {
        inner,
        child: std::sync::Mutex::new(Some(child)),
        reader_handle,
    };

    servers.insert(language.clone(), server);
    log::info!("LSP started: {} ({})", language, server_name);

    Ok(server_name)
}

#[tauri::command]
pub async fn lsp_stop(language: String, state: tauri::State<'_, LspState>) -> Result<(), String> {
    let server = {
        let mut servers = state.servers.lock().await;
        servers.remove(&language)
    };

    if let Some(server) = server {
        // Graceful shutdown
        let _ = send_request(&server.inner, "shutdown", Value::Null).await;
        let _ = send_notification(&server.inner, "exit", Value::Null).await;

        server.reader_handle.abort();

        let child = server.child.lock().ok().and_then(|mut g| g.take());
        if let Some(mut child) = child {
            let _ = child.kill().await;
        }

        log::info!("LSP stopped: {}", language);
    }

    Ok(())
}

#[tauri::command]
pub async fn lsp_stop_all(state: tauri::State<'_, LspState>) -> Result<(), String> {
    let servers: Vec<(String, LspServer)> = {
        let mut map = state.servers.lock().await;
        map.drain().collect()
    };

    for (lang, server) in servers {
        let _ = send_request(&server.inner, "shutdown", Value::Null).await;
        let _ = send_notification(&server.inner, "exit", Value::Null).await;
        server.reader_handle.abort();
        let child = server.child.lock().ok().and_then(|mut g| g.take());
        if let Some(mut child) = child {
            let _ = child.kill().await;
        }
        log::info!("LSP stopped: {}", lang);
    }

    Ok(())
}

#[tauri::command]
pub async fn lsp_did_open(
    file: String,
    content: String,
    state: tauri::State<'_, LspState>,
) -> Result<(), String> {
    let ext = ext_from_path(&file);
    let key = match server_key_for_extension(ext) {
        Some(k) => k,
        None => return Ok(()),
    };
    let lang_id = language_id_for_extension(ext);

    let inner = {
        let servers = state.servers.lock().await;
        match servers.get(key) {
            Some(s) => s.inner.clone(),
            None => return Ok(()),
        }
    };

    send_notification(
        &inner,
        "textDocument/didOpen",
        json!({
            "textDocument": {
                "uri": file_to_uri(&file),
                "languageId": lang_id,
                "version": 1,
                "text": content
            }
        }),
    )
    .await
}

#[tauri::command]
pub async fn lsp_did_change(
    file: String,
    content: String,
    version: i32,
    state: tauri::State<'_, LspState>,
) -> Result<(), String> {
    let ext = ext_from_path(&file);
    let key = match server_key_for_extension(ext) {
        Some(k) => k,
        None => return Ok(()),
    };

    let inner = {
        let servers = state.servers.lock().await;
        match servers.get(key) {
            Some(s) => s.inner.clone(),
            None => return Ok(()),
        }
    };

    send_notification(
        &inner,
        "textDocument/didChange",
        json!({
            "textDocument": {
                "uri": file_to_uri(&file),
                "version": version
            },
            "contentChanges": [{ "text": content }]
        }),
    )
    .await
}

#[tauri::command]
pub async fn lsp_did_save(file: String, state: tauri::State<'_, LspState>) -> Result<(), String> {
    let ext = ext_from_path(&file);
    let key = match server_key_for_extension(ext) {
        Some(k) => k,
        None => return Ok(()),
    };

    let inner = {
        let servers = state.servers.lock().await;
        match servers.get(key) {
            Some(s) => s.inner.clone(),
            None => return Ok(()),
        }
    };

    send_notification(
        &inner,
        "textDocument/didSave",
        json!({
            "textDocument": { "uri": file_to_uri(&file) }
        }),
    )
    .await
}

#[tauri::command]
pub async fn lsp_did_close(file: String, state: tauri::State<'_, LspState>) -> Result<(), String> {
    let ext = ext_from_path(&file);
    let key = match server_key_for_extension(ext) {
        Some(k) => k,
        None => return Ok(()),
    };

    let inner = {
        let servers = state.servers.lock().await;
        match servers.get(key) {
            Some(s) => s.inner.clone(),
            None => return Ok(()),
        }
    };

    send_notification(
        &inner,
        "textDocument/didClose",
        json!({
            "textDocument": { "uri": file_to_uri(&file) }
        }),
    )
    .await
}

#[tauri::command]
pub async fn lsp_hover(
    file: String,
    line: u32,
    col: u32,
    state: tauri::State<'_, LspState>,
) -> Result<Option<LspHoverResult>, String> {
    let ext = ext_from_path(&file);
    let key = match server_key_for_extension(ext) {
        Some(k) => k,
        None => return Ok(None),
    };

    let inner = {
        let servers = state.servers.lock().await;
        match servers.get(key) {
            Some(s) => s.inner.clone(),
            None => return Ok(None),
        }
    };

    let result = send_request(
        &inner,
        "textDocument/hover",
        json!({
            "textDocument": { "uri": file_to_uri(&file) },
            "position": { "line": line, "character": col }
        }),
    )
    .await?;

    if result.is_null() {
        return Ok(None);
    }

    let contents = extract_hover_contents(&result);
    if contents.is_empty() {
        Ok(None)
    } else {
        Ok(Some(LspHoverResult { contents }))
    }
}

#[tauri::command]
pub async fn lsp_completion(
    file: String,
    line: u32,
    col: u32,
    state: tauri::State<'_, LspState>,
) -> Result<Vec<LspCompletionItem>, String> {
    let ext = ext_from_path(&file);
    let key = match server_key_for_extension(ext) {
        Some(k) => k,
        None => return Ok(vec![]),
    };

    let inner = {
        let servers = state.servers.lock().await;
        match servers.get(key) {
            Some(s) => s.inner.clone(),
            None => return Ok(vec![]),
        }
    };

    let result = send_request(
        &inner,
        "textDocument/completion",
        json!({
            "textDocument": { "uri": file_to_uri(&file) },
            "position": { "line": line, "character": col }
        }),
    )
    .await?;

    let items = if let Some(items) = result.get("items").and_then(|v| v.as_array()) {
        items.clone()
    } else if let Some(items) = result.as_array() {
        items.clone()
    } else {
        return Ok(vec![]);
    };

    let completions = items
        .iter()
        .take(50)
        .map(|item| LspCompletionItem {
            label: item
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            kind: completion_kind_name(item.get("kind").and_then(|v| v.as_u64()).unwrap_or(1)),
            detail: item
                .get("detail")
                .and_then(|v| v.as_str())
                .map(String::from),
            insert_text: item
                .get("insertText")
                .and_then(|v| v.as_str())
                .map(String::from),
            documentation: extract_documentation(item),
        })
        .collect();

    Ok(completions)
}

#[tauri::command]
pub async fn lsp_goto_definition(
    file: String,
    line: u32,
    col: u32,
    state: tauri::State<'_, LspState>,
) -> Result<Vec<LspLocation>, String> {
    let ext = ext_from_path(&file);
    let key = match server_key_for_extension(ext) {
        Some(k) => k,
        None => return Ok(vec![]),
    };

    let inner = {
        let servers = state.servers.lock().await;
        match servers.get(key) {
            Some(s) => s.inner.clone(),
            None => return Ok(vec![]),
        }
    };

    let result = send_request(
        &inner,
        "textDocument/definition",
        json!({
            "textDocument": { "uri": file_to_uri(&file) },
            "position": { "line": line, "character": col }
        }),
    )
    .await?;

    Ok(parse_locations(&result))
}

// ===== New LSP Commands =====

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct TypeHierarchyItem {
    pub name: String,
    pub kind: u32,
    pub uri: String,
    pub range_start_line: u32,
    pub detail: Option<String>,
}

/// Get supertypes for a symbol (classes it extends/implements)
#[tauri::command]
pub async fn lsp_type_hierarchy_supertypes(
    language: String,
    file_uri: String,
    line: u32,
    character: u32,
    state: tauri::State<'_, LspState>,
) -> Result<Vec<TypeHierarchyItem>, String> {
    let inner = {
        let servers = state.servers.lock().await;
        match servers.get(&language) {
            Some(s) => s.inner.clone(),
            None => return Err(format!("LSP server not running for {}", language)),
        }
    };

    // Step 1: prepareTypeHierarchy
    let prepared = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        send_request(
            &inner,
            "textDocument/prepareTypeHierarchy",
            json!({
                "textDocument": { "uri": file_uri },
                "position": { "line": line, "character": character }
            }),
        ),
    )
    .await
    .map_err(|_| "prepareTypeHierarchy timed out".to_string())??;

    let prepare_items = match prepared.as_array() {
        Some(a) if !a.is_empty() => a.clone(),
        _ => return Ok(vec![]),
    };

    let mut all_items: Vec<TypeHierarchyItem> = Vec::new();

    // Step 2: typeHierarchy/supertypes
    for prep_item in &prepare_items {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            send_request(
                &inner,
                "typeHierarchy/supertypes",
                json!({ "item": prep_item }),
            ),
        )
        .await
        .map_err(|_| "typeHierarchy/supertypes timed out".to_string())??;

        if let Some(items) = result.as_array() {
            for item in items {
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let kind = item.get("kind").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let uri = item
                    .get("uri")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let range_start_line = item
                    .get("range")
                    .and_then(|r| r.get("start"))
                    .and_then(|s| s.get("line"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                let detail = item
                    .get("detail")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                all_items.push(TypeHierarchyItem {
                    name,
                    kind,
                    uri,
                    range_start_line,
                    detail,
                });
            }
        }
    }

    Ok(all_items)
}

/// Get subtypes for a symbol (classes that extend it)
#[tauri::command]
pub async fn lsp_type_hierarchy_subtypes(
    language: String,
    file_uri: String,
    line: u32,
    character: u32,
    state: tauri::State<'_, LspState>,
) -> Result<Vec<TypeHierarchyItem>, String> {
    let inner = {
        let servers = state.servers.lock().await;
        match servers.get(&language) {
            Some(s) => s.inner.clone(),
            None => return Err(format!("LSP server not running for {}", language)),
        }
    };

    // Step 1: prepareTypeHierarchy
    let prepared = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        send_request(
            &inner,
            "textDocument/prepareTypeHierarchy",
            json!({
                "textDocument": { "uri": file_uri },
                "position": { "line": line, "character": character }
            }),
        ),
    )
    .await
    .map_err(|_| "prepareTypeHierarchy timed out".to_string())??;

    let prepare_items = match prepared.as_array() {
        Some(a) if !a.is_empty() => a.clone(),
        _ => return Ok(vec![]),
    };

    let mut all_items: Vec<TypeHierarchyItem> = Vec::new();

    // Step 2: typeHierarchy/subtypes
    for prep_item in &prepare_items {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            send_request(
                &inner,
                "typeHierarchy/subtypes",
                json!({ "item": prep_item }),
            ),
        )
        .await
        .map_err(|_| "typeHierarchy/subtypes timed out".to_string())??;

        if let Some(items) = result.as_array() {
            for item in items {
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let kind = item.get("kind").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                let uri = item
                    .get("uri")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let range_start_line = item
                    .get("range")
                    .and_then(|r| r.get("start"))
                    .and_then(|s| s.get("line"))
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as u32;
                let detail = item
                    .get("detail")
                    .and_then(|v| v.as_str())
                    .map(String::from);
                all_items.push(TypeHierarchyItem {
                    name,
                    kind,
                    uri,
                    range_start_line,
                    detail,
                });
            }
        }
    }

    Ok(all_items)
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct CodeAction {
    pub title: String,
    pub kind: Option<String>,
    pub command: Option<String>,
    pub edit: Option<serde_json::Value>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct LspLocationFull {
    pub uri: String,
    pub start_line: u32,
    pub start_char: u32,
    pub end_line: u32,
    pub end_char: u32,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct InlayHint {
    pub line: u32,
    pub character: u32,
    pub label: String,
    pub kind: u32,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct CallHierarchyItem {
    pub name: String,
    pub kind: u32,
    pub uri: String,
    pub range_start_line: u32,
    pub detail: Option<String>,
}

#[tauri::command]
pub async fn lsp_code_action(
    language: String,
    file_uri: String,
    start_line: u32,
    start_char: u32,
    end_line: u32,
    end_char: u32,
    state: tauri::State<'_, LspState>,
) -> Result<Vec<CodeAction>, String> {
    let inner = {
        let servers = state.servers.lock().await;
        match servers.get(&language) {
            Some(s) => s.inner.clone(),
            None => return Err(format!("LSP server not running for {}", language)),
        }
    };

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        send_request(
            &inner,
            "textDocument/codeAction",
            json!({
                "textDocument": { "uri": file_uri },
                "range": {
                    "start": { "line": start_line, "character": start_char },
                    "end":   { "line": end_line,   "character": end_char }
                },
                "context": { "diagnostics": [] }
            }),
        ),
    )
    .await
    .map_err(|_| "lsp_code_action timed out".to_string())??;

    let items = match result.as_array() {
        Some(arr) => arr.clone(),
        None => return Ok(vec![]),
    };

    let actions = items
        .iter()
        .filter_map(|item| {
            let title = item.get("title").and_then(|v| v.as_str())?.to_string();
            let kind = item.get("kind").and_then(|v| v.as_str()).map(String::from);
            let command = item
                .get("command")
                .and_then(|c| c.get("command"))
                .and_then(|v| v.as_str())
                .map(String::from);
            let edit = item.get("edit").cloned();
            Some(CodeAction {
                title,
                kind,
                command,
                edit,
            })
        })
        .collect();

    Ok(actions)
}

#[tauri::command]
pub async fn lsp_rename(
    language: String,
    file_uri: String,
    line: u32,
    character: u32,
    new_name: String,
    state: tauri::State<'_, LspState>,
) -> Result<serde_json::Value, String> {
    let inner = {
        let servers = state.servers.lock().await;
        match servers.get(&language) {
            Some(s) => s.inner.clone(),
            None => return Err(format!("LSP server not running for {}", language)),
        }
    };

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        send_request(
            &inner,
            "textDocument/rename",
            json!({
                "textDocument": { "uri": file_uri },
                "position": { "line": line, "character": character },
                "newName": new_name
            }),
        ),
    )
    .await
    .map_err(|_| "lsp_rename timed out".to_string())??;

    Ok(result)
}

#[tauri::command]
pub async fn lsp_references(
    language: String,
    file_uri: String,
    line: u32,
    character: u32,
    state: tauri::State<'_, LspState>,
) -> Result<Vec<LspLocationFull>, String> {
    let inner = {
        let servers = state.servers.lock().await;
        match servers.get(&language) {
            Some(s) => s.inner.clone(),
            None => return Err(format!("LSP server not running for {}", language)),
        }
    };

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        send_request(
            &inner,
            "textDocument/references",
            json!({
                "textDocument": { "uri": file_uri },
                "position": { "line": line, "character": character },
                "context": { "includeDeclaration": true }
            }),
        ),
    )
    .await
    .map_err(|_| "lsp_references timed out".to_string())??;

    let arr = match result.as_array() {
        Some(a) => a.clone(),
        None => return Ok(vec![]),
    };

    let locations = arr
        .iter()
        .filter_map(|item| {
            let uri = item.get("uri").and_then(|v| v.as_str())?.to_string();
            let range = item.get("range")?;
            let start = range.get("start")?;
            let end = range.get("end")?;
            Some(LspLocationFull {
                uri,
                start_line: start.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                start_char: start.get("character").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                end_line: end.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                end_char: end.get("character").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            })
        })
        .collect();

    Ok(locations)
}

#[tauri::command]
pub async fn lsp_inlay_hints(
    language: String,
    file_uri: String,
    start_line: u32,
    end_line: u32,
    state: tauri::State<'_, LspState>,
) -> Result<Vec<InlayHint>, String> {
    let inner = {
        let servers = state.servers.lock().await;
        match servers.get(&language) {
            Some(s) => s.inner.clone(),
            None => return Err(format!("LSP server not running for {}", language)),
        }
    };

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        send_request(
            &inner,
            "textDocument/inlayHint",
            json!({
                "textDocument": { "uri": file_uri },
                "range": {
                    "start": { "line": start_line, "character": 0 },
                    "end":   { "line": end_line,   "character": 0 }
                }
            }),
        ),
    )
    .await
    .map_err(|_| "lsp_inlay_hints timed out".to_string())??;

    let arr = match result.as_array() {
        Some(a) => a.clone(),
        None => return Ok(vec![]),
    };

    let hints = arr
        .iter()
        .filter_map(|item| {
            let position = item.get("position")?;
            let line = position.get("line").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let character = position
                .get("character")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as u32;
            let label = if let Some(s) = item.get("label").and_then(|v| v.as_str()) {
                s.to_string()
            } else if let Some(parts) = item.get("label").and_then(|v| v.as_array()) {
                parts
                    .iter()
                    .filter_map(|p| p.get("value").and_then(|v| v.as_str()))
                    .collect::<Vec<_>>()
                    .join("")
            } else {
                return None;
            };
            let kind = item.get("kind").and_then(|v| v.as_u64()).unwrap_or(1) as u32;
            Some(InlayHint {
                line,
                character,
                label,
                kind,
            })
        })
        .collect();

    Ok(hints)
}

#[tauri::command]
pub async fn lsp_call_hierarchy_incoming(
    language: String,
    file_uri: String,
    line: u32,
    character: u32,
    state: tauri::State<'_, LspState>,
) -> Result<Vec<CallHierarchyItem>, String> {
    let inner = {
        let servers = state.servers.lock().await;
        match servers.get(&language) {
            Some(s) => s.inner.clone(),
            None => return Err(format!("LSP server not running for {}", language)),
        }
    };

    // Step 1: prepare call hierarchy
    let prepared = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        send_request(
            &inner,
            "textDocument/prepareCallHierarchy",
            json!({
                "textDocument": { "uri": file_uri },
                "position": { "line": line, "character": character }
            }),
        ),
    )
    .await
    .map_err(|_| "prepareCallHierarchy timed out".to_string())??;

    let prepare_items = match prepared.as_array() {
        Some(a) if !a.is_empty() => a.clone(),
        _ => return Ok(vec![]),
    };

    let mut all_items: Vec<CallHierarchyItem> = Vec::new();

    // Step 2: for each prepared item, get incoming calls
    for prep_item in &prepare_items {
        let incoming = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            send_request(
                &inner,
                "callHierarchy/incomingCalls",
                json!({ "item": prep_item }),
            ),
        )
        .await
        .map_err(|_| "incomingCalls timed out".to_string())??;

        if let Some(calls) = incoming.as_array() {
            for call in calls {
                if let Some(from) = call.get("from") {
                    let name = from
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let kind = from.get("kind").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                    let uri = from
                        .get("uri")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let range_start_line = from
                        .get("range")
                        .and_then(|r| r.get("start"))
                        .and_then(|s| s.get("line"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as u32;
                    let detail = from
                        .get("detail")
                        .and_then(|v| v.as_str())
                        .map(String::from);
                    all_items.push(CallHierarchyItem {
                        name,
                        kind,
                        uri,
                        range_start_line,
                        detail,
                    });
                }
            }
        }
    }

    Ok(all_items)
}
