use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

/// Messages the phone (or any client) can send to the persistent agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum UserMessage {
    /// Full AI chat request — carries all the args needed to call ai_chat_with_tools.
    /// The agent runner processes this on the PC, streaming events into the session
    /// ring buffer. Phone disconnect doesn't affect it.
    #[serde(rename = "chat")]
    Chat { text: String },

    /// Full AI chat with tools — forwarded from TauriInvoke
    #[serde(rename = "ai_chat")]
    AiChat {
        stream_id: String,
        args: serde_json::Value,
    },

    /// Cancel the currently running agent turn
    #[serde(rename = "cancel")]
    Cancel,

    /// Abort a specific AI chat stream
    #[serde(rename = "abort")]
    Abort { stream_id: String },

    /// Switch agent operating mode
    #[serde(rename = "set_mode")]
    SetMode { mode: String },

    /// Trigger RAG indexing
    #[serde(rename = "index_rag")]
    IndexRag,

    /// Switch file context
    #[serde(rename = "switch_file")]
    SwitchFile { path: String },

    /// Accept / apply an AI-generated diff
    #[serde(rename = "apply_diff")]
    ApplyDiff { diff: String },
}

/// Sender half — stored inside the Session so any WebSocket handler can push messages.
pub type AgentQueueTx = mpsc::Sender<UserMessage>;

/// Receiver half — handed to the agent_runner on spawn.
///
/// Wraps `mpsc::Receiver` to add `try_recv()` for non-blocking drain of
/// pending messages (e.g. absorbing "keep going" while the AI was running).
pub struct AgentQueueRx(pub mpsc::Receiver<UserMessage>);

impl AgentQueueRx {
    /// Blocking receive — waits for the next message.
    pub async fn recv(&mut self) -> Option<UserMessage> {
        self.0.recv().await
    }

    /// Non-blocking receive — returns `None` if the queue is empty.
    pub fn try_recv(&mut self) -> Option<UserMessage> {
        self.0.try_recv().ok()
    }
}

/// Channel capacity — matches UPGRADE.md spec (512).
const QUEUE_CAPACITY: usize = 512;

/// Create a new agent message queue.
pub fn agent_queue() -> (AgentQueueTx, AgentQueueRx) {
    let (tx, rx) = mpsc::channel(QUEUE_CAPACITY);
    (tx, AgentQueueRx(rx))
}
