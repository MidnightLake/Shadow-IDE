use crate::agent_runner;
use crate::session::Session;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::AppHandle;

/// Manages all active agent sessions.
///
/// `get_or_create()` is the main entry point — it either returns an existing
/// session or creates a new one **and immediately spawns the agent**.
pub struct SessionManager {
    sessions: Mutex<HashMap<String, Arc<Session>>>,
}

impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }

    /// Get an existing session or create a new one.
    ///
    /// If the session is new, the agent is spawned immediately with access to
    /// the AppHandle (for calling ai_chat_with_tools, accessing Tauri state, etc).
    /// Returns `(session, is_new)`.
    pub async fn get_or_create(
        &self,
        session_id: Option<String>,
        app: &AppHandle,
    ) -> (Arc<Session>, bool) {
        let id = session_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        // Fast path: session already exists
        if let Ok(sessions) = self.sessions.lock() {
            if let Some(session) = sessions.get(&id) {
                return (session.clone(), false);
            }
        }

        // Slow path: create + spawn
        let (session, queue_rx) = Session::new(id.clone());

        // Spawn the persistent agent loop with AppHandle
        agent_runner::spawn_agent(session.clone(), queue_rx, app.clone());

        if let Ok(mut sessions) = self.sessions.lock() {
            sessions.insert(id, session.clone());
        }

        (session, true)
    }

    /// Get a session by ID without creating one.
    pub fn get(&self, session_id: &str) -> Option<Arc<Session>> {
        self.sessions
            .lock()
            .ok()
            .and_then(|s| s.get(session_id).cloned())
    }

    /// Remove a session (stops the agent when the last Arc is dropped).
    pub fn remove(&self, session_id: &str) -> bool {
        self.sessions
            .lock()
            .ok()
            .and_then(|mut s| s.remove(session_id))
            .is_some()
    }

    /// Return IDs of sessions that have been idle longer than IDLE_TTL_SECS.
    pub fn expired_ids(&self) -> Vec<String> {
        if let Ok(sessions) = self.sessions.lock() {
            sessions
                .iter()
                .filter(|(_, s)| s.is_expired())
                .map(|(id, _)| id.clone())
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Number of active sessions.
    pub fn count(&self) -> usize {
        self.sessions.lock().map(|s| s.len()).unwrap_or(0)
    }
}
