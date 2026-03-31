use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

// ===== Constants =====

const CHUNK_LINES: usize = 150;
const OVERLAP_LINES: usize = 30;
const MAX_CHUNKS: usize = 10000;
const MAX_FILE_SIZE: u64 = 1024 * 1024; // 1MB
const MAX_CHUNK_CONTENT: usize = 10000;
const MAX_DEPTH: usize = 12;

const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "dist",
    "build",
    "__pycache__",
    ".next",
];

const TEXT_EXTENSIONS: &[&str] = &[
    "rs",
    "ts",
    "tsx",
    "js",
    "jsx",
    "py",
    "go",
    "c",
    "cpp",
    "h",
    "hpp",
    "java",
    "kt",
    "swift",
    "rb",
    "php",
    "css",
    "scss",
    "html",
    "vue",
    "svelte",
    "toml",
    "yaml",
    "yml",
    "json",
    "md",
    "sh",
    "bash",
    "zsh",
    "lua",
    "zig",
    "dart",
    "cs",
    "txt",
    "cfg",
    "ini",
    "xml",
    "sql",
    "graphql",
    "proto",
    "makefile",
    "dockerfile",
    "lock",
];

// ===== Structs =====

#[derive(Debug, Clone, Serialize)]
pub struct Chunk {
    pub file_path: String,
    pub line_start: usize,
    pub line_end: usize,
    pub content: String,
    pub tokens: Vec<String>,
    /// Timestamp when this chunk was indexed (seconds since epoch)
    pub indexed_at: u64,
    /// Number of times this chunk was returned in a query
    pub hit_count: u32,
    /// Optional embedding vector for semantic search
    #[serde(skip_serializing)]
    pub embedding: Option<Vec<f32>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RagStats {
    pub files_indexed: usize,
    pub total_chunks: usize,
    pub last_index_time: String,
}

pub struct RagState {
    pub index: Mutex<RagIndex>,
    pub embedding_config: Mutex<EmbeddingConfig>,
    /// Active file watcher for continuous indexing (None = not watching)
    watcher: Mutex<Option<RecommendedWatcher>>,
    /// Root path currently being watched
    watched_root: Mutex<Option<String>>,
}

pub struct RagIndex {
    pub chunks: Vec<Chunk>,
    files_indexed: usize,
    last_index_time: String,
}

impl RagState {
    pub fn new() -> Self {
        Self {
            index: Mutex::new(RagIndex {
                chunks: Vec::new(),
                files_indexed: 0,
                last_index_time: String::new(),
            }),
            embedding_config: Mutex::new(EmbeddingConfig::default()),
            watcher: Mutex::new(None),
            watched_root: Mutex::new(None),
        }
    }

    pub fn get_stats(&self) -> RagStats {
        match self.index.lock() {
            Ok(idx) => RagStats {
                files_indexed: idx.files_indexed,
                total_chunks: idx.chunks.len(),
                last_index_time: idx.last_index_time.clone(),
            },
            Err(_) => RagStats {
                files_indexed: 0,
                total_chunks: 0,
                last_index_time: String::new(),
            },
        }
    }

    /// Remove old and unused chunks to keep the index lean
    pub fn auto_clean(&self, max_age_secs: u64, delete_never_accessed: bool) -> (usize, usize) {
        let mut index = match self.index.lock() {
            Ok(idx) => idx,
            Err(_) => return (0, 0),
        };

        let now = now_secs();
        let _before_count = index.chunks.len();

        // 1. Remove chunks older than max_age
        let age_cutoff = now.saturating_sub(max_age_secs);
        let mut deleted_age = 0usize;
        index.chunks.retain(|c| {
            if c.indexed_at > 0 && c.indexed_at < age_cutoff {
                deleted_age += 1;
                false
            } else {
                true
            }
        });

        // 2. Remove chunks never accessed (hit_count == 0) older than 3 days
        let mut deleted_unused = 0usize;
        if delete_never_accessed {
            let unused_cutoff = now.saturating_sub(3 * 86400);
            index.chunks.retain(|c| {
                if c.hit_count == 0 && c.indexed_at > 0 && c.indexed_at < unused_cutoff {
                    deleted_unused += 1;
                    false
                } else {
                    true
                }
            });
        }

        // Update files_indexed count
        let mut unique_files = std::collections::HashSet::new();
        for chunk in &index.chunks {
            unique_files.insert(chunk.file_path.clone());
        }
        index.files_indexed = unique_files.len();

        (deleted_age, deleted_unused)
    }

    pub fn search(&self, query: &str, limit: usize) -> Vec<Chunk> {
        self.search_with_embedding(query, limit, None)
    }

    /// Search with optional query embedding for semantic scoring
    pub fn search_with_embedding(
        &self,
        query: &str,
        limit: usize,
        query_embedding: Option<&Vec<f32>>,
    ) -> Vec<Chunk> {
        let query_tokens = tokenize(query);
        if query_tokens.is_empty() && query_embedding.is_none() {
            return Vec::new();
        }

        let mut index = match self.index.lock() {
            Ok(idx) => idx,
            Err(_) => return Vec::new(),
        };

        let mut scored: Vec<(usize, f64)> = index
            .chunks
            .iter()
            .enumerate()
            .map(|(i, chunk)| (i, score_chunk_hybrid(chunk, &query_tokens, query_embedding)))
            .filter(|(_, score)| *score > 0.0)
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let result_indices: Vec<usize> = scored.iter().take(limit).map(|(i, _)| *i).collect();

        // Increment hit counts for returned chunks
        for &i in &result_indices {
            index.chunks[i].hit_count += 1;
        }

        result_indices
            .into_iter()
            .map(|i| index.chunks[i].clone())
            .collect()
    }

    /// Get the count of chunks that have embeddings
    #[allow(dead_code)]
    pub fn embedded_count(&self) -> usize {
        match self.index.lock() {
            Ok(idx) => idx.chunks.iter().filter(|c| c.embedding.is_some()).count(),
            Err(_) => 0,
        }
    }
}

// ===== Indexing =====

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub fn tokenize(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .filter(|w| w.len() >= 2)
        .map(|w| w.to_string())
        .collect()
}

fn is_text_file(path: &Path) -> bool {
    // Check by extension
    if let Some(ext) = path.extension() {
        let ext_lower = ext.to_string_lossy().to_lowercase();
        return TEXT_EXTENSIONS.contains(&ext_lower.as_str());
    }
    // Files without extension: check common names
    if let Some(name) = path.file_name() {
        let name_lower = name.to_string_lossy().to_lowercase();
        return matches!(
            name_lower.as_str(),
            "makefile" | "dockerfile" | "rakefile" | "gemfile" | "procfile" | "license" | "readme"
        );
    }
    false
}

fn scan_files_recursive(dir: &Path, files: &mut Vec<String>, depth: usize, chunk_budget: usize) {
    if depth > MAX_DEPTH || files.len() * 2 > chunk_budget {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut dirs = Vec::new();
    let mut file_entries = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip dotfiles/dotdirs
        if name.starts_with('.') {
            continue;
        }

        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);

        if is_dir {
            if !SKIP_DIRS.contains(&name.as_str()) {
                dirs.push(entry.path());
            }
        } else {
            file_entries.push(entry.path());
        }
    }

    // Add text files within size limits
    for path in file_entries {
        if !is_text_file(&path) {
            continue;
        }

        let size = match std::fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(_) => continue,
        };

        if size == 0 || size > MAX_FILE_SIZE {
            continue;
        }

        files.push(path.to_string_lossy().to_string());
    }

    // Recurse into subdirectories
    for dir_path in dirs {
        scan_files_recursive(&dir_path, files, depth + 1, chunk_budget);
    }
}

fn chunk_file(file_path: &str) -> Vec<Chunk> {
    // Handle PDF files via pdftotext
    if file_path.to_lowercase().ends_with(".pdf") {
        let content = match read_pdf_text(file_path) {
            Some(c) if !c.trim().is_empty() => c,
            _ => return Vec::new(),
        };
        let lines: Vec<&str> = content.lines().collect();
        if lines.is_empty() {
            return Vec::new();
        }
        return chunk_lines(file_path, &lines);
    }

    let content = match std::fs::read_to_string(file_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    // Quick binary check: if content has null bytes, skip
    if content.contains('\0') {
        return Vec::new();
    }

    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    // Select chunking strategy based on file type
    let ext = std::path::Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "rs" | "go" | "py" | "ts" | "tsx" | "js" | "jsx" | "java" | "c" | "cpp" | "h" | "hpp"
        | "cs" | "kt" | "swift" | "rb" | "dart" | "zig" => {
            chunk_code_aware(file_path, &lines, &ext)
        }
        "md" | "mdx" => chunk_markdown(file_path, &lines),
        _ => chunk_lines(file_path, &lines),
    }
}

/// Code-aware chunking: splits at function/struct/class boundaries
fn chunk_code_aware(file_path: &str, lines: &[&str], ext: &str) -> Vec<Chunk> {
    let boundary_patterns = match ext {
        "rs" => vec![
            "fn ",
            "pub fn ",
            "struct ",
            "pub struct ",
            "enum ",
            "pub enum ",
            "impl ",
            "mod ",
            "trait ",
            "pub trait ",
        ],
        "py" => vec!["def ", "class ", "async def "],
        "ts" | "tsx" | "js" | "jsx" => vec![
            "function ",
            "export function ",
            "export default function ",
            "class ",
            "export class ",
            "const ",
            "export const ",
        ],
        "go" => vec!["func ", "type "],
        "java" | "kt" => vec![
            "public ",
            "private ",
            "protected ",
            "class ",
            "interface ",
            "enum ",
        ],
        "c" | "cpp" | "h" | "hpp" => vec![
            "int ", "void ", "char ", "static ", "struct ", "class ", "enum ", "typedef ",
        ],
        "rb" => vec!["def ", "class ", "module "],
        _ => vec!["fn ", "def ", "class ", "func "],
    };

    let mut boundaries: Vec<usize> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        // Check if this line starts a new definition (not indented or at top level)
        let indent = line.len() - line.trim_start().len();
        let is_top_level = indent <= 4; // allow up to 4 spaces (1 level) as "top level"
        if is_top_level {
            for pat in &boundary_patterns {
                if trimmed.starts_with(pat) || trimmed.starts_with(&format!("pub {}", pat)) {
                    boundaries.push(i);
                    break;
                }
            }
        }
    }

    // If no boundaries found or too few, fall back to line-based chunking
    if boundaries.len() < 2 {
        return chunk_lines(file_path, lines);
    }

    // Add end sentinel
    boundaries.push(lines.len());

    let mut chunks = Vec::new();
    for window in boundaries.windows(2) {
        let start = window[0];
        let end = window[1];

        // If the chunk would be too large, split it with line-based chunking
        if end - start > CHUNK_LINES * 2 {
            let sub_lines = &lines[start..end];
            let sub_chunks = chunk_lines(file_path, sub_lines);
            for mut c in sub_chunks {
                c.line_start += start;
                c.line_end += start;
                chunks.push(c);
            }
        } else {
            let mut chunk_content = lines[start..end].join("\n");
            if chunk_content.len() > MAX_CHUNK_CONTENT {
                chunk_content.truncate(MAX_CHUNK_CONTENT);
            }
            let tokens = tokenize(&chunk_content);
            chunks.push(Chunk {
                file_path: file_path.to_string(),
                line_start: start + 1,
                line_end: end,
                content: chunk_content,
                tokens,
                indexed_at: now_secs(),
                hit_count: 0,
                embedding: None,
            });
        }
    }

    if chunks.is_empty() {
        return chunk_lines(file_path, lines);
    }
    chunks
}

/// Markdown-aware chunking: splits at headings
fn chunk_markdown(file_path: &str, lines: &[&str]) -> Vec<Chunk> {
    let mut boundaries: Vec<usize> = vec![0];
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with("# ") || line.starts_with("## ") || line.starts_with("### ") {
            if i > 0 {
                boundaries.push(i);
            }
        }
    }
    boundaries.push(lines.len());
    boundaries.dedup();

    let mut chunks = Vec::new();
    for window in boundaries.windows(2) {
        let start = window[0];
        let end = window[1];
        if start >= end {
            continue;
        }

        let mut chunk_content = lines[start..end].join("\n");
        if chunk_content.len() > MAX_CHUNK_CONTENT {
            chunk_content.truncate(MAX_CHUNK_CONTENT);
        }
        if chunk_content.trim().is_empty() {
            continue;
        }

        let tokens = tokenize(&chunk_content);
        chunks.push(Chunk {
            file_path: file_path.to_string(),
            line_start: start + 1,
            line_end: end,
            content: chunk_content,
            tokens,
            indexed_at: now_secs(),
            hit_count: 0,
            embedding: None,
        });
    }

    if chunks.is_empty() {
        return chunk_lines(file_path, lines);
    }
    chunks
}

fn chunk_lines(file_path: &str, lines: &[&str]) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < lines.len() {
        let end = (start + CHUNK_LINES).min(lines.len());
        let chunk_content_lines = &lines[start..end];
        let mut chunk_content = chunk_content_lines.join("\n");

        if chunk_content.len() > MAX_CHUNK_CONTENT {
            let mut trunc_at = MAX_CHUNK_CONTENT;
            while trunc_at > 0 && !chunk_content.is_char_boundary(trunc_at) {
                trunc_at -= 1;
            }
            chunk_content.truncate(trunc_at);
        }

        let tokens = tokenize(&chunk_content);

        chunks.push(Chunk {
            file_path: file_path.to_string(),
            line_start: start + 1,
            line_end: end,
            content: chunk_content,
            tokens,
            indexed_at: now_secs(),
            hit_count: 0,
            embedding: None,
        });

        if end == lines.len() {
            break;
        }
        start = end.saturating_sub(OVERLAP_LINES).max(start + 1);
    }

    chunks
}

// ===== Query scoring =====

fn score_chunk(chunk: &Chunk, query_tokens: &[String]) -> f64 {
    if query_tokens.is_empty() || chunk.tokens.is_empty() {
        return 0.0;
    }

    // Build frequency map for chunk tokens
    let mut chunk_freq: HashMap<&str, usize> = HashMap::new();
    for token in &chunk.tokens {
        *chunk_freq.entry(token.as_str()).or_insert(0) += 1;
    }

    // Score by term frequency of matching query tokens
    let mut score: f64 = 0.0;
    let mut matched_terms = 0usize;

    for qt in query_tokens {
        if let Some(&count) = chunk_freq.get(qt.as_str()) {
            score += count as f64;
            matched_terms += 1;
        }
    }

    // Bonus for matching more distinct query terms
    if matched_terms > 1 {
        score *= 1.0 + (matched_terms as f64 * 0.2);
    }

    score
}

// ===== Embedding & Semantic Search =====

/// Cosine similarity between two vectors
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

/// Score chunks using semantic similarity when embeddings are available,
/// with keyword TF fallback/hybrid.
fn score_chunk_hybrid(
    chunk: &Chunk,
    query_tokens: &[String],
    query_embedding: Option<&Vec<f32>>,
) -> f64 {
    let keyword_score = score_chunk(chunk, query_tokens);

    match (query_embedding, &chunk.embedding) {
        (Some(qe), Some(ce)) => {
            let semantic = cosine_similarity(qe, ce) as f64;
            // Hybrid: 70% semantic + 30% keyword (normalized)
            let norm_keyword = keyword_score.min(20.0) / 20.0;
            0.7 * semantic.max(0.0) + 0.3 * norm_keyword
        }
        _ => keyword_score,
    }
}

#[derive(Debug, Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

/// Call an OpenAI-compatible /v1/embeddings endpoint to get embeddings
async fn fetch_embeddings(
    base_url: &str,
    model: &str,
    texts: &[String],
    api_key: Option<&str>,
) -> Result<Vec<Vec<f32>>, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/v1/embeddings", base_url.trim_end_matches('/'));

    let mut req = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "model": model,
            "input": texts,
        }));

    if let Some(key) = api_key {
        if !key.is_empty() {
            req = req.header("Authorization", format!("Bearer {}", key));
        }
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("Embedding request failed: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("Embedding API error {}: {}", status, body));
    }

    let emb_resp: EmbeddingResponse = resp
        .json()
        .await
        .map_err(|e| format!("Failed to parse embedding response: {}", e))?;

    Ok(emb_resp.data.into_iter().map(|d| d.embedding).collect())
}

/// Embedding configuration stored alongside the RAG state
#[derive(Debug, Clone, Serialize)]
pub struct EmbeddingConfig {
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub batch_size: usize,
    pub dimensions: usize,
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            base_url: "http://localhost:1234".to_string(),
            model: "bge-small-en-v1.5".to_string(),
            api_key: String::new(),
            batch_size: 32,
            dimensions: 384,
        }
    }
}

fn format_results(scored: &[(usize, f64)], chunks: &[Chunk]) -> String {
    let mut output = String::new();

    for (i, &(chunk_idx, _score)) in scored.iter().enumerate() {
        let chunk = &chunks[chunk_idx];

        if i > 0 {
            output.push_str("\n\n");
        }

        output.push_str(&format!(
            "<file path=\"{}\" start_line=\"{}\" end_line=\"{}\">\n{}\n</file>",
            chunk.file_path, chunk.line_start, chunk.line_end, chunk.content
        ));
    }

    output
}

// ===== Timestamp =====

fn now_timestamp() -> String {
    let duration = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    // Format as simple UTC-ish timestamp: YYYY-MM-DD HH:MM:SS
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Simple date calculation from epoch days
    let mut y = 1970i64;
    let mut remaining_days = days as i64;

    loop {
        let days_in_year = if is_leap_year(y) { 366 } else { 365 };
        if remaining_days < days_in_year {
            break;
        }
        remaining_days -= days_in_year;
        y += 1;
    }

    let month_days: [i64; 12] = if is_leap_year(y) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };

    let mut m = 0usize;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining_days < md {
            m = i;
            break;
        }
        remaining_days -= md;
    }

    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        y,
        m + 1,
        remaining_days + 1,
        hours,
        minutes,
        seconds
    )
}

fn is_leap_year(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || (y % 400 == 0)
}

// ===== Tests =====

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenize_basic() {
        let tokens = tokenize("hello world");
        assert_eq!(tokens, vec!["hello", "world"]);
    }

    #[test]
    fn test_tokenize_code() {
        let tokens = tokenize("fn main() { let x_val = 42; }");
        assert!(tokens.contains(&"main".to_string()));
        assert!(tokens.contains(&"x_val".to_string()));
        assert!(tokens.contains(&"42".to_string()));
        // Single chars like "x" or "fn" should be included (len >= 2)
        assert!(tokens.contains(&"fn".to_string()));
    }

    #[test]
    fn test_tokenize_filters_short_words() {
        let tokens = tokenize("a b c de fg");
        assert!(!tokens.contains(&"a".to_string()));
        assert!(!tokens.contains(&"b".to_string()));
        assert!(tokens.contains(&"de".to_string()));
        assert!(tokens.contains(&"fg".to_string()));
    }

    #[test]
    fn test_tokenize_lowercase() {
        let tokens = tokenize("Hello WORLD FooBar");
        assert!(tokens.contains(&"hello".to_string()));
        assert!(tokens.contains(&"world".to_string()));
        assert!(tokens.contains(&"foobar".to_string()));
    }

    #[test]
    fn test_tokenize_empty() {
        assert!(tokenize("").is_empty());
        assert!(tokenize("   ").is_empty());
    }

    #[test]
    fn test_is_text_file_known_extensions() {
        assert!(is_text_file(Path::new("main.rs")));
        assert!(is_text_file(Path::new("app.tsx")));
        assert!(is_text_file(Path::new("style.css")));
        assert!(is_text_file(Path::new("data.json")));
        assert!(is_text_file(Path::new("script.py")));
        assert!(is_text_file(Path::new("query.sql")));
    }

    #[test]
    fn test_is_text_file_unknown_extensions() {
        assert!(!is_text_file(Path::new("image.png")));
        assert!(!is_text_file(Path::new("binary.exe")));
        assert!(!is_text_file(Path::new("archive.tar.gz")));
    }

    #[test]
    fn test_is_text_file_special_names() {
        assert!(is_text_file(Path::new("Makefile")));
        assert!(is_text_file(Path::new("Dockerfile")));
        assert!(is_text_file(Path::new("LICENSE")));
    }

    #[test]
    fn test_score_chunk_no_match() {
        let chunk = Chunk {
            file_path: "test.rs".to_string(),
            line_start: 1,
            line_end: 10,
            content: "fn main() {}".to_string(),
            tokens: tokenize("fn main"),
            indexed_at: 0,
            hit_count: 0,
            embedding: None,
        };
        let query = tokenize("nonexistent");
        assert_eq!(score_chunk(&chunk, &query), 0.0);
    }

    #[test]
    fn test_score_chunk_single_match() {
        let chunk = Chunk {
            file_path: "test.rs".to_string(),
            line_start: 1,
            line_end: 10,
            content: "fn main() {}".to_string(),
            tokens: tokenize("fn main"),
            indexed_at: 0,
            hit_count: 0,
            embedding: None,
        };
        let query = tokenize("main");
        assert!(score_chunk(&chunk, &query) > 0.0);
    }

    #[test]
    fn test_score_chunk_multi_match_bonus() {
        let chunk = Chunk {
            file_path: "test.rs".to_string(),
            line_start: 1,
            line_end: 10,
            content: "fn main hello world".to_string(),
            tokens: tokenize("fn main hello world"),
            indexed_at: 0,
            hit_count: 0,
            embedding: None,
        };
        let single_score = score_chunk(&chunk, &tokenize("main"));
        let multi_score = score_chunk(&chunk, &tokenize("main hello"));
        // Multi-match should score higher due to bonus
        assert!(multi_score > single_score);
    }

    #[test]
    fn test_score_chunk_empty_query() {
        let chunk = Chunk {
            file_path: "test.rs".to_string(),
            line_start: 1,
            line_end: 10,
            content: "fn main".to_string(),
            tokens: tokenize("fn main"),
            indexed_at: 0,
            hit_count: 0,
            embedding: None,
        };
        assert_eq!(score_chunk(&chunk, &[]), 0.0);
    }

    #[test]
    fn test_score_chunk_frequency() {
        let chunk = Chunk {
            file_path: "test.rs".to_string(),
            line_start: 1,
            line_end: 10,
            content: "error error error ok".to_string(),
            tokens: tokenize("error error error ok"),
            indexed_at: 0,
            hit_count: 0,
            embedding: None,
        };
        let query = tokenize("error");
        // 3 occurrences of "error"
        assert_eq!(score_chunk(&chunk, &query), 3.0);
    }

    #[test]
    fn test_chunk_file_with_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.rs");
        std::fs::write(&file_path, "line 1\nline 2\nline 3\n").unwrap();

        let chunks = chunk_file(file_path.to_str().unwrap());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].line_start, 1);
        assert_eq!(chunks[0].line_end, 3);
        assert!(chunks[0].content.contains("line 1"));
    }

    #[test]
    fn test_chunk_file_multiple_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("big.rs");
        let content: String = (1..=400).map(|i| format!("line {}\n", i)).collect();
        std::fs::write(&file_path, &content).unwrap();

        let chunks = chunk_file(file_path.to_str().unwrap());
        // 400 lines, 150 lines per chunk, 30 overlap:
        // Chunk 1: 1..150
        // Chunk 2: 121..270
        // Chunk 3: 241..390
        // Chunk 4: 361..400
        assert_eq!(chunks.len(), 4);
        assert_eq!(chunks[0].line_start, 1);
        assert_eq!(chunks[0].line_end, 150);
        assert_eq!(chunks[1].line_start, 121);
        assert_eq!(chunks[1].line_end, 270);
        assert_eq!(chunks[2].line_start, 241);
        assert_eq!(chunks[2].line_end, 390);
        assert_eq!(chunks[3].line_start, 361);
        assert_eq!(chunks[3].line_end, 400);
    }

    #[test]
    fn test_chunk_file_empty() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("empty.rs");
        std::fs::write(&file_path, "").unwrap();

        let chunks = chunk_file(file_path.to_str().unwrap());
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_file_binary_content() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("binary.rs");
        std::fs::write(&file_path, b"hello\0world").unwrap();

        let chunks = chunk_file(file_path.to_str().unwrap());
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_chunk_file_nonexistent() {
        let chunks = chunk_file("/nonexistent/path/file.rs");
        assert!(chunks.is_empty());
    }

    #[test]
    fn test_format_results() {
        let chunks = vec![Chunk {
            file_path: "src/main.rs".to_string(),
            line_start: 1,
            line_end: 10,
            content: "fn main() {}".to_string(),
            tokens: vec!["fn".to_string(), "main".to_string()],
            indexed_at: 0,
            hit_count: 0,
            embedding: None,
        }];
        let scored = vec![(0, 2.5)];
        let output = format_results(&scored, &chunks);
        assert!(output.contains("<file path=\"src/main.rs\""));
        assert!(output.contains("start_line=\"1\""));
        assert!(output.contains("end_line=\"10\""));
        assert!(output.contains("fn main() {}"));
    }

    #[test]
    fn test_cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim + 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_hybrid_scoring_with_embeddings() {
        let mut chunk = Chunk {
            file_path: "test.rs".to_string(),
            line_start: 1,
            line_end: 10,
            content: "fn main() {}".to_string(),
            tokens: tokenize("fn main"),
            indexed_at: 0,
            hit_count: 0,
            embedding: Some(vec![0.8, 0.2, 0.1]),
        };
        let query_tokens = tokenize("main");
        let query_emb = vec![0.7, 0.3, 0.1];

        // With embedding: should get hybrid score
        let hybrid = score_chunk_hybrid(&chunk, &query_tokens, Some(&query_emb));
        assert!(hybrid > 0.0);

        // Without embedding: should fall back to keyword
        chunk.embedding = None;
        let keyword_only = score_chunk_hybrid(&chunk, &query_tokens, Some(&query_emb));
        let pure_keyword = score_chunk(&chunk, &query_tokens);
        assert_eq!(keyword_only, pure_keyword);
    }

    #[test]
    fn test_search_with_embedding() {
        let state = RagState::new();
        {
            let mut idx = state.index.lock().unwrap();
            idx.chunks.push(Chunk {
                file_path: "a.rs".to_string(),
                line_start: 1,
                line_end: 5,
                content: "fn hello_world() {}".to_string(),
                tokens: tokenize("fn hello_world"),
                indexed_at: now_secs(),
                hit_count: 0,
                embedding: Some(vec![0.9, 0.1, 0.0]),
            });
            idx.chunks.push(Chunk {
                file_path: "b.rs".to_string(),
                line_start: 1,
                line_end: 5,
                content: "fn goodbye_world() {}".to_string(),
                tokens: tokenize("fn goodbye_world"),
                indexed_at: now_secs(),
                hit_count: 0,
                embedding: Some(vec![0.1, 0.9, 0.0]),
            });
        }

        // Query embedding close to first chunk
        let qe = vec![0.85, 0.15, 0.0];
        let results = state.search_with_embedding("hello", 2, Some(&qe));
        assert!(!results.is_empty());
        assert_eq!(results[0].file_path, "a.rs");
    }

    #[test]
    fn test_is_leap_year() {
        assert!(is_leap_year(2000)); // divisible by 400
        assert!(is_leap_year(2024)); // divisible by 4, not 100
        assert!(!is_leap_year(1900)); // divisible by 100, not 400
        assert!(!is_leap_year(2023)); // not divisible by 4
    }

    #[test]
    fn test_scan_files_recursive_temp_dir() {
        let dir = tempfile::tempdir().unwrap();
        // Create some files
        std::fs::write(dir.path().join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("lib.rs"), "pub mod foo;").unwrap();
        std::fs::write(dir.path().join("image.png"), "binary data").unwrap();

        let mut files = Vec::new();
        scan_files_recursive(dir.path(), &mut files, 0, MAX_CHUNKS);

        // Should include .rs files, not .png
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.ends_with("main.rs")));
        assert!(files.iter().any(|f| f.ends_with("lib.rs")));
    }

    #[test]
    fn test_scan_files_skips_dirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        std::fs::write(dir.path().join("node_modules/dep.js"), "code").unwrap();
        std::fs::write(dir.path().join("app.js"), "code").unwrap();

        let mut files = Vec::new();
        scan_files_recursive(dir.path(), &mut files, 0, MAX_CHUNKS);

        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("app.js"));
    }

    #[test]
    fn test_scan_files_skips_dotfiles() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "SECRET=x").unwrap();
        std::fs::write(dir.path().join("app.ts"), "code").unwrap();

        let mut files = Vec::new();
        scan_files_recursive(dir.path(), &mut files, 0, MAX_CHUNKS);

        assert_eq!(files.len(), 1);
        assert!(files[0].ends_with("app.ts"));
    }
}

// ===== Tauri Commands =====

#[tauri::command]
pub fn rag_build_index(
    root_path: String,
    state: tauri::State<'_, Arc<RagState>>,
) -> Result<RagStats, String> {
    let root = Path::new(&root_path);
    if !root.exists() || !root.is_dir() {
        return Err(format!("Invalid project root: {}", root_path));
    }

    // Collect files
    let mut files = Vec::new();
    scan_files_recursive(root, &mut files, 0, MAX_CHUNKS);

    // Chunk all files, respecting the budget
    let mut all_chunks: Vec<Chunk> = Vec::new();
    let files_indexed = files.len();

    for file_path in &files {
        if all_chunks.len() >= MAX_CHUNKS {
            break;
        }

        let file_chunks = chunk_file(file_path);
        let remaining = MAX_CHUNKS - all_chunks.len();
        let take = file_chunks.len().min(remaining);
        all_chunks.extend(file_chunks.into_iter().take(take));
    }

    let timestamp = now_timestamp();

    let stats = RagStats {
        files_indexed,
        total_chunks: all_chunks.len(),
        last_index_time: timestamp.clone(),
    };

    // Store in state
    let mut index = state
        .index
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    index.chunks = all_chunks;
    index.files_indexed = files_indexed;
    index.last_index_time = timestamp;

    Ok(stats)
}

#[tauri::command]
pub fn rag_query(
    query: String,
    top_k: Option<usize>,
    state: tauri::State<'_, Arc<RagState>>,
) -> Result<String, String> {
    let k = top_k.unwrap_or(10).min(50);

    if query.trim().is_empty() {
        return Err("Query cannot be empty".to_string());
    }

    let query_tokens = tokenize(&query);
    if query_tokens.is_empty() {
        return Err("Query produced no searchable tokens".to_string());
    }

    let index = state
        .index
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    if index.chunks.is_empty() {
        return Err("Index is empty. Run rag_build_index first.".to_string());
    }

    // Score all chunks
    let mut scored: Vec<(usize, f64)> = index
        .chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| (i, score_chunk(chunk, &query_tokens)))
        .filter(|&(_, score)| score > 0.0)
        .collect();

    // Sort by score descending
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    // Take top-k
    scored.truncate(k);

    if scored.is_empty() {
        return Ok("No matching chunks found for the query.".to_string());
    }

    Ok(format_results(&scored, &index.chunks))
}

#[derive(Debug, Clone, Serialize)]
pub struct RagResult {
    pub file_path: String,
    pub content: String,
    pub score: f64,
    pub line_start: usize,
    pub line_end: usize,
}

#[tauri::command]
pub fn rag_query_structured(
    query: String,
    top_k: Option<usize>,
    state: tauri::State<'_, Arc<RagState>>,
) -> Result<Vec<RagResult>, String> {
    let k = top_k.unwrap_or(5).min(20);

    if query.trim().is_empty() {
        return Ok(vec![]);
    }

    let query_tokens = tokenize(&query);
    if query_tokens.is_empty() {
        return Ok(vec![]);
    }

    let index = state
        .index
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    if index.chunks.is_empty() {
        return Ok(vec![]);
    }

    let mut scored: Vec<(usize, f64)> = index
        .chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| (i, score_chunk(chunk, &query_tokens)))
        .filter(|&(_, score)| score > 0.0)
        .collect();

    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(k);

    Ok(scored
        .iter()
        .map(|&(idx, score)| {
            let chunk = &index.chunks[idx];
            RagResult {
                file_path: chunk.file_path.clone(),
                content: chunk.content.clone(),
                score,
                line_start: chunk.line_start,
                line_end: chunk.line_end,
            }
        })
        .collect())
}

#[tauri::command]
pub fn rag_get_stats(state: tauri::State<'_, Arc<RagState>>) -> Result<RagStats, String> {
    let index = state
        .index
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    Ok(RagStats {
        files_indexed: index.files_indexed,
        total_chunks: index.chunks.len(),
        last_index_time: index.last_index_time.clone(),
    })
}

// ===== Document Indexing =====

const DOC_EXTENSIONS: &[&str] = &[
    "txt", "md", "rst", "org", "pdf", "html", "csv", "log", "json", "yaml", "yml", "xml", "toml",
];

fn scan_doc_files(dir: &Path, files: &mut Vec<String>, depth: usize, chunk_budget: usize) {
    if depth > MAX_DEPTH || files.len() * 2 > chunk_budget {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let mut dirs = Vec::new();
    let mut file_entries = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip dotfiles/dotdirs
        if name.starts_with('.') {
            continue;
        }

        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);

        if is_dir {
            if !SKIP_DIRS.contains(&name.as_str()) {
                dirs.push(entry.path());
            }
        } else {
            file_entries.push(entry.path());
        }
    }

    // Add doc files within size limits
    for path in file_entries {
        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        if !DOC_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }

        let size = match std::fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(_) => continue,
        };

        if size == 0 || size > MAX_FILE_SIZE {
            continue;
        }

        files.push(path.to_string_lossy().to_string());
    }

    // Recurse into subdirectories
    for dir_path in dirs {
        scan_doc_files(&dir_path, files, depth + 1, chunk_budget);
    }
}

#[tauri::command]
pub fn rag_index_documents(
    doc_path: String,
    state: tauri::State<'_, Arc<RagState>>,
) -> Result<RagStats, String> {
    let root = Path::new(&doc_path);
    if !root.exists() || !root.is_dir() {
        return Err(format!("Invalid documentation path: {}", doc_path));
    }

    // Collect doc files
    let mut files = Vec::new();
    scan_doc_files(root, &mut files, 0, MAX_CHUNKS);

    // Chunk all doc files
    let mut doc_chunks: Vec<Chunk> = Vec::new();
    let files_indexed = files.len();

    for file_path in &files {
        if doc_chunks.len() >= MAX_CHUNKS {
            break;
        }

        let file_chunks = chunk_file(file_path);
        let remaining = MAX_CHUNKS - doc_chunks.len();
        let take = file_chunks.len().min(remaining);
        doc_chunks.extend(file_chunks.into_iter().take(take));
    }

    let timestamp = now_timestamp();

    // Add to existing index (not replace)
    let mut index = state
        .index
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    index.chunks.extend(doc_chunks.iter().cloned());
    index.files_indexed += files_indexed;
    index.last_index_time = timestamp.clone();

    let total_chunks = index.chunks.len();
    let total_files = index.files_indexed;

    Ok(RagStats {
        files_indexed: total_files,
        total_chunks,
        last_index_time: timestamp,
    })
}

// ===== Documents Folder Management =====

#[derive(Debug, Clone, Serialize)]
pub struct DocFileInfo {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub extension: String,
    pub subfolder: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DocumentsFolderInfo {
    pub path: String,
    pub created: bool,
    pub files: Vec<DocFileInfo>,
    pub subfolders: Vec<String>,
    pub total_size: u64,
}

fn scan_documents_tree(
    dir: &Path,
    base: &Path,
    files: &mut Vec<DocFileInfo>,
    subfolders: &mut Vec<String>,
    depth: usize,
) {
    if depth > 5 {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }

        let path = entry.path();
        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);

        if is_dir {
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            subfolders.push(rel);
            scan_documents_tree(&path, base, files, subfolders, depth + 1);
        } else {
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if !DOC_EXTENSIONS.contains(&ext.as_str()) && !TEXT_EXTENSIONS.contains(&ext.as_str()) {
                continue;
            }
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            if size == 0 || size > MAX_FILE_SIZE * 10 {
                continue;
            }
            let subfolder = path
                .parent()
                .and_then(|p| p.strip_prefix(base).ok())
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            files.push(DocFileInfo {
                name,
                path: path.to_string_lossy().to_string(),
                size,
                extension: ext,
                subfolder,
            });
        }
    }
}

#[tauri::command]
pub fn ensure_documents_folder(root_path: String) -> Result<DocumentsFolderInfo, String> {
    let docs_path = Path::new(&root_path).join("Documents");
    let created = if !docs_path.exists() {
        std::fs::create_dir_all(&docs_path)
            .map_err(|e| format!("Cannot create Documents folder: {}", e))?;
        true
    } else {
        false
    };

    let mut files = Vec::new();
    let mut subfolders = Vec::new();
    scan_documents_tree(&docs_path, &docs_path, &mut files, &mut subfolders, 0);

    let total_size: u64 = files.iter().map(|f| f.size).sum();

    Ok(DocumentsFolderInfo {
        path: docs_path.to_string_lossy().to_string(),
        created,
        files,
        subfolders,
        total_size,
    })
}

#[tauri::command]
pub fn rag_list_documents(root_path: String) -> Result<Vec<DocFileInfo>, String> {
    let docs_path = Path::new(&root_path).join("Documents");
    if !docs_path.exists() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    let mut subfolders = Vec::new();
    scan_documents_tree(&docs_path, &docs_path, &mut files, &mut subfolders, 0);
    files.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(files)
}

#[tauri::command]
pub fn rag_auto_clean(
    max_age_days: Option<u64>,
    delete_unused: Option<bool>,
    state: tauri::State<'_, Arc<RagState>>,
) -> Result<serde_json::Value, String> {
    let max_age_secs = max_age_days.unwrap_or(14) * 86400;
    let delete_never_accessed = delete_unused.unwrap_or(true);
    let (deleted_age, deleted_unused) = state.auto_clean(max_age_secs, delete_never_accessed);
    let stats = state.get_stats();
    Ok(serde_json::json!({
        "deleted_expired": deleted_age,
        "deleted_unused": deleted_unused,
        "remaining_chunks": stats.total_chunks,
        "remaining_files": stats.files_indexed,
    }))
}

// ===== Embedding Tauri Commands =====

#[derive(Debug, Clone, Serialize)]
pub struct EmbeddingStats {
    pub total_chunks: usize,
    pub embedded_chunks: usize,
    pub failed_chunks: usize,
    pub batches_processed: usize,
    pub model: String,
}

#[tauri::command]
pub fn rag_configure_embeddings(
    base_url: Option<String>,
    model: Option<String>,
    api_key: Option<String>,
    batch_size: Option<usize>,
    state: tauri::State<'_, Arc<RagState>>,
) -> Result<EmbeddingConfig, String> {
    let mut cfg = state
        .embedding_config
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    if let Some(url) = base_url {
        cfg.base_url = url;
    }
    if let Some(m) = model {
        cfg.model = m;
    }
    if let Some(k) = api_key {
        cfg.api_key = k;
    }
    if let Some(bs) = batch_size {
        cfg.batch_size = bs.max(1).min(256);
    }

    Ok(cfg.clone())
}

#[tauri::command]
pub fn rag_get_embedding_config(
    state: tauri::State<'_, Arc<RagState>>,
) -> Result<EmbeddingConfig, String> {
    let cfg = state
        .embedding_config
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    Ok(cfg.clone())
}

#[tauri::command]
pub async fn rag_embed_chunks(
    state: tauri::State<'_, Arc<RagState>>,
) -> Result<EmbeddingStats, String> {
    // Read config
    let (base_url, model, api_key, batch_size) = {
        let cfg = state
            .embedding_config
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        (
            cfg.base_url.clone(),
            cfg.model.clone(),
            cfg.api_key.clone(),
            cfg.batch_size,
        )
    };

    // Collect chunks that need embeddings
    let (texts, indices): (Vec<String>, Vec<usize>) = {
        let index = state
            .index
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;

        index
            .chunks
            .iter()
            .enumerate()
            .filter(|(_, c)| c.embedding.is_none())
            .map(|(i, c)| {
                // Use first 512 chars of content for embedding (reasonable context window)
                let text = if c.content.len() > 512 {
                    c.content[..512].to_string()
                } else {
                    c.content.clone()
                };
                (text, i)
            })
            .unzip()
    };

    if texts.is_empty() {
        let index = state
            .index
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        return Ok(EmbeddingStats {
            total_chunks: index.chunks.len(),
            embedded_chunks: index
                .chunks
                .iter()
                .filter(|c| c.embedding.is_some())
                .count(),
            failed_chunks: 0,
            batches_processed: 0,
            model,
        });
    }

    let api_key_opt = if api_key.is_empty() {
        None
    } else {
        Some(api_key.as_str())
    };

    let mut _total_embedded = 0usize;
    let mut total_failed = 0usize;
    let mut batches = 0usize;

    // Process in batches
    for batch_start in (0..texts.len()).step_by(batch_size) {
        let batch_end = (batch_start + batch_size).min(texts.len());
        let batch_texts: Vec<String> = texts[batch_start..batch_end].to_vec();
        let batch_indices: Vec<usize> = indices[batch_start..batch_end].to_vec();

        match fetch_embeddings(&base_url, &model, &batch_texts, api_key_opt).await {
            Ok(embeddings) => {
                let mut index = state
                    .index
                    .lock()
                    .map_err(|e| format!("Lock error: {}", e))?;

                for (j, emb) in embeddings.into_iter().enumerate() {
                    if j < batch_indices.len() {
                        let chunk_idx = batch_indices[j];
                        if chunk_idx < index.chunks.len() {
                            index.chunks[chunk_idx].embedding = Some(emb);
                            _total_embedded += 1;
                        }
                    }
                }
                batches += 1;
            }
            Err(e) => {
                log::warn!("Embedding batch {} failed: {}", batches, e);
                total_failed += batch_indices.len();
                batches += 1;
            }
        }
    }

    let index = state
        .index
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    Ok(EmbeddingStats {
        total_chunks: index.chunks.len(),
        embedded_chunks: index
            .chunks
            .iter()
            .filter(|c| c.embedding.is_some())
            .count(),
        failed_chunks: total_failed,
        batches_processed: batches,
        model,
    })
}

#[tauri::command]
pub async fn rag_semantic_search(
    query: String,
    top_k: Option<usize>,
    state: tauri::State<'_, Arc<RagState>>,
) -> Result<Vec<RagResult>, String> {
    let k = top_k.unwrap_or(5).min(20);

    if query.trim().is_empty() {
        return Ok(vec![]);
    }

    // Get embedding config and embed the query
    let (base_url, model, api_key) = {
        let cfg = state
            .embedding_config
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        (cfg.base_url.clone(), cfg.model.clone(), cfg.api_key.clone())
    };

    let api_key_opt = if api_key.is_empty() {
        None
    } else {
        Some(api_key.as_str())
    };

    // Try to get query embedding
    let query_embedding =
        match fetch_embeddings(&base_url, &model, &[query.clone()], api_key_opt).await {
            Ok(mut embs) if !embs.is_empty() => Some(embs.remove(0)),
            Ok(_) => None,
            Err(e) => {
                log::warn!(
                    "Query embedding failed, falling back to keyword search: {}",
                    e
                );
                None
            }
        };

    let results = state.search_with_embedding(&query, k, query_embedding.as_ref());

    // Re-score for the response
    let query_tokens = tokenize(&query);
    Ok(results
        .iter()
        .map(|chunk| {
            let score = score_chunk_hybrid(chunk, &query_tokens, query_embedding.as_ref());
            RagResult {
                file_path: chunk.file_path.clone(),
                content: chunk.content.clone(),
                score,
                line_start: chunk.line_start,
                line_end: chunk.line_end,
            }
        })
        .collect())
}

#[tauri::command]
pub fn rag_embedding_stats(
    state: tauri::State<'_, Arc<RagState>>,
) -> Result<serde_json::Value, String> {
    let index = state
        .index
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    let total = index.chunks.len();
    let embedded = index
        .chunks
        .iter()
        .filter(|c| c.embedding.is_some())
        .count();
    let dimensions = index
        .chunks
        .iter()
        .find_map(|c| c.embedding.as_ref())
        .map(|e| e.len())
        .unwrap_or(0);

    let cfg = state
        .embedding_config
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;

    Ok(serde_json::json!({
        "total_chunks": total,
        "embedded_chunks": embedded,
        "coverage_percent": if total > 0 { (embedded * 100) / total } else { 0 },
        "dimensions": dimensions,
        "model": cfg.model,
        "base_url": cfg.base_url,
    }))
}

// ===== Continuous RAG Indexing (File Watcher) =====

/// Incrementally re-index a single file: remove old chunks for the file, add new ones.
fn reindex_file(state: &RagState, file_path: &str) {
    let path = Path::new(file_path);

    // Skip non-text files, binary files, and files in skip dirs
    if !path.is_file() || !is_text_file(path) {
        return;
    }
    if let Ok(meta) = std::fs::metadata(path) {
        if meta.len() == 0 || meta.len() > MAX_FILE_SIZE {
            return;
        }
    }

    // Check if path is inside a skip directory
    let path_str = file_path.to_lowercase();
    for skip in SKIP_DIRS {
        if path_str.contains(&format!("/{}/", skip)) || path_str.contains(&format!("\\{}\\", skip))
        {
            return;
        }
    }

    let new_chunks = chunk_file(file_path);

    if let Ok(mut index) = state.index.lock() {
        // Remove old chunks for this file
        index.chunks.retain(|c| c.file_path != file_path);
        // Add new chunks (respect budget)
        let remaining = MAX_CHUNKS.saturating_sub(index.chunks.len());
        let take = new_chunks.len().min(remaining);
        index.chunks.extend(new_chunks.into_iter().take(take));
        // Update stats
        let mut unique_files = std::collections::HashSet::new();
        for chunk in &index.chunks {
            unique_files.insert(chunk.file_path.as_str());
        }
        index.files_indexed = unique_files.len();
        index.last_index_time = now_timestamp();
    }
}

/// Remove all chunks for a deleted file.
fn remove_file_chunks(state: &RagState, file_path: &str) {
    if let Ok(mut index) = state.index.lock() {
        index.chunks.retain(|c| c.file_path != file_path);
        let mut unique_files = std::collections::HashSet::new();
        for chunk in &index.chunks {
            unique_files.insert(chunk.file_path.as_str());
        }
        index.files_indexed = unique_files.len();
    }
}

#[tauri::command]
pub fn rag_watch_start(
    root_path: String,
    state: tauri::State<'_, Arc<RagState>>,
) -> Result<String, String> {
    let root = Path::new(&root_path);
    if !root.exists() || !root.is_dir() {
        return Err(format!("Invalid project root: {}", root_path));
    }

    // Stop any existing watcher
    if let Ok(mut w) = state.watcher.lock() {
        *w = None;
    }

    let rag_state = Arc::clone(&state);
    let root_clone = root_path.clone();

    // Use a debounce channel: collect events for 500ms before processing
    let (tx, rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();

    let mut watcher = RecommendedWatcher::new(
        move |res| {
            let _ = tx.send(res);
        },
        notify::Config::default().with_poll_interval(std::time::Duration::from_secs(2)),
    )
    .map_err(|e| format!("Failed to create file watcher: {}", e))?;

    watcher
        .watch(root.as_ref(), RecursiveMode::Recursive)
        .map_err(|e| format!("Failed to watch directory: {}", e))?;

    // Background thread to process file events with debouncing
    std::thread::spawn(move || {
        use std::collections::HashSet;
        let debounce = std::time::Duration::from_millis(500);

        loop {
            // Wait for first event
            let event = match rx.recv() {
                Ok(Ok(ev)) => ev,
                Ok(Err(e)) => {
                    log::warn!("[rag-watcher] Watch error: {}", e);
                    continue;
                }
                Err(_) => break, // Channel closed, watcher dropped
            };

            // Collect more events within debounce window
            let mut changed_files = HashSet::new();
            let mut removed_files = HashSet::new();

            // Process initial event
            for path in &event.paths {
                let p = path.to_string_lossy().to_string();
                if event.kind.is_remove() {
                    removed_files.insert(p);
                } else {
                    changed_files.insert(p);
                }
            }

            // Drain pending events within debounce window
            let deadline = std::time::Instant::now() + debounce;
            loop {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match rx.recv_timeout(remaining) {
                    Ok(Ok(ev)) => {
                        for path in &ev.paths {
                            let p = path.to_string_lossy().to_string();
                            if ev.kind.is_remove() {
                                removed_files.insert(p);
                            } else {
                                changed_files.insert(p);
                            }
                        }
                    }
                    Ok(Err(_)) => continue,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
                }
            }

            // Process removals
            for path in &removed_files {
                remove_file_chunks(&rag_state, path);
            }

            // Process changes (creates + modifies)
            for path in &changed_files {
                if !removed_files.contains(path) {
                    reindex_file(&rag_state, path);
                }
            }

            if !changed_files.is_empty() || !removed_files.is_empty() {
                log::info!(
                    "[rag-watcher] Incremental reindex: {} changed, {} removed",
                    changed_files.len(),
                    removed_files.len()
                );
            }
        }
        log::info!("[rag-watcher] File watcher stopped for {}", root_clone);
    });

    if let Ok(mut w) = state.watcher.lock() {
        *w = Some(watcher);
    }
    if let Ok(mut r) = state.watched_root.lock() {
        *r = Some(root_path.clone());
    }

    log::info!("[rag-watcher] Started watching: {}", root_path);
    Ok(format!("File watcher started for {}", root_path))
}

#[tauri::command]
pub fn rag_watch_stop(state: tauri::State<'_, Arc<RagState>>) -> Result<String, String> {
    if let Ok(mut w) = state.watcher.lock() {
        *w = None;
    }
    let root = if let Ok(mut r) = state.watched_root.lock() {
        r.take().unwrap_or_default()
    } else {
        String::new()
    };
    log::info!("[rag-watcher] Stopped watching: {}", root);
    Ok(format!("File watcher stopped for {}", root))
}

#[tauri::command]
pub fn rag_watch_status(
    state: tauri::State<'_, Arc<RagState>>,
) -> Result<serde_json::Value, String> {
    let watching = state.watcher.lock().map(|w| w.is_some()).unwrap_or(false);
    let root = state.watched_root.lock().map(|r| r.clone()).unwrap_or(None);
    Ok(serde_json::json!({
        "watching": watching,
        "root_path": root,
    }))
}

/// Extract text from PDF using pdftotext (poppler-utils) if available.
fn read_pdf_text(path: &str) -> Option<String> {
    let output = {
        let mut cmd = std::process::Command::new("pdftotext");
        cmd.args(["-layout", "-nopgbrk", path, "-"]);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
    }
    .ok()?;
    if output.status.success() {
        Some(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        None
    }
}
