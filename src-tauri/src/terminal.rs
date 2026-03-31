use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use serde::Serialize;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter};

#[derive(Debug, Clone, Serialize)]
pub struct TerminalMeta {
    pub id: String,
    pub name: String,
    pub shell: String,
    pub cwd: String,
}

pub struct PtyInstance {
    writer: Box<dyn Write + Send>,
    _child: Box<dyn portable_pty::Child + Send>,
    _master: Box<dyn portable_pty::MasterPty + Send>,
    #[allow(dead_code)]
    pub meta: TerminalMeta,
}

#[derive(Debug, Clone)]
struct SharedTerminal {
    mode: String,
    owner: String,
    updated_at: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SharedTerminalState {
    pub terminal_id: String,
    pub mode: String,
    pub owner: String,
    pub active: bool,
    pub updated_at: u64,
}

pub struct TerminalManager {
    sessions: Mutex<HashMap<String, PtyInstance>>,
    shared: Mutex<HashMap<String, SharedTerminal>>,
}

impl TerminalManager {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            shared: Mutex::new(HashMap::new()),
        }
    }

    /// Create a PTY session and return a reader for output.
    /// The caller is responsible for consuming the reader (e.g. emit events or forward via WebSocket).
    pub fn create_pty(
        &self,
        id: String,
        rows: u16,
        cols: u16,
        cwd: Option<String>,
        shell: Option<String>,
    ) -> Result<Box<dyn Read + Send>, String> {
        let pty_system = native_pty_system();

        let pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Failed to open PTY: {}", e))?;

        let shell_name = shell.clone().unwrap_or_else(|| "default".to_string());
        let mut cmd = match &shell {
            Some(s) => CommandBuilder::new(s),
            None => CommandBuilder::new_default_prog(),
        };
        if let Some(ref dir) = cwd {
            cmd.cwd(dir);
        }

        let child = pair
            .slave
            .spawn_command(cmd)
            .map_err(|e| format!("Failed to spawn shell: {}", e))?;

        // Drop the slave - we only need the master side
        drop(pair.slave);

        let writer = pair
            .master
            .take_writer()
            .map_err(|e| format!("Failed to get PTY writer: {}", e))?;

        let reader = pair
            .master
            .try_clone_reader()
            .map_err(|e| format!("Failed to get PTY reader: {}", e))?;

        let meta = TerminalMeta {
            id: id.clone(),
            name: format!("Terminal ({})", shell_name),
            shell: shell_name,
            cwd: cwd.unwrap_or_default(),
        };

        let instance = PtyInstance {
            writer,
            _child: child,
            _master: pair.master,
            meta,
        };

        self.sessions
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?
            .insert(id, instance);

        Ok(reader)
    }

    /// Write data to a PTY session.
    pub fn write_pty(&self, id: &str, data: &[u8]) -> Result<(), String> {
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let instance = sessions
            .get_mut(id)
            .ok_or_else(|| format!("Terminal session not found: {}", id))?;

        instance
            .writer
            .write_all(data)
            .map_err(|e| format!("Failed to write to PTY: {}", e))?;

        instance
            .writer
            .flush()
            .map_err(|e| format!("Failed to flush PTY: {}", e))?;

        Ok(())
    }

    /// Resize a PTY session.
    pub fn resize_pty(&self, id: &str, rows: u16, cols: u16) -> Result<(), String> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        let instance = sessions
            .get(id)
            .ok_or_else(|| format!("Terminal session not found: {}", id))?;

        instance
            ._master
            .resize(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .map_err(|e| format!("Failed to resize PTY: {}", e))?;

        Ok(())
    }

    /// Close and remove a PTY session.
    pub fn close_pty(&self, id: &str) -> Result<(), String> {
        self.sessions
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?
            .remove(id);
        self.shared
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?
            .remove(id);
        Ok(())
    }

    pub fn start_share(
        &self,
        id: &str,
        mode: &str,
        owner: &str,
    ) -> Result<SharedTerminalState, String> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        if !sessions.contains_key(id) {
            return Err(format!("Terminal session not found: {}", id));
        }
        drop(sessions);

        let state = SharedTerminal {
            mode: mode.to_string(),
            owner: owner.to_string(),
            updated_at: now_ts(),
        };
        self.shared
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?
            .insert(id.to_string(), state.clone());
        Ok(SharedTerminalState {
            terminal_id: id.to_string(),
            mode: state.mode,
            owner: state.owner,
            active: true,
            updated_at: state.updated_at,
        })
    }

    pub fn stop_share(&self, id: &str) -> Result<(), String> {
        self.shared
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?
            .remove(id);
        Ok(())
    }

    pub fn shared_state(&self, id: &str) -> Option<SharedTerminalState> {
        let shared = self.shared.lock().ok()?;
        let session = shared.get(id)?;
        Some(SharedTerminalState {
            terminal_id: id.to_string(),
            mode: session.mode.clone(),
            owner: session.owner.clone(),
            active: true,
            updated_at: session.updated_at,
        })
    }

    pub fn shared_states(&self) -> Result<Vec<SharedTerminalState>, String> {
        let shared = self
            .shared
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut sessions = shared
            .iter()
            .map(|(terminal_id, session)| SharedTerminalState {
                terminal_id: terminal_id.clone(),
                mode: session.mode.clone(),
                owner: session.owner.clone(),
                active: true,
                updated_at: session.updated_at,
            })
            .collect::<Vec<_>>();
        sessions.sort_by(|a, b| a.terminal_id.cmp(&b.terminal_id));
        Ok(sessions)
    }
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn emit_share_state(app: &AppHandle, state: Option<SharedTerminalState>, terminal_id: &str) {
    let payload = match state {
        Some(state) => serde_json::json!(state),
        None => serde_json::json!({
            "terminal_id": terminal_id,
            "active": false,
        }),
    };
    let _ = app.emit("terminal-share-state", payload);
}

#[tauri::command]
pub fn create_terminal(
    id: String,
    rows: u16,
    cols: u16,
    cwd: Option<String>,
    shell: Option<String>,
    app: AppHandle,
    state: tauri::State<'_, Arc<TerminalManager>>,
) -> Result<(), String> {
    let mut reader = state.create_pty(id.clone(), rows, cols, cwd, shell)?;
    let terminal_id = id;
    let manager = state.inner().clone();
    let app_for_thread = app.clone();

    // Spawn a thread to read PTY output and emit to frontend
    std::thread::spawn(move || {
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => {
                    // PTY closed
                    let _ = app_for_thread.emit(&format!("terminal-exit-{}", terminal_id), ());
                    emit_share_state(&app_for_thread, None, &terminal_id);
                    let _ = manager.stop_share(&terminal_id);
                    break;
                }
                Ok(n) => {
                    let data = String::from_utf8_lossy(&buf[..n]).to_string();
                    let _ = app_for_thread
                        .emit(&format!("terminal-output-{}", terminal_id), data.clone());
                    if let Some(shared) = manager.shared_state(&terminal_id) {
                        let _ = app_for_thread.emit(
                            "terminal-share-output",
                            serde_json::json!({
                                "terminal_id": terminal_id,
                                "mode": shared.mode,
                                "owner": shared.owner,
                                "updated_at": shared.updated_at,
                                "data": data,
                            }),
                        );
                    }
                }
                Err(_) => {
                    let _ = app_for_thread.emit(&format!("terminal-exit-{}", terminal_id), ());
                    emit_share_state(&app_for_thread, None, &terminal_id);
                    let _ = manager.stop_share(&terminal_id);
                    break;
                }
            }
        }
    });

    Ok(())
}

#[tauri::command]
pub fn detect_shell() -> Result<Vec<String>, String> {
    let mut shells = Vec::new();

    // Check SHELL env var
    if let Ok(shell) = std::env::var("SHELL") {
        shells.push(shell);
    }

    // Check common shell paths
    let common = [
        "/bin/bash",
        "/bin/zsh",
        "/bin/sh",
        "/usr/bin/fish",
        "/bin/fish",
    ];
    for path in common {
        if std::path::Path::new(path).exists() && !shells.contains(&path.to_string()) {
            shells.push(path.to_string());
        }
    }

    // Windows
    if let Ok(comspec) = std::env::var("COMSPEC") {
        if !shells.contains(&comspec) {
            shells.push(comspec);
        }
    }

    if shells.is_empty() {
        shells.push("sh".to_string());
    }

    Ok(shells)
}

#[tauri::command]
pub fn list_terminals(
    state: tauri::State<'_, Arc<TerminalManager>>,
) -> Result<Vec<String>, String> {
    let sessions = state
        .sessions
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    Ok(sessions.keys().cloned().collect())
}

#[tauri::command]
pub fn write_terminal(
    id: String,
    data: String,
    app: AppHandle,
    state: tauri::State<'_, Arc<TerminalManager>>,
) -> Result<(), String> {
    state.write_pty(&id, data.as_bytes())?;
    if let Some(shared) = state.shared_state(&id) {
        let _ = app.emit(
            "terminal-share-input",
            serde_json::json!({
                "terminal_id": id,
                "mode": shared.mode,
                "owner": shared.owner,
                "updated_at": shared.updated_at,
                "data": data,
            }),
        );
    }
    Ok(())
}

#[tauri::command]
pub fn resize_terminal(
    id: String,
    rows: u16,
    cols: u16,
    state: tauri::State<'_, Arc<TerminalManager>>,
) -> Result<(), String> {
    state.resize_pty(&id, rows, cols)
}

#[tauri::command]
pub fn close_terminal(
    id: String,
    app: AppHandle,
    state: tauri::State<'_, Arc<TerminalManager>>,
) -> Result<(), String> {
    let _ = state.stop_share(&id);
    emit_share_state(&app, None, &id);
    state.close_pty(&id)
}

#[tauri::command]
pub fn terminal_share_start(
    id: String,
    mode: String,
    owner: String,
    app: AppHandle,
    state: tauri::State<'_, Arc<TerminalManager>>,
) -> Result<SharedTerminalState, String> {
    let normalized_mode = match mode.as_str() {
        "read" | "read-write" => mode,
        _ => "read".to_string(),
    };
    let shared = state.start_share(&id, &normalized_mode, &owner)?;
    emit_share_state(&app, Some(shared.clone()), &id);
    Ok(shared)
}

#[tauri::command]
pub fn terminal_share_stop(
    id: String,
    app: AppHandle,
    state: tauri::State<'_, Arc<TerminalManager>>,
) -> Result<(), String> {
    state.stop_share(&id)?;
    emit_share_state(&app, None, &id);
    Ok(())
}

#[tauri::command]
pub fn terminal_share_status(
    state: tauri::State<'_, Arc<TerminalManager>>,
) -> Result<Vec<SharedTerminalState>, String> {
    state.shared_states()
}

#[tauri::command]
pub fn terminal_share_write(
    id: String,
    data: String,
    app: AppHandle,
    state: tauri::State<'_, Arc<TerminalManager>>,
) -> Result<(), String> {
    let shared = state
        .shared_state(&id)
        .ok_or_else(|| format!("Terminal {} is not shared", id))?;
    if shared.mode != "read-write" {
        return Err("Shared terminal is read-only".to_string());
    }
    state.write_pty(&id, data.as_bytes())?;
    let _ = app.emit(
        "terminal-share-input",
        serde_json::json!({
            "terminal_id": id,
            "mode": shared.mode,
            "owner": shared.owner,
            "updated_at": now_ts(),
            "data": data,
        }),
    );
    Ok(())
}
