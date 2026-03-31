use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout};
use std::sync::{Arc, Mutex};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DapLaunchConfig {
    pub adapter: String, // "codelldb", "debugpy", "node-debug2", "delve"
    pub program: String, // path to executable/script
    pub args: Vec<String>,
    pub cwd: String,
    pub env: std::collections::HashMap<String, String>,
    pub stop_on_entry: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DapBreakpoint {
    pub file: String,
    pub line: u32,
    pub condition: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DapVariable {
    pub name: String,
    pub value: String,
    pub type_name: Option<String>,
    pub variables_reference: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DapStackFrame {
    pub id: i64,
    pub name: String,
    pub source: Option<String>,
    pub line: u32,
    pub column: u32,
}

static DAP_PROCESS: std::sync::OnceLock<Arc<Mutex<Option<DapSession>>>> =
    std::sync::OnceLock::new();

struct DapSession {
    child: Child,
    stdin: ChildStdin,
    seq: i64,
}

fn dap_state() -> &'static Arc<Mutex<Option<DapSession>>> {
    DAP_PROCESS.get_or_init(|| Arc::new(Mutex::new(None)))
}

fn make_request(method: &str, params: serde_json::Value, seq: i64) -> String {
    let msg = serde_json::json!({
        "seq": seq,
        "type": "request",
        "command": method,
        "arguments": params,
    });
    let body = msg.to_string();
    format!("Content-Length: {}\r\n\r\n{}", body.len(), body)
}

fn resolve_adapter_binary(adapter: &str) -> String {
    // Try common install paths
    match adapter {
        "codelldb" => {
            for path in &[
                "/usr/lib/codelldb/adapter/codelldb",
                "/usr/local/bin/codelldb",
            ] {
                if std::path::Path::new(path).exists() {
                    return path.to_string();
                }
            }
            "codelldb".to_string()
        }
        "debugpy" => "python3 -m debugpy --listen 5678".to_string(),
        "delve" => "dlv".to_string(),
        _ => adapter.to_string(),
    }
}

#[tauri::command]
pub async fn dap_launch(
    config: DapLaunchConfig,
    app_handle: tauri::AppHandle,
) -> Result<String, String> {
    use tauri::Emitter;
    let binary = resolve_adapter_binary(&config.adapter);
    let parts: Vec<&str> = binary.split_whitespace().collect();

    let mut child = std::process::Command::new(parts[0])
        .args(&parts[1..])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to start DAP adapter '{}': {}", config.adapter, e))?;

    let stdin = child.stdin.take().ok_or("No stdin")?;
    let stdout: ChildStdout = child.stdout.take().ok_or("No stdout")?;

    // Spawn reader thread that emits Tauri events for DAP events
    let handle = app_handle.clone();
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        let mut lines = reader.lines();
        let mut content_length: usize = 0;

        loop {
            match lines.next() {
                None => break,
                Some(Err(_)) => break,
                Some(Ok(line)) => {
                    if line.starts_with("Content-Length:") {
                        content_length = line
                            .trim_start_matches("Content-Length:")
                            .trim()
                            .parse()
                            .unwrap_or(0);
                    } else if line.is_empty() && content_length > 0 {
                        // Read body - approximate by reading chars
                        if let Some(Ok(body)) = lines.next() {
                            let _ = handle.emit("dap-event", &body);
                            content_length = 0;
                        }
                    }
                }
            }
        }
    });

    let mut session = DapSession {
        child,
        stdin,
        seq: 1,
    };

    // Send initialize request
    let init_req = make_request(
        "initialize",
        serde_json::json!({
            "clientID": "shadow-ide",
            "clientName": "Shadow IDE",
            "adapterID": config.adapter,
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "supportsRunInTerminalRequest": false,
        }),
        session.seq,
    );
    session.seq += 1;
    session
        .stdin
        .write_all(init_req.as_bytes())
        .map_err(|e| e.to_string())?;

    // Send launch request
    let launch_req = make_request(
        "launch",
        serde_json::json!({
            "program": config.program,
            "args": config.args,
            "cwd": config.cwd,
            "stopOnEntry": config.stop_on_entry,
            "env": config.env,
        }),
        session.seq,
    );
    session.seq += 1;
    session
        .stdin
        .write_all(launch_req.as_bytes())
        .map_err(|e| e.to_string())?;

    *dap_state().lock().unwrap() = Some(session);
    Ok("DAP session started".to_string())
}

#[tauri::command]
pub async fn dap_set_breakpoints(
    file: String,
    breakpoints: Vec<DapBreakpoint>,
) -> Result<String, String> {
    let mut guard = dap_state().lock().unwrap();
    let session = guard.as_mut().ok_or("No active DAP session")?;

    let bps: Vec<serde_json::Value> = breakpoints
        .iter()
        .map(|bp| {
            let mut obj = serde_json::json!({ "line": bp.line });
            if let Some(cond) = &bp.condition {
                obj["condition"] = serde_json::Value::String(cond.clone());
            }
            obj
        })
        .collect();

    let req = make_request(
        "setBreakpoints",
        serde_json::json!({
            "source": { "path": file },
            "breakpoints": bps,
        }),
        session.seq,
    );
    session.seq += 1;
    session
        .stdin
        .write_all(req.as_bytes())
        .map_err(|e| e.to_string())?;
    Ok("Breakpoints set".to_string())
}

#[tauri::command]
pub async fn dap_continue(thread_id: Option<i64>) -> Result<String, String> {
    send_dap_simple_request(
        "continue",
        serde_json::json!({ "threadId": thread_id.unwrap_or(1) }),
    )
}

#[tauri::command]
pub async fn dap_step_over(thread_id: Option<i64>) -> Result<String, String> {
    send_dap_simple_request(
        "next",
        serde_json::json!({ "threadId": thread_id.unwrap_or(1) }),
    )
}

#[tauri::command]
pub async fn dap_step_into(thread_id: Option<i64>) -> Result<String, String> {
    send_dap_simple_request(
        "stepIn",
        serde_json::json!({ "threadId": thread_id.unwrap_or(1) }),
    )
}

#[tauri::command]
pub async fn dap_step_out(thread_id: Option<i64>) -> Result<String, String> {
    send_dap_simple_request(
        "stepOut",
        serde_json::json!({ "threadId": thread_id.unwrap_or(1) }),
    )
}

#[tauri::command]
pub async fn dap_pause(thread_id: Option<i64>) -> Result<String, String> {
    send_dap_simple_request(
        "pause",
        serde_json::json!({ "threadId": thread_id.unwrap_or(1) }),
    )
}

#[tauri::command]
pub async fn dap_stop() -> Result<String, String> {
    let mut guard = dap_state().lock().unwrap();
    if let Some(mut session) = guard.take() {
        let req = make_request(
            "disconnect",
            serde_json::json!({ "restart": false, "terminateDebuggee": true }),
            session.seq,
        );
        let _ = session.stdin.write_all(req.as_bytes());
        let _ = session.child.kill();
    }
    Ok("DAP session stopped".to_string())
}

fn send_dap_simple_request(command: &str, args: serde_json::Value) -> Result<String, String> {
    let mut guard = dap_state().lock().unwrap();
    let session = guard.as_mut().ok_or("No active DAP session")?;
    let req = make_request(command, args, session.seq);
    session.seq += 1;
    session
        .stdin
        .write_all(req.as_bytes())
        .map_err(|e| e.to_string())?;
    Ok(format!("{} sent", command))
}

/// Get supported adapter names and their detection status
#[tauri::command]
pub async fn dap_list_adapters() -> Vec<DapAdapterInfo> {
    let adapters: Vec<(&str, Vec<&str>)> = vec![
        (
            "codelldb",
            vec![
                "/usr/lib/codelldb/adapter/codelldb",
                "/usr/local/bin/codelldb",
            ],
        ),
        ("debugpy", vec![]),
        ("delve", vec!["/usr/local/bin/dlv", "/usr/bin/dlv"]),
    ];

    adapters
        .into_iter()
        .map(|(name, paths)| {
            let installed = if paths.is_empty() {
                // Check via `which`
                std::process::Command::new("which")
                    .arg(match name {
                        "debugpy" => "python3",
                        _ => name,
                    })
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false)
            } else {
                paths.iter().any(|p| std::path::Path::new(p).exists())
            };
            DapAdapterInfo {
                name: name.to_string(),
                installed,
                languages: adapter_languages(name),
            }
        })
        .collect()
}

#[derive(Serialize, Deserialize, Debug)]
pub struct DapAdapterInfo {
    pub name: String,
    pub installed: bool,
    pub languages: Vec<String>,
}

fn adapter_languages(adapter: &str) -> Vec<String> {
    match adapter {
        "codelldb" => vec!["rust".to_string(), "c".to_string(), "cpp".to_string()],
        "debugpy" => vec!["python".to_string()],
        "delve" => vec!["go".to_string()],
        "node-debug2" => vec!["javascript".to_string(), "typescript".to_string()],
        _ => vec![],
    }
}
