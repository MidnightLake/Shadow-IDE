//! Bluetooth LE server for offline mobile <-> desktop communication.
//!
//! Uses BlueZ L2CAP streams on Linux. On other platforms (Windows, macOS),
//! the commands return "not supported" stubs so the app compiles everywhere.

use base64::Engine;
use std::sync::atomic::AtomicBool;
#[cfg(target_os = "linux")]
use std::sync::atomic::Ordering;
use tokio::sync::Mutex as TokioMutex;

/// Tauri-managed state for the Bluetooth server.
#[allow(dead_code)]
pub struct BluetoothState {
    running: AtomicBool,
    auth_token: TokioMutex<String>,
    stop_signal: TokioMutex<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl BluetoothState {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            auth_token: TokioMutex::new(String::new()),
            stop_signal: TokioMutex::new(None),
        }
    }
}

// ===== Linux implementation (BlueZ) =====

#[cfg(target_os = "linux")]
use bluer::l2cap::{SocketAddr as L2capAddr, Stream as L2capStream, StreamListener};
#[cfg(target_os = "linux")]
use serde::Serialize;
#[cfg(target_os = "linux")]
use std::io::{Seek, SeekFrom, Write};
#[cfg(target_os = "linux")]
use std::sync::Arc;
#[cfg(target_os = "linux")]
use tauri::{AppHandle, Emitter, Listener, Manager};
#[cfg(target_os = "linux")]
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

#[cfg(target_os = "linux")]
const L2CAP_PSM: u16 = 0x00C1;

#[cfg(target_os = "linux")]
pub const SERVICE_UUID: uuid::Uuid = uuid::Uuid::from_bytes([
    0x5a, 0xd0, 0x1d, 0xe0, 0xb1, 0x7e, 0x4c, 0x5a, 0x9f, 0x3b, 0x6c, 0x8d, 0x2e, 0xa1, 0xf0, 0x01,
]);

#[cfg(target_os = "linux")]
#[derive(Debug, Serialize, Clone)]
struct BtStatusEvent {
    running: bool,
    adapter: String,
    address: String,
}

#[cfg(target_os = "linux")]
#[tauri::command]
pub async fn bt_start_server(
    auth_token: String,
    app: AppHandle,
    state: tauri::State<'_, BluetoothState>,
) -> Result<String, String> {
    if state.running.load(Ordering::SeqCst) {
        return Err("Bluetooth server already running".to_string());
    }

    *state.auth_token.lock().await = auth_token.clone();

    let session = bluer::Session::new()
        .await
        .map_err(|e| format!("BlueZ session error: {}. Is bluetoothd running?", e))?;
    let adapter = session
        .default_adapter()
        .await
        .map_err(|e| format!("No Bluetooth adapter: {}", e))?;

    adapter
        .set_powered(true)
        .await
        .map_err(|e| format!("Cannot power on adapter: {}", e))?;

    let adapter_name = adapter.name().to_string();
    let adapter_addr = adapter
        .address()
        .await
        .map_err(|e| format!("Cannot get adapter address: {}", e))?;
    let addr_str = adapter_addr.to_string();

    let le_adv = bluer::adv::Advertisement {
        advertisement_type: bluer::adv::Type::Peripheral,
        service_uuids: vec![SERVICE_UUID].into_iter().collect(),
        local_name: Some("ShadowIDE".to_string()),
        discoverable: Some(true),
        ..Default::default()
    };

    let adv_handle = adapter
        .advertise(le_adv)
        .await
        .map_err(|e| format!("Cannot start BLE advertisement: {}", e))?;

    let sa = L2capAddr::new(adapter_addr, bluer::AddressType::LePublic, L2CAP_PSM);
    let listener = StreamListener::bind(sa)
        .await
        .map_err(|e| format!("L2CAP bind failed (PSM {}): {}", L2CAP_PSM, e))?;

    state.running.store(true, Ordering::SeqCst);

    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    *state.stop_signal.lock().await = Some(stop_tx);

    let _ = app.emit(
        "bt-status",
        BtStatusEvent {
            running: true,
            adapter: adapter_name.clone(),
            address: addr_str.clone(),
        },
    );

    let app_handle = app.clone();
    let tok = auth_token.clone();
    let ad_name = adapter_name.clone();
    let ad_addr = addr_str.clone();

    tokio::spawn(async move {
        tokio::select! {
            _ = async {
                while let Ok((stream, addr)) = listener.accept().await {
                    log::info!("Accepted Bluetooth L2CAP connection from {}", addr.addr);
                    let h = app_handle.clone();
                    let t = tok.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_bt_client(stream, &t, &h).await {
                            log::error!("BT client error: {}", e);
                        }
                    });
                }
            } => {},
            _ = stop_rx => {
                log::info!("Bluetooth server stopping...");
            }
        }
        drop(adv_handle);
        let _ = app_handle.emit(
            "bt-status",
            BtStatusEvent {
                running: false,
                adapter: ad_name,
                address: ad_addr,
            },
        );
    });

    Ok(format!(
        "BLE server started on {} (PSM {})",
        adapter_name, L2CAP_PSM
    ))
}

#[cfg(target_os = "linux")]
#[tauri::command]
pub async fn bt_stop_server(state: tauri::State<'_, BluetoothState>) -> Result<String, String> {
    if !state.running.load(Ordering::SeqCst) {
        return Err("Bluetooth server not running".to_string());
    }
    if let Some(tx) = state.stop_signal.lock().await.take() {
        let _ = tx.send(());
    }
    state.running.store(false, Ordering::SeqCst);
    Ok("Bluetooth server stopped".to_string())
}

#[cfg(target_os = "linux")]
#[tauri::command]
pub async fn bt_get_status(
    state: tauri::State<'_, BluetoothState>,
) -> Result<serde_json::Value, String> {
    let running = state.running.load(Ordering::SeqCst);

    let (adapter_name, adapter_addr) = match bluer::Session::new().await {
        Ok(session) => match session.default_adapter().await {
            Ok(adapter) => {
                let addr = adapter
                    .address()
                    .await
                    .map(|a| a.to_string())
                    .unwrap_or_default();
                (adapter.name().to_string(), addr)
            }
            Err(_) => ("none".into(), "".into()),
        },
        Err(_) => ("unavailable".into(), "".into()),
    };

    Ok(serde_json::json!({
        "running": running,
        "adapter": adapter_name,
        "address": adapter_addr,
        "psm": L2CAP_PSM,
    }))
}

#[cfg(target_os = "linux")]
#[tauri::command]
pub async fn bt_get_pairing_qr(
    state: tauri::State<'_, BluetoothState>,
) -> Result<(String, String), String> {
    if !state.running.load(Ordering::SeqCst) {
        return Err("Bluetooth server is not running".to_string());
    }

    let token = state.auth_token.lock().await.clone();
    if token.trim().is_empty() {
        return Err("Bluetooth server does not have an active pairing token".to_string());
    }

    let payload = serde_json::json!({
        "transport": "bluetooth",
        "name": "ShadowIDE BLE",
        "pairing_token": token,
        "service_uuid": SERVICE_UUID.to_string(),
        "psm": L2CAP_PSM,
    });

    let code = qrcode::QrCode::new(payload.to_string().as_bytes())
        .map_err(|e| format!("QR generation failed: {}", e))?;
    let svg = code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(200, 200)
        .quiet_zone(true)
        .build();
    let data_url = format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(svg.as_bytes())
    );

    Ok((data_url, token))
}

// ===== Linux-only client handling =====

#[cfg(target_os = "linux")]
async fn handle_bt_client(
    stream: L2capStream,
    auth_token: &str,
    app: &AppHandle,
) -> Result<(), String> {
    let (read_half, write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let writer = Arc::new(TokioMutex::new(write_half));
    let mut authenticated = false;
    let mut line = String::new();

    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("BT read: {}", e))?;
        if n == 0 {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let raw: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                send_json(
                    &writer,
                    &serde_json::json!({"type":"error","message":format!("Bad JSON: {}", e)}),
                )
                .await;
                continue;
            }
        };

        let req_id = raw.get("id").and_then(|v| v.as_u64());
        let msg_type = raw.get("type").and_then(|v| v.as_str()).unwrap_or("");

        if !authenticated {
            if msg_type == "auth" {
                let tok = raw.get("token").and_then(|v| v.as_str()).unwrap_or("");
                if tok == auth_token {
                    authenticated = true;
                    send_json_with_id(
                        &writer,
                        &serde_json::json!({
                            "type":"auth.ok",
                            "device_id": format!("bt-{}", uuid::Uuid::new_v4()),
                        }),
                        req_id,
                    )
                    .await;
                    let _ = app.emit("bt-client-authenticated", "");
                } else {
                    send_json_with_id(
                        &writer,
                        &serde_json::json!({"type":"auth.error","message":"Invalid token"}),
                        req_id,
                    )
                    .await;
                }
            } else {
                send_json_with_id(
                    &writer,
                    &serde_json::json!({"type":"auth.error","message":"Not authenticated"}),
                    req_id,
                )
                .await;
            }
            continue;
        }

        let resp = dispatch(msg_type, &raw, app, &writer).await;
        send_json_with_id(&writer, &resp, req_id).await;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
async fn send_json(
    w: &Arc<TokioMutex<bluer::l2cap::stream::OwnedWriteHalf>>,
    val: &serde_json::Value,
) {
    if let Ok(s) = serde_json::to_string(val) {
        let msg = format!("{}\n", s);
        if let Ok(mut guard) = w.try_lock() {
            let _ = guard.write_all(msg.as_bytes()).await;
            let _ = guard.flush().await;
        }
    }
}

#[cfg(target_os = "linux")]
async fn send_json_with_id(
    w: &Arc<TokioMutex<bluer::l2cap::stream::OwnedWriteHalf>>,
    val: &serde_json::Value,
    id: Option<u64>,
) {
    let val = if let Some(rid) = id {
        if let serde_json::Value::Object(mut map) = val.clone() {
            map.insert("id".to_string(), serde_json::json!(rid));
            serde_json::Value::Object(map)
        } else {
            val.clone()
        }
    } else {
        val.clone()
    };
    send_json(w, &val).await;
}

#[cfg(target_os = "linux")]
fn get_home_dir() -> Option<String> {
    dirs_next::home_dir().map(|p| p.to_string_lossy().to_string())
}

#[cfg(target_os = "linux")]
fn emit_ble_progress(
    app: &AppHandle,
    direction: &str,
    path: &str,
    transferred: usize,
    total: usize,
) {
    let percent = if total == 0 {
        100.0
    } else {
        (transferred as f64 / total as f64 * 100.0).clamp(0.0, 100.0)
    };
    let _ = app.emit(
        "ble-transfer-progress",
        serde_json::json!({
            "direction": direction,
            "path": path,
            "transferred": transferred,
            "total": total,
            "percent": percent,
        }),
    );
}

#[cfg(target_os = "linux")]
fn write_file_chunk(path: &str, offset: u64, content: &str) -> Result<usize, String> {
    crate::fs_commands::sanitize_path_str(path)?;
    let file_path = std::path::PathBuf::from(path);
    if let Some(parent) = file_path.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create parent directories: {}", e))?;
        }
    }

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(&file_path)
        .map_err(|e| format!("Failed to open file: {}", e))?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| format!("Failed to seek: {}", e))?;
    file.write_all(content.as_bytes())
        .map_err(|e| format!("Failed to write chunk: {}", e))?;
    file.flush()
        .map_err(|e| format!("Failed to flush chunk: {}", e))?;
    Ok(content.len())
}

#[cfg(target_os = "linux")]
fn forward_event_to_bt(
    writer: &Arc<TokioMutex<bluer::l2cap::stream::OwnedWriteHalf>>,
    event: &str,
    payload: serde_json::Value,
) {
    let tx = writer.clone();
    let message = serde_json::json!({
        "type": "tauri.event",
        "event": event,
        "payload": payload,
        "id": 0,
    })
    .to_string()
        + "\n";
    tokio::spawn(async move {
        if let Ok(mut guard) = tx.try_lock() {
            let _ = guard.write_all(message.as_bytes()).await;
            let _ = guard.flush().await;
        }
    });
}

#[cfg(target_os = "linux")]
fn register_sync_subscription(
    app: &AppHandle,
    writer: &Arc<TokioMutex<bluer::l2cap::stream::OwnedWriteHalf>>,
) {
    let events = [
        "workspace-file-opened",
        "workspace-file-closed",
        "workspace-file-saved",
        "workspace-cursor-moved",
        "workspace-file-changed",
        "workspace-fs-changed",
        "collab-document-state",
        "collab-call-signal",
        "terminal-share-state",
        "terminal-share-output",
        "terminal-share-input",
        "ble-transfer-progress",
        "ai-chat-complete-notify",
        "llm-server-started",
        "llm-server-stopped",
        "ferrum-message-saved",
    ];

    for event_name in events {
        let tx = writer.clone();
        let label = event_name.to_string();
        app.listen(event_name, move |event| {
            let payload = serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
            forward_event_to_bt(&tx, &label, payload);
        });
    }
}

#[cfg(target_os = "linux")]
async fn dispatch(
    msg_type: &str,
    raw: &serde_json::Value,
    app: &AppHandle,
    writer: &Arc<TokioMutex<bluer::l2cap::stream::OwnedWriteHalf>>,
) -> serde_json::Value {
    match msg_type {
        "sync.ping" => {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            serde_json::json!({"type":"sync.pong","timestamp":ts})
        }
        "sync.getState" => {
            serde_json::json!({"type":"sync.state","open_files":[],"active_file":null,"cursor_line":0,"cursor_column":0,"project_root":get_home_dir()})
        }
        "sync.subscribe" => {
            register_sync_subscription(app, writer);
            serde_json::json!({"type":"sync.subscribed"})
        }
        "fs.homeDir" => {
            serde_json::json!({"type":"fs.homeDir","path":get_home_dir().unwrap_or_else(|| "/".into())})
        }
        "fs.getFileInfo" => {
            let p = raw.get("path").and_then(|v| v.as_str()).unwrap_or("");
            match crate::fs_commands::get_file_info(p.to_string()) {
                Ok(info) => serde_json::json!({
                    "type":"fs.fileInfo",
                    "size":info.size,
                    "is_binary":info.is_binary,
                    "line_count":info.line_count
                }),
                Err(e) => serde_json::json!({"type":"error","message":e}),
            }
        }
        "fs.readDir" => {
            let p = raw.get("path").and_then(|v| v.as_str()).unwrap_or("/");
            match crate::fs_commands::read_directory(p.to_string(), None) {
                Ok(e) => serde_json::json!({"type":"fs.dirEntries","request_path":p,"entries":e}),
                Err(e) => serde_json::json!({"type":"error","message":e}),
            }
        }
        "fs.readFile" => {
            let p = raw.get("path").and_then(|v| v.as_str()).unwrap_or("");
            match crate::fs_commands::read_file_content(p.to_string()) {
                Ok(c) => serde_json::json!({"type":"fs.fileContent","path":p,"content":c}),
                Err(e) => serde_json::json!({"type":"error","message":e}),
            }
        }
        "fs.readChunk" => {
            let p = raw.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let offset = raw.get("offset").and_then(|v| v.as_u64()).unwrap_or(0);
            let length = raw
                .get("length")
                .and_then(|v| v.as_u64())
                .unwrap_or(64 * 1024);
            match crate::fs_commands::read_file_chunk(p.to_string(), offset, length) {
                Ok(content) => {
                    let total = std::fs::metadata(p)
                        .map(|meta| meta.len() as usize)
                        .unwrap_or(content.len());
                    let transferred = (offset as usize)
                        .saturating_add(content.len())
                        .min(total.max(content.len()));
                    emit_ble_progress(app, "download", p, transferred, total.max(content.len()));
                    serde_json::json!({
                        "type":"fs.fileChunk",
                        "path":p,
                        "offset":offset,
                        "length":content.len(),
                        "done": content.len() < length as usize || transferred >= total.max(content.len()),
                        "content":content
                    })
                }
                Err(e) => serde_json::json!({"type":"error","message":e}),
            }
        }
        "fs.writeFile" => {
            let p = raw.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let c = raw.get("content").and_then(|v| v.as_str()).unwrap_or("");
            match crate::fs_commands::write_file_content(p.to_string(), c.to_string()) {
                Ok(_) => serde_json::json!({"type":"fs.ok"}),
                Err(e) => serde_json::json!({"type":"error","message":e}),
            }
        }
        "fs.writeChunk" => {
            let p = raw.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let offset = raw.get("offset").and_then(|v| v.as_u64()).unwrap_or(0);
            let content = raw.get("content").and_then(|v| v.as_str()).unwrap_or("");
            match write_file_chunk(p, offset, content) {
                Ok(written) => {
                    let total = raw
                        .get("total")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as usize)
                        .unwrap_or_else(|| {
                            std::fs::metadata(p)
                                .map(|meta| meta.len() as usize)
                                .unwrap_or(written)
                        });
                    let target_total = total.max(written);
                    let transferred = (offset as usize).saturating_add(written).min(target_total);
                    emit_ble_progress(app, "upload", p, transferred, target_total);
                    serde_json::json!({
                        "type":"fs.chunkWritten",
                        "path":p,
                        "offset":offset,
                        "written":written,
                        "done": transferred >= target_total
                    })
                }
                Err(e) => serde_json::json!({"type":"error","message":e}),
            }
        }
        "fs.createDir" => {
            let p = raw.get("path").and_then(|v| v.as_str()).unwrap_or("");
            match crate::fs_commands::create_directory(p.to_string()) {
                Ok(_) => serde_json::json!({"type":"fs.ok"}),
                Err(e) => serde_json::json!({"type":"error","message":e}),
            }
        }
        "fs.delete" => {
            let p = raw.get("path").and_then(|v| v.as_str()).unwrap_or("");
            match std::fs::remove_file(p).or_else(|_| std::fs::remove_dir_all(p)) {
                Ok(_) => serde_json::json!({"type":"fs.ok"}),
                Err(e) => serde_json::json!({"type":"error","message":e.to_string()}),
            }
        }
        "fs.rename" => {
            let o = raw.get("old_path").and_then(|v| v.as_str()).unwrap_or("");
            let n = raw.get("new_path").and_then(|v| v.as_str()).unwrap_or("");
            match crate::fs_commands::rename_entry(o.to_string(), n.to_string()) {
                Ok(_) => serde_json::json!({"type":"fs.ok"}),
                Err(e) => serde_json::json!({"type":"error","message":e}),
            }
        }
        "llm.get_hardware_info" => match crate::llm_loader::detect_hardware() {
            Ok(i) => serde_json::json!({"type":"llm.hardware_info","info":i}),
            Err(e) => serde_json::json!({"type":"error","message":e}),
        },
        "llm.scan_local_models" => match crate::model_scanner::scan_local_models("".to_string()) {
            Ok(m) => serde_json::json!({"type":"llm.local_models","models":m}),
            Err(_) => serde_json::json!({"type":"llm.local_models","models":[]}),
        },
        "llm.get_llm_server_status" => {
            let st = app.state::<crate::llm_loader::LlmServerState>();
            match crate::llm_loader::get_llm_server_status(st) {
                Ok(s) => serde_json::json!({"type":"llm.server_status","status":s}),
                Err(e) => serde_json::json!({"type":"error","message":e}),
            }
        }
        "llm.detect_recommended_backend" => match crate::llm_loader::detect_recommended_backend() {
            Ok(b) => serde_json::json!({"type":"llm.recommended_backend","backend":b}),
            Err(e) => serde_json::json!({"type":"error","message":e}),
        },
        "llm.check_engine" => {
            let b = raw
                .get("backend")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            match crate::llm_loader::check_engine(b) {
                Ok(info) => {
                    serde_json::json!({"type":"llm.engine_info","installed":info.installed,"binary_path":info.binary_path,"version":info.version,"backend":info.backend})
                }
                Err(e) => serde_json::json!({"type":"error","message":e}),
            }
        }
        "llm.list_installed_engines" => match crate::llm_loader::list_installed_engines() {
            Ok(engines) => serde_json::json!({"type":"llm.installed_engines","engines":engines}),
            Err(e) => serde_json::json!({"type":"error","message":e}),
        },
        "llm.launch_llm_server" => {
            let model_path = raw
                .get("model_path")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let port = raw.get("port").and_then(|v| v.as_u64()).unwrap_or(8080) as u16;
            let backend = raw
                .get("backend")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            let config: crate::llm_loader::LlmConfig =
                serde_json::from_value(raw.get("config").cloned().unwrap_or(serde_json::json!({})))
                    .unwrap_or_default();
            let st = app.state::<crate::llm_loader::LlmServerState>();
            match crate::llm_loader::launch_llm_server(
                model_path,
                config,
                port,
                None,
                backend,
                app.clone(),
                st,
            ) {
                Ok(msg) => serde_json::json!({"type":"llm.server_started","message":msg}),
                Err(e) => serde_json::json!({"type":"error","message":e}),
            }
        }
        "llm.stop_llm_server" => {
            let st = app.state::<crate::llm_loader::LlmServerState>();
            match crate::llm_loader::stop_llm_server(st) {
                Ok(msg) => serde_json::json!({"type":"llm.server_stopped","message":msg}),
                Err(e) => serde_json::json!({"type":"error","message":e}),
            }
        }
        "chat.getSessions" => {
            serde_json::json!({"type":"chat.sessions","sessions_json":crate::ai_bridge::chat_load_sessions_raw()})
        }
        "chat.saveSessions" => {
            let j = raw
                .get("sessions_json")
                .and_then(|v| v.as_str())
                .unwrap_or("[]");
            match crate::ai_bridge::chat_save_sessions(j.to_string()) {
                Ok(_) => serde_json::json!({"type":"chat.ok"}),
                Err(e) => serde_json::json!({"type":"error","message":e}),
            }
        }
        "tauri.invoke" => {
            let cmd = raw.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
            let args = raw.get("args").cloned().unwrap_or(serde_json::Value::Null);

            if cmd == "ai_chat_with_tools" || cmd == "ai_chat_stream" {
                let stream_id = args
                    .get("streamId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let messages: Vec<crate::ai_bridge::ChatMessage> = match serde_json::from_value(
                    args.get("messages")
                        .cloned()
                        .unwrap_or(serde_json::Value::Array(vec![])),
                ) {
                    Ok(m) => m,
                    Err(e) => {
                        log::error!("[bluetooth] Invalid messages JSON: {}", e);
                        return serde_json::json!({"type": "error", "message": format!("Invalid messages JSON: {}", e)});
                    }
                };
                let model = args
                    .get("model")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let temperature = args.get("temperature").and_then(|v| v.as_f64());
                let max_tokens = args
                    .get("maxTokens")
                    .and_then(|v| v.as_i64())
                    .map(|v| v as i32);
                let tools_enabled = args
                    .get("toolsEnabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let root_path = args
                    .get("rootPath")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                let app_clone = app.clone();
                let writer_clone = writer.clone();
                let sid = stream_id.clone();

                let app_for_stream = app.clone();
                let tx_for_stream = writer_clone.clone();
                let sid_for_stream = stream_id.clone();
                let stream_handler_id =
                    app_for_stream.listen(format!("ai-chat-stream-{}", stream_id), move |event| {
                        let payload_val = serde_json::from_str(event.payload())
                            .unwrap_or(serde_json::Value::Null);
                        let resp = serde_json::json!({
                            "type": "tauri.event",
                            "event": format!("ai-chat-stream-{}", sid_for_stream),
                            "payload": payload_val,
                            "id": 0
                        });
                        let msg = format!("{}\n", resp.to_string());
                        let tx = tx_for_stream.clone();
                        tokio::spawn(async move {
                            if let Ok(mut guard) = tx.try_lock() {
                                let _ = guard.write_all(msg.as_bytes()).await;
                                let _ = guard.flush().await;
                            }
                        });
                    });

                let app_for_done = app.clone();
                let tx_for_done = writer_clone.clone();
                let sid_for_done = stream_id.clone();
                let done_handler_id =
                    app_for_done.listen(format!("ai-chat-done-{}", stream_id), move |_| {
                        let resp = serde_json::json!({
                            "type": "tauri.event",
                            "event": format!("ai-chat-done-{}", sid_for_done),
                            "payload": {},
                            "id": 0
                        });
                        let msg = format!("{}\n", resp.to_string());
                        let tx = tx_for_done.clone();
                        tokio::spawn(async move {
                            if let Ok(mut guard) = tx.try_lock() {
                                let _ = guard.write_all(msg.as_bytes()).await;
                                let _ = guard.flush().await;
                            }
                        });
                    });

                tokio::spawn(async move {
                    let ai_state = app_clone.state::<crate::ai_bridge::AiConfig>();
                    let token_cache = app_clone.state::<crate::token_optimizer::TokenCache>();
                    let warm_cache = app_clone.state::<crate::token_optimizer::WarmCache>();
                    let token_settings = app_clone.state::<crate::token_optimizer::TokenSettings>();
                    let rag_state = app_clone.state::<std::sync::Arc<crate::rag_index::RagState>>();
                    let shadow_config = app_clone.state::<crate::config::ConfigState>();

                    let _ = crate::ai_bridge::ai_chat_with_tools(
                        sid,
                        messages,
                        model,
                        None,
                        None,
                        temperature,
                        max_tokens,
                        tools_enabled,
                        "build".to_string(),
                        root_path,
                        app_clone.clone(),
                        ai_state,
                        token_cache,
                        warm_cache,
                        token_settings,
                        rag_state,
                        shadow_config,
                    )
                    .await;

                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    app_clone.unlisten(stream_handler_id);
                    app_clone.unlisten(done_handler_id);
                });
                serde_json::json!({"type":"tauri.invokeResult","result":null})
            } else if cmd == "abort_ai_chat" {
                let stream_id = args
                    .get("streamId")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let ai_state = app.state::<crate::ai_bridge::AiConfig>();
                let _ = crate::ai_bridge::abort_ai_chat(stream_id, ai_state);
                serde_json::json!({"type":"tauri.invokeResult","result":null})
            } else {
                serde_json::json!({"type":"error","message":"Unsupported Tauri command over Bluetooth"})
            }
        }
        "tauri.emitEvent" => {
            let event = raw.get("event").and_then(|v| v.as_str()).unwrap_or("");
            let payload = raw
                .get("payload")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let _ = app.emit(event, payload);
            serde_json::json!({"type":"fs.ok"})
        }
        _ => serde_json::json!({"type":"error","message":format!("Unknown: {}", msg_type)}),
    }
}

// ===== Non-Linux stub implementations =====

#[cfg(not(target_os = "linux"))]
#[tauri::command]
pub async fn bt_start_server(
    _auth_token: String,
    _app: tauri::AppHandle,
    _state: tauri::State<'_, BluetoothState>,
) -> Result<String, String> {
    Err("Bluetooth LE server is only supported on Linux (BlueZ)".to_string())
}

#[cfg(not(target_os = "linux"))]
#[tauri::command]
pub async fn bt_stop_server(_state: tauri::State<'_, BluetoothState>) -> Result<String, String> {
    Err("Bluetooth LE server is only supported on Linux (BlueZ)".to_string())
}

#[cfg(not(target_os = "linux"))]
#[tauri::command]
pub async fn bt_get_status(
    _state: tauri::State<'_, BluetoothState>,
) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "running": false,
        "adapter": "unsupported",
        "address": "",
        "psm": 0,
        "platform_note": "Bluetooth LE server is only supported on Linux"
    }))
}

#[cfg(not(target_os = "linux"))]
#[tauri::command]
pub async fn bt_get_pairing_qr(
    _state: tauri::State<'_, BluetoothState>,
) -> Result<(String, String), String> {
    Err("Bluetooth LE server is only supported on Linux (BlueZ)".to_string())
}
