use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize, Clone)]
pub struct TodoItem {
    pub file: String,
    pub line: usize,
    pub marker: String,
    pub text: String,
    pub priority: String,
}

const MARKERS: &[(&str, &str)] = &[
    ("BUG", "high"),
    ("FIXME", "high"),
    ("HACK", "medium"),
    ("TODO", "medium"),
    ("XXX", "medium"),
    ("WARN", "low"),
    ("NOTE", "low"),
];

const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "dist",
    "build",
    ".next",
    "__pycache__",
    ".cache",
    "vendor",
    ".venv",
];

const CODE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "py", "go", "c", "cpp", "h", "hpp", "java", "kt", "swift",
    "rb", "php", "css", "scss", "html", "vue", "svelte", "toml", "yaml", "yml", "json", "md", "sh",
    "bash", "zsh", "lua", "zig", "dart", "cs",
];

// ===== Scanning =====

pub fn scan_directory(root: &str) -> Vec<TodoItem> {
    let mut items = Vec::new();
    scan_recursive(Path::new(root), &mut items, 0);
    // Sort by priority (high first), then by file
    items.sort_by(|a, b| {
        priority_order(&a.priority)
            .cmp(&priority_order(&b.priority))
            .then_with(|| a.file.cmp(&b.file))
            .then_with(|| a.line.cmp(&b.line))
    });
    items
}

fn priority_order(priority: &str) -> u8 {
    match priority {
        "high" => 0,
        "medium" => 1,
        "low" => 2,
        _ => 3,
    }
}

fn scan_recursive(dir: &Path, results: &mut Vec<TodoItem>, depth: usize) {
    if depth > 8 || results.len() > 500 {
        return;
    }

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files/dirs
        if name.starts_with('.') && depth > 0 {
            continue;
        }

        let is_dir = entry.file_type().map(|ft| ft.is_dir()).unwrap_or(false);

        if is_dir {
            if !SKIP_DIRS.contains(&name.as_str()) {
                scan_recursive(&entry.path(), results, depth + 1);
            }
            continue;
        }

        // Check extension
        let ext = Path::new(&name)
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !CODE_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }

        // Skip large files
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if size > 1024 * 1024 || size == 0 {
            continue;
        }

        scan_file(&entry.path(), results);
    }
}

fn scan_file(path: &Path, results: &mut Vec<TodoItem>) {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let file_str = path.to_string_lossy().to_string();

    for (line_num, line) in content.lines().enumerate() {
        let upper = line.to_uppercase();

        for (marker, priority) in MARKERS {
            // Match marker followed by : or whitespace or end of line
            if let Some(pos) = upper.find(marker) {
                // Verify it's a standalone marker (not part of a word like "NOTABLE")
                let after_marker = pos + marker.len();
                let char_after = upper.chars().nth(after_marker);
                let is_boundary = match char_after {
                    None => true,
                    Some(c) => !c.is_alphanumeric() && c != '_',
                };

                if !is_boundary {
                    continue;
                }

                // Extract the text after the marker
                let rest = &line[after_marker..];
                let text = rest
                    .trim_start_matches(|c: char| c == ':' || c == ' ' || c == '(' || c == ')')
                    .trim();

                let display_text = if text.is_empty() {
                    line.trim().to_string()
                } else if text.len() > 150 {
                    format!("{}...", &text[..150])
                } else {
                    text.to_string()
                };

                results.push(TodoItem {
                    file: file_str.clone(),
                    line: line_num + 1,
                    marker: marker.to_string(),
                    text: display_text,
                    priority: priority.to_string(),
                });
                break; // Only first marker per line
            }
        }
    }
}

// ===== Tauri Commands =====

#[tauri::command]
pub fn scan_todos(path: String) -> Vec<TodoItem> {
    scan_directory(&path)
}

#[tauri::command]
pub fn scan_file_todos(path: String) -> Vec<TodoItem> {
    let mut items = Vec::new();
    scan_file(Path::new(&path), &mut items);
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn create_temp_file(name: &str, content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join(name);
        fs::write(&file_path, content).unwrap();
        (dir, file_path)
    }

    #[test]
    fn test_scan_file_finds_todo() {
        let (_dir, path) = create_temp_file("test.rs", "fn main() {\n    // TODO: fix this\n}\n");
        let mut items = Vec::new();
        scan_file(&path, &mut items);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].marker, "TODO");
        assert_eq!(items[0].line, 2);
        assert!(items[0].text.contains("fix this"));
    }

    #[test]
    fn test_scan_file_finds_multiple_markers() {
        let (_dir, path) = create_temp_file(
            "test.rs",
            "// BUG: crash here\nfn foo() {}\n// FIXME: broken\n// HACK: workaround\n",
        );
        let mut items = Vec::new();
        scan_file(&path, &mut items);
        assert_eq!(items.len(), 3);
        let markers: Vec<&str> = items.iter().map(|i| i.marker.as_str()).collect();
        assert!(markers.contains(&"BUG"));
        assert!(markers.contains(&"FIXME"));
        assert!(markers.contains(&"HACK"));
    }

    #[test]
    fn test_scan_file_notable_not_matched_as_note() {
        let (_dir, path) = create_temp_file("test.rs", "let notable = true;\n");
        let mut items = Vec::new();
        scan_file(&path, &mut items);
        // "NOTABLE" should NOT match "NOTE" because 'A' follows the marker
        assert_eq!(items.len(), 0);
    }

    #[test]
    fn test_scan_file_note_boundary() {
        let (_dir, path) = create_temp_file("test.rs", "// NOTE: this is important\n");
        let mut items = Vec::new();
        scan_file(&path, &mut items);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].marker, "NOTE");
    }

    #[test]
    fn test_scan_file_warn_at_end_of_line() {
        let (_dir, path) = create_temp_file("test.rs", "// WARN\n");
        let mut items = Vec::new();
        scan_file(&path, &mut items);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].marker, "WARN");
    }

    #[test]
    fn test_priority_order_values() {
        assert!(priority_order("high") < priority_order("medium"));
        assert!(priority_order("medium") < priority_order("low"));
        assert!(priority_order("low") < priority_order("unknown"));
    }

    #[test]
    fn test_scan_directory_sorts_by_priority() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("test.rs"),
            "// NOTE: low priority\n// BUG: high priority\n// TODO: medium priority\n",
        )
        .unwrap();
        let items = scan_directory(dir.path().to_str().unwrap());
        assert!(items.len() >= 3);
        // BUG (high) should come before TODO (medium) which comes before NOTE (low)
        let first_marker = &items[0].marker;
        assert_eq!(first_marker, "BUG");
    }

    #[test]
    fn test_scan_directory_skips_node_modules() {
        let dir = tempfile::tempdir().unwrap();
        let nm = dir.path().join("node_modules");
        fs::create_dir_all(&nm).unwrap();
        fs::write(nm.join("dep.js"), "// TODO: should be skipped\n").unwrap();
        fs::write(dir.path().join("main.rs"), "// TODO: should be found\n").unwrap();
        let items = scan_directory(dir.path().to_str().unwrap());
        assert_eq!(items.len(), 1);
        assert!(items[0].file.contains("main.rs"));
    }

    #[test]
    fn test_scan_file_empty_file() {
        let (_dir, path) = create_temp_file("empty.rs", "");
        let mut items = Vec::new();
        scan_file(&path, &mut items);
        assert_eq!(items.len(), 0);
    }

    #[test]
    fn test_scan_file_long_text_truncated() {
        let long_text = format!("// TODO: {}", "x".repeat(200));
        let (_dir, path) = create_temp_file("test.rs", &long_text);
        let mut items = Vec::new();
        scan_file(&path, &mut items);
        assert_eq!(items.len(), 1);
        assert!(items[0].text.ends_with("..."));
        assert!(items[0].text.len() <= 154); // 150 + "..."
    }
}
