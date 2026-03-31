use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Read as IoRead, Write as IoWrite};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

// ── LSP types ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspPosition {
    pub line: u32,
    pub character: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspRange {
    pub start: LspPosition,
    pub end: LspPosition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspDiagnostic {
    pub range: LspRange,
    pub severity: Option<u32>,
    pub message: String,
    pub source: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LspLocation {
    pub uri: String,
    pub range: LspRange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletionItem {
    pub label: String,
    pub detail: Option<String>,
    pub kind: Option<u32>,
    pub insert_text: Option<String>,
}

// ── JSON-RPC transport ─────────────────────────────────────────────────

struct LspTransport {
    stdin: std::process::ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
    next_id: u64,
}

impl LspTransport {
    fn send_request(&mut self, method: &str, params: serde_json::Value) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        self.send_raw(&msg)?;
        Ok(id)
    }

    fn send_notification(&mut self, method: &str, params: serde_json::Value) -> Result<()> {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });
        self.send_raw(&msg)
    }

    fn send_raw(&mut self, msg: &serde_json::Value) -> Result<()> {
        let body = serde_json::to_string(msg)?;
        let header = format!("Content-Length: {}\r\n\r\n", body.len());
        self.stdin.write_all(header.as_bytes())?;
        self.stdin.write_all(body.as_bytes())?;
        self.stdin.flush()?;
        Ok(())
    }

    fn read_message(&mut self) -> Result<serde_json::Value> {
        // Read headers
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            self.stdout.read_line(&mut line)?;
            let line = line.trim();
            if line.is_empty() {
                break;
            }
            if let Some(len_str) = line.strip_prefix("Content-Length: ") {
                content_length = len_str.parse()?;
            }
        }

        if content_length == 0 {
            bail!("empty LSP message");
        }

        let mut body = vec![0u8; content_length];
        self.stdout.read_exact(&mut body)?;
        let msg: serde_json::Value = serde_json::from_slice(&body)?;
        Ok(msg)
    }

    fn read_response(&mut self, expected_id: u64) -> Result<serde_json::Value> {
        // Read messages until we get our response, collecting notifications along the way
        loop {
            let msg = self.read_message()?;
            if let Some(id) = msg.get("id").and_then(|v| v.as_u64()) {
                if id == expected_id {
                    if let Some(err) = msg.get("error") {
                        bail!("LSP error: {}", err);
                    }
                    return Ok(msg.get("result").cloned().unwrap_or(serde_json::Value::Null));
                }
            }
            // Notification or different response — skip
        }
    }
}

// ── Public client ──────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ClangdConfig {
    pub executable: PathBuf,
    pub compile_commands: PathBuf,
}

pub struct ClangdClient {
    pub config: ClangdConfig,
    pub diagnostics: Vec<LspDiagnostic>,
    process: Option<Child>,
    transport: Option<LspTransport>,
    initialized: bool,
}

impl ClangdClient {
    pub fn new(compile_commands: PathBuf) -> Self {
        Self {
            config: ClangdConfig {
                executable: PathBuf::from("clangd"),
                compile_commands,
            },
            diagnostics: Vec::new(),
            process: None,
            transport: None,
            initialized: false,
        }
    }

    pub fn spawn(&mut self) -> Result<()> {
        if self.process.is_some() {
            return Ok(());
        }

        let compile_commands_dir = self
            .config
            .compile_commands
            .parent()
            .unwrap_or(Path::new("."))
            .to_path_buf();

        let mut child = Command::new(&self.config.executable)
            .args([
                "--background-index",
                "--clang-tidy",
                "--completion-style=detailed",
                &format!(
                    "--compile-commands-dir={}",
                    compile_commands_dir.display()
                ),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("Failed to spawn clangd — is it installed and on PATH?")?;

        let stdin = child.stdin.take().context("no stdin")?;
        let stdout = child.stdout.take().context("no stdout")?;

        self.transport = Some(LspTransport {
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        });
        self.process = Some(child);
        Ok(())
    }

    pub fn initialize(&mut self, project_root: &str) -> Result<()> {
        let transport = self.transport.as_mut().context("not spawned")?;
        let root_uri = format!("file://{}", project_root);

        let id = transport.send_request("initialize", serde_json::json!({
            "processId": std::process::id(),
            "rootUri": root_uri,
            "capabilities": {
                "textDocument": {
                    "completion": {
                        "completionItem": {
                            "snippetSupport": false
                        }
                    },
                    "hover": {},
                    "definition": {},
                    "references": {},
                    "publishDiagnostics": {
                        "relatedInformation": true
                    }
                }
            }
        }))?;

        let _result = transport.read_response(id)?;
        transport.send_notification("initialized", serde_json::json!({}))?;
        self.initialized = true;
        Ok(())
    }

    pub fn did_open(&mut self, uri: &str, language: &str, content: &str) -> Result<()> {
        let transport = self.transport.as_mut().context("not spawned")?;
        transport.send_notification("textDocument/didOpen", serde_json::json!({
            "textDocument": {
                "uri": uri,
                "languageId": language,
                "version": 1,
                "text": content
            }
        }))
    }

    pub fn did_change(&mut self, uri: &str, version: i32, content: &str) -> Result<()> {
        let transport = self.transport.as_mut().context("not spawned")?;
        transport.send_notification("textDocument/didChange", serde_json::json!({
            "textDocument": { "uri": uri, "version": version },
            "contentChanges": [{ "text": content }]
        }))
    }

    pub fn completion(&mut self, uri: &str, line: u32, character: u32) -> Result<Vec<CompletionItem>> {
        let transport = self.transport.as_mut().context("not spawned")?;
        let id = transport.send_request("textDocument/completion", serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        }))?;

        let result = transport.read_response(id)?;
        let items = result
            .get("items")
            .or(Some(&result))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| serde_json::from_value(v.clone()).ok())
                    .collect()
            })
            .unwrap_or_default();
        Ok(items)
    }

    pub fn goto_definition(&mut self, uri: &str, line: u32, character: u32) -> Result<Option<LspLocation>> {
        let transport = self.transport.as_mut().context("not spawned")?;
        let id = transport.send_request("textDocument/definition", serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        }))?;

        let result = transport.read_response(id)?;
        if let Some(arr) = result.as_array() {
            if let Some(first) = arr.first() {
                return Ok(serde_json::from_value(first.clone()).ok());
            }
        }
        Ok(serde_json::from_value(result).ok())
    }

    pub fn hover(&mut self, uri: &str, line: u32, character: u32) -> Result<Option<String>> {
        let transport = self.transport.as_mut().context("not spawned")?;
        let id = transport.send_request("textDocument/hover", serde_json::json!({
            "textDocument": { "uri": uri },
            "position": { "line": line, "character": character }
        }))?;

        let result = transport.read_response(id)?;
        let content = result
            .get("contents")
            .and_then(|c| {
                c.get("value")
                    .and_then(|v| v.as_str())
                    .or_else(|| c.as_str())
                    .map(String::from)
            });
        Ok(content)
    }

    pub fn is_running(&self) -> bool {
        self.process.is_some()
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn diagnostic_count(&self) -> usize {
        self.diagnostics.len()
    }

    pub fn kill(&mut self) {
        self.transport = None;
        if let Some(mut child) = self.process.take() {
            let _ = child.kill();
        }
        self.initialized = false;
    }

    pub fn status_line(&self) -> String {
        let cc = if self.config.compile_commands.exists() {
            "compile_commands OK"
        } else {
            "compile_commands missing — run Generate"
        };
        let state = if self.initialized {
            "initialized"
        } else if self.is_running() {
            "running (not initialized)"
        } else {
            "not started"
        };
        format!(
            "clangd {} | {} | {} diagnostics | {}",
            self.config.executable.display(),
            state,
            self.diagnostics.len(),
            cc
        )
    }
}

impl Drop for ClangdClient {
    fn drop(&mut self) {
        self.kill();
    }
}

