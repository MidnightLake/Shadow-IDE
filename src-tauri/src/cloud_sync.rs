use crate::ai_macros::{AiMacro, MacroState};
use crate::ferrum_bridge::FerrumState;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use ferrum_core::types::Message;
use ferrum_sessions::Session;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::State;

const BUNDLE_FILENAME: &str = "shadow-ide-sync.enc";

pub struct CloudSyncState {
    root: PathBuf,
}

impl CloudSyncState {
    pub fn new(data_dir: PathBuf) -> Self {
        let root = data_dir.join("cloud-sync");
        let _ = std::fs::create_dir_all(&root);
        Self { root }
    }

    fn snippets_path(&self) -> PathBuf {
        self.root.join("snippets.json")
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudSnippet {
    pub id: String,
    pub title: String,
    pub language: String,
    pub content: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CloudSnippetInput {
    pub id: Option<String>,
    pub title: String,
    pub language: String,
    pub content: String,
    pub tags: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CloudFrontendSnapshot {
    #[serde(default)]
    pub settings_json: String,
    #[serde(default)]
    pub themes_json: String,
    #[serde(default)]
    pub keybindings_json: String,
    #[serde(default)]
    pub ui_skills_json: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloudSessionBackup {
    pub session: Session,
    pub messages: Vec<Message>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CloudSyncBundle {
    version: u32,
    exported_at: u64,
    frontend: CloudFrontendSnapshot,
    snippets: Vec<CloudSnippet>,
    skills: Vec<AiMacro>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    sessions: Vec<CloudSessionBackup>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct EncryptedCloudBundle {
    version: u32,
    nonce_b64: String,
    checksum_hex: String,
    ciphertext_b64: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CloudBundleStatus {
    pub bundle_path: String,
    pub exists: bool,
    pub modified_at: Option<u64>,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CloudSyncExportResult {
    pub bundle_path: String,
    pub exported_at: u64,
    pub snippet_count: usize,
    pub skill_count: usize,
    pub session_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct CloudSyncImportResult {
    pub bundle_path: String,
    pub imported_at: u64,
    pub snippet_count: usize,
    pub skill_count: usize,
    pub restored_session_count: usize,
    pub frontend: CloudFrontendSnapshot,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn resolve_bundle_path(cloud_path: &str) -> PathBuf {
    let path = PathBuf::from(cloud_path);
    if path.extension().is_some() {
        path
    } else {
        path.join(BUNDLE_FILENAME)
    }
}

fn ensure_parent_dir(path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create sync directory: {}", e))?;
    }
    Ok(())
}

fn checksum_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{:02x}", b)).collect()
}

fn xor_keystream(data: &[u8], passphrase: &str, nonce: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut counter = 0u64;
    let mut offset = 0usize;

    while offset < data.len() {
        let mut hasher = Sha256::new();
        hasher.update(passphrase.as_bytes());
        hasher.update(nonce);
        hasher.update(counter.to_le_bytes());
        let block = hasher.finalize();

        for byte in block {
            if offset >= data.len() {
                break;
            }
            out.push(data[offset] ^ byte);
            offset += 1;
        }

        counter += 1;
    }

    out
}

fn encrypt_bundle(
    bundle: &CloudSyncBundle,
    passphrase: &str,
) -> Result<EncryptedCloudBundle, String> {
    if passphrase.trim().len() < 8 {
        return Err("Cloud sync passphrase must be at least 8 characters".to_string());
    }

    let plaintext = serde_json::to_vec_pretty(bundle)
        .map_err(|e| format!("Failed to serialize bundle: {}", e))?;
    let nonce = uuid::Uuid::new_v4().into_bytes().to_vec();
    let ciphertext = xor_keystream(&plaintext, passphrase, &nonce);

    Ok(EncryptedCloudBundle {
        version: 1,
        nonce_b64: BASE64.encode(&nonce),
        checksum_hex: checksum_hex(&plaintext),
        ciphertext_b64: BASE64.encode(ciphertext),
    })
}

fn decrypt_bundle(raw: &str, passphrase: &str) -> Result<CloudSyncBundle, String> {
    let encrypted: EncryptedCloudBundle = serde_json::from_str(raw)
        .map_err(|e| format!("Failed to parse encrypted sync bundle: {}", e))?;
    let nonce = BASE64
        .decode(encrypted.nonce_b64.as_bytes())
        .map_err(|e| format!("Invalid sync bundle nonce: {}", e))?;
    let ciphertext = BASE64
        .decode(encrypted.ciphertext_b64.as_bytes())
        .map_err(|e| format!("Invalid sync bundle payload: {}", e))?;
    let plaintext = xor_keystream(&ciphertext, passphrase, &nonce);

    if checksum_hex(&plaintext) != encrypted.checksum_hex {
        return Err("Failed to decrypt sync bundle. Check the passphrase.".to_string());
    }

    serde_json::from_slice::<CloudSyncBundle>(&plaintext)
        .map_err(|e| format!("Failed to decode sync bundle contents: {}", e))
}

fn load_snippets(state: &CloudSyncState) -> Result<Vec<CloudSnippet>, String> {
    let path = state.snippets_path();
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read snippet library: {}", e))?;
    serde_json::from_str::<Vec<CloudSnippet>>(&content)
        .map_err(|e| format!("Failed to parse snippet library: {}", e))
}

fn save_snippets(state: &CloudSyncState, snippets: &[CloudSnippet]) -> Result<(), String> {
    let path = state.snippets_path();
    ensure_parent_dir(&path)?;
    let json = serde_json::to_string_pretty(snippets)
        .map_err(|e| format!("Failed to encode snippet library: {}", e))?;
    std::fs::write(path, json).map_err(|e| format!("Failed to save snippet library: {}", e))
}

fn load_macros_from_disk() -> Vec<AiMacro> {
    let path = dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("shadow-ide")
        .join("ai-macros.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|content| serde_json::from_str::<Vec<AiMacro>>(&content).ok())
        .unwrap_or_default()
}

fn collect_session_backups(ferrum_state: &FerrumState) -> Result<Vec<CloudSessionBackup>, String> {
    let store = ferrum_state.store.lock().map_err(|e| e.to_string())?;
    let sessions = store.list_sessions().map_err(|e| e.to_string())?;
    let mut backups = Vec::with_capacity(sessions.len());
    for session in sessions {
        let messages = store
            .load_messages(&session.id)
            .map_err(|e| e.to_string())?;
        backups.push(CloudSessionBackup { session, messages });
    }
    Ok(backups)
}

#[tauri::command]
pub fn cloud_list_snippets(state: State<'_, CloudSyncState>) -> Result<Vec<CloudSnippet>, String> {
    let mut snippets = load_snippets(&state)?;
    snippets.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.title.cmp(&b.title))
    });
    Ok(snippets)
}

#[tauri::command]
pub fn cloud_save_snippet(
    snippet: CloudSnippetInput,
    state: State<'_, CloudSyncState>,
) -> Result<CloudSnippet, String> {
    let title = snippet.title.trim();
    if title.is_empty() {
        return Err("Snippet title is required".to_string());
    }
    if snippet.content.trim().is_empty() {
        return Err("Snippet content is required".to_string());
    }

    let mut snippets = load_snippets(&state)?;
    let now = now_secs();
    let tags = snippet
        .tags
        .unwrap_or_default()
        .into_iter()
        .map(|tag| tag.trim().to_string())
        .filter(|tag| !tag.is_empty())
        .collect::<Vec<_>>();

    let saved = if let Some(id) = snippet.id.clone() {
        if let Some(existing) = snippets.iter_mut().find(|item| item.id == id) {
            existing.title = title.to_string();
            existing.language = snippet.language.trim().to_string();
            existing.content = snippet.content;
            existing.tags = tags;
            existing.updated_at = now;
            existing.clone()
        } else {
            let new_snippet = CloudSnippet {
                id,
                title: title.to_string(),
                language: snippet.language.trim().to_string(),
                content: snippet.content,
                tags,
                created_at: now,
                updated_at: now,
            };
            snippets.push(new_snippet.clone());
            new_snippet
        }
    } else {
        let new_snippet = CloudSnippet {
            id: uuid::Uuid::new_v4().to_string(),
            title: title.to_string(),
            language: snippet.language.trim().to_string(),
            content: snippet.content,
            tags,
            created_at: now,
            updated_at: now,
        };
        snippets.push(new_snippet.clone());
        new_snippet
    };

    save_snippets(&state, &snippets)?;
    Ok(saved)
}

#[tauri::command]
pub fn cloud_delete_snippet(id: String, state: State<'_, CloudSyncState>) -> Result<(), String> {
    let mut snippets = load_snippets(&state)?;
    let before = snippets.len();
    snippets.retain(|snippet| snippet.id != id);
    if before == snippets.len() {
        return Err("Snippet not found".to_string());
    }
    save_snippets(&state, &snippets)
}

#[tauri::command]
pub fn cloud_get_bundle_status(cloud_path: String) -> Result<CloudBundleStatus, String> {
    let bundle_path = resolve_bundle_path(&cloud_path);
    let metadata = std::fs::metadata(&bundle_path).ok();
    let modified_at = metadata
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|m| m.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs());

    Ok(CloudBundleStatus {
        bundle_path: bundle_path.to_string_lossy().to_string(),
        exists: metadata.is_some(),
        modified_at,
        size_bytes: metadata.map(|m| m.len()),
    })
}

#[tauri::command]
pub fn cloud_export_bundle(
    cloud_path: String,
    passphrase: String,
    settings_json: String,
    themes_json: String,
    keybindings_json: String,
    ui_skills_json: Option<String>,
    include_sessions: bool,
    cloud_state: State<'_, CloudSyncState>,
    ferrum_state: State<'_, FerrumState>,
) -> Result<CloudSyncExportResult, String> {
    let snippets = load_snippets(&cloud_state)?;
    let skills = load_macros_from_disk();
    let sessions = if include_sessions {
        collect_session_backups(&ferrum_state)?
    } else {
        Vec::new()
    };

    let bundle = CloudSyncBundle {
        version: 1,
        exported_at: now_secs(),
        frontend: CloudFrontendSnapshot {
            settings_json,
            themes_json,
            keybindings_json,
            ui_skills_json: ui_skills_json.unwrap_or_default(),
        },
        snippets,
        skills,
        sessions,
    };

    let encrypted = encrypt_bundle(&bundle, &passphrase)?;
    let bundle_path = resolve_bundle_path(&cloud_path);
    ensure_parent_dir(&bundle_path)?;
    let json = serde_json::to_string_pretty(&encrypted)
        .map_err(|e| format!("Failed to encode encrypted bundle: {}", e))?;
    std::fs::write(&bundle_path, json)
        .map_err(|e| format!("Failed to write sync bundle: {}", e))?;

    Ok(CloudSyncExportResult {
        bundle_path: bundle_path.to_string_lossy().to_string(),
        exported_at: bundle.exported_at,
        snippet_count: bundle.snippets.len(),
        skill_count: bundle.skills.len(),
        session_count: bundle.sessions.len(),
    })
}

#[tauri::command]
pub fn cloud_import_bundle(
    cloud_path: String,
    passphrase: String,
    restore_sessions: bool,
    cloud_state: State<'_, CloudSyncState>,
    ferrum_state: State<'_, FerrumState>,
    macro_state: State<'_, Arc<MacroState>>,
) -> Result<CloudSyncImportResult, String> {
    let bundle_path = resolve_bundle_path(&cloud_path);
    let raw = std::fs::read_to_string(&bundle_path)
        .map_err(|e| format!("Failed to read sync bundle: {}", e))?;
    let bundle = decrypt_bundle(&raw, &passphrase)?;

    let mut merged_snippets = load_snippets(&cloud_state)?;
    let mut existing_by_id: HashMap<String, CloudSnippet> = merged_snippets
        .drain(..)
        .map(|snippet| (snippet.id.clone(), snippet))
        .collect();
    for snippet in bundle.snippets.iter().cloned() {
        match existing_by_id.get(&snippet.id) {
            Some(existing) if existing.updated_at > snippet.updated_at => {}
            _ => {
                existing_by_id.insert(snippet.id.clone(), snippet);
            }
        }
    }
    let mut merged = existing_by_id.into_values().collect::<Vec<_>>();
    merged.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.title.cmp(&b.title))
    });
    save_snippets(&cloud_state, &merged)?;

    macro_state.replace_all(bundle.skills.clone())?;

    let restored_session_count = if restore_sessions {
        let store = ferrum_state.store.lock().map_err(|e| e.to_string())?;
        let mut restored = 0usize;
        for backup in &bundle.sessions {
            if store
                .import_session(&backup.session, &backup.messages)
                .map_err(|e| e.to_string())?
            {
                restored += 1;
            }
        }
        restored
    } else {
        0
    };

    Ok(CloudSyncImportResult {
        bundle_path: bundle_path.to_string_lossy().to_string(),
        imported_at: now_secs(),
        snippet_count: merged.len(),
        skill_count: bundle.skills.len(),
        restored_session_count,
        frontend: bundle.frontend,
    })
}
