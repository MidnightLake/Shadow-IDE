use serde::{Deserialize, Serialize};
use crate::error::FerrumError;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default, rename = "profiles")]
    pub profiles: Vec<Profile>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Defaults {
    #[serde(default = "default_profile")]
    pub profile: String,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_auto_compact")]
    pub auto_compact_at: f64,
    #[serde(default = "default_rag_clean_days")]
    pub rag_auto_clean_days: u32,
    #[serde(default = "default_cache_ttl")]
    pub cache_ttl_seconds: u64,
    #[serde(default = "default_true")]
    pub show_token_bar: bool,
    #[serde(default = "default_true")]
    pub show_thinking: bool,
    #[serde(default)]
    pub stream_delay_ms: u32,
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            profile: default_profile(),
            theme: default_theme(),
            auto_compact_at: default_auto_compact(),
            rag_auto_clean_days: default_rag_clean_days(),
            cache_ttl_seconds: default_cache_ttl(),
            show_token_bar: true,
            show_thinking: true,
            stream_delay_ms: 0,
        }
    }
}

fn default_profile() -> String { "local-llama".to_string() }
fn default_theme() -> String { "dark".to_string() }
fn default_auto_compact() -> f64 { 0.80 }
fn default_rag_clean_days() -> u32 { 14 }
fn default_cache_ttl() -> u64 { 3600 }
fn default_true() -> bool { true }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    #[serde(default = "default_provider")]
    pub provider: String,
    pub model: String,
    pub base_url: String,
    #[serde(default)]
    pub api_key_env: String,
    #[serde(default = "default_context")]
    pub max_context_tokens: u32,
    #[serde(default)]
    pub system_prompt: String,
    #[serde(default)]
    pub tools: Vec<String>,
}

fn default_provider() -> String { "openai".to_string() }
fn default_context() -> u32 { 120000 }

impl Config {
    pub fn config_path() -> PathBuf {
        dirs_next::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("ferrum-chat")
            .join("config.toml")
    }

    pub fn load() -> Result<Self, FerrumError> {
        let path = Self::config_path();
        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let config: Config = toml::from_str(&content)?;
            Ok(config)
        } else {
            Ok(Self::default_config())
        }
    }

    pub fn save(&self) -> Result<(), FerrumError> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)
            .map_err(|e| FerrumError::Config(e.to_string()))?;
        std::fs::write(&path, content)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    pub fn default_config() -> Self {
        Config {
            defaults: Defaults::default(),
            profiles: vec![
                Profile {
                    name: "local-llama".to_string(),
                    provider: "ollama".to_string(),
                    model: "llama3.2".to_string(),
                    base_url: "http://localhost:11434/v1".to_string(),
                    api_key_env: String::new(),
                    max_context_tokens: 120000,
                    system_prompt: "You are a helpful assistant.".to_string(),
                    tools: vec!["shell".into(), "read_file".into()],
                },
                Profile {
                    name: "lm-studio".to_string(),
                    provider: "openai".to_string(),
                    model: "default".to_string(),
                    base_url: "http://localhost:1234/v1".to_string(),
                    api_key_env: String::new(),
                    max_context_tokens: 120000,
                    system_prompt: "You are a helpful assistant.".to_string(),
                    tools: vec!["shell".into(), "read_file".into(), "write_file".into()],
                },
            ],
        }
    }

    pub fn get_profile(&self, name: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.name == name)
    }

    pub fn get_default_profile(&self) -> Option<&Profile> {
        self.get_profile(&self.defaults.profile)
            .or_else(|| self.profiles.first())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_two_profiles() {
        let config = Config::default_config();
        assert_eq!(config.profiles.len(), 2);
        assert_eq!(config.profiles[0].name, "local-llama");
        assert_eq!(config.profiles[1].name, "lm-studio");
    }

    #[test]
    fn default_config_defaults() {
        let config = Config::default_config();
        assert_eq!(config.defaults.profile, "local-llama");
        assert_eq!(config.defaults.theme, "dark");
        assert!((config.defaults.auto_compact_at - 0.80).abs() < f64::EPSILON);
        assert_eq!(config.defaults.cache_ttl_seconds, 3600);
        assert!(config.defaults.show_token_bar);
        assert!(config.defaults.show_thinking);
    }

    #[test]
    fn get_profile_found() {
        let config = Config::default_config();
        let profile = config.get_profile("local-llama");
        assert!(profile.is_some());
        assert_eq!(profile.unwrap().model, "llama3.2");
    }

    #[test]
    fn get_profile_not_found() {
        let config = Config::default_config();
        assert!(config.get_profile("nonexistent").is_none());
    }

    #[test]
    fn get_default_profile_matches_defaults() {
        let config = Config::default_config();
        let profile = config.get_default_profile().unwrap();
        assert_eq!(profile.name, "local-llama");
    }

    #[test]
    fn get_default_profile_falls_back_to_first() {
        let mut config = Config::default_config();
        config.defaults.profile = "nonexistent".to_string();
        let profile = config.get_default_profile().unwrap();
        assert_eq!(profile.name, "local-llama");
    }

    #[test]
    fn get_default_profile_empty_profiles() {
        let mut config = Config::default_config();
        config.defaults.profile = "nonexistent".to_string();
        config.profiles.clear();
        assert!(config.get_default_profile().is_none());
    }

    #[test]
    fn config_toml_roundtrip() {
        let config = Config::default_config();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let parsed: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(parsed.profiles.len(), 2);
        assert_eq!(parsed.defaults.profile, "local-llama");
        assert_eq!(parsed.profiles[0].model, "llama3.2");
    }

    #[test]
    fn config_deserialize_minimal() {
        let toml_str = r#"
[defaults]
profile = "test"

[[profiles]]
name = "test"
model = "gpt-4"
base_url = "http://localhost:8080/v1"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.profiles.len(), 1);
        assert_eq!(config.profiles[0].max_context_tokens, 120000); // default
        assert_eq!(config.profiles[0].provider, "openai"); // default
    }

    #[test]
    fn profile_tools_list() {
        let config = Config::default_config();
        let llama = config.get_profile("local-llama").unwrap();
        assert_eq!(llama.tools, vec!["shell", "read_file"]);
        let studio = config.get_profile("lm-studio").unwrap();
        assert_eq!(studio.tools, vec!["shell", "read_file", "write_file"]);
    }
}
