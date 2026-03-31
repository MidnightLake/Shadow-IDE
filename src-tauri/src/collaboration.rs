use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Listener, Manager};

#[derive(Default)]
pub struct CollaborationState {
    documents: Mutex<HashMap<String, SharedDocument>>,
}

impl CollaborationState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn snapshot_for(&self, file_path: &str) -> Option<CollaborationSnapshot> {
        let docs = self.documents.lock().ok()?;
        let document = docs.get(file_path)?;
        Some(build_snapshot(document, None))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CollaboratorPresence {
    pub collaborator_id: String,
    pub name: String,
    pub color: String,
    pub file_path: String,
    pub line: u32,
    pub column: u32,
    #[serde(default)]
    pub selection_start: Option<u32>,
    #[serde(default)]
    pub selection_end: Option<u32>,
    #[serde(default)]
    pub voice_active: bool,
    #[serde(default)]
    pub video_active: bool,
    pub last_seen: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewComment {
    pub id: String,
    pub file_path: String,
    pub line: u32,
    pub column: u32,
    pub author: String,
    pub body: String,
    pub created_at: u64,
    #[serde(default)]
    pub resolved: bool,
    #[serde(default)]
    pub resolved_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReviewApproval {
    pub reviewer: String,
    pub status: String,
    #[serde(default)]
    pub note: String,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CollaborationSnapshot {
    pub file_path: String,
    pub content: String,
    pub version: u64,
    pub source_collaborator_id: Option<String>,
    pub presences: Vec<CollaboratorPresence>,
    pub comments: Vec<ReviewComment>,
    pub approvals: Vec<ReviewApproval>,
}

#[derive(Debug, Clone)]
struct TextOperation {
    version: u64,
    offset: usize,
    delete_len: usize,
    insert_len: usize,
}

#[derive(Debug, Clone)]
struct SharedDocument {
    file_path: String,
    content: String,
    version: u64,
    operations: Vec<TextOperation>,
    presences: HashMap<String, CollaboratorPresence>,
    comments: Vec<ReviewComment>,
    approvals: HashMap<String, ReviewApproval>,
}

#[derive(Debug, Clone, Deserialize)]
struct JoinEvent {
    file_path: String,
    collaborator_id: String,
    name: String,
    color: String,
    #[serde(default)]
    content: String,
}

#[derive(Debug, Clone, Deserialize)]
struct LeaveEvent {
    file_path: String,
    collaborator_id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct PresenceEvent {
    file_path: String,
    collaborator_id: String,
    name: String,
    color: String,
    line: u32,
    column: u32,
    #[serde(default)]
    selection_start: Option<u32>,
    #[serde(default)]
    selection_end: Option<u32>,
    #[serde(default)]
    voice_active: bool,
    #[serde(default)]
    video_active: bool,
}

#[derive(Debug, Clone, Deserialize)]
struct EditEvent {
    file_path: String,
    collaborator_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    color: String,
    base_version: u64,
    offset: usize,
    delete_len: usize,
    text: String,
}

#[derive(Debug, Clone, Deserialize)]
struct AddCommentEvent {
    file_path: String,
    line: u32,
    column: u32,
    author: String,
    body: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ResolveCommentEvent {
    file_path: String,
    comment_id: String,
    resolved: bool,
    actor: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ApprovalEvent {
    file_path: String,
    reviewer: String,
    status: String,
    #[serde(default)]
    note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallSignalEvent {
    pub room_id: String,
    pub file_path: String,
    pub sender_id: String,
    pub sender_name: String,
    #[serde(default)]
    pub target_id: Option<String>,
    pub signal_type: String,
    pub payload: serde_json::Value,
    pub timestamp: u64,
}

fn now_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn apply_rebased_span(start: usize, end: usize, text: &str, content: &mut String) {
    let safe_start = start.min(content.len());
    let safe_end = end.min(content.len()).max(safe_start);
    content.replace_range(safe_start..safe_end, text);
}

fn rebase_span(
    start: usize,
    end: usize,
    base_version: u64,
    operations: &[TextOperation],
) -> (usize, usize) {
    let mut rebased_start = start;
    let mut rebased_end = end;

    for op in operations.iter().filter(|op| op.version > base_version) {
        if op.insert_len > 0 {
            if op.offset < rebased_start || op.offset == rebased_start {
                rebased_start += op.insert_len;
            }
            if op.offset < rebased_end || op.offset == rebased_end {
                rebased_end += op.insert_len;
            }
        }

        if op.delete_len > 0 {
            let del_start = op.offset;
            let del_end = op.offset + op.delete_len;

            if del_start < rebased_start {
                let removed_before = del_end.min(rebased_start).saturating_sub(del_start);
                rebased_start = rebased_start.saturating_sub(removed_before);
                rebased_end = rebased_end.saturating_sub(removed_before);
            }

            if del_start < rebased_end {
                let overlap = del_end
                    .min(rebased_end)
                    .saturating_sub(del_start.max(rebased_start));
                rebased_end = rebased_end.saturating_sub(overlap);
            }
        }
    }

    if rebased_end < rebased_start {
        rebased_end = rebased_start;
    }
    (rebased_start, rebased_end)
}

fn emit_snapshot(
    app: &AppHandle,
    document: &SharedDocument,
    source_collaborator_id: Option<String>,
) {
    let snapshot = build_snapshot(document, source_collaborator_id);
    let _ = app.emit("collab-document-state", snapshot);
}

fn build_snapshot(
    document: &SharedDocument,
    source_collaborator_id: Option<String>,
) -> CollaborationSnapshot {
    CollaborationSnapshot {
        file_path: document.file_path.clone(),
        content: document.content.clone(),
        version: document.version,
        source_collaborator_id,
        presences: {
            let mut values = document.presences.values().cloned().collect::<Vec<_>>();
            values.sort_by(|a, b| {
                a.name
                    .cmp(&b.name)
                    .then_with(|| a.collaborator_id.cmp(&b.collaborator_id))
            });
            values
        },
        comments: {
            let mut comments = document.comments.clone();
            comments.sort_by(|a, b| {
                a.line
                    .cmp(&b.line)
                    .then_with(|| a.column.cmp(&b.column))
                    .then_with(|| a.created_at.cmp(&b.created_at))
            });
            comments
        },
        approvals: {
            let mut approvals = document.approvals.values().cloned().collect::<Vec<_>>();
            approvals.sort_by(|a, b| a.reviewer.cmp(&b.reviewer));
            approvals
        },
    }
}

#[tauri::command]
pub fn collab_get_snapshot(
    file_path: String,
    state: tauri::State<'_, Arc<CollaborationState>>,
) -> Option<CollaborationSnapshot> {
    state.snapshot_for(&file_path)
}

pub fn install_event_bridge(app: &AppHandle) {
    let handle = app.clone();
    let state = handle.state::<Arc<CollaborationState>>().inner().clone();

    let join_handle = handle.clone();
    let join_state = state.clone();
    handle.listen("collab-join", move |event| {
        let Ok(payload) = serde_json::from_str::<JoinEvent>(event.payload()) else {
            return;
        };
        if payload.file_path.trim().is_empty() || payload.collaborator_id.trim().is_empty() {
            return;
        }
        let mut docs = match join_state.documents.lock() {
            Ok(docs) => docs,
            Err(_) => return,
        };
        let document = docs
            .entry(payload.file_path.clone())
            .or_insert_with(|| SharedDocument {
                file_path: payload.file_path.clone(),
                content: payload.content.clone(),
                version: 0,
                operations: Vec::new(),
                presences: HashMap::new(),
                comments: Vec::new(),
                approvals: HashMap::new(),
            });
        if document.content.is_empty() && !payload.content.is_empty() {
            document.content = payload.content;
        }
        document.presences.insert(
            payload.collaborator_id.clone(),
            CollaboratorPresence {
                collaborator_id: payload.collaborator_id,
                name: payload.name,
                color: payload.color,
                file_path: payload.file_path,
                line: 1,
                column: 1,
                selection_start: None,
                selection_end: None,
                voice_active: false,
                video_active: false,
                last_seen: now_ts(),
            },
        );
        emit_snapshot(&join_handle, document, None);
    });

    let leave_handle = handle.clone();
    let leave_state = state.clone();
    handle.listen("collab-leave", move |event| {
        let Ok(payload) = serde_json::from_str::<LeaveEvent>(event.payload()) else {
            return;
        };
        let mut docs = match leave_state.documents.lock() {
            Ok(docs) => docs,
            Err(_) => return,
        };
        if let Some(document) = docs.get_mut(&payload.file_path) {
            document.presences.remove(&payload.collaborator_id);
            emit_snapshot(&leave_handle, document, None);
        }
    });

    let presence_handle = handle.clone();
    let presence_state = state.clone();
    handle.listen("collab-presence", move |event| {
        let Ok(payload) = serde_json::from_str::<PresenceEvent>(event.payload()) else {
            return;
        };
        let mut docs = match presence_state.documents.lock() {
            Ok(docs) => docs,
            Err(_) => return,
        };
        let document = docs
            .entry(payload.file_path.clone())
            .or_insert_with(|| SharedDocument {
                file_path: payload.file_path.clone(),
                content: String::new(),
                version: 0,
                operations: Vec::new(),
                presences: HashMap::new(),
                comments: Vec::new(),
                approvals: HashMap::new(),
            });
        document.presences.insert(
            payload.collaborator_id.clone(),
            CollaboratorPresence {
                collaborator_id: payload.collaborator_id,
                name: payload.name,
                color: payload.color,
                file_path: payload.file_path,
                line: payload.line,
                column: payload.column,
                selection_start: payload.selection_start,
                selection_end: payload.selection_end,
                voice_active: payload.voice_active,
                video_active: payload.video_active,
                last_seen: now_ts(),
            },
        );
        emit_snapshot(&presence_handle, document, None);
    });

    let edit_handle = handle.clone();
    let edit_state = state.clone();
    handle.listen("collab-edit", move |event| {
        let Ok(payload) = serde_json::from_str::<EditEvent>(event.payload()) else {
            return;
        };
        let mut docs = match edit_state.documents.lock() {
            Ok(docs) => docs,
            Err(_) => return,
        };
        let document = docs
            .entry(payload.file_path.clone())
            .or_insert_with(|| SharedDocument {
                file_path: payload.file_path.clone(),
                content: String::new(),
                version: 0,
                operations: Vec::new(),
                presences: HashMap::new(),
                comments: Vec::new(),
                approvals: HashMap::new(),
            });

        if let Some(presence) = document.presences.get_mut(&payload.collaborator_id) {
            if !payload.name.trim().is_empty() {
                presence.name = payload.name.clone();
            }
            if !payload.color.trim().is_empty() {
                presence.color = payload.color.clone();
            }
            presence.last_seen = now_ts();
        }

        let requested_start = payload.offset.min(document.content.len());
        let requested_end = requested_start
            .saturating_add(payload.delete_len)
            .min(document.content.len());
        let (start, end) = rebase_span(
            requested_start,
            requested_end,
            payload.base_version,
            &document.operations,
        );
        apply_rebased_span(start, end, &payload.text, &mut document.content);

        document.version += 1;
        document.operations.push(TextOperation {
            version: document.version,
            offset: start,
            delete_len: end.saturating_sub(start),
            insert_len: payload.text.len(),
        });
        if document.operations.len() > 256 {
            let drain_to = document.operations.len().saturating_sub(256);
            document.operations.drain(0..drain_to);
        }

        emit_snapshot(&edit_handle, document, Some(payload.collaborator_id));
    });

    let comment_handle = handle.clone();
    let comment_state = state.clone();
    handle.listen("collab-add-comment", move |event| {
        let Ok(payload) = serde_json::from_str::<AddCommentEvent>(event.payload()) else {
            return;
        };
        let mut docs = match comment_state.documents.lock() {
            Ok(docs) => docs,
            Err(_) => return,
        };
        let document = docs
            .entry(payload.file_path.clone())
            .or_insert_with(|| SharedDocument {
                file_path: payload.file_path.clone(),
                content: String::new(),
                version: 0,
                operations: Vec::new(),
                presences: HashMap::new(),
                comments: Vec::new(),
                approvals: HashMap::new(),
            });
        document.comments.push(ReviewComment {
            id: uuid::Uuid::new_v4().to_string(),
            file_path: payload.file_path,
            line: payload.line,
            column: payload.column,
            author: payload.author,
            body: payload.body,
            created_at: now_ts(),
            resolved: false,
            resolved_by: None,
        });
        emit_snapshot(&comment_handle, document, None);
    });

    let resolve_handle = handle.clone();
    let resolve_state = state.clone();
    handle.listen("collab-resolve-comment", move |event| {
        let Ok(payload) = serde_json::from_str::<ResolveCommentEvent>(event.payload()) else {
            return;
        };
        let mut docs = match resolve_state.documents.lock() {
            Ok(docs) => docs,
            Err(_) => return,
        };
        if let Some(document) = docs.get_mut(&payload.file_path) {
            if let Some(comment) = document
                .comments
                .iter_mut()
                .find(|comment| comment.id == payload.comment_id)
            {
                comment.resolved = payload.resolved;
                comment.resolved_by = if payload.resolved {
                    Some(payload.actor)
                } else {
                    None
                };
            }
            emit_snapshot(&resolve_handle, document, None);
        }
    });

    let approval_handle = handle.clone();
    let approval_state = state.clone();
    handle.listen("collab-set-approval", move |event| {
        let Ok(payload) = serde_json::from_str::<ApprovalEvent>(event.payload()) else {
            return;
        };
        let mut docs = match approval_state.documents.lock() {
            Ok(docs) => docs,
            Err(_) => return,
        };
        let document = docs
            .entry(payload.file_path.clone())
            .or_insert_with(|| SharedDocument {
                file_path: payload.file_path.clone(),
                content: String::new(),
                version: 0,
                operations: Vec::new(),
                presences: HashMap::new(),
                comments: Vec::new(),
                approvals: HashMap::new(),
            });
        document.approvals.insert(
            payload.reviewer.clone(),
            ReviewApproval {
                reviewer: payload.reviewer,
                status: payload.status,
                note: payload.note,
                updated_at: now_ts(),
            },
        );
        emit_snapshot(&approval_handle, document, None);
    });

    let call_handle = handle.clone();
    handle.listen("collab-call-signal", move |event| {
        let Ok(payload) = serde_json::from_str::<CallSignalEvent>(event.payload()) else {
            return;
        };
        let _ = call_handle.emit("collab-call-signal", payload);
    });
}
