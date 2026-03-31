use anyhow::{Context, Result};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// RFC-3339-ish UTC timestamp without pulling in chrono.
fn utc_now() -> String {
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Convert seconds to a readable timestamp: YYYY-MM-DDTHH:MM:SSZ
    let s = secs;
    let days = s / 86400;
    let time_of_day = s % 86400;
    let h = time_of_day / 3600;
    let m = (time_of_day % 3600) / 60;
    let sec = time_of_day % 60;
    // Days since 1970-01-01
    let (year, month, day) = days_to_ymd(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        year, month, day, h, m, sec
    )
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let month_days: &[u64] = if is_leap(year) {
        &[31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        &[31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 1u64;
    for &md in month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }
    (year, month, days + 1)
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

// ---------------------------------------------------------------------------
// TaskType — classifies a user prompt to determine token budget
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskType {
    SimpleAnswer,
    CodeEditSmall,
    CodeEditLarge,
    Planning,
    Debugging,
}

impl TaskType {
    /// Classify a user prompt into a `TaskType` using keyword matching.
    pub fn classify(prompt: &str) -> TaskType {
        let lower = prompt.to_lowercase();

        // Debugging takes priority — error messages are urgent.
        if [
            "debug",
            "error",
            "bug",
            "crash",
            "traceback",
            "exception",
            "failing",
        ]
        .iter()
        .any(|kw| lower.contains(kw))
        {
            return TaskType::Debugging;
        }

        // Planning keywords.
        if ["plan", "design", "architect", "how should", "strategy"]
            .iter()
            .any(|kw| lower.contains(kw))
        {
            return TaskType::Planning;
        }

        // Large code edits — refactor / rewrite / multi-file.
        if ["refactor", "rewrite", "restructure"]
            .iter()
            .any(|kw| lower.contains(kw))
        {
            return TaskType::CodeEditLarge;
        }

        // Count file-like mentions (paths with '/' or '.' extensions).
        let file_mentions = lower
            .split_whitespace()
            .filter(|w| w.contains('/') || (w.contains('.') && w.len() > 3))
            .count();
        if file_mentions > 1 {
            return TaskType::CodeEditLarge;
        }

        // Small code edits.
        if ["fix", "change", "edit", "modify", "update"]
            .iter()
            .any(|kw| lower.contains(kw))
        {
            return TaskType::CodeEditSmall;
        }

        // Simple answers — short prompts or question-style keywords.
        if prompt.len() < 50
            || ["what is", "explain", "how does", "tell me"]
                .iter()
                .any(|kw| lower.contains(kw))
        {
            return TaskType::SimpleAnswer;
        }

        // Default fallback.
        TaskType::SimpleAnswer
    }

    /// Return the `n_predict` token budget for this task type.
    pub fn budget(&self) -> u32 {
        match self {
            TaskType::SimpleAnswer => 256,
            TaskType::CodeEditSmall => 1024,
            TaskType::CodeEditLarge => 4096,
            TaskType::Planning => 2048,
            TaskType::Debugging => 3072,
        }
    }
}

impl std::fmt::Display for TaskType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let label = match self {
            TaskType::SimpleAnswer => "SimpleAnswer",
            TaskType::CodeEditSmall => "CodeEditSmall",
            TaskType::CodeEditLarge => "CodeEditLarge",
            TaskType::Planning => "Planning",
            TaskType::Debugging => "Debugging",
        };
        write!(f, "{}", label)
    }
}

// ---------------------------------------------------------------------------
// TaskBudget — pairs a classified task with its token usage
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TaskBudget {
    pub task_type: TaskType,
    pub n_predict: u32,
    pub actual_tokens_used: u32,
}

impl TaskBudget {
    /// Create a new budget from a prompt. `actual_tokens_used` starts at 0.
    pub fn from_prompt(prompt: &str) -> Self {
        let task_type = TaskType::classify(prompt);
        Self {
            task_type,
            n_predict: task_type.budget(),
            actual_tokens_used: 0,
        }
    }

    /// Create a budget using auto-scaled values from the tracker.
    pub fn from_prompt_scaled(prompt: &str, tracker: &BudgetTracker) -> Self {
        let task_type = TaskType::classify(prompt);
        Self {
            task_type,
            n_predict: tracker.adjusted_budget(task_type),
            actual_tokens_used: 0,
        }
    }

    /// Record how many tokens were actually consumed.
    pub fn record_usage(&mut self, tokens: u32) {
        self.actual_tokens_used = tokens;
    }

    /// Returns true if actual usage exceeded the budget.
    pub fn overran(&self) -> bool {
        self.actual_tokens_used > self.n_predict
    }

    /// How many tokens over budget (0 if within budget).
    pub fn overrun_amount(&self) -> u32 {
        self.actual_tokens_used.saturating_sub(self.n_predict)
    }
}

// ---------------------------------------------------------------------------
// StateWriter — manages the 4 state files
// ---------------------------------------------------------------------------

pub struct StateWriter {
    state_dir: PathBuf,
}

impl StateWriter {
    /// Create a new `StateWriter`.
    ///
    /// Resolution order for the state directory:
    /// 1. `SHADOWAI_STATE_DIR` environment variable (if set)
    /// 2. The provided `default_dir` (if `Some`)
    /// 3. `./state/` relative to the current working directory
    pub fn new(default_dir: Option<PathBuf>) -> Self {
        let state_dir = std::env::var("SHADOWAI_STATE_DIR")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                default_dir.unwrap_or_else(|| {
                    // Use the app's data directory so state files are always
                    // created regardless of the process's CWD.
                    dirs_next::data_dir()
                        .unwrap_or_else(|| PathBuf::from("."))
                        .join("shadow-ide")
                        .join("state")
                })
            });

        // Ensure the directory exists
        fs::create_dir_all(&state_dir).ok();

        Self { state_dir }
    }

    /// Expose the state directory path for use by `BudgetTracker`.
    pub fn state_dir(&self) -> &Path {
        &self.state_dir
    }

    // -- path helpers -------------------------------------------------------

    fn memory_path(&self) -> PathBuf {
        self.state_dir.join("memory.md")
    }

    fn completed_path(&self) -> PathBuf {
        self.state_dir.join("Completed.md")
    }

    fn errors_path(&self) -> PathBuf {
        self.state_dir.join("errors.md")
    }

    fn fixed_path(&self) -> PathBuf {
        self.state_dir.join("fixed.md")
    }

    // -- public API ---------------------------------------------------------

    /// Log the start of a task into `memory.md` (overwrites Current Task section).
    pub fn log_task_start(&self, task_id: &str, task_type: &TaskType) -> Result<()> {
        let now = utc_now();
        let content = format!(
            "# Agent Memory\n\n\
             ## Current Task\n\
             - id: {task_id}\n\
             - type: {task_type}\n\
             - status: running\n\
             - started: {now}\n\n\
             ## Context Summary\n\
             (populated at runtime)\n\n\
             ## Session History\n\
             (populated at runtime)\n"
        );
        fs::write(self.memory_path(), &content)
            .with_context(|| format!("Failed to write {}", self.memory_path().display()))?;
        Ok(())
    }

    /// Append a completed-task entry to `Completed.md`.
    pub fn log_task_complete(
        &self,
        task_id: &str,
        task_type: &TaskType,
        budget: &TaskBudget,
    ) -> Result<()> {
        let now = utc_now();
        let entry = format!(
            "\n## [{now}] Task `{task_id}`\n\
             - type: {task_type}\n\
             - budget: {} tokens\n\
             - used: {} tokens\n",
            budget.n_predict, budget.actual_tokens_used,
        );
        self.append_to_file(&self.completed_path(), &entry)
    }

    /// Append an error entry to `errors.md`.
    pub fn log_error(&self, task_id: &str, error_msg: &str) -> Result<()> {
        let now = utc_now();
        let entry = format!(
            "\n## [{now}] Error in task `{task_id}`\n\
             ```\n{error_msg}\n```\n"
        );
        self.append_to_file(&self.errors_path(), &entry)
    }

    /// Append a fix entry to `fixed.md`.
    pub fn log_fix(&self, task_id: &str, description: &str) -> Result<()> {
        let now = utc_now();
        let entry = format!(
            "\n## [{now}] Fix for task `{task_id}`\n\
             {description}\n"
        );
        self.append_to_file(&self.fixed_path(), &entry)
    }

    /// Log a budget overrun to `errors.md`.
    pub fn log_overrun(&self, task_id: &str, budget: &TaskBudget) -> Result<()> {
        let now = utc_now();
        let entry = format!(
            "\n## [{now}] Budget overrun in task `{task_id}`\n\
             - type: {}\n\
             - budget: {} tokens\n\
             - actual: {} tokens\n\
             - overrun: +{} tokens\n",
            budget.task_type,
            budget.n_predict,
            budget.actual_tokens_used,
            budget.overrun_amount(),
        );
        self.append_to_file(&self.errors_path(), &entry)
    }

    /// Log a WebSocket disconnect event to `errors.md`.
    pub fn log_disconnect(&self, reason: &str) -> Result<()> {
        let now = utc_now();
        let entry = format!(
            "\n## [{now}] Disconnect\n\
             - reason: {reason}\n"
        );
        self.append_to_file(&self.errors_path(), &entry)
    }

    /// Log a successful reconnect to `Completed.md`.
    pub fn log_reconnect(&self, gap_seconds: u64) -> Result<()> {
        let now = utc_now();
        let entry = format!(
            "\n## [{now}] Reconnected\n\
             - gap: {gap_seconds}s\n"
        );
        self.append_to_file(&self.completed_path(), &entry)
    }

    /// Mark the current task in `memory.md` as idle after completion.
    pub fn mark_idle(&self) -> Result<()> {
        let now = utc_now();
        // Preserve existing Context Summary and Session History if available
        let existing = self.read_memory().unwrap_or_default();
        let context_summary = extract_section(&existing, "## Context Summary")
            .unwrap_or_else(|| "(none)".to_string());
        let session_history = extract_section(&existing, "## Session History")
            .unwrap_or_else(|| "(none)".to_string());

        let content = format!(
            "# Agent Memory\n\n\
             ## Current Task\n\
             - id: none\n\
             - type: idle\n\
             - status: ready\n\
             - last_completed: {now}\n\n\
             ## Context Summary\n\
             {context_summary}\n\n\
             ## Session History\n\
             {session_history}\n"
        );
        self.write_locked(&self.memory_path(), &content)
    }

    /// Build a minimal recovery prompt from `memory.md` for use after reconnect.
    /// Returns the memory content if the agent was mid-task, None if idle.
    pub fn recovery_context(&self) -> Option<String> {
        let memory = self.read_memory().ok()?;
        if memory.contains("- status: running") {
            Some(memory)
        } else {
            None
        }
    }

    /// Overwrite `memory.md` with arbitrary content (e.g. updated context).
    pub fn update_memory(&self, content: &str) -> Result<()> {
        self.write_locked(&self.memory_path(), content)
    }

    /// Read the current contents of `memory.md`.
    pub fn read_memory(&self) -> Result<String> {
        fs::read_to_string(self.memory_path())
            .with_context(|| format!("Failed to read {}", self.memory_path().display()))
    }

    // -- internal -----------------------------------------------------------

    /// Append to a file with advisory file locking.
    fn append_to_file(&self, path: &Path, content: &str) -> Result<()> {
        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("Failed to open {} for appending", path.display()))?;

        // Advisory lock — blocks until the lock is acquired
        lock_file(&file, path)?;
        file.write_all(content.as_bytes())
            .with_context(|| format!("Failed to append to {}", path.display()))?;
        // Lock released when `file` is dropped
        Ok(())
    }

    /// Write (overwrite) a file with advisory locking.
    fn write_locked(&self, path: &Path, content: &str) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }

        let file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(path)
            .with_context(|| format!("Failed to open {} for writing", path.display()))?;

        lock_file(&file, path)?;
        (&file)
            .write_all(content.as_bytes())
            .with_context(|| format!("Failed to write {}", path.display()))?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// KV Cache Flush — erase a llama.cpp slot to free context
// ---------------------------------------------------------------------------

/// Attempt to flush the KV cache for a llama.cpp slot.
/// Uses the `/slots/{id}` endpoint if the server supports it,
/// otherwise falls back to a no-op (the server will reuse the slot naturally).
pub async fn flush_kv_cache(base_url: &str, slot_id: u32) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    // Try the slots API first (llama.cpp server with --slots enabled)
    let url = format!("{}/slots/{}", base_url, slot_id);
    let res = client
        .post(&url)
        .json(&serde_json::json!({"action": "erase"}))
        .send()
        .await;

    match res {
        Ok(resp) if resp.status().is_success() => {
            log::info!("[token_budget] KV cache flushed for slot {}", slot_id);
            Ok(())
        }
        Ok(resp) => {
            let status = resp.status();
            // 404 means the /slots endpoint doesn't exist — not an error
            if status.as_u16() == 404 {
                log::debug!("[token_budget] /slots endpoint not available — skipping KV flush");
                Ok(())
            } else {
                Err(format!("KV flush failed: HTTP {}", status))
            }
        }
        Err(e) => {
            // Connection error — server might not support slots, not fatal
            log::debug!("[token_budget] KV flush request failed (non-fatal): {}", e);
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract a markdown section by header (e.g. "## Context Summary").
/// Returns the content between this header and the next `##` header (or EOF).
fn extract_section(text: &str, header: &str) -> Option<String> {
    let start = text.find(header)?;
    let after_header = start + header.len();
    let content_start = text[after_header..]
        .find('\n')
        .map(|i| after_header + i + 1)?;
    let content_end = text[content_start..]
        .find("\n## ")
        .map(|i| content_start + i)
        .unwrap_or(text.len());
    let section = text[content_start..content_end].trim().to_string();
    if section.is_empty() {
        None
    } else {
        Some(section)
    }
}

// ---------------------------------------------------------------------------
// BudgetTracker — auto-scaling based on historical usage
// ---------------------------------------------------------------------------

/// Tracks actual token usage per `TaskType` across tasks and auto-scales
/// budgets when overruns are frequent.
///
/// The tracker keeps a fixed-size ring of recent measurements per type.
/// When `adjusted_budget()` is called, it returns the base budget bumped up
/// if the recent average usage exceeds a configurable threshold (default 85%).
pub struct BudgetTracker {
    /// Per-type usage history: (n_predict, actual_used) pairs.
    history: std::collections::HashMap<&'static str, Vec<(u32, u32)>>,
    /// Max entries kept per task type.
    max_history: usize,
    /// Fraction of budget that triggers an upscale (0.0–1.0). Default 0.85.
    upscale_threshold: f64,
    /// Multiplier applied when upscaling. Default 1.5.
    upscale_factor: f64,
    /// Hard ceiling — never exceed this.
    max_budget: u32,
}

impl BudgetTracker {
    pub fn new() -> Self {
        Self {
            history: std::collections::HashMap::new(),
            max_history: 20,
            upscale_threshold: 0.85,
            upscale_factor: 1.5,
            max_budget: 8192,
        }
    }

    /// Record a completed task's token usage.
    pub fn record(&mut self, task_type: TaskType, budget: u32, actual: u32) {
        let key = task_type_key(task_type);
        let ring = self.history.entry(key).or_insert_with(Vec::new);
        if ring.len() >= self.max_history {
            ring.remove(0);
        }
        ring.push((budget, actual));
    }

    /// Return an adjusted budget for a given task type based on history.
    /// If no history exists, returns the base budget unchanged.
    pub fn adjusted_budget(&self, task_type: TaskType) -> u32 {
        let base = task_type.budget();
        let key = task_type_key(task_type);
        let ring = match self.history.get(key) {
            Some(r) if r.len() >= 3 => r, // need at least 3 samples
            _ => return base,
        };

        // Count how many of the last N tasks exceeded the threshold
        let overruns = ring
            .iter()
            .filter(|(budget, actual)| {
                *budget > 0 && (*actual as f64 / *budget as f64) > self.upscale_threshold
            })
            .count();

        // If >50% of recent tasks exceeded 85% of budget, scale up
        if overruns * 2 > ring.len() {
            let avg_actual: u32 = ring.iter().map(|(_, a)| a).sum::<u32>() / ring.len() as u32;
            let scaled = ((avg_actual as f64) * self.upscale_factor) as u32;
            scaled.max(base).min(self.max_budget)
        } else {
            base
        }
    }

    /// Load history from a simple CSV file (`state/budget_history.csv`).
    /// Format: `task_type,budget,actual` per line.
    pub fn load_from_file(state_dir: &Path) -> Self {
        let mut tracker = Self::new();
        let path = state_dir.join("budget_history.csv");
        if let Ok(content) = fs::read_to_string(&path) {
            for line in content.lines() {
                let parts: Vec<&str> = line.split(',').collect();
                if parts.len() == 3 {
                    if let (Some(tt), Ok(b), Ok(a)) = (
                        parse_task_type(parts[0]),
                        parts[1].parse::<u32>(),
                        parts[2].parse::<u32>(),
                    ) {
                        tracker.record(tt, b, a);
                    }
                }
            }
        }
        tracker
    }

    /// Append a record to the CSV history file.
    pub fn save_record(
        state_dir: &Path,
        task_type: TaskType,
        budget: u32,
        actual: u32,
    ) -> Result<()> {
        let path = state_dir.join("budget_history.csv");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).ok();
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("Failed to open {}", path.display()))?;
        writeln!(file, "{},{},{}", task_type_key(task_type), budget, actual)?;
        Ok(())
    }
}

fn task_type_key(tt: TaskType) -> &'static str {
    match tt {
        TaskType::SimpleAnswer => "SimpleAnswer",
        TaskType::CodeEditSmall => "CodeEditSmall",
        TaskType::CodeEditLarge => "CodeEditLarge",
        TaskType::Planning => "Planning",
        TaskType::Debugging => "Debugging",
    }
}

fn parse_task_type(s: &str) -> Option<TaskType> {
    match s {
        "SimpleAnswer" => Some(TaskType::SimpleAnswer),
        "CodeEditSmall" => Some(TaskType::CodeEditSmall),
        "CodeEditLarge" => Some(TaskType::CodeEditLarge),
        "Planning" => Some(TaskType::Planning),
        "Debugging" => Some(TaskType::Debugging),
        _ => None,
    }
}

/// Advisory file lock using `flock` on Unix, no-op on other platforms.
fn lock_file(file: &std::fs::File, path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::io::AsRawFd;
        let fd = file.as_raw_fd();
        let ret = unsafe { libc::flock(fd, libc::LOCK_EX) };
        if ret != 0 {
            return Err(anyhow::anyhow!(
                "Failed to acquire lock on {}: {}",
                path.display(),
                std::io::Error::last_os_error()
            ));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (file, path); // suppress unused warnings
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_debugging() {
        assert_eq!(
            TaskType::classify("I have a crash in main"),
            TaskType::Debugging
        );
        assert_eq!(
            TaskType::classify("there is an error when compiling"),
            TaskType::Debugging
        );
    }

    #[test]
    fn classify_planning() {
        assert_eq!(
            TaskType::classify("plan the architecture for the new service"),
            TaskType::Planning
        );
    }

    #[test]
    fn classify_code_edit_large() {
        assert_eq!(
            TaskType::classify("refactor the authentication module"),
            TaskType::CodeEditLarge
        );
    }

    #[test]
    fn classify_code_edit_small() {
        assert_eq!(
            TaskType::classify("fix the off-by-one in parser.rs"),
            TaskType::CodeEditSmall
        );
    }

    #[test]
    fn classify_simple_answer() {
        assert_eq!(
            TaskType::classify("what is a monad"),
            TaskType::SimpleAnswer
        );
        assert_eq!(TaskType::classify("hi"), TaskType::SimpleAnswer);
    }

    #[test]
    fn budget_values() {
        assert_eq!(TaskType::SimpleAnswer.budget(), 256);
        assert_eq!(TaskType::CodeEditSmall.budget(), 1024);
        assert_eq!(TaskType::CodeEditLarge.budget(), 4096);
        assert_eq!(TaskType::Planning.budget(), 2048);
        assert_eq!(TaskType::Debugging.budget(), 3072);
    }

    #[test]
    fn task_budget_from_prompt() {
        let mut tb = TaskBudget::from_prompt("explain how does Rust ownership work");
        assert_eq!(tb.task_type, TaskType::SimpleAnswer);
        assert_eq!(tb.n_predict, 256);
        assert_eq!(tb.actual_tokens_used, 0);
        tb.record_usage(180);
        assert_eq!(tb.actual_tokens_used, 180);
    }

    #[test]
    fn overrun_detection() {
        let mut tb = TaskBudget::from_prompt("what is rust");
        assert_eq!(tb.n_predict, 256);
        tb.record_usage(200);
        assert!(!tb.overran());
        assert_eq!(tb.overrun_amount(), 0);
        tb.record_usage(300);
        assert!(tb.overran());
        assert_eq!(tb.overrun_amount(), 44);
    }

    #[test]
    fn extract_section_works() {
        let text = "# Memory\n\n## Context Summary\nsome context here\nmore context\n\n## Session History\nold stuff\n";
        let section = extract_section(text, "## Context Summary");
        assert_eq!(section, Some("some context here\nmore context".to_string()));
        let history = extract_section(text, "## Session History");
        assert_eq!(history, Some("old stuff".to_string()));
        assert_eq!(extract_section(text, "## Missing"), None);
    }

    #[test]
    fn state_writer_roundtrip() {
        let dir = std::env::temp_dir().join(format!("shadowai_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();

        let sw = StateWriter::new(Some(dir.clone()));
        sw.log_task_start("t1", &TaskType::Debugging).unwrap();

        let mem = sw.read_memory().unwrap();
        assert!(mem.contains("- id: t1"));
        assert!(mem.contains("- status: running"));

        // Should detect running task
        assert!(sw.recovery_context().is_some());

        sw.mark_idle().unwrap();
        assert!(sw.recovery_context().is_none());

        let mut tb = TaskBudget::from_prompt("debug the crash");
        tb.record_usage(3500);
        sw.log_task_complete("t1", &TaskType::Debugging, &tb)
            .unwrap();

        let completed = fs::read_to_string(dir.join("Completed.md")).unwrap();
        assert!(completed.contains("t1"));

        sw.log_error("t1", "segfault in main").unwrap();
        let errors = fs::read_to_string(dir.join("errors.md")).unwrap();
        assert!(errors.contains("segfault"));

        sw.log_overrun("t1", &tb).unwrap();
        let errors2 = fs::read_to_string(dir.join("errors.md")).unwrap();
        assert!(errors2.contains("overrun"));
        assert!(errors2.contains("+428"));

        sw.log_disconnect("timeout").unwrap();
        sw.log_reconnect(5).unwrap();

        sw.log_fix("t1", "patched the null pointer").unwrap();
        let fixed = fs::read_to_string(dir.join("fixed.md")).unwrap();
        assert!(fixed.contains("patched"));

        // Cleanup
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn auto_scaling_no_history() {
        let tracker = BudgetTracker::new();
        // No history → returns base budget
        assert_eq!(tracker.adjusted_budget(TaskType::SimpleAnswer), 256);
        assert_eq!(tracker.adjusted_budget(TaskType::Debugging), 3072);
    }

    #[test]
    fn auto_scaling_upscale_on_frequent_overruns() {
        let mut tracker = BudgetTracker::new();
        // Simulate 5 debugging tasks that all used >85% of budget
        for _ in 0..5 {
            tracker.record(TaskType::Debugging, 3072, 2900); // 94% usage
        }
        let adjusted = tracker.adjusted_budget(TaskType::Debugging);
        // avg_actual=2900, scaled=2900*1.5=4350, clamped to max 8192
        assert_eq!(adjusted, 4350);
        assert!(adjusted > 3072); // must be higher than base
    }

    #[test]
    fn auto_scaling_no_upscale_when_within_budget() {
        let mut tracker = BudgetTracker::new();
        // 5 tasks well within budget (<85%)
        for _ in 0..5 {
            tracker.record(TaskType::CodeEditSmall, 1024, 500); // 49% usage
        }
        assert_eq!(tracker.adjusted_budget(TaskType::CodeEditSmall), 1024);
    }

    #[test]
    fn auto_scaling_respects_max_budget() {
        let mut tracker = BudgetTracker::new();
        // Simulate extreme overruns
        for _ in 0..5 {
            tracker.record(TaskType::SimpleAnswer, 256, 7000);
        }
        let adjusted = tracker.adjusted_budget(TaskType::SimpleAnswer);
        assert!(adjusted <= 8192); // hard ceiling
    }

    #[test]
    fn auto_scaling_csv_roundtrip() {
        let dir = std::env::temp_dir().join(format!("shadowai_scale_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();

        // Save some records
        for i in 0..5u32 {
            BudgetTracker::save_record(&dir, TaskType::Planning, 2048, 1800 + i * 100).unwrap();
        }

        // Load and verify
        let tracker = BudgetTracker::load_from_file(&dir);
        let adjusted = tracker.adjusted_budget(TaskType::Planning);
        // avg usage = (1800+1900+2000+2100+2200)/5 = 2000, all >85% of 2048
        // scaled = 2000 * 1.5 = 3000
        assert_eq!(adjusted, 3000);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn disconnect_reconnect_simulation() {
        // Simulates a full disconnect/reconnect cycle using state files:
        // 1. Start a task → memory shows "running"
        // 2. Disconnect → error logged
        // 3. Reconnect → recovery_context returns the mid-task state
        // 4. Complete task → memory shows "idle", completed logged

        let dir = std::env::temp_dir().join(format!("shadowai_disconnect_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let sw = StateWriter::new(Some(dir.clone()));

        // Step 1: task starts
        sw.log_task_start("t-dc-1", &TaskType::CodeEditLarge)
            .unwrap();
        let mem = sw.read_memory().unwrap();
        assert!(mem.contains("- status: running"));
        assert!(mem.contains("CodeEditLarge"));

        // Step 2: disconnect mid-task
        sw.log_disconnect("WebSocket closed by server").unwrap();
        let errors = fs::read_to_string(dir.join("errors.md")).unwrap();
        assert!(errors.contains("Disconnect"));
        assert!(errors.contains("WebSocket closed by server"));

        // Step 3: reconnect — recovery context should exist
        let recovery = sw.recovery_context();
        assert!(recovery.is_some());
        let ctx = recovery.unwrap();
        assert!(ctx.contains("t-dc-1"));
        assert!(ctx.contains("- status: running"));

        // Log reconnect
        sw.log_reconnect(12).unwrap();
        let completed = fs::read_to_string(dir.join("Completed.md")).unwrap();
        assert!(completed.contains("Reconnected"));
        assert!(completed.contains("gap: 12s"));

        // Step 4: finish the task
        let mut tb = TaskBudget::from_prompt("refactor the auth module");
        tb.record_usage(3800);
        sw.log_task_complete("t-dc-1", &TaskType::CodeEditLarge, &tb)
            .unwrap();
        sw.mark_idle().unwrap();

        // Verify final state
        assert!(sw.recovery_context().is_none());
        let final_mem = sw.read_memory().unwrap();
        assert!(final_mem.contains("- status: ready"));

        let final_completed = fs::read_to_string(dir.join("Completed.md")).unwrap();
        assert!(final_completed.contains("t-dc-1"));
        assert!(final_completed.contains("3800"));

        fs::remove_dir_all(&dir).ok();
    }
}
