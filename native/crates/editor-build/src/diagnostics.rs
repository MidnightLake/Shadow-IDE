use regex::Regex;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct BuildDiagnostic {
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
    pub severity: DiagnosticSeverity,
    pub message: String,
    pub code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSeverity {
    Error,
    Warning,
    Note,
}

impl std::fmt::Display for DiagnosticSeverity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Error => write!(f, "error"),
            Self::Warning => write!(f, "warning"),
            Self::Note => write!(f, "note"),
        }
    }
}

/// Parse clang/gcc compiler output into structured diagnostics.
///
/// Handles formats:
///   file.cpp:10:5: error: undeclared identifier 'x'
///   file.cpp:10:5: error: undeclared identifier 'x' [-Werror,-Wundefined]
///   file.cpp:10: error: ...
pub fn parse_compiler_output(output: &str) -> Vec<BuildDiagnostic> {
    // file:line:col: severity: message
    let re_full = Regex::new(
        r"^(.+?):(\d+):(\d+):\s*(error|warning|note):\s*(.+)$"
    ).unwrap();
    // file:line: severity: message (no column)
    let re_no_col = Regex::new(
        r"^(.+?):(\d+):\s*(error|warning|note):\s*(.+)$"
    ).unwrap();

    let mut diagnostics = Vec::new();

    for line in output.lines() {
        let line = line.trim();

        if let Some(caps) = re_full.captures(line) {
            let message_raw = caps[5].to_string();
            let (message, code) = extract_diagnostic_code(&message_raw);
            diagnostics.push(BuildDiagnostic {
                file: PathBuf::from(&caps[1]),
                line: caps[2].parse().unwrap_or(0),
                column: caps[3].parse().unwrap_or(0),
                severity: parse_severity(&caps[4]),
                message,
                code,
            });
        } else if let Some(caps) = re_no_col.captures(line) {
            let message_raw = caps[4].to_string();
            let (message, code) = extract_diagnostic_code(&message_raw);
            diagnostics.push(BuildDiagnostic {
                file: PathBuf::from(&caps[1]),
                line: caps[2].parse().unwrap_or(0),
                column: 0,
                severity: parse_severity(&caps[3]),
                message,
                code,
            });
        }
    }

    diagnostics
}

fn parse_severity(s: &str) -> DiagnosticSeverity {
    match s {
        "error" => DiagnosticSeverity::Error,
        "warning" => DiagnosticSeverity::Warning,
        "note" => DiagnosticSeverity::Note,
        _ => DiagnosticSeverity::Error,
    }
}

/// Extract diagnostic code from brackets at end of message, e.g. [-Werror,-Wfoo]
fn extract_diagnostic_code(message: &str) -> (String, Option<String>) {
    let re_code = Regex::new(r"\s*\[([^\]]+)\]\s*$").unwrap();
    if let Some(caps) = re_code.captures(message) {
        let code = caps[1].to_string();
        let clean_msg = message[..caps.get(0).unwrap().start()].to_string();
        (clean_msg, Some(code))
    } else {
        (message.to_string(), None)
    }
}

/// Summary counts for quick display.
pub fn diagnostic_summary(diagnostics: &[BuildDiagnostic]) -> (usize, usize, usize) {
    let errors = diagnostics.iter().filter(|d| d.severity == DiagnosticSeverity::Error).count();
    let warnings = diagnostics.iter().filter(|d| d.severity == DiagnosticSeverity::Warning).count();
    let notes = diagnostics.iter().filter(|d| d.severity == DiagnosticSeverity::Note).count();
    (errors, warnings, notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_clang_error() {
        let output = "src/main.cpp:42:10: error: use of undeclared identifier 'foo'";
        let diags = parse_compiler_output(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, 42);
        assert_eq!(diags[0].column, 10);
        assert_eq!(diags[0].severity, DiagnosticSeverity::Error);
        assert!(diags[0].message.contains("undeclared identifier"));
    }

    #[test]
    fn test_parse_warning_with_code() {
        let output = "src/player.cpp:15:3: warning: unused variable 'x' [-Wunused-variable]";
        let diags = parse_compiler_output(output);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, DiagnosticSeverity::Warning);
        assert_eq!(diags[0].code.as_deref(), Some("-Wunused-variable"));
        assert!(!diags[0].message.contains("[-W"));
    }

    #[test]
    fn test_parse_multiple() {
        let output = "\
src/a.cpp:1:1: error: expected ';'
src/a.cpp:2:5: warning: implicit conversion [-Wconversion]
src/a.cpp:3:1: note: in expansion of macro";
        let diags = parse_compiler_output(output);
        assert_eq!(diags.len(), 3);
    }
}
