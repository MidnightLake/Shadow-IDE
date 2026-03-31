use std::fmt;

#[derive(Debug)]
pub enum FerrumError {
    Config(String),
    Io(std::io::Error),
    Db(String),
    Api(String),
    Parse(String),
}

impl fmt::Display for FerrumError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Config(msg) => write!(f, "Config error: {}", msg),
            Self::Io(err) => write!(f, "IO error: {}", err),
            Self::Db(msg) => write!(f, "Database error: {}", msg),
            Self::Api(msg) => write!(f, "API error: {}", msg),
            Self::Parse(msg) => write!(f, "Parse error: {}", msg),
        }
    }
}

impl std::error::Error for FerrumError {}

impl From<std::io::Error> for FerrumError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<toml::de::Error> for FerrumError {
    fn from(err: toml::de::Error) -> Self {
        Self::Parse(err.to_string())
    }
}

impl From<serde_json::Error> for FerrumError {
    fn from(err: serde_json::Error) -> Self {
        Self::Parse(err.to_string())
    }
}
