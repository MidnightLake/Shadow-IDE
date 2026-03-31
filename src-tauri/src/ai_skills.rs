//! AI Coding Skills Stack — 6-layer orchestration pipeline
//!
//! Layers (bottom-up):
//! 1. **Model Routing** — Select optimal model for the task (llm_loader::recommend_model_for_task)
//! 2. **Prompt Engineering** — Task-aware system prompts, context injection, mode selection
//! 3. **RAG Intelligence** — Codebase-aware context retrieval (rag_index)
//! 4. **Tool-Augmented Execution** — Code editing, shell, search, file ops (tool_calling)
//! 5. **Self-Healing** — Error detection, retry strategies, diagnostic injection (tool_calling::healing)
//! 6. **Feedback Loop** — Macro triggers, compaction, memory extraction, quality scoring

use serde::{Deserialize, Serialize};

// MARK: - Skill Classification

/// High-level coding skill categories that determine pipeline behavior
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CodingSkill {
    /// Write new code (greenfield, scaffolding, boilerplate)
    Implement,
    /// Fix a bug or resolve an error
    Debug,
    /// Restructure existing code without changing behavior
    Refactor,
    /// Write or generate tests
    Test,
    /// Explain code, answer questions
    Explain,
    /// Review code for issues, suggest improvements
    Review,
    /// Plan architecture, design systems
    Architect,
    /// Execute shell commands, build, deploy
    DevOps,
}

/// Skill classification result with confidence
#[derive(Debug, Clone, Serialize)]
pub struct SkillClassification {
    pub skill: CodingSkill,
    pub confidence: f32,
    pub reasoning: String,
}

/// Classify the user's request into a coding skill
pub fn classify_skill(message: &str) -> SkillClassification {
    let lower = message.to_lowercase();

    // Score each skill
    let mut scores: Vec<(CodingSkill, f32, &str)> = vec![
        (
            CodingSkill::Debug,
            score_debug(&lower),
            "bug/error keywords detected",
        ),
        (
            CodingSkill::Implement,
            score_implement(&lower),
            "implementation keywords detected",
        ),
        (
            CodingSkill::Refactor,
            score_refactor(&lower),
            "refactoring keywords detected",
        ),
        (
            CodingSkill::Test,
            score_test(&lower),
            "testing keywords detected",
        ),
        (
            CodingSkill::Explain,
            score_explain(&lower),
            "explanation keywords detected",
        ),
        (
            CodingSkill::Review,
            score_review(&lower),
            "review keywords detected",
        ),
        (
            CodingSkill::Architect,
            score_architect(&lower),
            "architecture keywords detected",
        ),
        (
            CodingSkill::DevOps,
            score_devops(&lower),
            "devops/build keywords detected",
        ),
    ];

    scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let (skill, confidence, reasoning) =
        scores
            .into_iter()
            .next()
            .unwrap_or((CodingSkill::Implement, 0.3, "default fallback"));

    SkillClassification {
        skill,
        confidence,
        reasoning: reasoning.to_string(),
    }
}

fn score_debug(msg: &str) -> f32 {
    let mut score = 0.0f32;
    let keywords = [
        "fix",
        "bug",
        "error",
        "crash",
        "broken",
        "doesn't work",
        "fails",
        "not working",
        "issue",
        "wrong",
        "unexpected",
        "panic",
        "exception",
    ];
    for k in &keywords {
        if msg.contains(k) {
            score += 0.2;
        }
    }
    score.min(1.0)
}

fn score_implement(msg: &str) -> f32 {
    let mut score: f32 = 0.1; // slight baseline — most requests are implementation
    let keywords = [
        "add",
        "create",
        "implement",
        "build",
        "make",
        "write",
        "new feature",
        "scaffold",
        "generate",
        "set up",
    ];
    for k in &keywords {
        if msg.contains(k) {
            score += 0.15;
        }
    }
    score.min(1.0)
}

fn score_refactor(msg: &str) -> f32 {
    let mut score = 0.0f32;
    let keywords = [
        "refactor",
        "restructure",
        "reorganize",
        "clean up",
        "simplify",
        "extract",
        "rename",
        "move",
        "split",
        "deduplicate",
        "dry",
    ];
    for k in &keywords {
        if msg.contains(k) {
            score += 0.2;
        }
    }
    score.min(1.0)
}

fn score_test(msg: &str) -> f32 {
    let mut score = 0.0f32;
    let keywords = [
        "test",
        "spec",
        "coverage",
        "assert",
        "mock",
        "unit test",
        "integration test",
        "e2e",
        "tdd",
    ];
    for k in &keywords {
        if msg.contains(k) {
            score += 0.2;
        }
    }
    score.min(1.0)
}

fn score_explain(msg: &str) -> f32 {
    let mut score = 0.0f32;
    let keywords = [
        "explain",
        "what does",
        "how does",
        "why",
        "understand",
        "describe",
        "what is",
        "walk me through",
        "documentation",
    ];
    for k in &keywords {
        if msg.contains(k) {
            score += 0.2;
        }
    }
    score.min(1.0)
}

fn score_review(msg: &str) -> f32 {
    let mut score = 0.0f32;
    let keywords = [
        "review",
        "check",
        "audit",
        "look at",
        "feedback",
        "improve",
        "suggestions",
        "code quality",
        "lint",
    ];
    for k in &keywords {
        if msg.contains(k) {
            score += 0.2;
        }
    }
    score.min(1.0)
}

fn score_architect(msg: &str) -> f32 {
    let mut score = 0.0f32;
    let keywords = [
        "architect",
        "design",
        "plan",
        "system design",
        "structure",
        "pattern",
        "api design",
        "schema",
        "database design",
        "module",
    ];
    for k in &keywords {
        if msg.contains(k) {
            score += 0.2;
        }
    }
    score.min(1.0)
}

fn score_devops(msg: &str) -> f32 {
    let mut score = 0.0f32;
    let keywords = [
        "deploy",
        "build",
        "ci/cd",
        "docker",
        "kubernetes",
        "pipeline",
        "compile",
        "bundle",
        "release",
        "npm",
        "cargo",
        "run",
    ];
    for k in &keywords {
        if msg.contains(k) {
            score += 0.15;
        }
    }
    score.min(1.0)
}

// MARK: - Skill-Specific System Prompts

/// Get skill-specific system prompt instructions
pub fn skill_system_prompt(skill: &CodingSkill) -> &'static str {
    match skill {
        CodingSkill::Implement => {
            "You are implementing new code. Focus on:\n\
             - Clean, idiomatic code following the project's existing patterns\n\
             - Read existing files first to understand conventions\n\
             - Use the project's error handling patterns\n\
             - Add appropriate imports\n\
             - Prefer small, targeted changes over large rewrites"
        }
        CodingSkill::Debug => {
            "You are debugging an issue. Follow this process:\n\
             1. Read the relevant code and error output carefully\n\
             2. Form a hypothesis about the root cause\n\
             3. Verify by reading related code (callers, dependencies)\n\
             4. Make the minimal fix that addresses the root cause\n\
             5. Explain what was wrong and why the fix works"
        }
        CodingSkill::Refactor => {
            "You are refactoring code. Rules:\n\
             - Behavior must remain identical (no functional changes)\n\
             - Make changes incrementally — one refactoring step at a time\n\
             - If tests exist, run them after each change\n\
             - Explain the refactoring pattern you're applying\n\
             - Prefer renaming and extracting over rewriting"
        }
        CodingSkill::Test => {
            "You are writing tests. Guidelines:\n\
             - Match the project's existing test framework and patterns\n\
             - Test behavior, not implementation details\n\
             - Cover edge cases and error paths\n\
             - Use descriptive test names that explain the scenario\n\
             - Prefer assertion messages that help diagnose failures"
        }
        CodingSkill::Explain => {
            "You are explaining code. Guidelines:\n\
             - Read the code before explaining\n\
             - Start with a high-level overview, then drill into details\n\
             - Use analogies for complex concepts\n\
             - Reference specific line numbers\n\
             - Distinguish between 'what' and 'why'"
        }
        CodingSkill::Review => {
            "You are reviewing code. Check for:\n\
             - Security vulnerabilities (OWASP top 10)\n\
             - Performance issues (N+1 queries, unnecessary allocations)\n\
             - Error handling gaps\n\
             - Naming clarity and code readability\n\
             - Missing edge cases\n\
             - Provide specific, actionable feedback with examples"
        }
        CodingSkill::Architect => {
            "You are designing system architecture. Process:\n\
             - Understand requirements and constraints first\n\
             - Consider scalability, maintainability, and simplicity\n\
             - Propose concrete file structure and module boundaries\n\
             - Define interfaces/APIs between components\n\
             - Discuss trade-offs explicitly\n\
             - Do NOT implement yet — plan only"
        }
        CodingSkill::DevOps => {
            "You are working on build/deploy tasks. Guidelines:\n\
             - Read existing CI/CD configs and scripts first\n\
             - Prefer declarative config over imperative scripts\n\
             - Test commands locally before modifying pipelines\n\
             - Consider rollback strategy\n\
             - Keep secrets out of code and configs"
        }
    }
}

// MARK: - Skill Pipeline Configuration

/// Per-skill pipeline tuning parameters
#[derive(Debug, Clone, Serialize)]
pub struct SkillConfig {
    /// Number of RAG results to inject
    pub rag_results: usize,
    /// Maximum self-healing retry attempts
    pub max_heal_attempts: usize,
    /// Whether to auto-run tests after changes
    pub auto_test: bool,
    /// Preferred chat mode (plan/build/auto)
    pub chat_mode: String,
    /// Whether to enable tool usage
    pub tools_enabled: bool,
    /// Token cleaning mode for context
    pub clean_mode: String,
}

pub fn config_for_skill(skill: &CodingSkill) -> SkillConfig {
    match skill {
        CodingSkill::Implement => SkillConfig {
            rag_results: 5,
            max_heal_attempts: 3,
            auto_test: true,
            chat_mode: "build".to_string(),
            tools_enabled: true,
            clean_mode: "structural".to_string(),
        },
        CodingSkill::Debug => SkillConfig {
            rag_results: 7,       // more context for debugging
            max_heal_attempts: 5, // more retries for fix attempts
            auto_test: true,
            chat_mode: "build".to_string(),
            tools_enabled: true,
            clean_mode: "trim".to_string(), // preserve comments for context
        },
        CodingSkill::Refactor => SkillConfig {
            rag_results: 5,
            max_heal_attempts: 3,
            auto_test: true,
            chat_mode: "build".to_string(),
            tools_enabled: true,
            clean_mode: "structural".to_string(),
        },
        CodingSkill::Test => SkillConfig {
            rag_results: 3,
            max_heal_attempts: 4, // test writing often needs iterations
            auto_test: true,
            chat_mode: "build".to_string(),
            tools_enabled: true,
            clean_mode: "trim".to_string(),
        },
        CodingSkill::Explain => SkillConfig {
            rag_results: 5,
            max_heal_attempts: 0,
            auto_test: false,
            chat_mode: "plan".to_string(),
            tools_enabled: true,            // needs to read files
            clean_mode: "none".to_string(), // preserve all context
        },
        CodingSkill::Review => SkillConfig {
            rag_results: 5,
            max_heal_attempts: 0,
            auto_test: false,
            chat_mode: "plan".to_string(),
            tools_enabled: true, // needs to read files
            clean_mode: "none".to_string(),
        },
        CodingSkill::Architect => SkillConfig {
            rag_results: 3,
            max_heal_attempts: 0,
            auto_test: false,
            chat_mode: "plan".to_string(),
            tools_enabled: false, // no file changes during planning
            clean_mode: "structural".to_string(),
        },
        CodingSkill::DevOps => SkillConfig {
            rag_results: 3,
            max_heal_attempts: 2,
            auto_test: false,
            chat_mode: "build".to_string(),
            tools_enabled: true,
            clean_mode: "trim".to_string(),
        },
    }
}

// MARK: - Quality Scoring

/// Score the quality of an AI response for feedback loop
#[derive(Debug, Clone, Serialize)]
#[allow(dead_code)]
pub struct QualityScore {
    pub overall: f32,
    pub tool_success_rate: f32,
    pub heal_attempts_used: usize,
    pub files_modified: usize,
    pub tests_passed: bool,
}

#[allow(dead_code)]
pub fn compute_quality_score(
    tool_results: &[(bool, String)], // (success, tool_name)
    heal_attempts: usize,
    max_heal_attempts: usize,
    tests_passed: Option<bool>,
    files_modified: usize,
) -> QualityScore {
    let total_tools = tool_results.len();
    let successful_tools = tool_results.iter().filter(|(s, _)| *s).count();
    let tool_success_rate = if total_tools > 0 {
        successful_tools as f32 / total_tools as f32
    } else {
        1.0
    };

    // Penalize for healing attempts
    let heal_penalty = if max_heal_attempts > 0 {
        1.0 - (heal_attempts as f32 / (max_heal_attempts as f32 * 2.0)).min(0.5)
    } else {
        1.0
    };

    // Bonus for passing tests
    let test_bonus = match tests_passed {
        Some(true) => 1.1,
        Some(false) => 0.7,
        None => 1.0,
    };

    let overall = (tool_success_rate * heal_penalty * test_bonus).min(1.0);

    QualityScore {
        overall,
        tool_success_rate,
        heal_attempts_used: heal_attempts,
        files_modified,
        tests_passed: tests_passed.unwrap_or(false),
    }
}

// MARK: - Tauri Commands

#[tauri::command]
pub fn ai_classify_skill(message: String) -> SkillClassification {
    classify_skill(&message)
}

#[tauri::command]
pub fn ai_skill_config(skill: String) -> SkillConfig {
    let coding_skill = match skill.to_lowercase().as_str() {
        "implement" => CodingSkill::Implement,
        "debug" => CodingSkill::Debug,
        "refactor" => CodingSkill::Refactor,
        "test" => CodingSkill::Test,
        "explain" => CodingSkill::Explain,
        "review" => CodingSkill::Review,
        "architect" => CodingSkill::Architect,
        "devops" => CodingSkill::DevOps,
        _ => CodingSkill::Implement,
    };
    config_for_skill(&coding_skill)
}

#[tauri::command]
pub fn ai_skill_prompt(skill: String) -> String {
    let coding_skill = match skill.to_lowercase().as_str() {
        "implement" => CodingSkill::Implement,
        "debug" => CodingSkill::Debug,
        "refactor" => CodingSkill::Refactor,
        "test" => CodingSkill::Test,
        "explain" => CodingSkill::Explain,
        "review" => CodingSkill::Review,
        "architect" => CodingSkill::Architect,
        "devops" => CodingSkill::DevOps,
        _ => CodingSkill::Implement,
    };
    skill_system_prompt(&coding_skill).to_string()
}

// MARK: - Tests

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify_debug() {
        let result = classify_skill("fix the bug where the server crashes on startup");
        assert_eq!(result.skill, CodingSkill::Debug);
        assert!(result.confidence > 0.3);
    }

    #[test]
    fn test_classify_implement() {
        let result = classify_skill("add a new endpoint for user registration");
        assert_eq!(result.skill, CodingSkill::Implement);
    }

    #[test]
    fn test_classify_refactor() {
        let result = classify_skill("refactor the auth module to extract the token validation");
        assert_eq!(result.skill, CodingSkill::Refactor);
    }

    #[test]
    fn test_classify_test() {
        let result = classify_skill("write unit tests for the cache module");
        assert_eq!(result.skill, CodingSkill::Test);
    }

    #[test]
    fn test_classify_explain() {
        let result = classify_skill("explain how the WebSocket connection handling works");
        assert_eq!(result.skill, CodingSkill::Explain);
    }

    #[test]
    fn test_classify_review() {
        let result = classify_skill("review this code for security vulnerabilities");
        assert_eq!(result.skill, CodingSkill::Review);
    }

    #[test]
    fn test_classify_architect() {
        let result = classify_skill("design the database schema for the new feature");
        assert_eq!(result.skill, CodingSkill::Architect);
    }

    #[test]
    fn test_classify_devops() {
        let result = classify_skill("set up the docker deployment pipeline");
        assert_eq!(result.skill, CodingSkill::DevOps);
    }

    #[test]
    fn test_skill_config_debug_has_more_rag() {
        let debug_config = config_for_skill(&CodingSkill::Debug);
        let impl_config = config_for_skill(&CodingSkill::Implement);
        assert!(debug_config.rag_results > impl_config.rag_results);
    }

    #[test]
    fn test_skill_config_explain_no_tools() {
        let config = config_for_skill(&CodingSkill::Architect);
        assert!(!config.tools_enabled);
    }

    #[test]
    fn test_quality_score_perfect() {
        let score = compute_quality_score(
            &[
                (true, "write_file".to_string()),
                (true, "shell_exec".to_string()),
            ],
            0,
            3,
            Some(true),
            1,
        );
        assert!(score.overall > 0.9);
        assert_eq!(score.tool_success_rate, 1.0);
    }

    #[test]
    fn test_quality_score_failures() {
        let score = compute_quality_score(
            &[
                (false, "write_file".to_string()),
                (true, "shell_exec".to_string()),
            ],
            2,
            3,
            Some(false),
            1,
        );
        assert!(score.overall < 0.6);
        assert_eq!(score.tool_success_rate, 0.5);
    }

    #[test]
    fn test_skill_prompt_not_empty() {
        for skill in [
            CodingSkill::Implement,
            CodingSkill::Debug,
            CodingSkill::Refactor,
            CodingSkill::Test,
            CodingSkill::Explain,
            CodingSkill::Review,
            CodingSkill::Architect,
            CodingSkill::DevOps,
        ] {
            assert!(!skill_system_prompt(&skill).is_empty());
        }
    }
}
