/// Shared error type for Shadow IDE backend.
/// Implements conversions from std and third-party error types without thiserror.

#[derive(Debug)]
pub enum ShadowError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Reqwest(reqwest::Error),
    Tauri(String),
    Custom(String),
}

impl std::fmt::Display for ShadowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShadowError::Io(e) => write!(f, "IO error: {}", e),
            ShadowError::Json(e) => write!(f, "JSON error: {}", e),
            ShadowError::Reqwest(e) => write!(f, "HTTP error: {}", e),
            ShadowError::Tauri(s) => write!(f, "Tauri error: {}", s),
            ShadowError::Custom(s) => write!(f, "{}", s),
        }
    }
}

impl std::error::Error for ShadowError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ShadowError::Io(e) => Some(e),
            ShadowError::Json(e) => Some(e),
            ShadowError::Reqwest(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ShadowError {
    fn from(e: std::io::Error) -> Self {
        ShadowError::Io(e)
    }
}

impl From<serde_json::Error> for ShadowError {
    fn from(e: serde_json::Error) -> Self {
        ShadowError::Json(e)
    }
}

impl From<reqwest::Error> for ShadowError {
    fn from(e: reqwest::Error) -> Self {
        ShadowError::Reqwest(e)
    }
}

impl From<String> for ShadowError {
    fn from(s: String) -> Self {
        ShadowError::Custom(s)
    }
}

impl From<&str> for ShadowError {
    fn from(s: &str) -> Self {
        ShadowError::Custom(s.to_string())
    }
}

pub type ShadowResult<T> = Result<T, ShadowError>;

impl ShadowError {
    pub fn to_tauri_err(self) -> String {
        self.to_string()
    }
}
