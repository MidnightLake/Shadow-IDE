use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub token_count: usize,
    #[serde(default)]
    pub is_compacted: bool,
    #[serde(default)]
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ConnectionStatus {
    Connected,
    Cached,
    Error,
    Disconnected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBarState {
    pub used: u32,
    pub max: u32,
    pub percentage: f64,
    pub level: TokenLevel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TokenLevel {
    Ok,
    Warning,
    Critical,
}

impl TokenBarState {
    pub fn new(used: u32, max: u32) -> Self {
        let percentage = if max > 0 { used as f64 / max as f64 } else { 0.0 };
        let level = if percentage < 0.5 {
            TokenLevel::Ok
        } else if percentage < 0.8 {
            TokenLevel::Warning
        } else {
            TokenLevel::Critical
        };
        Self { used, max, percentage, level }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_bar_ok_level() {
        let state = TokenBarState::new(100, 1000);
        assert_eq!(state.used, 100);
        assert_eq!(state.max, 1000);
        assert!((state.percentage - 0.1).abs() < f64::EPSILON);
        assert!(matches!(state.level, TokenLevel::Ok));
    }

    #[test]
    fn token_bar_warning_level() {
        let state = TokenBarState::new(500, 1000);
        assert!(matches!(state.level, TokenLevel::Warning));
    }

    #[test]
    fn token_bar_critical_level() {
        let state = TokenBarState::new(800, 1000);
        assert!(matches!(state.level, TokenLevel::Critical));
    }

    #[test]
    fn token_bar_at_boundary_50_percent() {
        let state = TokenBarState::new(500, 1000);
        assert!(matches!(state.level, TokenLevel::Warning));
    }

    #[test]
    fn token_bar_at_boundary_80_percent() {
        let state = TokenBarState::new(800, 1000);
        assert!(matches!(state.level, TokenLevel::Critical));
    }

    #[test]
    fn token_bar_zero_max() {
        let state = TokenBarState::new(100, 0);
        assert!((state.percentage - 0.0).abs() < f64::EPSILON);
        assert!(matches!(state.level, TokenLevel::Ok));
    }

    #[test]
    fn token_bar_full() {
        let state = TokenBarState::new(1000, 1000);
        assert!((state.percentage - 1.0).abs() < f64::EPSILON);
        assert!(matches!(state.level, TokenLevel::Critical));
    }

    #[test]
    fn token_bar_over_max() {
        let state = TokenBarState::new(1500, 1000);
        assert!(state.percentage > 1.0);
        assert!(matches!(state.level, TokenLevel::Critical));
    }

    #[test]
    fn message_serialization_roundtrip() {
        let msg = Message {
            role: "user".to_string(),
            content: "Hello world".to_string(),
            tool_calls: None,
            tool_name: None,
            token_count: 5,
            is_compacted: false,
            created_at: 1000,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let deserialized: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.role, "user");
        assert_eq!(deserialized.content, "Hello world");
        assert_eq!(deserialized.token_count, 5);
        assert!(deserialized.tool_calls.is_none());
    }

    #[test]
    fn message_optional_fields_skip_none() {
        let msg = Message {
            role: "assistant".to_string(),
            content: "Hi".to_string(),
            tool_calls: None,
            tool_name: None,
            token_count: 0,
            is_compacted: false,
            created_at: 0,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("tool_calls"));
        assert!(!json.contains("tool_name"));
    }

    #[test]
    fn finish_reason_equality() {
        assert_eq!(FinishReason::Stop, FinishReason::Stop);
        assert_ne!(FinishReason::Stop, FinishReason::Length);
    }

    #[test]
    fn usage_serialization() {
        let usage = Usage { prompt_tokens: 10, completion_tokens: 20, total_tokens: 30 };
        let json = serde_json::to_string(&usage).unwrap();
        let deserialized: Usage = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.total_tokens, 30);
    }
}
