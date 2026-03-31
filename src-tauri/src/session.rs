use crate::agent_queue::{self, AgentQueueRx, AgentQueueTx, UserMessage};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

/// Client connection state — protected by a single Mutex so that
/// `connected_clients` and `last_client_ts` are always read/written atomically.
struct ClientState {
    connected_clients: u64,
    last_client_ts: u64,
}

/// An event emitted by the agent during a turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    /// Monotonically increasing sequence number (per session).
    pub seq: u64,
    /// Event kind — mirrors the Tauri event names the frontend already handles.
    #[serde(rename = "type")]
    pub event_type: String,
    /// Arbitrary JSON payload.
    pub payload: serde_json::Value,
}

/// Ring-buffer capacity for agent events.
const EVENT_BUFFER_SIZE: usize = 500;

/// Broadcast channel capacity for live event streaming to connected clients.
const BROADCAST_CAPACITY: usize = 256;

/// Idle TTL — sessions with no connected clients are removed after 24h.
pub const IDLE_TTL_SECS: u64 = 86400;

/// A persistent agent session.
///
/// The session lives as long as the `SessionManager` keeps it.  Phones
/// connect / disconnect / reconnect — the session (and its agent) never stop.
pub struct Session {
    pub id: String,
    /// Send user messages to the running agent.
    pub message_tx: AgentQueueTx,
    /// Buffered agent events (ring buffer, capped at EVENT_BUFFER_SIZE).
    events: Mutex<VecDeque<AgentEvent>>,
    /// Next sequence number to assign.
    next_seq: AtomicU64,
    /// Broadcast channel for live event streaming to connected clients.
    event_tx: broadcast::Sender<AgentEvent>,
    /// Client connection tracking (count + timestamp, atomically updated together).
    client_state: Mutex<ClientState>,
}

impl Session {
    /// Create a new session.  Returns `(session, queue_rx)` so the caller
    /// can hand `queue_rx` to the agent runner.
    pub fn new(id: String) -> (Arc<Self>, AgentQueueRx) {
        let (tx, rx) = agent_queue::agent_queue();
        let (event_tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let session = Arc::new(Self {
            id,
            message_tx: tx,
            events: Mutex::new(VecDeque::with_capacity(EVENT_BUFFER_SIZE)),
            next_seq: AtomicU64::new(1),
            event_tx,
            client_state: Mutex::new(ClientState {
                connected_clients: 0,
                last_client_ts: now,
            }),
        });
        (session, rx)
    }

    /// Emit an agent event.  Assigns a sequence number, stores it in the
    /// ring buffer, and broadcasts to all connected clients.  Returns the assigned seq.
    pub fn emit(&self, event_type: impl Into<String>, payload: serde_json::Value) -> u64 {
        let seq = self.next_seq.fetch_add(1, Ordering::SeqCst);
        let event = AgentEvent {
            seq,
            event_type: event_type.into(),
            payload,
        };

        // Store in ring buffer
        if let Ok(mut buf) = self.events.lock() {
            if buf.len() >= EVENT_BUFFER_SIZE {
                buf.pop_front();
            }
            buf.push_back(event.clone());
        }

        // Broadcast to connected clients (ignore errors — means no subscribers)
        let _ = self.event_tx.send(event);

        seq
    }

    /// Subscribe to live agent events.  Returns a receiver that will get all
    /// events emitted after this call.
    pub fn subscribe(&self) -> broadcast::Receiver<AgentEvent> {
        self.event_tx.subscribe()
    }

    /// Return all events with `seq > since`.  Used for reconnect replay.
    pub fn events_since(&self, since: u64) -> Vec<AgentEvent> {
        if let Ok(buf) = self.events.lock() {
            buf.iter().filter(|e| e.seq > since).cloned().collect()
        } else {
            Vec::new()
        }
    }

    /// Current highest sequence number.
    pub fn current_seq(&self) -> u64 {
        self.next_seq.load(Ordering::SeqCst).saturating_sub(1)
    }

    /// Push a user message into the agent queue (non-blocking attempt).
    pub async fn send(&self, msg: UserMessage) -> Result<(), String> {
        self.message_tx
            .send(msg)
            .await
            .map_err(|e| format!("Agent queue closed: {}", e))
    }

    /// Track a client connection.
    #[allow(dead_code)]
    pub fn client_connected(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if let Ok(mut state) = self.client_state.lock() {
            state.connected_clients += 1;
            state.last_client_ts = now;
        }
    }

    /// Track a client disconnection.
    #[allow(dead_code)]
    pub fn client_disconnected(&self) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if let Ok(mut state) = self.client_state.lock() {
            state.connected_clients = state.connected_clients.saturating_sub(1);
            state.last_client_ts = now;
        }
    }

    /// Check if session is expired (no clients for IDLE_TTL_SECS).
    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if let Ok(state) = self.client_state.lock() {
            if state.connected_clients > 0 {
                return false;
            }
            now.saturating_sub(state.last_client_ts) > IDLE_TTL_SECS
        } else {
            // Mutex poisoned — treat as expired to allow cleanup
            true
        }
    }
}
