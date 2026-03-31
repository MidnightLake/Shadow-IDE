use crate::fs_commands;
use crate::pairing::PairingManager;
use crate::terminal::TerminalManager;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use rustls::ServerConfig;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::io::Read;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use tauri::{AppHandle, Emitter, Listener, Manager};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::Message;

const NOISE_PARAMS: &str = "Noise_XX_25519_ChaChaPoly_SHA256";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRecordingSummary {
    pub id: String,
    pub label: Option<String>,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub event_count: u64,
    pub file_path: String,
    pub active: bool,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteRecordingEntry {
    pub timestamp: u64,
    pub direction: String,
    pub client_id: Option<String>,
    pub request_id: Option<u64>,
    pub message_type: String,
    pub payload: serde_json::Value,
}

struct ActiveRemoteRecording {
    summary: RemoteRecordingSummary,
    file: std::fs::File,
}

pub struct RemoteRecorder {
    dir: PathBuf,
    active: std::sync::Mutex<Option<ActiveRemoteRecording>>,
}

impl RemoteRecorder {
    fn new(dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        Self {
            dir,
            active: std::sync::Mutex::new(None),
        }
    }

    fn start(&self, label: Option<String>) -> Result<RemoteRecordingSummary, String> {
        let mut active = self
            .active
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        if active.is_some() {
            return Err("A remote session recording is already active".to_string());
        }

        std::fs::create_dir_all(&self.dir)
            .map_err(|e| format!("Create recording dir failed: {}", e))?;

        let started_at = now_ts();
        let safe_label = label
            .as_deref()
            .map(sanitize_recording_label)
            .filter(|value| !value.is_empty());
        let id = match safe_label {
            Some(ref value) => format!("{}-{}", started_at, value),
            None => format!(
                "{}-{}",
                started_at,
                &uuid::Uuid::new_v4().simple().to_string()[..8]
            ),
        };
        let file_path = self.dir.join(format!("{}.ndjson", id));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file_path)
            .map_err(|e| format!("Failed to open recording file: {}", e))?;

        let summary = RemoteRecordingSummary {
            id: id.clone(),
            label,
            started_at,
            ended_at: None,
            event_count: 0,
            file_path: file_path.to_string_lossy().to_string(),
            active: true,
            size_bytes: 0,
        };
        *active = Some(ActiveRemoteRecording {
            summary: summary.clone(),
            file,
        });

        self.record_value(
            "system",
            None,
            None,
            json!({
                "type": "recording.started",
                "id": id,
                "label": summary.label,
            }),
        );

        Ok(summary)
    }

    fn stop(&self) -> Result<RemoteRecordingSummary, String> {
        let mut active = self
            .active
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let mut recording = active
            .take()
            .ok_or_else(|| "No remote session recording is active".to_string())?;

        let ended_at = now_ts();
        let stop_entry = RemoteRecordingEntry {
            timestamp: ended_at,
            direction: "system".to_string(),
            client_id: None,
            request_id: None,
            message_type: "recording.stopped".to_string(),
            payload: json!({
                "type": "recording.stopped",
                "id": recording.summary.id,
                "event_count": recording.summary.event_count,
            }),
        };
        let line = serde_json::to_string(&stop_entry).map_err(|e| e.to_string())?;
        use std::io::Write;
        writeln!(recording.file, "{}", line)
            .map_err(|e| format!("Write recording failed: {}", e))?;
        recording
            .file
            .flush()
            .map_err(|e| format!("Flush recording failed: {}", e))?;

        recording.summary.event_count += 1;
        recording.summary.ended_at = Some(ended_at);
        recording.summary.active = false;
        recording.summary.size_bytes = std::fs::metadata(&recording.summary.file_path)
            .map(|meta| meta.len())
            .unwrap_or(0);

        let meta_path = self.dir.join(format!("{}.json", recording.summary.id));
        std::fs::write(
            &meta_path,
            serde_json::to_vec_pretty(&recording.summary).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("Write recording metadata failed: {}", e))?;

        Ok(recording.summary)
    }

    fn active_summary(&self) -> Result<Option<RemoteRecordingSummary>, String> {
        let active = self
            .active
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        Ok(active.as_ref().map(|recording| {
            let mut summary = recording.summary.clone();
            summary.size_bytes = std::fs::metadata(&summary.file_path)
                .map(|meta| meta.len())
                .unwrap_or(0);
            summary
        }))
    }

    fn list(&self) -> Result<Vec<RemoteRecordingSummary>, String> {
        let mut recordings = Vec::new();
        if self.dir.exists() {
            for entry in std::fs::read_dir(&self.dir)
                .map_err(|e| format!("Read recording dir failed: {}", e))?
            {
                let entry = entry.map_err(|e| e.to_string())?;
                let path = entry.path();
                if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                    continue;
                }
                if let Ok(bytes) = std::fs::read(&path) {
                    if let Ok(summary) = serde_json::from_slice::<RemoteRecordingSummary>(&bytes) {
                        recordings.push(summary);
                    }
                }
            }
        }

        if let Some(active) = self.active_summary()? {
            recordings.retain(|recording| recording.id != active.id);
            recordings.push(active);
        }

        recordings.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        Ok(recordings)
    }

    fn load(
        &self,
        recording_id: &str,
        limit: Option<usize>,
    ) -> Result<Vec<RemoteRecordingEntry>, String> {
        let path = self.dir.join(format!("{}.ndjson", recording_id));
        let bytes = std::fs::read(&path)
            .map_err(|e| format!("Failed to read recording {}: {}", recording_id, e))?;
        let mut entries = Vec::new();
        for line in String::from_utf8_lossy(&bytes).lines() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(entry) = serde_json::from_str::<RemoteRecordingEntry>(line) {
                entries.push(entry);
            }
        }
        if let Some(limit) = limit {
            if entries.len() > limit {
                return Ok(entries.split_off(entries.len() - limit));
            }
        }
        Ok(entries)
    }

    fn record_text(
        &self,
        direction: &str,
        client_id: Option<&str>,
        request_id: Option<u64>,
        raw: &str,
    ) {
        let payload = serde_json::from_str::<serde_json::Value>(raw)
            .unwrap_or_else(|_| json!({ "raw": raw }));
        self.record_value(direction, client_id, request_id, payload);
    }

    fn record_value(
        &self,
        direction: &str,
        client_id: Option<&str>,
        request_id: Option<u64>,
        payload: serde_json::Value,
    ) {
        let mut active = match self.active.lock() {
            Ok(active) => active,
            Err(_) => return,
        };
        let Some(recording) = active.as_mut() else {
            return;
        };
        let message_type = payload
            .get("type")
            .and_then(|value| value.as_str())
            .unwrap_or(direction)
            .to_string();
        let entry = RemoteRecordingEntry {
            timestamp: now_ts(),
            direction: direction.to_string(),
            client_id: client_id.map(|id| id.to_string()),
            request_id,
            message_type,
            payload,
        };
        let Ok(line) = serde_json::to_string(&entry) else {
            return;
        };
        use std::io::Write;
        if writeln!(recording.file, "{}", line).is_ok() {
            let _ = recording.file.flush();
            recording.summary.event_count += 1;
            recording.summary.size_bytes = recording
                .summary
                .size_bytes
                .saturating_add(line.len() as u64 + 1);
        }
    }
}

fn sanitize_recording_label(label: &str) -> String {
    label
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch
            } else if ch == '-' || ch == '_' {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

// ===== PrependedStream: replay already-read bytes before delegating to inner stream =====

struct PrependedStream<S> {
    prefix: Vec<u8>,
    prefix_pos: usize,
    inner: S,
}

impl<S> PrependedStream<S> {
    fn new(prefix: Vec<u8>, inner: S) -> Self {
        Self {
            prefix,
            prefix_pos: 0,
            inner,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for PrependedStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.prefix_pos < this.prefix.len() {
            let remaining = &this.prefix[this.prefix_pos..];
            let n = std::cmp::min(remaining.len(), buf.remaining());
            buf.put_slice(&remaining[..n]);
            this.prefix_pos += n;
            Poll::Ready(Ok(()))
        } else {
            Pin::new(&mut this.inner).poll_read(cx, buf)
        }
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for PrependedStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

// ===== SecureSender: wraps mpsc channel + optional Noise transport for encrypted sending =====

#[derive(Clone)]
struct SecureSender {
    tx: mpsc::UnboundedSender<Message>,
    noise: Arc<std::sync::Mutex<Option<snow::TransportState>>>,
    recorder: Arc<RemoteRecorder>,
    client_id: String,
}

impl SecureSender {
    fn new(
        tx: mpsc::UnboundedSender<Message>,
        recorder: Arc<RemoteRecorder>,
        client_id: String,
    ) -> Self {
        Self {
            tx,
            noise: Arc::new(std::sync::Mutex::new(None)),
            recorder,
            client_id,
        }
    }

    fn send_json(&self, resp: &RemoteResponse) {
        self.send_json_with_id(resp, None);
    }

    /// Send a JSON response, optionally injecting a request `id` for mobile client promise resolution.
    /// Large payloads (>4KB) are deflate-compressed before sending.
    fn send_json_with_id(&self, resp: &RemoteResponse, request_id: Option<u64>) {
        if let Ok(json) = serde_json::to_string(resp) {
            let json = if let Some(rid) = request_id {
                // Inject id into the serialized JSON for mobile bridge promise matching
                if json.starts_with('{') {
                    format!("{{\"id\":{},{}", rid, &json[1..])
                } else {
                    json
                }
            } else {
                json
            };
            self.recorder
                .record_text("outbound", Some(&self.client_id), request_id, &json);
            // Compress large payloads before encryption
            let json = self.maybe_compress(json);
            let msg_text = if let Ok(guard) = self.noise.lock() {
                if let Some(ref _transport) = *guard {
                    drop(guard);
                    self.encrypt_text(&json).unwrap_or(json)
                } else {
                    json
                }
            } else {
                json
            };
            let _ = self.tx.send(Message::Text(msg_text.into()));
        }
    }

    fn encrypt_text(&self, plaintext: &str) -> Option<String> {
        let mut guard = self.noise.lock().ok()?;
        let transport = guard.as_mut()?;
        let mut buf = vec![0u8; plaintext.len() + 64];
        let len = transport
            .write_message(plaintext.as_bytes(), &mut buf)
            .ok()?;
        let encoded = base64::engine::general_purpose::STANDARD.encode(&buf[..len]);
        Some(format!(r#"{{"type":"e","d":"{}"}}"#, encoded))
    }

    fn set_noise(&self, transport: snow::TransportState) {
        if let Ok(mut guard) = self.noise.lock() {
            *guard = Some(transport);
        }
    }

    fn decrypt_text(&self, raw: &str) -> Option<String> {
        let wrapper: serde_json::Value = serde_json::from_str(raw).ok()?;
        if wrapper.get("type")?.as_str()? != "e" {
            return None;
        }
        let data_b64 = wrapper.get("d")?.as_str()?;
        let encrypted = base64::engine::general_purpose::STANDARD
            .decode(data_b64)
            .ok()?;
        let mut guard = self.noise.lock().ok()?;
        let transport = guard.as_mut()?;
        let mut buf = vec![0u8; encrypted.len()];
        let len = transport.read_message(&encrypted, &mut buf).ok()?;
        String::from_utf8(buf[..len].to_vec()).ok()
    }

    /// Compress a JSON string if it exceeds the threshold (4KB).
    /// Returns a `{"type":"z","d":"<base64 deflate>"}` wrapper, or the original string if small.
    fn maybe_compress(&self, json: String) -> String {
        const COMPRESS_THRESHOLD: usize = 4096;
        if json.len() < COMPRESS_THRESHOLD {
            return json;
        }
        use flate2::write::DeflateEncoder;
        use flate2::Compression;
        use std::io::Write;
        let mut encoder = DeflateEncoder::new(Vec::new(), Compression::fast());
        if encoder.write_all(json.as_bytes()).is_err() {
            return json;
        }
        match encoder.finish() {
            Ok(compressed) => {
                // Only use compression if it actually saves space
                if compressed.len() >= json.len() {
                    return json;
                }
                let b64 = base64::engine::general_purpose::STANDARD.encode(&compressed);
                format!(r#"{{"type":"z","d":"{}"}}"#, b64)
            }
            Err(_) => json,
        }
    }

    fn has_noise(&self) -> bool {
        self.noise.lock().map(|g| g.is_some()).unwrap_or(false)
    }
}

// ===== State =====

pub struct RemoteServerState {
    pub running: AtomicBool,
    pub port: std::sync::Mutex<u16>,
    pub pairing: Arc<PairingManager>,
    pub connected_clients: std::sync::Mutex<Vec<ConnectedClient>>,
    pub session_timeout_secs: std::sync::Mutex<u64>,
    pub ide_state: std::sync::Mutex<IdeState>,
    pub recorder: Arc<RemoteRecorder>,
}

/// Shared IDE state for cross-device sync.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IdeState {
    pub open_files: Vec<String>,
    pub active_file: Option<String>,
    pub cursor_line: u32,
    pub cursor_column: u32,
    pub project_root: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectedClient {
    pub id: String,
    pub addr: String,
    pub connected_at: u64,
    pub last_activity: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RemoteServerInfo {
    pub running: bool,
    pub port: u16,
    pub local_ip: String,
    pub connected_clients: Vec<ConnectedClient>,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkInfo {
    pub local_ip: String,
    pub tailscale_ip: Option<String>,
    pub tailscale_hostname: Option<String>,
    pub wireguard_ip: Option<String>,
}

/// Protocol version — increment when breaking changes are made to message format.
pub const PROTOCOL_VERSION: u32 = 2;

// ===== Remote Protocol Messages =====

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum RemoteRequest {
    // Authentication
    #[serde(rename = "auth")]
    Auth { token: String, device_name: String },

    // File system
    #[serde(rename = "fs.getFileInfo")]
    GetFileInfo { path: String },
    #[serde(rename = "fs.readDir")]
    ReadDir { path: String },
    #[serde(rename = "fs.readFile")]
    ReadFile { path: String },
    #[serde(rename = "fs.writeFile")]
    WriteFile { path: String, content: String },
    #[serde(rename = "fs.createDir")]
    CreateDir { path: String },
    #[serde(rename = "fs.delete")]
    Delete { path: String },
    #[serde(rename = "fs.rename")]
    Rename {
        #[serde(alias = "oldPath")]
        old_path: String,
        #[serde(alias = "newPath")]
        new_path: String,
    },
    #[serde(rename = "fs.homeDir")]
    HomeDir,
    /// Apply a line-level patch: replace lines `start_line..end_line` with `new_content`
    #[serde(rename = "fs.patchFile")]
    PatchFile {
        path: String,
        /// 1-based start line (inclusive)
        start_line: usize,
        /// 1-based end line (inclusive) — lines start_line..=end_line are replaced
        end_line: usize,
        /// Replacement content (may be empty to delete lines, or contain more/fewer lines)
        new_content: String,
    },

    // Terminal
    #[serde(rename = "term.create")]
    TermCreate {
        id: String,
        rows: u16,
        cols: u16,
        cwd: Option<String>,
    },
    #[serde(rename = "term.write")]
    TermWrite { id: String, data: String },
    #[serde(rename = "term.resize")]
    TermResize { id: String, rows: u16, cols: u16 },
    #[serde(rename = "term.close")]
    TermClose { id: String },

    // State sync
    #[serde(rename = "sync.ping")]
    Ping,
    #[serde(rename = "sync.getState")]
    GetState,
    #[serde(rename = "sync.openFile")]
    OpenFile { path: String },

    // Noise protocol handshake
    #[serde(rename = "noise.init")]
    NoiseInit { data: String },
    #[serde(rename = "noise.handshake")]
    NoiseHandshake { data: String },

    // LLM Forwarding
    #[serde(rename = "llm.get_hardware_info")]
    LlmGetHardwareInfo,
    #[serde(rename = "llm.scan_local_models")]
    LlmScanLocalModels,
    #[serde(rename = "llm.get_llm_server_status")]
    LlmGetServerStatus,
    #[serde(rename = "llm.check_engine")]
    LlmCheckEngine { backend: Option<String> },
    #[serde(rename = "llm.list_installed_engines")]
    LlmListInstalledEngines,
    #[serde(rename = "llm.detect_recommended_backend")]
    LlmDetectRecommendedBackend,
    #[serde(rename = "llm.launch_llm_server")]
    LlmLaunchServer {
        #[serde(alias = "modelPath")]
        model_path: String,
        config: crate::llm_loader::LlmConfig,
        port: u16,
        backend: Option<String>,
    },
    #[serde(rename = "llm.stop_llm_server")]
    LlmStopServer,
    #[serde(rename = "llm.get_llm_network_info")]
    LlmGetNetworkInfo { port: u16 },

    // Chat sync (legacy AiChat)
    #[serde(rename = "chat.getSessions")]
    ChatGetSessions,
    #[serde(rename = "chat.saveSessions")]
    ChatSaveSessions {
        #[serde(alias = "sessionsJson")]
        sessions_json: String,
    },

    // FerrumChat over remote
    #[serde(rename = "ferrum.listSessions")]
    FerrumListSessions,
    #[serde(rename = "ferrum.getLatestSession")]
    FerrumGetLatestSession,
    #[serde(rename = "ferrum.createSession")]
    FerrumCreateSession { name: String, profile: String },
    #[serde(rename = "ferrum.loadMessages")]
    FerrumLoadMessages {
        #[serde(alias = "sessionId")]
        session_id: String,
    },
    #[serde(rename = "ferrum.saveMessage")]
    FerrumSaveMessage {
        #[serde(alias = "sessionId")]
        session_id: String,
        message: serde_json::Value,
    },
    #[serde(rename = "ferrum.deleteSession")]
    FerrumDeleteSession {
        #[serde(alias = "sessionId")]
        session_id: String,
    },
    #[serde(rename = "ferrum.renameSession")]
    FerrumRenameSession {
        #[serde(alias = "sessionId")]
        session_id: String,
        #[serde(alias = "newName")]
        new_name: String,
    },
    #[serde(rename = "ferrum.getProfiles")]
    FerrumGetProfiles,
    #[serde(rename = "ferrum.checkProvider")]
    FerrumCheckProvider {
        #[serde(alias = "baseUrl")]
        base_url: String,
    },
    #[serde(rename = "ferrum.listProviderModels")]
    FerrumListProviderModels {
        #[serde(alias = "baseUrl")]
        base_url: String,
    },
    #[serde(rename = "ferrum.getTokenCount")]
    FerrumGetTokenCount {
        #[serde(alias = "sessionId")]
        session_id: String,
    },
    #[serde(rename = "ferrum.exportSession")]
    FerrumExportSession {
        #[serde(alias = "sessionId")]
        session_id: String,
    },

    // Workspace subscription
    #[serde(rename = "sync.subscribe")]
    SubscribeWorkspace,

    // Heartbeat
    #[serde(rename = "heartbeat")]
    Heartbeat,

    #[serde(rename = "tauri.invoke")]
    TauriInvoke {
        cmd: String,
        args: serde_json::Value,
    },

    #[serde(rename = "tauri.emitEvent")]
    TauriEmitEvent {
        event: String,
        payload: serde_json::Value,
    },

    // ---- Persistent Agent Session ----
    /// Connect to (or create) a persistent agent session.
    /// If session_id is null, a new session is created.
    /// last_seq is used to replay missed events on reconnect.
    #[serde(rename = "agent.connect")]
    AgentConnect {
        #[serde(alias = "sessionId", alias = "session_id")]
        session_id: Option<String>,
        #[serde(alias = "lastSeq", alias = "last_seq", default)]
        last_seq: u64,
    },

    /// Send a user message to the persistent agent.
    #[serde(rename = "agent.message")]
    AgentMessage {
        #[serde(alias = "sessionId", alias = "session_id")]
        session_id: String,
        #[serde(flatten)]
        message: crate::agent_queue::UserMessage,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
pub enum RemoteResponse {
    #[serde(rename = "auth.ok")]
    AuthOk {
        device_id: String,
        protocol_version: u32,
    },
    #[serde(rename = "auth.error")]
    AuthError { message: String },

    #[serde(rename = "fs.dirEntries")]
    DirEntries {
        request_path: String,
        entries: Vec<fs_commands::FileEntry>,
    },
    #[serde(rename = "fs.fileContent")]
    FileContent { path: String, content: String },
    #[serde(rename = "fs.fileInfo")]
    FileInfoResult {
        size: u64,
        is_binary: bool,
        line_count: Option<u64>,
    },
    #[serde(rename = "fs.ok")]
    FsOk,
    #[serde(rename = "fs.homeDir")]
    HomeDirResult { path: String },

    #[serde(rename = "term.ok")]
    TermOk,
    #[serde(rename = "term.output")]
    TermOutput { id: String, data: String },
    #[serde(rename = "term.exit")]
    TermExit { id: String },

    #[serde(rename = "sync.pong")]
    Pong { timestamp: u64 },
    #[serde(rename = "sync.state")]
    SyncState {
        open_files: Vec<String>,
        active_file: Option<String>,
        cursor_line: u32,
        cursor_column: u32,
        project_root: Option<String>,
    },
    #[serde(rename = "sync.fileOpened")]
    FileOpened { path: String },

    #[serde(rename = "noise.handshake")]
    NoiseHandshakeResp { data: String },
    #[serde(rename = "noise.ready")]
    NoiseReady,

    // LLM Responses
    #[serde(rename = "llm.hardware_info")]
    LlmHardwareInfo {
        info: crate::llm_loader::HardwareInfo,
    },
    #[serde(rename = "llm.local_models")]
    LlmLocalModels {
        models: Vec<crate::model_scanner::LocalModel>,
    },
    #[serde(rename = "llm.server_status")]
    LlmServerStatus { status: serde_json::Value },
    #[serde(rename = "llm.engine_info")]
    LlmEngineInfo { info: crate::llm_loader::EngineInfo },
    #[serde(rename = "llm.installed_engines")]
    LlmInstalledEngines { engines: Vec<String> },
    #[serde(rename = "llm.recommended_backend")]
    LlmRecommendedBackend { backend: String },
    #[serde(rename = "llm.ok")]
    LlmOk,

    // Chat sync
    #[serde(rename = "chat.sessions")]
    ChatSessions { sessions_json: String },
    #[serde(rename = "chat.ok")]
    ChatOk,

    // FerrumChat responses
    #[serde(rename = "ferrum.sessions")]
    FerrumSessions { sessions: serde_json::Value },
    #[serde(rename = "ferrum.session")]
    FerrumSession { session: serde_json::Value },
    #[serde(rename = "ferrum.messages")]
    FerrumMessages { messages: serde_json::Value },
    #[serde(rename = "ferrum.profiles")]
    FerrumProfiles { profiles: serde_json::Value },
    #[serde(rename = "ferrum.tokenCount")]
    FerrumTokenCount { count: usize },
    #[serde(rename = "ferrum.export")]
    FerrumExport { markdown: String },
    #[serde(rename = "ferrum.ok")]
    FerrumOk,
    #[serde(rename = "ferrum.providerCheck")]
    FerrumProviderCheck { connected: bool },
    #[serde(rename = "ferrum.providerModels")]
    FerrumProviderModels {
        models: Vec<String>,
        connected: bool,
    },

    // Workspace events
    #[serde(rename = "workspace.event")]
    WorkspaceEvent {
        event: String,
        payload: serde_json::Value,
    },

    // Heartbeat
    #[serde(rename = "heartbeat.ack")]
    HeartbeatAck { timestamp: u64 },

    #[serde(rename = "tauri.event")]
    TauriEvent {
        event: String,
        payload: serde_json::Value,
    },
    #[serde(rename = "tauri.invokeResult")]
    TauriInvokeResult { result: serde_json::Value },

    // ---- Persistent Agent Session ----
    #[serde(rename = "agent.connected")]
    AgentConnected {
        session_id: String,
        current_seq: u64,
    },
    #[serde(rename = "agent.replay")]
    AgentReplay {
        events: Vec<crate::session::AgentEvent>,
    },
    #[serde(rename = "agent.event")]
    AgentEventResp {
        session_id: String,
        #[serde(flatten)]
        event: crate::session::AgentEvent,
    },
    #[serde(rename = "agent.queued")]
    AgentQueued,

    #[serde(rename = "error")]
    Error {
        message: String,
        /// Structured error code for programmatic handling (optional for backward compat).
        #[serde(skip_serializing_if = "Option::is_none")]
        code: Option<ErrorCode>,
    },
}

/// Structured error codes for remote protocol errors.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum ErrorCode {
    /// Authentication required or token invalid.
    AuthRequired,
    /// Requested file/resource not found.
    NotFound,
    /// Permission denied (e.g., path traversal blocked).
    PermissionDenied,
    /// Invalid request format or parameters.
    InvalidRequest,
    /// Server-side error (e.g., I/O failure, mutex poisoned).
    InternalError,
    /// Requested feature or command not supported.
    Unsupported,
    /// Resource conflict (e.g., file modified concurrently).
    Conflict,
}

impl RemoteResponse {
    /// Create an error response with just a message (no error code).
    fn err(message: String) -> Self {
        Self::Error {
            message,
            code: None,
        }
    }
    /// Create an error response with a message and structured error code.
    #[allow(dead_code)]
    fn err_with_code(message: String, code: ErrorCode) -> Self {
        Self::Error {
            message,
            code: Some(code),
        }
    }
}

impl RemoteServerState {
    pub fn new(pairing: Arc<PairingManager>) -> Self {
        let recordings_dir = pairing.data_dir.join("remote-recordings");
        Self {
            running: AtomicBool::new(false),
            port: std::sync::Mutex::new(9876),
            pairing,
            connected_clients: std::sync::Mutex::new(Vec::new()),
            session_timeout_secs: std::sync::Mutex::new(1800),
            ide_state: std::sync::Mutex::new(IdeState::default()),
            recorder: Arc::new(RemoteRecorder::new(recordings_dir)),
        }
    }
}

// ===== Tauri Commands =====

#[tauri::command]
pub fn remote_get_info(
    state: tauri::State<'_, Arc<RemoteServerState>>,
) -> Result<RemoteServerInfo, String> {
    let port = *state
        .port
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let clients = state
        .connected_clients
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?
        .clone();

    Ok(RemoteServerInfo {
        running: state.running.load(Ordering::SeqCst),
        port,
        local_ip: "see app".to_string(),
        connected_clients: clients,
    })
}

#[tauri::command]
pub fn remote_generate_cert(
    state: tauri::State<'_, Arc<RemoteServerState>>,
) -> Result<String, String> {
    state.pairing.generate_server_cert()?;
    state.pairing.server_fingerprint()
}

#[tauri::command]
pub fn remote_get_qr_code(
    state: tauri::State<'_, Arc<RemoteServerState>>,
) -> Result<(String, String), String> {
    let port = *state
        .port
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    state.pairing.generate_qr_code(port)
}

#[tauri::command]
pub fn remote_list_devices(
    state: tauri::State<'_, Arc<RemoteServerState>>,
) -> Result<Vec<crate::pairing::PairedDevice>, String> {
    state.pairing.list_paired_devices()
}

#[tauri::command]
pub fn remote_remove_device(
    id: String,
    state: tauri::State<'_, Arc<RemoteServerState>>,
) -> Result<(), String> {
    state.pairing.remove_paired_device(&id)
}

#[tauri::command]
pub fn remote_update_device_permissions(
    id: String,
    permissions: Vec<String>,
    state: tauri::State<'_, Arc<RemoteServerState>>,
) -> Result<crate::pairing::PairedDevice, String> {
    state.pairing.update_device_permissions(&id, permissions)
}

#[tauri::command]
pub fn remote_get_recording_status(
    state: tauri::State<'_, Arc<RemoteServerState>>,
) -> Result<Option<RemoteRecordingSummary>, String> {
    state.recorder.active_summary()
}

#[tauri::command]
pub fn remote_start_recording(
    label: Option<String>,
    state: tauri::State<'_, Arc<RemoteServerState>>,
) -> Result<RemoteRecordingSummary, String> {
    state.recorder.start(label)
}

#[tauri::command]
pub fn remote_stop_recording(
    state: tauri::State<'_, Arc<RemoteServerState>>,
) -> Result<RemoteRecordingSummary, String> {
    state.recorder.stop()
}

#[tauri::command]
pub fn remote_list_recordings(
    state: tauri::State<'_, Arc<RemoteServerState>>,
) -> Result<Vec<RemoteRecordingSummary>, String> {
    state.recorder.list()
}

#[tauri::command]
pub fn remote_load_recording(
    recording_id: String,
    limit: Option<usize>,
    state: tauri::State<'_, Arc<RemoteServerState>>,
) -> Result<Vec<RemoteRecordingEntry>, String> {
    state.recorder.load(&recording_id, limit)
}

#[tauri::command]
pub async fn remote_start_server(
    port: Option<u16>,
    app: AppHandle,
    state: tauri::State<'_, Arc<RemoteServerState>>,
    terminal_state: tauri::State<'_, Arc<TerminalManager>>,
) -> Result<(), String> {
    if state.running.load(Ordering::SeqCst) {
        return Ok(());
    }

    let server_state = Arc::clone(&*state);
    let terminal_manager = Arc::clone(&*terminal_state);
    let app_handle = app.clone();

    tokio::spawn(async move {
        // Use a block to ensure mutexes are dropped early
        let cert_data = {
            let cert_pem = server_state
                .pairing
                .server_cert_pem
                .lock()
                .ok()
                .and_then(|g| g.clone());
            let key_pem = server_state
                .pairing
                .server_key_pem
                .lock()
                .ok()
                .and_then(|g| g.clone());

            match (cert_pem, key_pem) {
                (Some(c), Some(k)) => Some((c, k)),
                _ => {
                    if let Err(e) = server_state.pairing.generate_server_cert() {
                        let _ = app_handle.emit(
                            "remote-server-error",
                            format!("Cert generation failed: {}", e),
                        );
                        return;
                    }
                    let c = server_state
                        .pairing
                        .server_cert_pem
                        .lock()
                        .ok()
                        .and_then(|g| g.clone());
                    let k = server_state
                        .pairing
                        .server_key_pem
                        .lock()
                        .ok()
                        .and_then(|g| g.clone());
                    c.zip(k)
                }
            }
        };

        let (cert_pem, key_pem) = match cert_data {
            Some(d) => d,
            None => return,
        };

        let listen_port = port
            .or_else(|| std::env::var("SHADOW_WS_PORT").ok()?.parse().ok())
            .unwrap_or(9876);

        if let Ok(mut p) = server_state.port.lock() {
            *p = listen_port;
        }

        let tls_config = match build_tls_config(&cert_pem, &key_pem) {
            Ok(c) => c,
            Err(e) => {
                let _ = app_handle.emit("remote-server-error", format!("TLS config failed: {}", e));
                return;
            }
        };
        let acceptor = TlsAcceptor::from(Arc::new(tls_config));
        let noise_keypair = get_or_create_noise_keypair().ok();

        let addr = SocketAddr::from(([0, 0, 0, 0], listen_port));
        let listener = match TcpListener::bind(addr).await {
            Ok(l) => l,
            Err(e) => {
                let _ = app_handle.emit("remote-server-error", format!("Bind failed: {}", e));
                return;
            }
        };

        server_state.running.store(true, Ordering::SeqCst);
        let _ = app_handle.emit("remote-server-started", listen_port);

        // Write discovery file so the CLI can auto-detect the server port
        if let Some(config_dir) = dirs_next::config_dir() {
            let shadow_dir = config_dir.join("shadowai");
            let _ = std::fs::create_dir_all(&shadow_dir);
            let _ = std::fs::write(shadow_dir.join("server.port"), listen_port.to_string());
        }

        while server_state.running.load(Ordering::SeqCst) {
            let (stream, peer_addr) = match listener.accept().await {
                Ok(s) => s,
                Err(_) => continue,
            };

            let acceptor = acceptor.clone();
            let state = server_state.clone();
            let tm = terminal_manager.clone();
            let app = app_handle.clone();
            let noise_kp = noise_keypair.clone();

            tokio::spawn(async move {
                let mut buf = [0u8; 1];
                let is_tls = match stream.peek(&mut buf).await {
                    Ok(1) => buf[0] == 0x16,
                    _ => false,
                };

                if is_tls {
                    if let Ok(tls_stream) = acceptor.accept(stream).await {
                        handle_http_or_ws(tls_stream, peer_addr, state, tm, app, noise_kp).await;
                    }
                } else {
                    handle_http_or_ws(stream, peer_addr, state, tm, app, noise_kp).await;
                }
            });
        }
        let _ = app_handle.emit("remote-server-stopped", ());
    });

    Ok(())
}

async fn handle_http_or_ws<S>(
    mut stream: S,
    peer_addr: SocketAddr,
    state: Arc<RemoteServerState>,
    tm: Arc<TerminalManager>,
    app: AppHandle,
    noise_kp: Option<(Vec<u8>, Vec<u8>)>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    // Read initial HTTP request to determine WebSocket upgrade vs PWA request
    let mut request_buf = vec![0u8; 4096];
    let mut total = 0;
    loop {
        match tokio::io::AsyncReadExt::read(&mut stream, &mut request_buf[total..]).await {
            Ok(0) => break,
            Ok(n) => {
                total += n;
                if request_buf[..total].windows(4).any(|w| w == b"\r\n\r\n") || total >= 4096 {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    let request_str = String::from_utf8_lossy(&request_buf[..total]).to_lowercase();
    let is_websocket = request_str.contains("upgrade: websocket");

    if is_websocket {
        // Replay already-read bytes for WebSocket handshake
        let prepended = PrependedStream::new(request_buf[..total].to_vec(), stream);
        match tokio_tungstenite::accept_async(prepended).await {
            Ok(ws) => handle_client(ws, peer_addr, state, tm, app, noise_kp).await,
            Err(e) => {
                log::warn!("WebSocket upgrade failed from {}: {}", peer_addr, e);
            }
        }
    } else {
        // Serve PWA mobile client
        let html = include_str!("pwa_client.html");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-cache\r\n\r\n{}",
            html.len(),
            html
        );
        let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes()).await;
    }
}

#[tauri::command]
pub fn remote_stop_server(state: tauri::State<'_, Arc<RemoteServerState>>) -> Result<(), String> {
    state.running.store(false, Ordering::SeqCst);
    // Remove discovery file
    if let Some(config_dir) = dirs_next::config_dir() {
        let _ = std::fs::remove_file(config_dir.join("shadowai").join("server.port"));
    }
    Ok(())
}

#[tauri::command]
pub fn remote_set_timeout(
    seconds: u64,
    state: tauri::State<'_, Arc<RemoteServerState>>,
) -> Result<(), String> {
    *state
        .session_timeout_secs
        .lock()
        .map_err(|e| format!("Lock error: {}", e))? = seconds;
    Ok(())
}

#[tauri::command]
pub fn remote_check_cert_expiry(
    state: tauri::State<'_, Arc<RemoteServerState>>,
) -> Result<Option<u64>, String> {
    let cert_pem = state
        .pairing
        .server_cert_pem
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    match &*cert_pem {
        Some(_) => {
            // Certs are generated with expiry 2034-01-01
            let expiry_timestamp: u64 = 2019686400; // 2034-01-01 UTC
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            if now >= expiry_timestamp {
                Ok(Some(0))
            } else {
                Ok(Some((expiry_timestamp - now) / 86400))
            }
        }
        None => Ok(None),
    }
}

/// Push IDE state from frontend for cross-device sync.
#[tauri::command]
pub fn remote_update_state(
    open_files: Vec<String>,
    active_file: Option<String>,
    cursor_line: u32,
    cursor_column: u32,
    project_root: Option<String>,
    state: tauri::State<'_, Arc<RemoteServerState>>,
) -> Result<(), String> {
    let mut ide_state = state
        .ide_state
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    ide_state.open_files = open_files;
    ide_state.active_file = active_file;
    ide_state.cursor_line = cursor_line;
    ide_state.cursor_column = cursor_column;
    ide_state.project_root = project_root;
    Ok(())
}

/// Detect network info including Tailscale and WireGuard.
#[tauri::command]
pub fn remote_detect_network() -> Result<NetworkInfo, String> {
    let local_ip = local_ip_address::local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    let (tailscale_ip, tailscale_hostname) = detect_tailscale();
    let wireguard_ip = detect_wireguard();

    Ok(NetworkInfo {
        local_ip,
        tailscale_ip,
        tailscale_hostname,
        wireguard_ip,
    })
}

/// Get the server's Noise protocol static public key (base64).
#[tauri::command]
pub fn remote_get_noise_pubkey() -> Result<String, String> {
    let (_, public) = get_or_create_noise_keypair()?;
    Ok(base64::engine::general_purpose::STANDARD.encode(&public))
}

// ===== Internal Functions =====

fn detect_tailscale() -> (Option<String>, Option<String>) {
    let ip = {
        let mut cmd = std::process::Command::new("tailscale");
        cmd.args(["ip", "-4"]);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
    }
    .ok()
    .and_then(|o| {
        if o.status.success() {
            String::from_utf8(o.stdout)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        } else {
            None
        }
    });

    let hostname = {
        let mut cmd = std::process::Command::new("tailscale");
        cmd.args(["status", "--self", "--json"]);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
    }
    .ok()
    .and_then(|o| {
        if o.status.success() {
            serde_json::from_slice::<serde_json::Value>(&o.stdout)
                .ok()
                .and_then(|v| {
                    v["Self"]["DNSName"]
                        .as_str()
                        .map(|s| s.trim_end_matches('.').to_string())
                })
        } else {
            None
        }
    });

    (ip, hostname)
}

fn detect_wireguard() -> Option<String> {
    // Get WireGuard interface names
    let interfaces = {
        let mut cmd = std::process::Command::new("wg");
        cmd.args(["show", "interfaces"]);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
    }
    .ok()
    .and_then(|o| {
        if o.status.success() {
            String::from_utf8(o.stdout)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        } else {
            None
        }
    })?;

    let first_iface = interfaces.split_whitespace().next()?;

    // Get IP address from the WireGuard interface
    {
        let mut cmd = std::process::Command::new("ip");
        cmd.args(["-4", "addr", "show", first_iface]);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
    }
    .ok()
    .and_then(|o| {
        if o.status.success() {
            let output = String::from_utf8(o.stdout).ok()?;
            output
                .lines()
                .find(|l| l.contains("inet "))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|s| s.split('/').next())
                .map(|s| s.to_string())
        } else {
            None
        }
    })
}

/// Get or create persistent Noise X25519 keypair (stored in ~/.config/shadowide/noise.key).
fn get_or_create_noise_keypair() -> Result<(Vec<u8>, Vec<u8>), String> {
    let config_dir = dirs_next::config_dir()
        .ok_or("No config directory")?
        .join("shadowide");
    let key_path = config_dir.join("noise.key");

    // Try to load existing keypair
    if key_path.exists() {
        if let Ok(data) = std::fs::read(&key_path) {
            if data.len() == 64 {
                return Ok((data[..32].to_vec(), data[32..].to_vec()));
            }
        }
    }

    // Generate new keypair
    let params: snow::params::NoiseParams = NOISE_PARAMS
        .parse()
        .map_err(|_| "Invalid noise params".to_string())?;
    let keypair = snow::Builder::new(params)
        .generate_keypair()
        .map_err(|e| format!("Noise keygen failed: {}", e))?;

    // Store persistently (32 bytes private + 32 bytes public)
    std::fs::create_dir_all(&config_dir).map_err(|e| format!("Create config dir: {}", e))?;
    let mut data = Vec::with_capacity(64);
    data.extend_from_slice(&keypair.private);
    data.extend_from_slice(&keypair.public);
    std::fs::write(&key_path, &data).map_err(|e| format!("Write noise key: {}", e))?;

    Ok((keypair.private, keypair.public))
}

fn build_tls_config(cert_pem: &str, key_pem: &str) -> Result<ServerConfig, String> {
    let mut cert_reader = std::io::BufReader::new(cert_pem.as_bytes());
    let certs = rustls_pemfile::certs(&mut cert_reader)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("Failed to parse cert PEM: {}", e))?;

    let mut key_reader = std::io::BufReader::new(key_pem.as_bytes());
    let key = rustls_pemfile::private_key(&mut key_reader)
        .map_err(|e| format!("Failed to parse key PEM: {}", e))?
        .ok_or("No private key found in PEM")?;

    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| format!("TLS config error: {}", e))?;

    Ok(config)
}

/// Check if an IP address is loopback, including IPv4-mapped IPv6 addresses
/// like `::ffff:127.0.0.1` which Rust's `is_loopback()` does not cover.
fn is_loopback_addr(ip: &std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => v4.is_loopback(),
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback() || matches!(v6.to_ipv4_mapped(), Some(v4) if v4.is_loopback())
        }
    }
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn required_permission(request: &RemoteRequest) -> Option<&'static str> {
    match request {
        RemoteRequest::Auth { .. }
        | RemoteRequest::NoiseInit { .. }
        | RemoteRequest::NoiseHandshake { .. } => None,
        RemoteRequest::GetFileInfo { .. }
        | RemoteRequest::ReadDir { .. }
        | RemoteRequest::ReadFile { .. }
        | RemoteRequest::WriteFile { .. }
        | RemoteRequest::CreateDir { .. }
        | RemoteRequest::Delete { .. }
        | RemoteRequest::Rename { .. }
        | RemoteRequest::HomeDir
        | RemoteRequest::PatchFile { .. } => Some("filesystem"),
        RemoteRequest::TermCreate { .. }
        | RemoteRequest::TermWrite { .. }
        | RemoteRequest::TermResize { .. }
        | RemoteRequest::TermClose { .. } => Some("terminal"),
        RemoteRequest::Ping
        | RemoteRequest::GetState
        | RemoteRequest::OpenFile { .. }
        | RemoteRequest::SubscribeWorkspace
        | RemoteRequest::Heartbeat
        | RemoteRequest::TauriEmitEvent { .. } => Some("workspace"),
        RemoteRequest::LlmGetHardwareInfo
        | RemoteRequest::LlmScanLocalModels
        | RemoteRequest::LlmGetServerStatus
        | RemoteRequest::LlmCheckEngine { .. }
        | RemoteRequest::LlmListInstalledEngines
        | RemoteRequest::LlmDetectRecommendedBackend
        | RemoteRequest::LlmLaunchServer { .. }
        | RemoteRequest::LlmStopServer
        | RemoteRequest::LlmGetNetworkInfo { .. }
        | RemoteRequest::ChatGetSessions
        | RemoteRequest::ChatSaveSessions { .. }
        | RemoteRequest::FerrumListSessions
        | RemoteRequest::FerrumGetLatestSession
        | RemoteRequest::FerrumCreateSession { .. }
        | RemoteRequest::FerrumLoadMessages { .. }
        | RemoteRequest::FerrumSaveMessage { .. }
        | RemoteRequest::FerrumDeleteSession { .. }
        | RemoteRequest::FerrumRenameSession { .. }
        | RemoteRequest::FerrumGetProfiles
        | RemoteRequest::FerrumCheckProvider { .. }
        | RemoteRequest::FerrumListProviderModels { .. }
        | RemoteRequest::FerrumGetTokenCount { .. }
        | RemoteRequest::FerrumExportSession { .. } => Some("llm"),
        RemoteRequest::TauriInvoke { .. }
        | RemoteRequest::AgentConnect { .. }
        | RemoteRequest::AgentMessage { .. } => Some("agent"),
    }
}

async fn handle_client<S>(
    ws_stream: tokio_tungstenite::WebSocketStream<S>,
    peer_addr: SocketAddr,
    state: Arc<RemoteServerState>,
    terminal_manager: Arc<TerminalManager>,
    app: AppHandle,
    noise_keypair: Option<(Vec<u8>, Vec<u8>)>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let client_id = uuid::Uuid::new_v4().to_string();
    let client = ConnectedClient {
        id: client_id.clone(),
        addr: peer_addr.to_string(),
        connected_at: now_ts(),
        last_activity: now_ts(),
    };

    // Add to connected clients
    if let Ok(mut clients) = state.connected_clients.lock() {
        clients.push(client.clone());
    }
    let _ = app.emit("remote-client-connected", &client);
    state.recorder.record_value(
        "system",
        Some(&client_id),
        None,
        json!({
            "type": "client.connected",
            "client_id": client_id.clone(),
            "address": peer_addr.to_string(),
        }),
    );

    let (ws_write, mut ws_read) = ws_stream.split();

    // Channel for outgoing WebSocket messages (allows concurrent terminal output + responses)
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Message>();

    // Writer task: reads from channel and writes to WebSocket
    let writer_task = tokio::spawn(async move {
        let mut ws_write = ws_write;
        while let Some(msg) = out_rx.recv().await {
            if ws_write.send(msg).await.is_err() {
                break;
            }
        }
    });

    let sender = SecureSender::new(out_tx.clone(), state.recorder.clone(), client.id.clone());

    let mut authenticated = false;
    let mut client_permissions = std::collections::HashSet::<String>::new();
    let mut msg_count: u32 = 0;
    let mut window_start = std::time::Instant::now();

    // Noise handshake state (XX pattern, responder side)
    let mut noise_handshake: Option<snow::HandshakeState> = None;

    // Track remote terminal IDs for cleanup on disconnect
    let remote_terminals: Arc<std::sync::Mutex<Vec<String>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));

    // Track this client's persistent agent session
    let mut client_agent_session: Option<std::sync::Arc<crate::session::Session>> = None;

    loop {
        // Enforce session timeout: disconnect idle clients
        let timeout_secs = state
            .session_timeout_secs
            .lock()
            .map(|t| *t)
            .unwrap_or(1800);
        let timeout_duration = std::time::Duration::from_secs(timeout_secs);
        let msg = match tokio::time::timeout(timeout_duration, ws_read.next()).await {
            Ok(Some(msg)) => msg,
            Ok(None) => break, // stream ended
            Err(_) => {
                log::info!(
                    "[remote] Client {} timed out after {}s of inactivity",
                    peer_addr,
                    timeout_secs
                );
                let _ = out_tx.send(Message::Close(None));
                break;
            }
        };
        // Rate limiting: max 100 msgs/sec
        if window_start.elapsed() > std::time::Duration::from_secs(1) {
            msg_count = 0;
            window_start = std::time::Instant::now();
        }
        msg_count += 1;
        if msg_count > 100 {
            sender.send_json(&RemoteResponse::err("Rate limit exceeded".to_string()));
            continue;
        }

        // Update last_activity
        if let Ok(mut clients) = state.connected_clients.lock() {
            if let Some(c) = clients.iter_mut().find(|c| c.id == client_id) {
                c.last_activity = now_ts();
            }
        }

        let text = match msg {
            Ok(Message::Text(text)) => text,
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(data)) => {
                let _ = out_tx.send(Message::Pong(data));
                continue;
            }
            Ok(_) => continue,
            Err(e) => {
                log::warn!("WebSocket error from {}: {}", peer_addr, e);
                break;
            }
        };

        // Decrypt if Noise tunnel is active
        let decrypted = if sender.has_noise() {
            match sender.decrypt_text(&text) {
                Some(d) => d,
                None => text.to_string(), // fallback for non-encrypted messages
            }
        } else {
            text.to_string()
        };

        // Extract optional request id for mobile bridge promise resolution
        let request_id: Option<u64> = serde_json::from_str::<serde_json::Value>(&decrypted)
            .ok()
            .and_then(|v| v.get("id").and_then(|id| id.as_u64()));
        state
            .recorder
            .record_text("inbound", Some(&client_id), request_id, &decrypted);

        let request: RemoteRequest = match serde_json::from_str(&decrypted) {
            Ok(r) => r,
            Err(e) => {
                sender.send_json_with_id(
                    &RemoteResponse::err_with_code(
                        format!("Invalid request: {}", e),
                        ErrorCode::InvalidRequest,
                    ),
                    request_id,
                );
                continue;
            }
        };

        // Require authentication before any other operation
        if !authenticated {
            match &request {
                RemoteRequest::Auth { token, device_name } => {
                    // Verify token against stored hashed tokens
                    let no_devices_yet = state
                        .pairing
                        .list_paired_devices()
                        .map(|d| d.is_empty())
                        .unwrap_or(true);

                    let is_local = is_loopback_addr(&peer_addr.ip());

                    if let Ok(Some(device)) = state.pairing.get_paired_device_by_token(token) {
                        authenticated = true;
                        client_permissions = device.permissions.into_iter().collect();
                        sender.send_json_with_id(
                            &RemoteResponse::AuthOk {
                                device_id: client_id.clone(),
                                protocol_version: PROTOCOL_VERSION,
                            },
                            request_id,
                        );
                    } else if no_devices_yet || is_local {
                        // Auto-pair: first device (TOFU) or localhost connections (CLI)
                        // Trust-on-first-use: auto-pair the first device
                        let default_permissions = crate::pairing::default_remote_permissions();
                        if let Ok(device) = state
                            .pairing
                            .add_paired_device(device_name.clone(), token.clone())
                        {
                            client_permissions = device.permissions.into_iter().collect();
                        } else {
                            client_permissions = default_permissions.into_iter().collect();
                        }
                        authenticated = true;
                        let reason = if is_local { "localhost" } else { "TOFU" };
                        log::info!("[remote] Device '{}' auto-paired ({})", device_name, reason);
                        sender.send_json_with_id(
                            &RemoteResponse::AuthOk {
                                device_id: client_id.clone(),
                                protocol_version: PROTOCOL_VERSION,
                            },
                            request_id,
                        );
                    } else {
                        // Unknown device — require user confirmation before pairing
                        log::info!(
                            "[remote] New device pairing request from '{}' at {}",
                            device_name,
                            peer_addr
                        );
                        let confirm_id = uuid::Uuid::new_v4().to_string();
                        let _ = app.emit(
                            "remote-pairing-request",
                            serde_json::json!({
                                "confirm_id": confirm_id,
                                "device_name": device_name,
                                "address": peer_addr.to_string(),
                            }),
                        );

                        // Wait for user response (approve/deny) with 60s timeout
                        let (pair_tx, pair_rx) = std::sync::mpsc::channel::<bool>();
                        let event_name = format!("remote-pairing-response-{}", confirm_id);
                        let pair_tx_clone = pair_tx.clone();
                        let listener_id = app.listen(&event_name, move |event| {
                            let approved =
                                serde_json::from_str::<serde_json::Value>(event.payload())
                                    .ok()
                                    .and_then(|v| v["approved"].as_bool())
                                    .unwrap_or(false);
                            let _ = pair_tx_clone.send(approved);
                        });

                        let approved = pair_rx
                            .recv_timeout(std::time::Duration::from_secs(60))
                            .unwrap_or(false);
                        app.unlisten(listener_id);

                        if approved {
                            let default_permissions = crate::pairing::default_remote_permissions();
                            if let Ok(device) = state
                                .pairing
                                .add_paired_device(device_name.clone(), token.clone())
                            {
                                client_permissions = device.permissions.into_iter().collect();
                            } else {
                                client_permissions = default_permissions.into_iter().collect();
                            }
                            authenticated = true;
                            log::info!("[remote] Device '{}' approved and paired", device_name);
                            sender.send_json_with_id(
                                &RemoteResponse::AuthOk {
                                    device_id: client_id.clone(),
                                    protocol_version: PROTOCOL_VERSION,
                                },
                                request_id,
                            );
                        } else {
                            log::warn!(
                                "[remote] Device '{}' pairing denied or timed out",
                                device_name
                            );
                            sender.send_json_with_id(
                                &RemoteResponse::AuthError {
                                    message: "Pairing request denied by user.".to_string(),
                                },
                                request_id,
                            );
                            break; // Disconnect the client
                        }
                    }
                    continue;
                }
                _ => {
                    sender.send_json_with_id(
                        &RemoteResponse::AuthError {
                            message: "Not authenticated. Send auth first.".to_string(),
                        },
                        request_id,
                    );
                    continue;
                }
            }
        }

        if let Some(permission) = required_permission(&request) {
            if !client_permissions.contains(permission) {
                sender.send_json_with_id(
                    &RemoteResponse::err_with_code(
                        format!(
                            "Remote permission '{}' is not granted for this device",
                            permission
                        ),
                        ErrorCode::PermissionDenied,
                    ),
                    request_id,
                );
                continue;
            }
        }

        // Dispatch authenticated requests — use send_json_with_id to echo
        // the request id back for mobile bridge promise resolution.
        match request {
            RemoteRequest::Auth { .. } => {
                sender.send_json_with_id(
                    &RemoteResponse::err("Already authenticated".to_string()),
                    request_id,
                );
            }

            // ---- Noise protocol handshake ----
            RemoteRequest::NoiseInit { data } => {
                if let Some(ref kp) = noise_keypair {
                    match handle_noise_init(&kp.0, &data) {
                        Ok((hs, response_data)) => {
                            noise_handshake = Some(hs);
                            sender.send_json_with_id(
                                &RemoteResponse::NoiseHandshakeResp {
                                    data: response_data,
                                },
                                request_id,
                            );
                        }
                        Err(e) => {
                            sender.send_json_with_id(
                                &RemoteResponse::err(format!("Noise init failed: {}", e)),
                                request_id,
                            );
                        }
                    }
                } else {
                    sender.send_json_with_id(
                        &RemoteResponse::err_with_code(
                            "Noise not available on server".to_string(),
                            ErrorCode::Unsupported,
                        ),
                        request_id,
                    );
                }
            }

            RemoteRequest::NoiseHandshake { data } => {
                if let Some(hs) = noise_handshake.take() {
                    match handle_noise_finalize(hs, &data) {
                        Ok(transport) => {
                            sender.set_noise(transport);
                            sender.send_json_with_id(&RemoteResponse::NoiseReady, request_id);
                            log::info!("Noise tunnel established with {}", peer_addr);
                        }
                        Err(e) => {
                            sender.send_json_with_id(
                                &RemoteResponse::err(format!("Noise handshake failed: {}", e)),
                                request_id,
                            );
                        }
                    }
                } else {
                    sender.send_json_with_id(
                        &RemoteResponse::err("No pending noise handshake".to_string()),
                        request_id,
                    );
                }
            }

            // ---- Terminal operations (forwarded via TerminalManager) ----
            RemoteRequest::TermCreate {
                id,
                rows,
                cols,
                cwd,
            } => {
                match terminal_manager.create_pty(id.clone(), rows, cols, cwd, None) {
                    Ok(mut reader) => {
                        if let Ok(mut terms) = remote_terminals.lock() {
                            terms.push(id.clone());
                        }
                        // Spawn reader thread: forwards PTY output to WebSocket via channel
                        // Terminal streaming uses send_json (no id) since these are push events
                        let term_id = id;
                        let term_sender = sender.clone();
                        std::thread::spawn(move || {
                            let mut buf = [0u8; 4096];
                            loop {
                                match reader.read(&mut buf) {
                                    Ok(0) => {
                                        term_sender.send_json(&RemoteResponse::TermExit {
                                            id: term_id.clone(),
                                        });
                                        break;
                                    }
                                    Ok(n) => {
                                        let data = String::from_utf8_lossy(&buf[..n]).to_string();
                                        term_sender.send_json(&RemoteResponse::TermOutput {
                                            id: term_id.clone(),
                                            data,
                                        });
                                    }
                                    Err(_) => {
                                        term_sender.send_json(&RemoteResponse::TermExit {
                                            id: term_id.clone(),
                                        });
                                        break;
                                    }
                                }
                            }
                        });
                        sender.send_json_with_id(&RemoteResponse::TermOk, request_id);
                    }
                    Err(e) => {
                        sender.send_json_with_id(&RemoteResponse::err(e), request_id);
                    }
                }
            }

            RemoteRequest::TermWrite { id, data } => {
                let resp = match terminal_manager.write_pty(&id, data.as_bytes()) {
                    Ok(()) => RemoteResponse::TermOk,
                    Err(e) => RemoteResponse::err(e),
                };
                sender.send_json_with_id(&resp, request_id);
            }

            RemoteRequest::TermResize { id, rows, cols } => {
                let resp = match terminal_manager.resize_pty(&id, rows, cols) {
                    Ok(()) => RemoteResponse::TermOk,
                    Err(e) => RemoteResponse::err(e),
                };
                sender.send_json_with_id(&resp, request_id);
            }

            RemoteRequest::TermClose { id } => {
                let resp = match terminal_manager.close_pty(&id) {
                    Ok(()) => RemoteResponse::TermOk,
                    Err(e) => RemoteResponse::err(e),
                };
                if let Ok(mut terms) = remote_terminals.lock() {
                    terms.retain(|t| t != &id);
                }
                sender.send_json_with_id(&resp, request_id);
            }

            // ---- State sync ----
            RemoteRequest::GetState => {
                let resp = if let Ok(ide_state) = state.ide_state.lock() {
                    RemoteResponse::SyncState {
                        open_files: ide_state.open_files.clone(),
                        active_file: ide_state.active_file.clone(),
                        cursor_line: ide_state.cursor_line,
                        cursor_column: ide_state.cursor_column,
                        project_root: ide_state.project_root.clone(),
                    }
                } else {
                    RemoteResponse::err_with_code(
                        "Failed to read IDE state".to_string(),
                        ErrorCode::InternalError,
                    )
                };
                sender.send_json_with_id(&resp, request_id);
            }

            RemoteRequest::OpenFile { path } => {
                let _ = app.emit("remote-open-file", &path);
                sender.send_json_with_id(&RemoteResponse::FileOpened { path }, request_id);
            }

            // ---- File system operations ----
            RemoteRequest::ReadDir { path } => {
                let resp = match fs_commands::read_directory(path.clone(), None) {
                    Ok(entries) => RemoteResponse::DirEntries {
                        request_path: path,
                        entries,
                    },
                    Err(e) => RemoteResponse::err(e),
                };
                sender.send_json_with_id(&resp, request_id);
            }

            RemoteRequest::GetFileInfo { path } => {
                let resp = match fs_commands::get_file_info(path) {
                    Ok(info) => RemoteResponse::FileInfoResult {
                        size: info.size,
                        is_binary: info.is_binary,
                        line_count: info.line_count,
                    },
                    Err(e) => RemoteResponse::err(e),
                };
                sender.send_json_with_id(&resp, request_id);
            }

            RemoteRequest::ReadFile { path } => {
                let resp = match fs_commands::read_file_content(path.clone()) {
                    Ok(content) => RemoteResponse::FileContent { path, content },
                    Err(e) => RemoteResponse::err(e),
                };
                sender.send_json_with_id(&resp, request_id);
            }

            RemoteRequest::WriteFile { path, content } => {
                // Conflict detection: check if file was modified since client last read it
                // The client can optionally include expected_mtime to detect concurrent edits
                let resp = match fs_commands::write_file_content(path, content) {
                    Ok(()) => RemoteResponse::FsOk,
                    Err(e) => RemoteResponse::err(e),
                };
                sender.send_json_with_id(&resp, request_id);
                // Notify other connected clients about the file change
                let _ = app.emit("workspace-file-saved", serde_json::json!({}));
            }

            RemoteRequest::PatchFile {
                path,
                start_line,
                end_line,
                new_content,
            } => {
                match fs_commands::patch_file_lines(
                    path.clone(),
                    start_line,
                    end_line,
                    &new_content,
                ) {
                    Ok(()) => {
                        sender.send_json_with_id(&RemoteResponse::FsOk, request_id);
                        let _ =
                            app.emit("workspace-file-saved", serde_json::json!({ "path": path }));
                    }
                    Err(e) => {
                        sender.send_json_with_id(&RemoteResponse::err(e), request_id);
                    }
                }
            }

            RemoteRequest::CreateDir { path } => {
                let resp = match fs_commands::create_directory(path) {
                    Ok(()) => RemoteResponse::FsOk,
                    Err(e) => RemoteResponse::err(e),
                };
                sender.send_json_with_id(&resp, request_id);
            }

            RemoteRequest::Delete { path } => {
                let resp = match fs_commands::delete_entry(path) {
                    Ok(()) => RemoteResponse::FsOk,
                    Err(e) => RemoteResponse::err(e),
                };
                sender.send_json_with_id(&resp, request_id);
            }

            RemoteRequest::Rename { old_path, new_path } => {
                let resp = match fs_commands::rename_entry(old_path, new_path) {
                    Ok(()) => RemoteResponse::FsOk,
                    Err(e) => RemoteResponse::err(e),
                };
                sender.send_json_with_id(&resp, request_id);
            }

            RemoteRequest::HomeDir => {
                let resp = match fs_commands::get_home_dir() {
                    Ok(path) => RemoteResponse::HomeDirResult { path },
                    Err(e) => RemoteResponse::err(e),
                };
                sender.send_json_with_id(&resp, request_id);
            }

            RemoteRequest::Ping => {
                sender.send_json_with_id(
                    &RemoteResponse::Pong {
                        timestamp: now_ts(),
                    },
                    request_id,
                );
            }

            // ---- LLM Forwarding ----
            RemoteRequest::LlmGetHardwareInfo => {
                let resp = match crate::llm_loader::detect_hardware() {
                    Ok(info) => RemoteResponse::LlmHardwareInfo { info },
                    Err(e) => RemoteResponse::err(e.to_string()),
                };
                sender.send_json_with_id(&resp, request_id);
            }
            RemoteRequest::LlmScanLocalModels => {
                let resp = match crate::model_scanner::scan_local_models("".to_string()) {
                    Ok(models) => RemoteResponse::LlmLocalModels { models },
                    Err(e) => RemoteResponse::err(e.to_string()),
                };
                sender.send_json_with_id(&resp, request_id);
            }
            RemoteRequest::LlmGetServerStatus => {
                let llm_state = app.state::<crate::llm_loader::LlmServerState>();
                let resp = match crate::llm_loader::get_llm_server_status(llm_state) {
                    Ok(status) => RemoteResponse::LlmServerStatus { status },
                    Err(e) => RemoteResponse::err(e.to_string()),
                };
                sender.send_json_with_id(&resp, request_id);
            }
            RemoteRequest::LlmCheckEngine { backend } => {
                let resp = match crate::llm_loader::check_engine(backend) {
                    Ok(info) => RemoteResponse::LlmEngineInfo { info },
                    Err(e) => RemoteResponse::err(e.to_string()),
                };
                sender.send_json_with_id(&resp, request_id);
            }
            RemoteRequest::LlmListInstalledEngines => {
                let resp = match crate::llm_loader::list_installed_engines() {
                    Ok(engines) => RemoteResponse::LlmInstalledEngines { engines },
                    Err(e) => RemoteResponse::err(e.to_string()),
                };
                sender.send_json_with_id(&resp, request_id);
            }
            RemoteRequest::LlmDetectRecommendedBackend => {
                let resp = match crate::llm_loader::detect_recommended_backend() {
                    Ok(backend) => RemoteResponse::LlmRecommendedBackend { backend },
                    Err(e) => RemoteResponse::err(e.to_string()),
                };
                sender.send_json_with_id(&resp, request_id);
            }
            RemoteRequest::LlmLaunchServer {
                model_path,
                config,
                port,
                backend,
            } => {
                let llm_state = app.state::<crate::llm_loader::LlmServerState>();
                let result = crate::llm_loader::launch_llm_server(
                    model_path,
                    config,
                    port,
                    None,
                    backend,
                    app.clone(),
                    llm_state,
                );
                match result {
                    Ok(_) => sender.send_json_with_id(&RemoteResponse::LlmOk, request_id),
                    Err(e) => {
                        sender.send_json_with_id(&RemoteResponse::err(e.to_string()), request_id)
                    }
                }
            }
            RemoteRequest::LlmStopServer => {
                let llm_state = app.state::<crate::llm_loader::LlmServerState>();
                let resp = match crate::llm_loader::stop_llm_server(llm_state) {
                    Ok(_) => RemoteResponse::LlmOk,
                    Err(e) => RemoteResponse::err(e.to_string()),
                };
                sender.send_json_with_id(&resp, request_id);
            }
            RemoteRequest::LlmGetNetworkInfo { port } => {
                let resp = match crate::llm_loader::get_llm_network_info(port) {
                    Ok(info) => RemoteResponse::LlmServerStatus { status: info },
                    Err(e) => RemoteResponse::err(e.to_string()),
                };
                sender.send_json_with_id(&resp, request_id);
            }
            RemoteRequest::ChatGetSessions => {
                let sessions_json = crate::ai_bridge::chat_load_sessions_raw();
                sender
                    .send_json_with_id(&RemoteResponse::ChatSessions { sessions_json }, request_id);
            }
            RemoteRequest::ChatSaveSessions { sessions_json } => {
                let resp = match crate::ai_bridge::chat_save_sessions(sessions_json) {
                    Ok(_) => RemoteResponse::ChatOk,
                    Err(e) => RemoteResponse::err(e.to_string()),
                };
                sender.send_json_with_id(&resp, request_id);
            }

            // ---- FerrumChat remote commands ----
            RemoteRequest::FerrumListSessions => {
                let ferrum_state = app.state::<crate::ferrum_bridge::FerrumState>();
                let resp = match crate::ferrum_bridge::ferrum_list_sessions(ferrum_state) {
                    Ok(sessions) => {
                        let val = serde_json::to_value(&sessions)
                            .unwrap_or(serde_json::Value::Array(vec![]));
                        RemoteResponse::FerrumSessions { sessions: val }
                    }
                    Err(e) => RemoteResponse::err(e),
                };
                sender.send_json_with_id(&resp, request_id);
            }
            RemoteRequest::FerrumGetLatestSession => {
                let ferrum_state = app.state::<crate::ferrum_bridge::FerrumState>();
                let resp = match crate::ferrum_bridge::ferrum_get_latest_session(ferrum_state) {
                    Ok(session) => {
                        let val = serde_json::to_value(&session).unwrap_or(serde_json::Value::Null);
                        RemoteResponse::FerrumSession { session: val }
                    }
                    Err(e) => RemoteResponse::err(e),
                };
                sender.send_json_with_id(&resp, request_id);
            }
            RemoteRequest::FerrumCreateSession { name, profile } => {
                let ferrum_state = app.state::<crate::ferrum_bridge::FerrumState>();
                let resp = match crate::ferrum_bridge::ferrum_create_session(
                    name,
                    profile,
                    ferrum_state,
                ) {
                    Ok(session) => {
                        let val = serde_json::to_value(&session).unwrap_or(serde_json::Value::Null);
                        RemoteResponse::FerrumSession { session: val }
                    }
                    Err(e) => RemoteResponse::err(e),
                };
                sender.send_json_with_id(&resp, request_id);
            }
            RemoteRequest::FerrumLoadMessages { session_id } => {
                let ferrum_state = app.state::<crate::ferrum_bridge::FerrumState>();
                let resp =
                    match crate::ferrum_bridge::ferrum_load_messages(session_id, ferrum_state) {
                        Ok(messages) => {
                            let val = serde_json::to_value(&messages)
                                .unwrap_or(serde_json::Value::Array(vec![]));
                            RemoteResponse::FerrumMessages { messages: val }
                        }
                        Err(e) => RemoteResponse::err(e),
                    };
                sender.send_json_with_id(&resp, request_id);
            }
            RemoteRequest::FerrumSaveMessage {
                session_id,
                message,
            } => {
                let ferrum_state = app.state::<crate::ferrum_bridge::FerrumState>();
                let msg_clone = message.clone();
                let resp = match serde_json::from_value::<ferrum_core::types::Message>(message) {
                    Ok(msg) => match crate::ferrum_bridge::ferrum_save_message(
                        session_id.clone(),
                        msg,
                        ferrum_state,
                    ) {
                        Ok(()) => {
                            // Emit event so PC's FerrumChat can refresh
                            let _ = app.emit(
                                "ferrum-message-saved",
                                serde_json::json!({
                                    "session_id": session_id,
                                    "message": msg_clone,
                                }),
                            );
                            RemoteResponse::FerrumOk
                        }
                        Err(e) => RemoteResponse::err(e),
                    },
                    Err(e) => RemoteResponse::err_with_code(
                        format!("Invalid message format: {}", e),
                        ErrorCode::InvalidRequest,
                    ),
                };
                sender.send_json_with_id(&resp, request_id);
            }
            RemoteRequest::FerrumDeleteSession { session_id } => {
                let ferrum_state = app.state::<crate::ferrum_bridge::FerrumState>();
                let resp =
                    match crate::ferrum_bridge::ferrum_delete_session(session_id, ferrum_state) {
                        Ok(()) => RemoteResponse::FerrumOk,
                        Err(e) => RemoteResponse::err(e),
                    };
                sender.send_json_with_id(&resp, request_id);
            }
            RemoteRequest::FerrumRenameSession {
                session_id,
                new_name,
            } => {
                let ferrum_state = app.state::<crate::ferrum_bridge::FerrumState>();
                let resp = match crate::ferrum_bridge::ferrum_rename_session(
                    session_id,
                    new_name,
                    ferrum_state,
                ) {
                    Ok(()) => RemoteResponse::FerrumOk,
                    Err(e) => RemoteResponse::err(e),
                };
                sender.send_json_with_id(&resp, request_id);
            }
            RemoteRequest::FerrumGetProfiles => {
                let resp = match crate::ferrum_bridge::ferrum_get_profiles() {
                    Ok(profiles) => {
                        let val = serde_json::to_value(&profiles)
                            .unwrap_or(serde_json::Value::Array(vec![]));
                        RemoteResponse::FerrumProfiles { profiles: val }
                    }
                    Err(e) => RemoteResponse::err(e),
                };
                sender.send_json_with_id(&resp, request_id);
            }
            RemoteRequest::FerrumCheckProvider { base_url } => {
                let resp = match crate::ferrum_bridge::ferrum_check_provider(base_url).await {
                    Ok(connected) => RemoteResponse::FerrumProviderCheck { connected },
                    Err(e) => RemoteResponse::err(e),
                };
                sender.send_json_with_id(&resp, request_id);
            }
            RemoteRequest::FerrumListProviderModels { base_url } => {
                let resp = match crate::ferrum_bridge::ferrum_list_provider_models(base_url).await {
                    Ok(result) => RemoteResponse::FerrumProviderModels {
                        models: result.models,
                        connected: result.connected,
                    },
                    Err(e) => RemoteResponse::err(e),
                };
                sender.send_json_with_id(&resp, request_id);
            }
            RemoteRequest::FerrumGetTokenCount { session_id } => {
                let ferrum_state = app.state::<crate::ferrum_bridge::FerrumState>();
                let resp = match crate::ferrum_bridge::ferrum_get_session_token_count(
                    session_id,
                    ferrum_state,
                ) {
                    Ok(count) => RemoteResponse::FerrumTokenCount { count },
                    Err(e) => RemoteResponse::err(e),
                };
                sender.send_json_with_id(&resp, request_id);
            }
            RemoteRequest::FerrumExportSession { session_id } => {
                let ferrum_state = app.state::<crate::ferrum_bridge::FerrumState>();
                let resp =
                    match crate::ferrum_bridge::ferrum_export_session(session_id, ferrum_state) {
                        Ok(markdown) => RemoteResponse::FerrumExport { markdown },
                        Err(e) => RemoteResponse::err(e),
                    };
                sender.send_json_with_id(&resp, request_id);
            }

            // ---- Workspace subscription ----
            RemoteRequest::SubscribeWorkspace => {
                // Subscribe to IDE workspace events and forward them to the client
                let ws_sender = sender.clone();
                let ws_app = app.clone();

                // Forward file-opened events
                let s1 = ws_sender.clone();
                ws_app.listen("workspace-file-opened", move |event| {
                    let payload =
                        serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
                    s1.send_json(&RemoteResponse::WorkspaceEvent {
                        event: "fileOpened".to_string(),
                        payload,
                    });
                });

                // Forward file-closed events
                let s2 = ws_sender.clone();
                ws_app.listen("workspace-file-closed", move |event| {
                    let payload =
                        serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
                    s2.send_json(&RemoteResponse::WorkspaceEvent {
                        event: "fileClosed".to_string(),
                        payload,
                    });
                });

                // Forward file-saved events
                let s3 = ws_sender.clone();
                ws_app.listen("workspace-file-saved", move |event| {
                    let payload =
                        serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
                    s3.send_json(&RemoteResponse::WorkspaceEvent {
                        event: "fileSaved".to_string(),
                        payload,
                    });
                });

                // Forward cursor-moved events
                let s4 = ws_sender.clone();
                ws_app.listen("workspace-cursor-moved", move |event| {
                    let payload =
                        serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
                    s4.send_json(&RemoteResponse::WorkspaceEvent {
                        event: "cursorMoved".to_string(),
                        payload,
                    });
                });

                // Forward file-changed events (content updates)
                let s5 = ws_sender.clone();
                ws_app.listen("workspace-file-changed", move |event| {
                    let payload =
                        serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
                    s5.send_json(&RemoteResponse::WorkspaceEvent {
                        event: "fileChanged".to_string(),
                        payload,
                    });
                });

                // Forward AI completion notifications to mobile
                let s6 = ws_sender.clone();
                ws_app.listen("ai-chat-complete-notify", move |event| {
                    let payload =
                        serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
                    s6.send_json(&RemoteResponse::TauriEvent {
                        event: "ai-chat-complete-notify".to_string(),
                        payload,
                    });
                });

                // Forward ALL AI streaming events to mobile via the relay channel.
                // ai_bridge emits "ai-remote-relay" with {event, payload} for every AI event.
                let s_ai = ws_sender.clone();
                ws_app.listen("ai-remote-relay", move |event| {
                    if let Ok(relay) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                        let event_name = relay["event"].as_str().unwrap_or("unknown").to_string();
                        let payload = relay["payload"].clone();
                        s_ai.send_json(&RemoteResponse::TauriEvent {
                            event: event_name,
                            payload,
                        });
                    }
                });

                // Forward LLM server started to mobile
                let s_llm = ws_sender.clone();
                ws_app.listen("llm-server-started", move |event| {
                    let payload =
                        serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
                    s_llm.send_json(&RemoteResponse::TauriEvent {
                        event: "llm-server-started".to_string(),
                        payload,
                    });
                });

                // Forward LLM server stopped to mobile
                let s_llm2 = ws_sender.clone();
                ws_app.listen("llm-server-stopped", move |event| {
                    let payload =
                        serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
                    s_llm2.send_json(&RemoteResponse::TauriEvent {
                        event: "llm-server-stopped".to_string(),
                        payload,
                    });
                });

                // Forward workspace filesystem changes to mobile
                let s7 = ws_sender.clone();
                ws_app.listen("workspace-fs-changed", move |event| {
                    let payload =
                        serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
                    s7.send_json(&RemoteResponse::TauriEvent {
                        event: "workspace-fs-changed".to_string(),
                        payload,
                    });
                });

                // Forward ferrum message saved events to mobile
                let s8 = ws_sender.clone();
                ws_app.listen("ferrum-message-saved", move |event| {
                    let payload =
                        serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
                    s8.send_json(&RemoteResponse::TauriEvent {
                        event: "ferrum-message-saved".to_string(),
                        payload,
                    });
                });

                let s9 = ws_sender.clone();
                ws_app.listen("workspace-fs-changed", move |event| {
                    let payload =
                        serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
                    s9.send_json(&RemoteResponse::TauriEvent {
                        event: "workspace-fs-changed".to_string(),
                        payload,
                    });
                });

                let s10 = ws_sender.clone();
                ws_app.listen("collab-document-state", move |event| {
                    let payload =
                        serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
                    s10.send_json(&RemoteResponse::TauriEvent {
                        event: "collab-document-state".to_string(),
                        payload,
                    });
                });

                let s11 = ws_sender.clone();
                ws_app.listen("collab-call-signal", move |event| {
                    let payload =
                        serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
                    s11.send_json(&RemoteResponse::TauriEvent {
                        event: "collab-call-signal".to_string(),
                        payload,
                    });
                });

                let s12 = ws_sender.clone();
                ws_app.listen("terminal-share-state", move |event| {
                    let payload =
                        serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
                    s12.send_json(&RemoteResponse::TauriEvent {
                        event: "terminal-share-state".to_string(),
                        payload,
                    });
                });

                let s13 = ws_sender.clone();
                ws_app.listen("terminal-share-output", move |event| {
                    let payload =
                        serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
                    s13.send_json(&RemoteResponse::TauriEvent {
                        event: "terminal-share-output".to_string(),
                        payload,
                    });
                });

                let s14 = ws_sender.clone();
                ws_app.listen("terminal-share-input", move |event| {
                    let payload =
                        serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
                    s14.send_json(&RemoteResponse::TauriEvent {
                        event: "terminal-share-input".to_string(),
                        payload,
                    });
                });

                let s15 = ws_sender.clone();
                ws_app.listen("ble-transfer-progress", move |event| {
                    let payload =
                        serde_json::from_str(event.payload()).unwrap_or(serde_json::Value::Null);
                    s15.send_json(&RemoteResponse::TauriEvent {
                        event: "ble-transfer-progress".to_string(),
                        payload,
                    });
                });

                sender.send_json_with_id(&RemoteResponse::FerrumOk, request_id);
            }

            // ---- Heartbeat ----
            RemoteRequest::Heartbeat => {
                sender.send_json_with_id(
                    &RemoteResponse::HeartbeatAck {
                        timestamp: now_ts(),
                    },
                    request_id,
                );
            }

            RemoteRequest::TauriInvoke { cmd, args } => {
                if cmd == "ai_chat_with_tools" || cmd == "ai_chat_stream" {
                    // Route AI chat through the persistent session queue.
                    // The agent_runner handles it on the PC — events go into
                    // the session ring buffer and survive phone disconnects.
                    let stream_id = args
                        .get("streamId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    let session_mgr = app.state::<std::sync::Arc<crate::manager::SessionManager>>();

                    // Get or create a session for this client (reuse across requests)
                    let session = if let Some(ref s) = client_agent_session {
                        s.clone()
                    } else {
                        let (s, _) = session_mgr.get_or_create(None, &app).await;

                        // Subscribe to the session broadcast and forward events
                        // as TauriEvent format — this is what the JS web app expects.
                        let mut event_rx = s.subscribe();
                        let live_tx = out_tx.clone();
                        tokio::spawn(async move {
                            while let Ok(event) = event_rx.recv().await {
                                // Wrap as TauriEvent — exactly the format the JS expects
                                let resp = RemoteResponse::TauriEvent {
                                    event: event.event_type.clone(),
                                    payload: event.payload.clone(),
                                };
                                let mut wrapper = serde_json::Map::new();
                                if let serde_json::Value::Object(map) =
                                    serde_json::to_value(&resp).unwrap_or(serde_json::Value::Null)
                                {
                                    wrapper = map;
                                }
                                wrapper
                                    .insert("id".to_string(), serde_json::Value::Number(0.into()));
                                let _ = live_tx.send(Message::Text(
                                    serde_json::Value::Object(wrapper).to_string().into(),
                                ));
                            }
                        });

                        client_agent_session = Some(s.clone());
                        s
                    };

                    // Push the AI chat request into the session queue
                    let msg = crate::agent_queue::UserMessage::AiChat {
                        stream_id: stream_id.clone(),
                        args: args.clone(),
                    };
                    match session.send(msg).await {
                        Ok(()) => {
                            sender.send_json_with_id(
                                &RemoteResponse::TauriInvokeResult {
                                    result: serde_json::Value::Null,
                                },
                                request_id,
                            );
                        }
                        Err(e) => {
                            sender.send_json_with_id(&RemoteResponse::err(e), request_id);
                        }
                    }
                } else if cmd == "abort_ai_chat" {
                    let stream_id = args
                        .get("streamId")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let ai_state = app.state::<crate::ai_bridge::AiConfig>();
                    let _ = crate::ai_bridge::abort_ai_chat(stream_id, ai_state);
                    sender.send_json_with_id(
                        &RemoteResponse::TauriInvokeResult {
                            result: serde_json::Value::Null,
                        },
                        request_id,
                    );
                } else {
                    sender.send_json_with_id(
                        &RemoteResponse::err_with_code(
                            "Unsupported Tauri command".to_string(),
                            ErrorCode::Unsupported,
                        ),
                        request_id,
                    );
                }
            }

            RemoteRequest::TauriEmitEvent { event, payload } => {
                // Re-emit the event locally on the app handle so the backend can react
                let _ = app.emit(&event, payload);
                sender.send_json_with_id(&RemoteResponse::FsOk, request_id);
            }

            // ---- Persistent Agent Session ----
            RemoteRequest::AgentConnect {
                session_id,
                last_seq,
            } => {
                let session_mgr = app.state::<std::sync::Arc<crate::manager::SessionManager>>();
                let (session, is_new) = session_mgr.get_or_create(session_id, &app).await;
                let sid = session.id.clone();

                // Replay missed events
                let missed = session.events_since(last_seq);
                if !missed.is_empty() {
                    sender.send_json(&RemoteResponse::AgentReplay { events: missed });
                }

                sender.send_json_with_id(
                    &RemoteResponse::AgentConnected {
                        session_id: sid.clone(),
                        current_seq: session.current_seq(),
                    },
                    request_id,
                );

                // Subscribe to live events and forward them to this WebSocket client
                let mut event_rx = session.subscribe();
                let live_sender = sender.clone();
                let live_sid = sid.clone();
                tokio::spawn(async move {
                    while let Ok(event) = event_rx.recv().await {
                        live_sender.send_json(&RemoteResponse::AgentEventResp {
                            session_id: live_sid.clone(),
                            event,
                        });
                    }
                });

                if is_new {
                    log::info!("New agent session created: {}", sid);
                } else {
                    log::info!(
                        "Reconnected to agent session: {} (replaying from seq {})",
                        sid,
                        last_seq
                    );
                }
            }

            RemoteRequest::AgentMessage {
                session_id,
                message,
            } => {
                let session_mgr = app.state::<std::sync::Arc<crate::manager::SessionManager>>();
                if let Some(session) = session_mgr.get(&session_id) {
                    match session.send(message).await {
                        Ok(()) => {
                            sender.send_json_with_id(&RemoteResponse::AgentQueued, request_id)
                        }
                        Err(e) => sender.send_json_with_id(&RemoteResponse::err(e), request_id),
                    }
                } else {
                    sender.send_json_with_id(
                        &RemoteResponse::err_with_code(
                            format!("No agent session: {}", session_id),
                            ErrorCode::NotFound,
                        ),
                        request_id,
                    );
                }
            }
        }
    }

    // Cleanup: close all remote terminals created by this client
    if let Ok(terms) = remote_terminals.lock() {
        for term_id in terms.iter() {
            let _ = terminal_manager.close_pty(term_id);
        }
    }

    // Drop sender so writer task can finish
    drop(sender);
    drop(out_tx);
    let _ = writer_task.await;

    // Remove from connected clients
    if let Ok(mut clients) = state.connected_clients.lock() {
        clients.retain(|c| c.id != client_id);
    }
    state.recorder.record_value(
        "system",
        Some(&client_id),
        None,
        json!({
            "type": "client.disconnected",
            "client_id": client_id.clone(),
            "address": peer_addr.to_string(),
        }),
    );
    let _ = app.emit("remote-client-disconnected", &client_id);
}

// ===== Noise Protocol Helpers =====

/// Handle Noise XX handshake step 1: receive initiator's first message, send responder's reply.
/// Returns the HandshakeState (for step 3) and the base64-encoded response message.
fn handle_noise_init(
    private_key: &[u8],
    init_data_b64: &str,
) -> Result<(snow::HandshakeState, String), String> {
    let init_msg = base64::engine::general_purpose::STANDARD
        .decode(init_data_b64)
        .map_err(|e| format!("Base64 decode: {}", e))?;

    let params: snow::params::NoiseParams = NOISE_PARAMS
        .parse()
        .map_err(|_| "Invalid noise params".to_string())?;
    let mut responder = snow::Builder::new(params)
        .local_private_key(private_key)
        .build_responder()
        .map_err(|e| format!("Noise responder init: {}", e))?;

    // Read initiator's first message (-> e)
    let mut read_buf = vec![0u8; 65535];
    let _len = responder
        .read_message(&init_msg, &mut read_buf)
        .map_err(|e| format!("Noise read msg1: {}", e))?;

    // Write responder's message (<- e, ee, s, es)
    let mut write_buf = vec![0u8; 65535];
    let len = responder
        .write_message(&[], &mut write_buf)
        .map_err(|e| format!("Noise write msg2: {}", e))?;

    let response_b64 = base64::engine::general_purpose::STANDARD.encode(&write_buf[..len]);

    Ok((responder, response_b64))
}

/// Handle Noise XX handshake step 3: receive initiator's final message, transition to transport.
fn handle_noise_finalize(
    mut handshake: snow::HandshakeState,
    final_data_b64: &str,
) -> Result<snow::TransportState, String> {
    let final_msg = base64::engine::general_purpose::STANDARD
        .decode(final_data_b64)
        .map_err(|e| format!("Base64 decode: {}", e))?;

    // Read initiator's final message (-> s, se)
    let mut read_buf = vec![0u8; 65535];
    let _len = handshake
        .read_message(&final_msg, &mut read_buf)
        .map_err(|e| format!("Noise read msg3: {}", e))?;

    // Transition to transport mode
    handshake
        .into_transport_mode()
        .map_err(|e| format!("Noise transport mode: {}", e))
}
