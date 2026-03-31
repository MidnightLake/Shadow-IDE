use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;

// ===== Token Cache =====

pub struct TokenCache {
    entries: Mutex<HashMap<String, CacheEntry>>,
    max_entries: usize,
    ttl_seconds: Mutex<u64>,
    insertion_order: Mutex<std::collections::VecDeque<String>>,
}

#[derive(Clone)]
struct CacheEntry {
    response: String,
    created_at: u64,
    hit_count: u32,
}

#[derive(Debug, Serialize, Clone)]
pub struct CacheStats {
    pub entries: usize,
    pub total_hits: u32,
    pub enabled: bool,
    pub ttl_seconds: u64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[allow(dead_code)]
pub struct TokenStats {
    pub input_tokens: usize,
    pub output_tokens: usize,
    pub cached: bool,
    pub cleaned_tokens_saved: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub enum CleanMode {
    None,
    Trim,
    Strip,
    /// Structural mode: language-aware cleaning that preserves code structure
    /// (function signatures, type definitions, imports) while removing
    /// docstrings, decorators, logging statements, and redundant whitespace.
    Structural,
}

impl Default for CleanMode {
    fn default() -> Self {
        CleanMode::Trim
    }
}

impl TokenCache {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            max_entries: 200,
            ttl_seconds: Mutex::new(300), // 5 minutes
            insertion_order: Mutex::new(std::collections::VecDeque::new()),
        }
    }

    pub fn get(&self, key: &str) -> Option<String> {
        let mut entries = self.entries.lock().ok()?;
        let ttl = *self.ttl_seconds.lock().ok()?;
        let now = now_secs();

        if let Some(entry) = entries.get_mut(key) {
            if now - entry.created_at < ttl {
                entry.hit_count += 1;
                return Some(entry.response.clone());
            } else {
                entries.remove(key);
            }
        }
        None
    }

    pub fn put(&self, key: String, response: String) {
        if let Ok(mut entries) = self.entries.lock() {
            let mut order = self
                .insertion_order
                .lock()
                .unwrap_or_else(|e| e.into_inner());

            // Evict old entries if at capacity
            if entries.len() >= self.max_entries {
                let ttl = self.ttl_seconds.lock().map(|t| *t).unwrap_or(300);
                let now = now_secs();
                entries.retain(|_, v| now - v.created_at < ttl);
                order.retain(|k| entries.contains_key(k));

                // If still full, remove oldest via insertion order (O(1) eviction)
                while entries.len() >= self.max_entries {
                    if let Some(oldest) = order.pop_front() {
                        entries.remove(&oldest);
                    } else {
                        break;
                    }
                }
            }

            // Remove old position if key already exists
            if entries.contains_key(&key) {
                order.retain(|k| k != &key);
            }

            order.push_back(key.clone());
            entries.insert(
                key,
                CacheEntry {
                    response,
                    created_at: now_secs(),
                    hit_count: 0,
                },
            );
        }
    }

    pub fn clear(&self) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.clear();
        }
        if let Ok(mut order) = self.insertion_order.lock() {
            order.clear();
        }
    }

    pub fn stats(&self) -> CacheStats {
        let entries = self.entries.lock().map(|e| e.len()).unwrap_or(0);
        let total_hits = self
            .entries
            .lock()
            .map(|e| e.values().map(|v| v.hit_count).sum())
            .unwrap_or(0);
        let ttl = self.ttl_seconds.lock().map(|t| *t).unwrap_or(300);

        CacheStats {
            entries,
            total_hits,
            enabled: true,
            ttl_seconds: ttl,
        }
    }

    pub fn set_ttl(&self, seconds: u64) {
        if let Ok(mut ttl) = self.ttl_seconds.lock() {
            *ttl = seconds;
        }
    }
}

// ===== Warm SQLite Cache (Level 2) =====

pub struct WarmCache {
    conn: Mutex<Option<Connection>>,
    max_entries: usize,
    ttl_seconds: u64,
}

#[derive(Debug, Serialize, Clone)]
pub struct WarmCacheStats {
    pub entries: usize,
    pub embedded_entries: usize,
    pub total_hits: u64,
    pub db_size_bytes: u64,
    pub db_path: String,
}

impl WarmCache {
    pub fn new() -> Self {
        let db_path = Self::db_path();
        let conn = Self::open_db(&db_path);
        Self {
            conn: Mutex::new(conn),
            max_entries: 5000,
            ttl_seconds: 7 * 86400, // 7 days default
        }
    }

    fn db_path() -> std::path::PathBuf {
        dirs_next::cache_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("shadow-ide")
            .join("llm-cache.db")
    }

    fn open_db(path: &std::path::Path) -> Option<Connection> {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let conn = Connection::open(path).ok()?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA busy_timeout=3000;
             CREATE TABLE IF NOT EXISTS cache (
                 key TEXT PRIMARY KEY,
                 response TEXT NOT NULL,
                 model TEXT NOT NULL DEFAULT '',
                 prompt_summary TEXT NOT NULL DEFAULT '',
                 embedding BLOB,
                 created_at INTEGER NOT NULL,
                 hit_count INTEGER NOT NULL DEFAULT 0,
                 token_count INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS idx_cache_created ON cache(created_at);
             CREATE INDEX IF NOT EXISTS idx_cache_hits ON cache(hit_count);",
        )
        .ok()?;
        Some(conn)
    }

    /// Exact key lookup
    pub fn get(&self, key: &str) -> Option<String> {
        let conn = self.conn.lock().ok()?;
        let conn = conn.as_ref()?;
        let now = now_secs();
        let cutoff = now.saturating_sub(self.ttl_seconds);

        let result: Option<String> = conn
            .query_row(
                "SELECT response FROM cache WHERE key = ?1 AND created_at > ?2",
                params![key, cutoff],
                |row| row.get(0),
            )
            .ok();

        if result.is_some() {
            let _ = conn.execute(
                "UPDATE cache SET hit_count = hit_count + 1 WHERE key = ?1",
                params![key],
            );
        }
        result
    }

    /// Store a response with optional embedding and metadata
    pub fn put(
        &self,
        key: String,
        response: String,
        model: &str,
        prompt_summary: &str,
        embedding: Option<&[f32]>,
        token_count: usize,
    ) {
        if let Ok(conn) = self.conn.lock() {
            if let Some(conn) = conn.as_ref() {
                // Evict if over capacity
                let count: usize = conn
                    .query_row("SELECT COUNT(*) FROM cache", [], |r| r.get(0))
                    .unwrap_or(0);

                if count >= self.max_entries {
                    // Remove expired entries first (LIMIT to avoid blocking)
                    let cutoff = now_secs().saturating_sub(self.ttl_seconds);
                    let _ = conn.execute(
                        "DELETE FROM cache WHERE rowid IN (SELECT rowid FROM cache WHERE created_at < ?1 LIMIT 1000)",
                        params![cutoff],
                    );

                    // If still over capacity, remove oldest low-hit entries
                    let still: usize = conn
                        .query_row("SELECT COUNT(*) FROM cache", [], |r| r.get(0))
                        .unwrap_or(0);
                    if still >= self.max_entries {
                        let to_remove = still - self.max_entries + 100; // remove 100 extra for headroom
                        let _ = conn.execute(
                            "DELETE FROM cache WHERE key IN (
                                SELECT key FROM cache ORDER BY hit_count ASC, created_at ASC LIMIT ?1
                            )",
                            params![to_remove],
                        );
                    }
                }

                let emb_blob: Option<Vec<u8>> =
                    embedding.map(|e| e.iter().flat_map(|f| f.to_le_bytes()).collect());

                let _ = conn.execute(
                    "INSERT OR REPLACE INTO cache (key, response, model, prompt_summary, embedding, created_at, hit_count, token_count)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)",
                    params![
                        key,
                        response,
                        model,
                        prompt_summary,
                        emb_blob,
                        now_secs() as i64,
                        token_count as i64,
                    ],
                );
            }
        }
    }

    /// Semantic similarity search: find cached responses with similar embeddings
    pub fn semantic_search(
        &self,
        query_embedding: &[f32],
        threshold: f32,
        limit: usize,
    ) -> Vec<(String, String, f32)> {
        // Returns: Vec<(key, response, similarity)>
        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        let conn = match conn.as_ref() {
            Some(c) => c,
            None => return Vec::new(),
        };

        let cutoff = now_secs().saturating_sub(self.ttl_seconds);
        let mut stmt = match conn.prepare(
            "SELECT key, response, embedding FROM cache WHERE embedding IS NOT NULL AND created_at > ?1",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let mut results: Vec<(String, String, f32)> = Vec::new();

        let rows = stmt.query_map(params![cutoff], |row| {
            let key: String = row.get(0)?;
            let response: String = row.get(1)?;
            let emb_blob: Vec<u8> = row.get(2)?;
            Ok((key, response, emb_blob))
        });

        if let Ok(rows) = rows {
            for row in rows.flatten() {
                let (key, response, emb_blob) = row;
                if let Some(stored_emb) = blob_to_embedding(&emb_blob) {
                    let sim = cosine_similarity(query_embedding, &stored_emb);
                    if sim >= threshold {
                        results.push((key, response, sim));
                    }
                }
            }
        }

        results.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(limit);
        results
    }

    /// Get stats about the warm cache
    pub fn stats(&self) -> WarmCacheStats {
        let db_path = Self::db_path();
        let db_size = std::fs::metadata(&db_path).map(|m| m.len()).unwrap_or(0);

        let conn = match self.conn.lock() {
            Ok(c) => c,
            Err(_) => {
                return WarmCacheStats {
                    entries: 0,
                    embedded_entries: 0,
                    total_hits: 0,
                    db_size_bytes: db_size,
                    db_path: db_path.to_string_lossy().to_string(),
                }
            }
        };

        if let Some(conn) = conn.as_ref() {
            let entries: usize = conn
                .query_row("SELECT COUNT(*) FROM cache", [], |r| r.get(0))
                .unwrap_or(0);
            let embedded: usize = conn
                .query_row(
                    "SELECT COUNT(*) FROM cache WHERE embedding IS NOT NULL",
                    [],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let hits: u64 = conn
                .query_row("SELECT COALESCE(SUM(hit_count), 0) FROM cache", [], |r| {
                    r.get(0)
                })
                .unwrap_or(0);

            WarmCacheStats {
                entries,
                embedded_entries: embedded,
                total_hits: hits,
                db_size_bytes: db_size,
                db_path: db_path.to_string_lossy().to_string(),
            }
        } else {
            WarmCacheStats {
                entries: 0,
                embedded_entries: 0,
                total_hits: 0,
                db_size_bytes: db_size,
                db_path: db_path.to_string_lossy().to_string(),
            }
        }
    }

    /// Clear all entries
    pub fn clear(&self) {
        if let Ok(conn) = self.conn.lock() {
            if let Some(conn) = conn.as_ref() {
                let _ = conn.execute("DELETE FROM cache", []);
            }
        }
    }

    /// Evict expired entries
    pub fn evict_expired(&self) -> usize {
        if let Ok(conn) = self.conn.lock() {
            if let Some(conn) = conn.as_ref() {
                let cutoff = now_secs().saturating_sub(self.ttl_seconds);
                return conn
                    .execute("DELETE FROM cache WHERE rowid IN (SELECT rowid FROM cache WHERE created_at < ?1 LIMIT 1000)", params![cutoff])
                    .unwrap_or(0);
            }
        }
        0
    }

    #[allow(dead_code)]
    pub fn set_ttl(&self, seconds: u64) {
        // TTL is not behind a mutex since WarmCache itself is behind Arc<Mutex>
        // For simplicity, we store it but it's read at query time
        // Since we can't mutate &self, TTL changes require a new WarmCache
        let _ = seconds; // TTL is set at construction; use config reload for changes
    }
}

fn blob_to_embedding(blob: &[u8]) -> Option<Vec<f32>> {
    if blob.len() % 4 != 0 {
        return None;
    }
    Some(
        blob.chunks_exact(4)
            .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
            .collect(),
    )
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0f32;
    let mut norm_a = 0.0f32;
    let mut norm_b = 0.0f32;
    for i in 0..a.len() {
        dot += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }
    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom < 1e-10 {
        0.0
    } else {
        dot / denom
    }
}

/// Generate a short summary of the prompt for semantic matching.
/// Extracts the last user message as a representative summary.
pub fn prompt_summary(messages_json: &str) -> String {
    if let Ok(messages) = serde_json::from_str::<Vec<serde_json::Value>>(messages_json) {
        // Find the last user message
        for msg in messages.iter().rev() {
            if msg["role"].as_str() == Some("user") {
                if let Some(content) = msg["content"].as_str() {
                    // Take first 256 chars as summary
                    let summary: String = content.chars().take(256).collect();
                    return summary;
                }
            }
        }
    }
    String::new()
}

use std::path::PathBuf;

// ===== Token Settings =====

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TokenSettingsState {
    pub clean_mode: CleanMode,
    pub cache_enabled: bool,
    pub max_context_tokens: usize,
}

impl Default for TokenSettingsState {
    fn default() -> Self {
        Self {
            clean_mode: CleanMode::Trim,
            cache_enabled: true,
            max_context_tokens: 120000,
        }
    }
}

pub struct TokenSettings {
    pub state: Mutex<TokenSettingsState>,
}

impl TokenSettings {
    fn settings_path() -> PathBuf {
        dirs_next::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("shadow-ide")
            .join("token_settings.json")
    }

    pub fn new() -> Self {
        let path = Self::settings_path();
        let state = if path.exists() {
            let raw = std::fs::read_to_string(&path).unwrap_or_default();
            serde_json::from_str(&raw).unwrap_or_default()
        } else {
            TokenSettingsState::default()
        };

        Self {
            state: Mutex::new(state),
        }
    }

    pub fn save(&self) {
        if let Ok(st) = self.state.lock() {
            let path = Self::settings_path();
            if let Some(p) = path.parent() {
                let _ = std::fs::create_dir_all(p);
            }
            if let Ok(raw) = serde_json::to_string_pretty(&*st) {
                let _ = std::fs::write(&path, raw);
            }
        }
    }
}

#[tauri::command]
pub fn token_update_settings(
    clean_mode: String,
    cache_enabled: bool,
    max_context: usize,
    settings: tauri::State<'_, TokenSettings>,
) -> Result<(), String> {
    if let Ok(mut st) = settings.state.lock() {
        st.clean_mode = match clean_mode.as_str() {
            "none" => CleanMode::None,
            "strip" => CleanMode::Strip,
            "structural" | "ast" => CleanMode::Structural,
            _ => CleanMode::Trim,
        };
        st.cache_enabled = cache_enabled;
        st.max_context_tokens = max_context;
    }
    settings.save();
    Ok(())
}

// ===== Cache Key Generation =====

pub fn cache_key(messages_json: &str, model: &str, temperature: f64) -> String {
    let mut hasher = Sha256::new();
    hasher.update(model.as_bytes());
    hasher.update(format!("{:.2}", temperature).as_bytes());
    hasher.update(messages_json.as_bytes());
    format!("{:x}", hasher.finalize())
}

// ===== Token Counting =====

/// Token count using character-class analysis that approximates BPE tokenization.
/// More accurate than simple chars/4 — handles words, punctuation, numbers,
/// whitespace, and non-ASCII characters with class-specific ratios.
pub fn count_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }

    let mut tokens = 0;
    let bytes = text.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        let b = bytes[i];

        if b == b'\n' {
            // Newlines are their own token
            tokens += 1;
            i += 1;
        } else if b == b'\r' {
            tokens += 1;
            i += 1;
            if i < len && bytes[i] == b'\n' {
                i += 1;
            }
        } else if b == b' ' || b == b'\t' {
            // Whitespace runs: single space merges with adjacent token (free),
            // indentation/multiple spaces cost ~1 token per 4 chars
            let start = i;
            while i < len && (bytes[i] == b' ' || bytes[i] == b'\t') {
                i += 1;
            }
            let ws_len = i - start;
            if ws_len > 1 {
                tokens += (ws_len + 3) / 4;
            }
        } else if b.is_ascii_alphabetic() || b == b'_' {
            // Word token: collect alphanumeric + underscore
            let start = i;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            let word_len = i - start;
            // BPE subwords are typically 3-5 chars
            if word_len <= 5 {
                tokens += 1;
            } else {
                tokens += 1 + (word_len - 5 + 3) / 4;
            }
        } else if b.is_ascii_digit() {
            // Number sequences: 1-3 digits = 1 token
            let start = i;
            while i < len && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let num_len = i - start;
            tokens += (num_len + 2) / 3;
        } else if b >= 0x80 {
            // Non-ASCII (UTF-8 multibyte): typically 2-3 tokens per char
            i += 1;
            while i < len && bytes[i] & 0xC0 == 0x80 {
                i += 1;
            }
            tokens += 2;
        } else {
            // ASCII punctuation/symbols: 1 token each
            tokens += 1;
            i += 1;
        }
    }

    tokens
}

/// Count tokens for a set of chat messages.
pub fn count_message_tokens(messages: &[serde_json::Value]) -> usize {
    let mut total = 0;
    for msg in messages {
        // Each message has ~4 tokens overhead (role, formatting)
        total += 4;
        if let Some(content) = msg["content"].as_str() {
            total += count_tokens(content);
        }
        // Tool calls have extra overhead
        if let Some(tool_calls) = msg["tool_calls"].as_array() {
            for tc in tool_calls {
                total += 4; // tool call overhead
                if let Some(args) = tc["function"]["arguments"].as_str() {
                    total += count_tokens(args);
                }
                if let Some(name) = tc["function"]["name"].as_str() {
                    total += count_tokens(name);
                }
            }
        }
    }
    total
}

// ===== Token Cleaning =====

/// Clean context text to reduce token usage.
pub fn clean_context(text: &str, mode: &CleanMode) -> String {
    match mode {
        CleanMode::None => text.to_string(),
        CleanMode::Trim => clean_trim(text),
        CleanMode::Strip => clean_strip(text),
        CleanMode::Structural => clean_structural(text, None),
    }
}

/// Clean context text with language hint for structural mode.
#[allow(dead_code)]
pub fn clean_context_with_lang(text: &str, mode: &CleanMode, lang: Option<&str>) -> String {
    match mode {
        CleanMode::Structural => clean_structural(text, lang),
        _ => clean_context(text, mode),
    }
}

/// Trim: remove trailing whitespace from lines, collapse multiple blank lines to one.
fn clean_trim(text: &str) -> String {
    let mut result = Vec::new();
    let mut blank_count = 0;

    for line in text.lines() {
        let trimmed_end = line.trim_end();
        if trimmed_end.is_empty() {
            blank_count += 1;
            if blank_count <= 1 {
                result.push("");
            }
        } else {
            blank_count = 0;
            result.push(trimmed_end);
        }
    }

    result.join("\n")
}

/// Strip: remove trailing whitespace, collapse blank lines, remove single-line comments.
fn clean_strip(text: &str) -> String {
    let mut result = Vec::new();
    let mut blank_count = 0;
    let mut in_multiline_comment = false;

    for line in text.lines() {
        let trimmed = line.trim();

        // Track multiline comments
        if in_multiline_comment {
            if trimmed.contains("*/") {
                in_multiline_comment = false;
            }
            continue;
        }

        if trimmed.starts_with("/*") && !trimmed.contains("*/") {
            in_multiline_comment = true;
            continue;
        }

        // Skip pure single-line comments
        if trimmed.starts_with("//") || trimmed.starts_with('#') {
            // Keep shebangs and pragma/lint comments
            if trimmed.starts_with("#!")
                || trimmed.starts_with("#[")
                || trimmed.contains("TODO")
                || trimmed.contains("FIXME")
            {
                result.push(line.trim_end());
                blank_count = 0;
                continue;
            }
            continue;
        }

        // Handle blank lines
        if trimmed.is_empty() {
            blank_count += 1;
            if blank_count <= 1 {
                result.push("");
            }
        } else {
            blank_count = 0;
            result.push(line.trim_end());
        }
    }

    result.join("\n")
}

/// Structural cleaning: language-aware token reduction.
/// Detects language from content patterns if no hint provided. Removes:
/// - Docstrings (Python triple-quotes, Rust ///, JSDoc /** */)
/// - Single-line comments (except TODO/FIXME/SAFETY/HACK annotations)
/// - Multi-line comments
/// - Consecutive blank lines (collapses to one)
/// - Trailing whitespace
/// - Debug/logging statements (console.log, println!, print(), log.*)
/// - Redundant import grouping whitespace
/// Preserves: all code, function signatures, type definitions, #[] attributes,
/// shebangs, pragma comments, structural annotations.
fn clean_structural(text: &str, lang_hint: Option<&str>) -> String {
    let lang = lang_hint.unwrap_or_else(|| detect_language(text));

    let mut result = Vec::new();
    let mut blank_count = 0;
    let mut in_multiline_comment = false;
    let mut in_docstring = false;
    let lines: Vec<&str> = text.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Track Python triple-quote docstrings
        if (lang == "python" || lang == "unknown") && !in_multiline_comment {
            if in_docstring {
                if trimmed.contains("\"\"\"") || trimmed.contains("'''") {
                    in_docstring = false;
                }
                i += 1;
                continue;
            }
            // Docstring start: line is just """...""" on one line (keep if it has code after close)
            if trimmed.starts_with("\"\"\"") || trimmed.starts_with("'''") {
                let quote = if trimmed.starts_with("\"\"\"") {
                    "\"\"\""
                } else {
                    "'''"
                };
                // Check if it closes on the same line (after the opening)
                let rest = &trimmed[3..];
                if rest.contains(quote) {
                    // Single-line docstring — skip it
                    i += 1;
                    continue;
                }
                in_docstring = true;
                i += 1;
                continue;
            }
        }

        // Track multiline comments (C-style)
        if in_multiline_comment {
            if trimmed.contains("*/") {
                in_multiline_comment = false;
            }
            i += 1;
            continue;
        }

        // Multi-line comment start
        if trimmed.starts_with("/*") || trimmed.starts_with("/**") {
            // Keep if it contains annotations like @param, @returns (JSDoc)
            // but remove plain documentation blocks
            let is_jsdoc_content = trimmed.contains("@param")
                || trimmed.contains("@returns")
                || trimmed.contains("@type")
                || trimmed.contains("@deprecated");

            if !trimmed.contains("*/") {
                if is_jsdoc_content {
                    // Keep JSDoc with annotations — just this line though
                    result.push(line.trim_end());
                    blank_count = 0;
                }
                in_multiline_comment = true;
                i += 1;
                continue;
            }
            // Single-line /* ... */ — skip unless it has annotations
            if !is_jsdoc_content {
                i += 1;
                continue;
            }
        }

        // Rust doc comments (///) — keep the first line (summary), skip rest
        if (lang == "rust" || lang == "unknown") && trimmed.starts_with("///") {
            // Keep if it's the first /// in a block (summary line)
            if i == 0 || !lines[i - 1].trim().starts_with("///") {
                result.push(line.trim_end());
                blank_count = 0;
            }
            i += 1;
            continue;
        }

        // Single-line comments
        if trimmed.starts_with("//")
            || (trimmed.starts_with('#')
                && lang != "python"
                && !trimmed.starts_with("#[")
                && !trimmed.starts_with("#!")
                && !trimmed.starts_with("#include"))
        {
            // Keep annotations and pragmas
            if is_keeper_comment(trimmed) {
                result.push(line.trim_end());
                blank_count = 0;
            }
            i += 1;
            continue;
        }

        // Python # comments
        if lang == "python" && trimmed.starts_with('#') && !trimmed.starts_with("#!") {
            if is_keeper_comment(trimmed) {
                result.push(line.trim_end());
                blank_count = 0;
            }
            i += 1;
            continue;
        }

        // Remove debug/logging statements
        if is_debug_statement(trimmed, lang) {
            i += 1;
            continue;
        }

        // Blank lines
        if trimmed.is_empty() {
            blank_count += 1;
            if blank_count <= 1 {
                result.push("");
            }
            i += 1;
            continue;
        }

        // Keep everything else — it's actual code
        blank_count = 0;
        result.push(line.trim_end());
        i += 1;
    }

    result.join("\n")
}

/// Detect language from content patterns
fn detect_language(text: &str) -> &'static str {
    let sample: String = text.lines().take(30).collect::<Vec<_>>().join("\n");

    if sample.contains("fn ")
        && (sample.contains("-> ") || sample.contains("let ") || sample.contains("pub "))
    {
        return "rust";
    }
    if sample.contains("def ") && sample.contains("import ") && !sample.contains('{') {
        return "python";
    }
    if sample.contains("function ") || sample.contains("const ") || sample.contains("=>") {
        if sample.contains(": ") && (sample.contains("interface ") || sample.contains("<")) {
            return "typescript";
        }
        return "javascript";
    }
    if sample.contains("func ") && sample.contains("package ") {
        return "go";
    }
    if sample.contains("class ") && sample.contains("public ") && sample.contains(';') {
        return "java";
    }
    if sample.contains("struct ") && sample.contains("import ") && sample.contains("var ") {
        return "swift";
    }
    "unknown"
}

/// Check if a comment should be kept (annotations, TODOs, pragmas, safety notes)
fn is_keeper_comment(trimmed: &str) -> bool {
    trimmed.contains("TODO")
        || trimmed.contains("FIXME")
        || trimmed.contains("HACK")
        || trimmed.contains("SAFETY")
        || trimmed.contains("WARN")
        || trimmed.contains("NOTE:")
        || trimmed.starts_with("#!")
        || trimmed.starts_with("#[")
        || trimmed.starts_with("#include")
        || trimmed.starts_with("// MARK:")
        || trimmed.starts_with("// PRAGMA")
        || trimmed.contains("eslint-disable")
        || trimmed.contains("@ts-")
        || trimmed.contains("type:")
        || trimmed.contains("noqa")
        || trimmed.contains("pylint:")
}

/// Detect debug/logging statements that are safe to remove for token budget
fn is_debug_statement(trimmed: &str, lang: &str) -> bool {
    match lang {
        "javascript" | "typescript" => {
            trimmed.starts_with("console.log(")
                || trimmed.starts_with("console.debug(")
                || trimmed.starts_with("console.info(")
                || trimmed.starts_with("console.warn(")
        }
        "rust" => {
            trimmed.starts_with("println!(")
                || trimmed.starts_with("eprintln!(")
                || trimmed.starts_with("dbg!(")
                || (trimmed.starts_with("log::debug!(") || trimmed.starts_with("log::trace!("))
        }
        "python" => {
            trimmed.starts_with("print(")
                || trimmed.starts_with("logging.debug(")
                || trimmed.starts_with("logging.info(")
                || trimmed.starts_with("logger.debug(")
                || trimmed.starts_with("logger.info(")
        }
        "go" => {
            trimmed.starts_with("fmt.Println(")
                || trimmed.starts_with("fmt.Printf(")
                || trimmed.starts_with("log.Println(")
                || trimmed.starts_with("log.Printf(")
        }
        _ => false,
    }
}

/// Smart truncation: keep the most important parts of code (function signatures, imports, structs).
pub fn truncate_smart(text: &str, max_tokens: usize) -> String {
    let current = count_tokens(text);
    if current <= max_tokens {
        return text.to_string();
    }

    let lines: Vec<&str> = text.lines().collect();
    let target_lines = (lines.len() * max_tokens) / current;

    // Keep first 40% and last 40% of the file, drop the middle
    let keep_start = (target_lines * 2) / 5;
    let keep_end = (target_lines * 2) / 5;
    let skip = lines.len().saturating_sub(keep_end);

    let start_part = &lines[..keep_start.min(lines.len())];
    let end_part = if skip < lines.len() {
        &lines[skip..]
    } else {
        &[]
    };
    let omitted = lines.len() - start_part.len() - end_part.len();

    let mut output = start_part.join("\n");
    output.push_str(&format!("\n// ... ({} lines omitted) ...\n", omitted));
    output.push_str(&end_part.join("\n"));
    output
}

// ===== Tauri Commands =====

#[tauri::command]
pub fn token_get_cache_stats(cache: tauri::State<'_, TokenCache>) -> CacheStats {
    cache.stats()
}

#[tauri::command]
pub fn token_clear_cache(cache: tauri::State<'_, TokenCache>) -> Result<(), String> {
    cache.clear();
    Ok(())
}

#[tauri::command]
pub fn token_set_cache_ttl(
    seconds: u64,
    cache: tauri::State<'_, TokenCache>,
) -> Result<(), String> {
    cache.set_ttl(seconds);
    Ok(())
}

#[tauri::command]
pub fn token_set_clean_mode(
    mode: String,
    settings: tauri::State<'_, TokenSettings>,
) -> Result<(), String> {
    let clean_mode = match mode.as_str() {
        "none" => CleanMode::None,
        "trim" => CleanMode::Trim,
        "strip" => CleanMode::Strip,
        "structural" | "ast" => CleanMode::Structural,
        _ => return Err(format!("Invalid clean mode: {}", mode)),
    };
    if let Ok(mut st) = settings.state.lock() {
        st.clean_mode = clean_mode;
    }
    settings.save();
    Ok(())
}

#[tauri::command]
pub fn token_set_max_context(
    max_tokens: usize,
    settings: tauri::State<'_, TokenSettings>,
) -> Result<(), String> {
    if let Ok(mut st) = settings.state.lock() {
        st.max_context_tokens = max_tokens;
    }
    settings.save();
    Ok(())
}

#[tauri::command]
pub fn token_count_text(text: String) -> usize {
    count_tokens(&text)
}

// ===== Warm Cache Tauri Commands =====

#[tauri::command]
pub fn warm_cache_get(key: String, cache: tauri::State<'_, WarmCache>) -> Option<String> {
    cache.get(&key)
}

#[tauri::command]
pub fn warm_cache_stats(cache: tauri::State<'_, WarmCache>) -> WarmCacheStats {
    cache.stats()
}

#[tauri::command]
pub fn warm_cache_clear(cache: tauri::State<'_, WarmCache>) -> Result<(), String> {
    cache.clear();
    Ok(())
}

#[tauri::command]
pub fn warm_cache_evict(cache: tauri::State<'_, WarmCache>) -> Result<usize, String> {
    Ok(cache.evict_expired())
}

#[tauri::command]
pub fn warm_cache_semantic_search(
    embedding: Vec<f32>,
    threshold: Option<f32>,
    limit: Option<usize>,
    cache: tauri::State<'_, WarmCache>,
) -> Vec<serde_json::Value> {
    let thresh = threshold.unwrap_or(0.75);
    let lim = limit.unwrap_or(5).min(20);
    cache
        .semantic_search(&embedding, thresh, lim)
        .into_iter()
        .map(|(key, response, similarity)| {
            serde_json::json!({
                "key": key,
                "response": response,
                "similarity": similarity,
            })
        })
        .collect()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_tokens_empty() {
        assert_eq!(count_tokens(""), 0);
    }

    #[test]
    fn test_count_tokens_basic() {
        // "hello" = 5 chars, single word ≤ 5 = 1 token
        assert_eq!(count_tokens("hello"), 1);
        // "test" = 4 chars, single word ≤ 5 = 1 token
        assert_eq!(count_tokens("test"), 1);
        // 100 'a' chars = one long word: 1 + (100-5+3)/4 = 1 + 24 = 25
        let text = "a".repeat(100);
        assert_eq!(count_tokens(&text), 25);
    }

    #[test]
    fn test_count_tokens_words_and_punctuation() {
        // "Hello, world!" → "Hello"(1) + ","(1) + " "(free) + "world"(1) + "!"(1) = 4
        assert_eq!(count_tokens("Hello, world!"), 4);
        // "fn main() {" → "fn"(1) + " "(free) + "main"(1) + "("(1) + ")"(1) + " "(free) + "{"(1) = 5
        assert_eq!(count_tokens("fn main() {"), 5);
    }

    #[test]
    fn test_count_tokens_newlines() {
        // Each newline = 1 token
        assert_eq!(count_tokens("\n"), 1);
        assert_eq!(count_tokens("\n\n\n"), 3);
        // "a\nb" → "a"(1) + "\n"(1) + "b"(1) = 3
        assert_eq!(count_tokens("a\nb"), 3);
    }

    #[test]
    fn test_count_tokens_indentation() {
        // 4 spaces = (4+3)/4 = 1 token
        assert_eq!(count_tokens("    x"), 2); // 4 spaces(1) + "x"(1)
                                              // 8 spaces = (8+3)/4 = 2 tokens
        assert_eq!(count_tokens("        x"), 3); // 8 spaces(2) + "x"(1)
    }

    #[test]
    fn test_count_tokens_numbers() {
        // "123" = 3 digits, (3+2)/3 = 1 token
        assert_eq!(count_tokens("123"), 1);
        // "123456" = 6 digits, (6+2)/3 = 2 tokens
        assert_eq!(count_tokens("123456"), 2);
    }

    #[test]
    fn test_count_tokens_long_word() {
        // "calculateTotal" = 14 chars, 1 + (14-5+3)/4 = 1 + 3 = 4 tokens
        assert_eq!(count_tokens("calculateTotal"), 4);
    }

    #[test]
    fn test_count_message_tokens() {
        let messages = vec![serde_json::json!({"role": "user", "content": "Hello world"})];
        let count = count_message_tokens(&messages);
        // 4 overhead + count_tokens("Hello world") = 4 + 2 = 6
        // "Hello"(1) + " "(free) + "world"(1) = 2
        assert_eq!(count, 6);
    }

    #[test]
    fn test_count_message_tokens_with_tool_calls() {
        let messages = vec![serde_json::json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{
                "function": {
                    "name": "read_file",
                    "arguments": "{\"path\": \"test.rs\"}"
                }
            }]
        })];
        let count = count_message_tokens(&messages);
        assert!(count > 4); // Should include overhead + tool call tokens
    }

    #[test]
    fn test_clean_trim_collapses_blanks() {
        let input = "line1\n\n\n\nline2\n\n\nline3";
        let result = clean_trim(input);
        assert_eq!(result, "line1\n\nline2\n\nline3");
    }

    #[test]
    fn test_clean_trim_strips_trailing_whitespace() {
        let input = "hello   \nworld  ";
        let result = clean_trim(input);
        assert_eq!(result, "hello\nworld");
    }

    #[test]
    fn test_clean_strip_removes_comments() {
        let input = "fn main() {\n// this is a comment\n    println!(\"hello\");\n}";
        let result = clean_strip(input);
        assert!(result.contains("fn main()"));
        assert!(result.contains("println!"));
        assert!(!result.contains("this is a comment"));
    }

    #[test]
    fn test_clean_strip_keeps_shebangs() {
        let input = "#!/bin/bash\n# normal comment\necho hello";
        let result = clean_strip(input);
        assert!(result.contains("#!/bin/bash"));
        assert!(!result.contains("normal comment"));
    }

    #[test]
    fn test_clean_strip_keeps_todo_comments() {
        let input = "// TODO: fix this\n// regular comment\ncode();";
        let result = clean_strip(input);
        assert!(result.contains("TODO: fix this"));
        assert!(!result.contains("regular comment"));
    }

    #[test]
    fn test_clean_strip_removes_multiline_comments() {
        let input = "before\n/* multi\nline\ncomment */\nafter";
        let result = clean_strip(input);
        assert!(result.contains("before"));
        assert!(result.contains("after"));
        assert!(!result.contains("multi"));
        assert!(!result.contains("line"));
    }

    #[test]
    fn test_clean_context_none_mode() {
        let input = "  hello  \n\n\n  world  ";
        let result = clean_context(input, &CleanMode::None);
        assert_eq!(result, input);
    }

    #[test]
    fn test_truncate_smart_no_truncation_needed() {
        let short = "line1\nline2\nline3";
        let result = truncate_smart(short, 1000);
        assert_eq!(result, short);
    }

    #[test]
    fn test_truncate_smart_truncates() {
        let lines: Vec<String> = (0..100)
            .map(|i| format!("line {} with some content here", i))
            .collect();
        let text = lines.join("\n");
        let result = truncate_smart(&text, 50);
        assert!(result.contains("lines omitted"));
        assert!(result.len() < text.len());
    }

    #[test]
    fn test_cache_key_deterministic() {
        let key1 = cache_key("test messages", "model-a", 0.7);
        let key2 = cache_key("test messages", "model-a", 0.7);
        assert_eq!(key1, key2);
    }

    #[test]
    fn test_cache_key_differs_with_model() {
        let key1 = cache_key("test", "model-a", 0.7);
        let key2 = cache_key("test", "model-b", 0.7);
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_cache_key_differs_with_temp() {
        let key1 = cache_key("test", "model", 0.5);
        let key2 = cache_key("test", "model", 0.7);
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_token_cache_put_get() {
        let cache = TokenCache::new();
        cache.put("key1".to_string(), "response1".to_string());
        assert_eq!(cache.get("key1"), Some("response1".to_string()));
    }

    #[test]
    fn test_token_cache_miss() {
        let cache = TokenCache::new();
        assert_eq!(cache.get("nonexistent"), None);
    }

    #[test]
    fn test_token_cache_clear() {
        let cache = TokenCache::new();
        cache.put("key1".to_string(), "val".to_string());
        cache.clear();
        assert_eq!(cache.get("key1"), None);
    }

    #[test]
    fn test_token_cache_stats() {
        let cache = TokenCache::new();
        let stats = cache.stats();
        assert_eq!(stats.entries, 0);
        assert_eq!(stats.total_hits, 0);
        assert!(stats.enabled);
        assert_eq!(stats.ttl_seconds, 300);
    }

    #[test]
    fn test_token_cache_stats_after_put() {
        let cache = TokenCache::new();
        cache.put("k".to_string(), "v".to_string());
        let stats = cache.stats();
        assert_eq!(stats.entries, 1);
    }

    #[test]
    fn test_token_cache_hit_count() {
        let cache = TokenCache::new();
        cache.put("k".to_string(), "v".to_string());
        cache.get("k");
        cache.get("k");
        let stats = cache.stats();
        assert_eq!(stats.total_hits, 2);
    }

    #[test]
    fn test_token_cache_eviction() {
        let cache = TokenCache {
            entries: std::sync::Mutex::new(HashMap::new()),
            insertion_order: std::sync::Mutex::new(std::collections::VecDeque::new()),
            max_entries: 3,
            ttl_seconds: std::sync::Mutex::new(300),
        };
        cache.put("a".to_string(), "1".to_string());
        cache.put("b".to_string(), "2".to_string());
        cache.put("c".to_string(), "3".to_string());
        cache.put("d".to_string(), "4".to_string()); // should evict oldest
        assert_eq!(cache.stats().entries, 3);
    }

    #[test]
    fn test_warm_cache_put_get() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test-cache.db");
        let conn = WarmCache::open_db(&db_path).unwrap();
        let wc = WarmCache {
            conn: Mutex::new(Some(conn)),
            max_entries: 100,
            ttl_seconds: 3600,
        };
        wc.put(
            "key1".to_string(),
            "response1".to_string(),
            "test-model",
            "test prompt",
            None,
            25,
        );
        assert_eq!(wc.get("key1"), Some("response1".to_string()));
        assert_eq!(wc.get("nonexistent"), None);
    }

    #[test]
    fn test_warm_cache_stats() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test-stats.db");
        let conn = WarmCache::open_db(&db_path).unwrap();
        let wc = WarmCache {
            conn: Mutex::new(Some(conn)),
            max_entries: 100,
            ttl_seconds: 3600,
        };
        wc.put("k1".to_string(), "v1".to_string(), "m", "", None, 10);
        wc.put("k2".to_string(), "v2".to_string(), "m", "", None, 10);
        let stats = wc.stats();
        assert_eq!(stats.entries, 2);
        assert_eq!(stats.embedded_entries, 0);
    }

    #[test]
    fn test_warm_cache_clear() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test-clear.db");
        let conn = WarmCache::open_db(&db_path).unwrap();
        let wc = WarmCache {
            conn: Mutex::new(Some(conn)),
            max_entries: 100,
            ttl_seconds: 3600,
        };
        wc.put("k".to_string(), "v".to_string(), "m", "", None, 10);
        wc.clear();
        assert_eq!(wc.get("k"), None);
        assert_eq!(wc.stats().entries, 0);
    }

    #[test]
    fn test_warm_cache_semantic_search() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test-semantic.db");
        let conn = WarmCache::open_db(&db_path).unwrap();
        let wc = WarmCache {
            conn: Mutex::new(Some(conn)),
            max_entries: 100,
            ttl_seconds: 3600,
        };

        let emb1 = vec![0.9f32, 0.1, 0.0];
        let emb2 = vec![0.1f32, 0.9, 0.0];

        wc.put(
            "k1".to_string(),
            "hello response".to_string(),
            "m",
            "hello",
            Some(&emb1),
            10,
        );
        wc.put(
            "k2".to_string(),
            "goodbye response".to_string(),
            "m",
            "goodbye",
            Some(&emb2),
            10,
        );

        // Query close to emb1
        let query = vec![0.85f32, 0.15, 0.0];
        let results = wc.semantic_search(&query, 0.5, 5);
        assert!(!results.is_empty());
        assert_eq!(results[0].0, "k1"); // k1 should be most similar
        assert!(results[0].2 > results.get(1).map(|r| r.2).unwrap_or(0.0));
    }

    #[test]
    fn test_warm_cache_with_embeddings_stats() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test-emb-stats.db");
        let conn = WarmCache::open_db(&db_path).unwrap();
        let wc = WarmCache {
            conn: Mutex::new(Some(conn)),
            max_entries: 100,
            ttl_seconds: 3600,
        };

        let emb = vec![1.0f32, 0.0, 0.0];
        wc.put("k1".to_string(), "v1".to_string(), "m", "", Some(&emb), 10);
        wc.put("k2".to_string(), "v2".to_string(), "m", "", None, 10);

        let stats = wc.stats();
        assert_eq!(stats.entries, 2);
        assert_eq!(stats.embedded_entries, 1);
    }

    #[test]
    fn test_warm_cache_hit_count() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test-hits.db");
        let conn = WarmCache::open_db(&db_path).unwrap();
        let wc = WarmCache {
            conn: Mutex::new(Some(conn)),
            max_entries: 100,
            ttl_seconds: 3600,
        };
        wc.put("k".to_string(), "v".to_string(), "m", "", None, 10);
        wc.get("k");
        wc.get("k");
        wc.get("k");
        let stats = wc.stats();
        assert_eq!(stats.total_hits, 3);
    }

    #[test]
    fn test_prompt_summary_extracts_last_user() {
        let msgs = serde_json::to_string(&vec![
            serde_json::json!({"role": "system", "content": "You are helpful"}),
            serde_json::json!({"role": "user", "content": "First question"}),
            serde_json::json!({"role": "assistant", "content": "First answer"}),
            serde_json::json!({"role": "user", "content": "Second question about Rust"}),
        ])
        .unwrap();
        let summary = prompt_summary(&msgs);
        assert_eq!(summary, "Second question about Rust");
    }

    #[test]
    fn test_blob_embedding_roundtrip() {
        let original = vec![1.0f32, -0.5, 0.25, 3.14];
        let blob: Vec<u8> = original.iter().flat_map(|f| f.to_le_bytes()).collect();
        let recovered = blob_to_embedding(&blob).unwrap();
        assert_eq!(original, recovered);
    }

    #[test]
    fn test_cosine_similarity_warm() {
        let a = vec![1.0f32, 0.0, 0.0];
        let b = vec![1.0f32, 0.0, 0.0];
        assert!((cosine_similarity(&a, &b) - 1.0).abs() < 1e-6);

        let c = vec![0.0f32, 1.0, 0.0];
        assert!(cosine_similarity(&a, &c).abs() < 1e-6);
    }

    #[test]
    fn test_set_ttl() {
        let cache = TokenCache::new();
        cache.set_ttl(60);
        let stats = cache.stats();
        assert_eq!(stats.ttl_seconds, 60);
    }

    // ===== Structural cleaning tests =====

    #[test]
    fn test_structural_removes_rust_comments() {
        let input =
            "fn main() {\n    // this is a comment\n    let x = 5;\n    println!(\"debug\");\n}";
        let result = clean_structural(input, Some("rust"));
        assert!(result.contains("fn main()"));
        assert!(result.contains("let x = 5;"));
        assert!(!result.contains("this is a comment"));
        // println! is a debug statement in Rust — removed
        assert!(!result.contains("println!"));
    }

    #[test]
    fn test_structural_keeps_todo_comments() {
        let input = "// TODO: fix this\n// regular comment\nfn foo() {}";
        let result = clean_structural(input, Some("rust"));
        assert!(result.contains("TODO: fix this"));
        assert!(!result.contains("regular comment"));
        assert!(result.contains("fn foo()"));
    }

    #[test]
    fn test_structural_keeps_rust_doc_summary() {
        let input = "/// Main entry point for the application.\n/// This is a longer description.\n/// More details here.\nfn main() {}";
        let result = clean_structural(input, Some("rust"));
        // Keeps first /// line (summary), strips continuation
        assert!(result.contains("Main entry point"));
        assert!(!result.contains("longer description"));
        assert!(result.contains("fn main()"));
    }

    #[test]
    fn test_structural_removes_python_docstrings() {
        let input = "def hello():\n    \"\"\"This is a docstring.\"\"\"\n    return 42";
        let result = clean_structural(input, Some("python"));
        assert!(result.contains("def hello():"));
        assert!(result.contains("return 42"));
        assert!(!result.contains("docstring"));
    }

    #[test]
    fn test_structural_removes_multiline_docstrings() {
        let input = "def hello():\n    \"\"\"\n    This is a long\n    docstring.\n    \"\"\"\n    return 42";
        let result = clean_structural(input, Some("python"));
        assert!(result.contains("def hello():"));
        assert!(result.contains("return 42"));
        assert!(!result.contains("long"));
    }

    #[test]
    fn test_structural_removes_js_console_log() {
        let input = "function foo() {\n    console.log('debug');\n    return 5;\n}";
        let result = clean_structural(input, Some("javascript"));
        assert!(result.contains("function foo()"));
        assert!(result.contains("return 5;"));
        assert!(!result.contains("console.log"));
    }

    #[test]
    fn test_structural_removes_multiline_comments() {
        let input = "before();\n/* this is\na multi-line\ncomment */\nafter();";
        let result = clean_structural(input, Some("rust"));
        assert!(result.contains("before();"));
        assert!(result.contains("after();"));
        assert!(!result.contains("multi-line"));
    }

    #[test]
    fn test_structural_collapses_blank_lines() {
        let input = "line1\n\n\n\n\nline2";
        let result = clean_structural(input, Some("rust"));
        assert_eq!(result, "line1\n\nline2");
    }

    #[test]
    fn test_structural_detects_rust() {
        assert_eq!(detect_language("fn main() {\n    let x = 5;\n}"), "rust");
    }

    #[test]
    fn test_structural_detects_python() {
        assert_eq!(
            detect_language("import os\ndef hello():\n    pass"),
            "python"
        );
    }

    #[test]
    fn test_structural_detects_javascript() {
        assert_eq!(
            detect_language("const x = 5;\nfunction foo() {\n    return x;\n}"),
            "javascript"
        );
    }

    #[test]
    fn test_structural_keeps_attributes() {
        let input = "#[derive(Debug)]\n// A comment\nstruct Foo {}";
        let result = clean_structural(input, Some("rust"));
        assert!(result.contains("#[derive(Debug)]"));
        assert!(result.contains("struct Foo"));
        assert!(!result.contains("A comment"));
    }

    #[test]
    fn test_structural_keeps_python_shebangs() {
        let input = "#!/usr/bin/env python3\n# regular comment\nimport sys";
        let result = clean_structural(input, Some("python"));
        assert!(result.contains("#!/usr/bin/env python3"));
        assert!(!result.contains("regular comment"));
    }

    #[test]
    fn test_clean_context_structural_mode() {
        let input = "fn main() {\n    // debug\n    println!(\"test\");\n    let x = 1;\n}";
        let result = clean_context(input, &CleanMode::Structural);
        assert!(result.contains("fn main()"));
        assert!(result.contains("let x = 1;"));
        assert!(!result.contains("// debug"));
    }
}
