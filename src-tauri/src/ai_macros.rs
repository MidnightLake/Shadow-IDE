use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

// ===== Types =====

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum TriggerType {
    /// Fires when a file is saved
    OnSave,
    /// Fires when a compile/build error is detected
    OnCompileError,
    /// Fires when a test fails
    OnTestFail,
    /// Fires on a file open
    OnFileOpen,
    /// Fires on a manual trigger (button click)
    Manual,
}

impl std::fmt::Display for TriggerType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TriggerType::OnSave => write!(f, "on_save"),
            TriggerType::OnCompileError => write!(f, "on_compile_error"),
            TriggerType::OnTestFail => write!(f, "on_test_fail"),
            TriggerType::OnFileOpen => write!(f, "on_file_open"),
            TriggerType::Manual => write!(f, "manual"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiMacro {
    /// Unique ID
    pub id: String,
    /// User-facing name
    pub name: String,
    /// When this macro fires
    pub trigger: TriggerType,
    /// AI prompt template — supports `{{file_path}}`, `{{error}}`, `{{file_ext}}` placeholders
    pub prompt_template: String,
    /// Optional file extension filter (e.g., "rs", "ts") — empty = all files
    #[serde(default)]
    pub file_filter: Vec<String>,
    /// Whether this macro is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Chat mode to use: "plan", "build", or "auto"
    #[serde(default = "default_build_mode")]
    pub chat_mode: String,
    /// Cooldown in seconds between repeated triggers (prevents spam)
    #[serde(default = "default_cooldown")]
    pub cooldown_secs: u64,
}

fn default_true() -> bool {
    true
}
fn default_build_mode() -> String {
    "build".to_string()
}
fn default_cooldown() -> u64 {
    10
}

pub struct MacroState {
    macros: Mutex<Vec<AiMacro>>,
    /// Last trigger time per macro ID (for cooldown)
    last_triggered: Mutex<HashMap<String, u64>>,
}

impl MacroState {
    pub fn new() -> Self {
        Self {
            macros: Mutex::new(Vec::new()),
            last_triggered: Mutex::new(HashMap::new()),
        }
    }

    /// Load macros from disk
    pub fn load(&self) {
        let path = macros_path();
        if let Ok(content) = std::fs::read_to_string(&path) {
            if let Ok(macros) = serde_json::from_str::<Vec<AiMacro>>(&content) {
                if let Ok(mut m) = self.macros.lock() {
                    *m = macros;
                }
            }
        }
    }

    /// Save macros to disk
    pub fn save(&self) -> Result<(), String> {
        let macros = self
            .macros
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        let path = macros_path();
        if let Some(parent) = std::path::Path::new(&path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let json = serde_json::to_string_pretty(&*macros)
            .map_err(|e| format!("Serialize error: {}", e))?;
        std::fs::write(&path, json).map_err(|e| format!("Write error: {}", e))?;
        Ok(())
    }

    /// Replace the full macro set from a synced backup and persist it.
    pub fn replace_all(&self, macros: Vec<AiMacro>) -> Result<(), String> {
        {
            let mut current = self
                .macros
                .lock()
                .map_err(|e| format!("Lock error: {}", e))?;
            *current = macros;
        }
        self.save()
    }

    /// Get macros matching a trigger, respecting cooldown
    pub fn get_triggered(
        &self,
        trigger: &TriggerType,
        context: &TriggerContext,
    ) -> Vec<(AiMacro, String)> {
        let macros = match self.macros.lock() {
            Ok(m) => m.clone(),
            Err(_) => return Vec::new(),
        };
        let now = now_secs();

        let mut results = Vec::new();
        for mac in &macros {
            if !mac.enabled || mac.trigger != *trigger {
                continue;
            }

            // File extension filter
            if !mac.file_filter.is_empty() {
                if let Some(ref ext) = context.file_ext {
                    if !mac.file_filter.iter().any(|f| f.eq_ignore_ascii_case(ext)) {
                        continue;
                    }
                } else {
                    continue; // No extension and filter requires one
                }
            }

            // Cooldown check
            if let Ok(last) = self.last_triggered.lock() {
                if let Some(&last_time) = last.get(&mac.id) {
                    if now - last_time < mac.cooldown_secs {
                        continue;
                    }
                }
            }

            // Expand template
            let prompt = expand_template(&mac.prompt_template, context);
            results.push((mac.clone(), prompt));
        }

        // Update last triggered times
        if let Ok(mut last) = self.last_triggered.lock() {
            for (mac, _) in &results {
                last.insert(mac.id.clone(), now);
            }
        }

        results
    }
}

#[derive(Debug, Clone, Default)]
pub struct TriggerContext {
    pub file_path: Option<String>,
    pub file_ext: Option<String>,
    pub error_message: Option<String>,
    pub test_name: Option<String>,
}

fn expand_template(template: &str, ctx: &TriggerContext) -> String {
    let mut result = template.to_string();
    if let Some(ref fp) = ctx.file_path {
        result = result.replace("{{file_path}}", fp);
    }
    if let Some(ref ext) = ctx.file_ext {
        result = result.replace("{{file_ext}}", ext);
    }
    if let Some(ref err) = ctx.error_message {
        result = result.replace("{{error}}", err);
    }
    if let Some(ref test) = ctx.test_name {
        result = result.replace("{{test_name}}", test);
    }
    // Clean up unexpanded placeholders
    result = result
        .replace("{{file_path}}", "")
        .replace("{{file_ext}}", "")
        .replace("{{error}}", "")
        .replace("{{test_name}}", "");
    result
}

fn macros_path() -> String {
    dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("shadow-ide")
        .join("ai-macros.json")
        .to_string_lossy()
        .to_string()
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ===== Tauri Commands =====

#[tauri::command]
pub fn macro_list(
    state: tauri::State<'_, std::sync::Arc<MacroState>>,
) -> Result<Vec<AiMacro>, String> {
    state
        .macros
        .lock()
        .map(|m| m.clone())
        .map_err(|e| format!("Lock error: {}", e))
}

#[tauri::command]
pub fn macro_add(
    name: String,
    trigger: TriggerType,
    prompt_template: String,
    file_filter: Option<Vec<String>>,
    chat_mode: Option<String>,
    cooldown_secs: Option<u64>,
    state: tauri::State<'_, std::sync::Arc<MacroState>>,
) -> Result<AiMacro, String> {
    let mac = AiMacro {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        trigger,
        prompt_template,
        file_filter: file_filter.unwrap_or_default(),
        enabled: true,
        chat_mode: chat_mode.unwrap_or_else(default_build_mode),
        cooldown_secs: cooldown_secs.unwrap_or(default_cooldown()),
    };

    {
        let mut macros = state
            .macros
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        macros.push(mac.clone());
    }
    state.save()?;
    Ok(mac)
}

#[tauri::command]
pub fn macro_update(
    id: String,
    name: Option<String>,
    trigger: Option<TriggerType>,
    prompt_template: Option<String>,
    file_filter: Option<Vec<String>>,
    enabled: Option<bool>,
    chat_mode: Option<String>,
    cooldown_secs: Option<u64>,
    state: tauri::State<'_, std::sync::Arc<MacroState>>,
) -> Result<AiMacro, String> {
    let mut macros = state
        .macros
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let mac = macros
        .iter_mut()
        .find(|m| m.id == id)
        .ok_or_else(|| format!("Macro not found: {}", id))?;

    if let Some(n) = name {
        mac.name = n;
    }
    if let Some(t) = trigger {
        mac.trigger = t;
    }
    if let Some(p) = prompt_template {
        mac.prompt_template = p;
    }
    if let Some(f) = file_filter {
        mac.file_filter = f;
    }
    if let Some(e) = enabled {
        mac.enabled = e;
    }
    if let Some(m) = chat_mode {
        mac.chat_mode = m;
    }
    if let Some(c) = cooldown_secs {
        mac.cooldown_secs = c;
    }

    let updated = mac.clone();
    drop(macros);
    state.save()?;
    Ok(updated)
}

#[tauri::command]
pub fn macro_delete(
    id: String,
    state: tauri::State<'_, std::sync::Arc<MacroState>>,
) -> Result<String, String> {
    let mut macros = state
        .macros
        .lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let before = macros.len();
    macros.retain(|m| m.id != id);
    if macros.len() == before {
        return Err(format!("Macro not found: {}", id));
    }
    drop(macros);
    state.save()?;
    Ok("Deleted".to_string())
}

#[tauri::command]
pub fn macro_trigger(
    trigger: TriggerType,
    file_path: Option<String>,
    error_message: Option<String>,
    test_name: Option<String>,
    state: tauri::State<'_, std::sync::Arc<MacroState>>,
) -> Result<Vec<serde_json::Value>, String> {
    let file_ext = file_path.as_ref().and_then(|p| {
        std::path::Path::new(p)
            .extension()
            .map(|e| e.to_string_lossy().to_string())
    });

    let ctx = TriggerContext {
        file_path,
        file_ext,
        error_message,
        test_name,
    };

    let triggered = state.get_triggered(&trigger, &ctx);
    let results: Vec<serde_json::Value> = triggered
        .iter()
        .map(|(mac, prompt)| {
            serde_json::json!({
                "macro_id": mac.id,
                "macro_name": mac.name,
                "prompt": prompt,
                "chat_mode": mac.chat_mode,
            })
        })
        .collect();

    Ok(results)
}

// ===== Built-in macro presets =====

#[tauri::command]
pub fn macro_load_presets(
    state: tauri::State<'_, std::sync::Arc<MacroState>>,
) -> Result<Vec<AiMacro>, String> {
    let presets = vec![
        AiMacro {
            id: "preset-lint-on-save".to_string(),
            name: "Auto-lint on save".to_string(),
            trigger: TriggerType::OnSave,
            prompt_template: "Review the file {{file_path}} for common issues: unused imports, \
                type errors, and style violations. Fix any problems you find."
                .to_string(),
            file_filter: vec![
                "rs".to_string(),
                "ts".to_string(),
                "tsx".to_string(),
                "py".to_string(),
            ],
            enabled: false,
            chat_mode: "build".to_string(),
            cooldown_secs: 30,
        },
        AiMacro {
            id: "preset-fix-compile-error".to_string(),
            name: "Auto-fix compile errors".to_string(),
            trigger: TriggerType::OnCompileError,
            prompt_template: "A compile error occurred:\n```\n{{error}}\n```\n\
                Analyze the error and fix the root cause in the relevant source file."
                .to_string(),
            file_filter: Vec::new(),
            enabled: false,
            chat_mode: "build".to_string(),
            cooldown_secs: 15,
        },
        AiMacro {
            id: "preset-fix-test-fail".to_string(),
            name: "Auto-fix test failures".to_string(),
            trigger: TriggerType::OnTestFail,
            prompt_template: "Test `{{test_name}}` failed with error:\n```\n{{error}}\n```\n\
                Investigate and fix the failing test or the code it tests."
                .to_string(),
            file_filter: Vec::new(),
            enabled: false,
            chat_mode: "build".to_string(),
            cooldown_secs: 20,
        },
        AiMacro {
            id: "preset-explain-on-open".to_string(),
            name: "Explain file on open".to_string(),
            trigger: TriggerType::OnFileOpen,
            prompt_template: "Briefly explain what {{file_path}} does — its purpose, key types, \
                and how it fits into the project architecture. Keep it under 200 words."
                .to_string(),
            file_filter: Vec::new(),
            enabled: false,
            chat_mode: "plan".to_string(),
            cooldown_secs: 5,
        },
    ];

    {
        let mut macros = state
            .macros
            .lock()
            .map_err(|e| format!("Lock error: {}", e))?;
        for preset in &presets {
            if !macros.iter().any(|m| m.id == preset.id) {
                macros.push(preset.clone());
            }
        }
    }
    state.save()?;

    Ok(presets)
}

// ===== Tests =====

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state() -> std::sync::Arc<MacroState> {
        std::sync::Arc::new(MacroState::new())
    }

    #[test]
    fn test_expand_template() {
        let ctx = TriggerContext {
            file_path: Some("/src/main.rs".to_string()),
            file_ext: Some("rs".to_string()),
            error_message: Some("type mismatch".to_string()),
            test_name: None,
        };
        let result = expand_template("Fix {{error}} in {{file_path}}", &ctx);
        assert_eq!(result, "Fix type mismatch in /src/main.rs");
    }

    #[test]
    fn test_expand_template_missing_vars() {
        let ctx = TriggerContext::default();
        let result = expand_template("Fix {{error}} in {{file_path}}", &ctx);
        assert_eq!(result, "Fix  in ");
    }

    #[test]
    fn test_trigger_context_file_filter() {
        let state = make_state();
        {
            let mut macros = state.macros.lock().unwrap();
            macros.push(AiMacro {
                id: "test1".to_string(),
                name: "Test".to_string(),
                trigger: TriggerType::OnSave,
                prompt_template: "lint {{file_path}}".to_string(),
                file_filter: vec!["rs".to_string()],
                enabled: true,
                chat_mode: "build".to_string(),
                cooldown_secs: 0,
            });
        }

        // Should match .rs files
        let ctx = TriggerContext {
            file_path: Some("main.rs".to_string()),
            file_ext: Some("rs".to_string()),
            ..Default::default()
        };
        let results = state.get_triggered(&TriggerType::OnSave, &ctx);
        assert_eq!(results.len(), 1);

        // Should not match .py files
        let ctx = TriggerContext {
            file_path: Some("main.py".to_string()),
            file_ext: Some("py".to_string()),
            ..Default::default()
        };
        let results = state.get_triggered(&TriggerType::OnSave, &ctx);
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_cooldown() {
        let state = make_state();
        {
            let mut macros = state.macros.lock().unwrap();
            macros.push(AiMacro {
                id: "cd".to_string(),
                name: "Cooldown Test".to_string(),
                trigger: TriggerType::OnSave,
                prompt_template: "test".to_string(),
                file_filter: Vec::new(),
                enabled: true,
                chat_mode: "build".to_string(),
                cooldown_secs: 60,
            });
        }

        let ctx = TriggerContext::default();

        // First trigger should work
        let r1 = state.get_triggered(&TriggerType::OnSave, &ctx);
        assert_eq!(r1.len(), 1);

        // Second trigger within cooldown should be blocked
        let r2 = state.get_triggered(&TriggerType::OnSave, &ctx);
        assert_eq!(r2.len(), 0);
    }

    #[test]
    fn test_disabled_macro_not_triggered() {
        let state = make_state();
        {
            let mut macros = state.macros.lock().unwrap();
            macros.push(AiMacro {
                id: "disabled".to_string(),
                name: "Disabled".to_string(),
                trigger: TriggerType::OnSave,
                prompt_template: "test".to_string(),
                file_filter: Vec::new(),
                enabled: false,
                chat_mode: "build".to_string(),
                cooldown_secs: 0,
            });
        }

        let ctx = TriggerContext::default();
        let results = state.get_triggered(&TriggerType::OnSave, &ctx);
        assert_eq!(results.len(), 0);
    }

    #[test]
    fn test_wrong_trigger_not_fired() {
        let state = make_state();
        {
            let mut macros = state.macros.lock().unwrap();
            macros.push(AiMacro {
                id: "save-only".to_string(),
                name: "Save Only".to_string(),
                trigger: TriggerType::OnSave,
                prompt_template: "test".to_string(),
                file_filter: Vec::new(),
                enabled: true,
                chat_mode: "build".to_string(),
                cooldown_secs: 0,
            });
        }

        let ctx = TriggerContext::default();
        let results = state.get_triggered(&TriggerType::OnCompileError, &ctx);
        assert_eq!(results.len(), 0);
    }
}
