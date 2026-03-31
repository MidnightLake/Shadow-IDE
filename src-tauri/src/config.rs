use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::UNIX_EPOCH;

/// Centralized ShadowIDE configuration loaded from `shadow-ide.toml`.
/// All fields have sensible defaults so the app works without a config file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ShadowConfig {
    pub tools: ToolsConfig,
    pub self_healing: SelfHealingConfig,
    pub compaction: CompactionConfig,
    pub lmcache: LmCacheConfig,
    pub rag: RagConfig,
    pub ai: AiConfig,
    pub security: SecurityConfig,
    /// Warn at 80% and block at 100% of daily spend (USD). None = no limit.
    pub max_daily_spend: Option<f64>,
    /// Privacy mode: same as air_gap but also disables telemetry/analytics.
    pub privacy_mode: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SecurityConfig {
    /// Air-gap mode: only allow local providers (Ollama, llama.cpp, etc.)
    pub air_gap: bool,
    /// Whether pre_tool_use hooks are enabled
    pub pre_tool_hooks_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolsConfig {
    /// Commands allowed in shell_exec (empty = use built-in allowlist)
    pub extra_allowed_commands: Vec<String>,
    /// Skip confirmation for None/Low risk tools
    pub auto_yes_low_risk: bool,
    /// Always confirm Medium risk tools
    pub confirm_medium: bool,
    /// Always confirm High risk tools
    pub confirm_high: bool,
    /// Max concurrent parallel tool calls
    pub parallel_max: usize,
    /// Max characters in tool results before trimming
    pub max_tool_result_chars: usize,
    /// Default timeout for shell_exec (seconds)
    pub shell_timeout_secs: u64,
    /// Default timeout for HTTP requests (seconds)
    pub http_timeout_secs: u64,
    /// Summarize large tool results automatically
    pub auto_summarize_large_results: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SelfHealingConfig {
    pub enabled: bool,
    pub max_attempts: u32,
    /// Ask user if all strategies exhausted
    pub escalate_on_exhaustion: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CompactionConfig {
    pub enabled: bool,
    /// Compact when context usage exceeds this fraction (0.0–1.0)
    pub trigger_at_percent: f32,
    /// Always keep last N turns verbatim
    pub keep_last_turns: usize,
    /// Preserve code blocks during compaction
    pub keep_code_blocks: bool,
    /// Extract memories before compacting
    pub extract_memories: bool,
    /// Max chars per inline-summarized turn
    pub inline_summary_max_chars: usize,
    /// Compact every N tool-calling iterations
    pub turn_interval: usize,
    /// Strategy: "summarize" (default), "keyfacts", "selective"
    pub strategy: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LmCacheConfig {
    pub enabled: bool,
    /// Max in-memory hot cache entries
    pub hot_capacity: usize,
    /// Cache TTL in seconds
    pub ttl_seconds: u64,
    /// Minimum output tokens to cache a response
    pub min_tokens_to_cache: usize,
    /// Cache tool-enabled conversations
    pub cache_tool_results: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct RagConfig {
    pub enabled: bool,
    /// Lines per chunk
    pub chunk_lines: usize,
    /// Overlap lines between chunks
    pub overlap_lines: usize,
    /// Max total chunks in index
    pub max_chunks: usize,
    /// Max file size to index (bytes)
    pub max_file_size: usize,
    /// Max directory recursion depth
    pub max_depth: usize,
    /// Number of RAG results injected per query
    pub top_k: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    /// Default LLM base URL
    pub default_base_url: String,
    /// HTTP client timeout (seconds)
    pub client_timeout_secs: u64,
    /// Default temperature
    pub default_temperature: f64,
    /// Default max context tokens
    pub default_max_context: usize,
    /// Max tool-calling iterations (auto mode)
    pub max_iterations_auto: usize,
    /// Max tool-calling iterations (plan/build modes)
    pub max_iterations_other: usize,
    /// Min messages to keep during compaction
    pub min_keep_messages: usize,
}

// ===== Defaults =====

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            air_gap: false,
            pre_tool_hooks_enabled: true,
        }
    }
}

impl Default for ShadowConfig {
    fn default() -> Self {
        Self {
            tools: ToolsConfig::default(),
            self_healing: SelfHealingConfig::default(),
            compaction: CompactionConfig::default(),
            lmcache: LmCacheConfig::default(),
            rag: RagConfig::default(),
            ai: AiConfig::default(),
            security: SecurityConfig::default(),
            max_daily_spend: None,
            privacy_mode: false,
        }
    }
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            extra_allowed_commands: Vec::new(),
            auto_yes_low_risk: true,
            confirm_medium: true,
            confirm_high: true,
            parallel_max: 4,
            max_tool_result_chars: 12000,
            shell_timeout_secs: 30,
            http_timeout_secs: 15,
            auto_summarize_large_results: true,
        }
    }
}

impl Default for SelfHealingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_attempts: 4,
            escalate_on_exhaustion: true,
        }
    }
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            trigger_at_percent: 0.85,
            keep_last_turns: 12,
            keep_code_blocks: true,
            extract_memories: true,
            inline_summary_max_chars: 500,
            turn_interval: 25,
            strategy: "keyfacts".to_string(),
        }
    }
}

impl Default for LmCacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            hot_capacity: 200,
            ttl_seconds: 300,
            min_tokens_to_cache: 50,
            cache_tool_results: true,
        }
    }
}

impl Default for RagConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            chunk_lines: 150,
            overlap_lines: 30,
            max_chunks: 10000,
            max_file_size: 1024 * 1024,
            max_depth: 12,
            top_k: 5,
        }
    }
}

impl Default for AiConfig {
    fn default() -> Self {
        Self {
            default_base_url: "http://localhost:1234/v1".to_string(),
            client_timeout_secs: 600,
            default_temperature: 0.7,
            default_max_context: 120000,
            max_iterations_auto: 200,
            max_iterations_other: 15,
            min_keep_messages: 8,
        }
    }
}

// ===== Loading =====

impl ShadowConfig {
    /// Clamp numeric config values to valid ranges so bad TOML values
    /// cannot cause panics or logic errors downstream.
    pub fn validate(&mut self) {
        // Compaction trigger: must be a fraction 0.0..=1.0
        self.compaction.trigger_at_percent = self.compaction.trigger_at_percent.clamp(0.0, 1.0);

        // Timeouts: must be positive (at least 1 second)
        self.tools.shell_timeout_secs = self.tools.shell_timeout_secs.max(1);
        self.tools.http_timeout_secs = self.tools.http_timeout_secs.max(1);
        self.ai.client_timeout_secs = self.ai.client_timeout_secs.max(1);
        self.lmcache.ttl_seconds = self.lmcache.ttl_seconds.max(1);

        // Parallelism: at least 1
        self.tools.parallel_max = self.tools.parallel_max.max(1);

        // AI temperature: 0.0..=2.0 (common LLM range)
        self.ai.default_temperature = self.ai.default_temperature.clamp(0.0, 2.0);

        // Iteration limits: at least 1
        self.ai.max_iterations_auto = self.ai.max_iterations_auto.max(1);
        self.ai.max_iterations_other = self.ai.max_iterations_other.max(1);
        self.ai.min_keep_messages = self.ai.min_keep_messages.max(1);

        // Self-healing: at least 1 attempt
        self.self_healing.max_attempts = self.self_healing.max_attempts.max(1);

        // RAG sanity
        self.rag.chunk_lines = self.rag.chunk_lines.max(1);
        self.rag.top_k = self.rag.top_k.max(1);
        self.rag.max_depth = self.rag.max_depth.max(1);
    }

    /// Load config from a TOML file, falling back to defaults for missing fields.
    pub fn load(path: &Path) -> Self {
        if !path.exists() {
            return Self::default();
        }
        let mut config = match std::fs::read_to_string(path) {
            Ok(content) => match toml::from_str(&content) {
                Ok(config) => config,
                Err(e) => {
                    eprintln!(
                        "[config] Failed to parse {}: {}. Using defaults.",
                        path.display(),
                        e
                    );
                    Self::default()
                }
            },
            Err(e) => {
                eprintln!(
                    "[config] Failed to read {}: {}. Using defaults.",
                    path.display(),
                    e
                );
                Self::default()
            }
        };
        config.validate();
        config
    }

    /// Load config from a project root directory (looks for `shadow-ide.toml`).
    pub fn load_from_project(root: &str) -> Self {
        let path = Path::new(root).join("shadow-ide.toml");
        Self::load(&path)
    }

    /// Save current config to a TOML file.
    /// Writes to a `.tmp` file first, then atomically renames to prevent
    /// corruption if the process crashes mid-write.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let content = toml::to_string_pretty(self)
            .map_err(|e| format!("Failed to serialize config: {}", e))?;

        let tmp_path = path.with_extension("toml.tmp");
        std::fs::write(&tmp_path, &content)
            .map_err(|e| format!("Failed to write temp config file: {}", e))?;
        std::fs::rename(&tmp_path, path).map_err(|e| {
            // Clean up the temp file on rename failure
            let _ = std::fs::remove_file(&tmp_path);
            format!("Failed to atomically replace config file: {}", e)
        })
    }

    /// Generate a default config file with comments.
    pub fn generate_default_toml() -> String {
        r#"# ShadowIDE Configuration
# Place this file as `shadow-ide.toml` in your project root.

[tools]
# extra_allowed_commands = ["docker", "kubectl"]
auto_yes_low_risk = true
confirm_medium = true
confirm_high = true
parallel_max = 4
max_tool_result_chars = 12000
shell_timeout_secs = 30
http_timeout_secs = 15

[self_healing]
enabled = true
max_attempts = 4
escalate_on_exhaustion = true

[compaction]
enabled = true
trigger_at_percent = 0.85
keep_last_turns = 12
keep_code_blocks = true
extract_memories = true
inline_summary_max_chars = 500
turn_interval = 25
# Strategy: "keyfacts" (extract bullet points), "summarize" (inline summarization), "selective" (importance scoring)
strategy = "keyfacts"

[lmcache]
enabled = true
hot_capacity = 200
ttl_seconds = 300
min_tokens_to_cache = 50
cache_tool_results = true

[rag]
enabled = true
chunk_lines = 150
overlap_lines = 30
max_chunks = 10000
max_file_size = 1048576
max_depth = 12
top_k = 5

[ai]
default_base_url = "http://localhost:1234/v1"
client_timeout_secs = 600
default_temperature = 0.7
default_max_context = 120000
max_iterations_auto = 200
max_iterations_other = 15
min_keep_messages = 8
"#.to_string()
    }
}

// ===== Config Hot-Reload Watcher =====

static CONFIG_FILE_MTIME: AtomicU64 = AtomicU64::new(0);

/// Start a background config file watcher that polls every 2 seconds.
/// Emits a "config-reloaded" Tauri event when the file's modification time changes.
pub fn start_config_watcher(app_handle: tauri::AppHandle, config_path: std::path::PathBuf) {
    use tauri::Emitter;
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
            if let Ok(meta) = std::fs::metadata(&config_path) {
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let prev = CONFIG_FILE_MTIME.load(Ordering::Relaxed);
                if mtime > prev {
                    CONFIG_FILE_MTIME.store(mtime, Ordering::Relaxed);
                    let _ = app_handle.emit("config-reloaded", ());
                }
            }
        }
    });
}

// ===== Tauri Commands =====

pub type ConfigState = std::sync::Arc<std::sync::Mutex<ShadowConfig>>;

#[tauri::command]
pub fn config_load(
    root_path: String,
    state: tauri::State<'_, ConfigState>,
) -> Result<ShadowConfig, String> {
    let config = ShadowConfig::load_from_project(&root_path);
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    *s = config.clone();
    Ok(config)
}

#[tauri::command]
pub fn config_save(
    root_path: String,
    config: ShadowConfig,
    state: tauri::State<'_, ConfigState>,
) -> Result<(), String> {
    let path = Path::new(&root_path).join("shadow-ide.toml");
    config.save(&path)?;
    let mut s = state.lock().unwrap_or_else(|e| e.into_inner());
    *s = config;
    Ok(())
}

#[tauri::command]
pub fn config_get(state: tauri::State<'_, ConfigState>) -> Result<ShadowConfig, String> {
    state
        .lock()
        .map(|s| s.clone())
        .map_err(|e| format!("Lock error: {}", e))
}

#[tauri::command]
pub fn config_generate_default() -> String {
    ShadowConfig::generate_default_toml()
}
