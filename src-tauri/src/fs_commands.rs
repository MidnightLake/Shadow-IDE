use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::Emitter;

/// Sensitive system directories that should never be accessed through the UI.
const SENSITIVE_DIRS: &[&str] = &[
    "/etc",
    "/proc",
    "/sys",
    "/root",
    "/boot",
    "/dev",
    "/var/run",
    "/var/lock",
    "/run",
];

/// Sanitize and validate a file path string to prevent path traversal attacks.
/// Returns Err if path is suspicious or contains forbidden characters.
/// Call this before constructing a PathBuf from untrusted input.
pub fn sanitize_path_str(path: &str) -> Result<PathBuf, String> {
    if path.is_empty() {
        return Err("Path cannot be empty".to_string());
    }
    if path.contains('\0') {
        return Err("Path contains null bytes".to_string());
    }
    if path.contains("../") || path.contains("..\\") || path == ".." {
        return Err("Path traversal detected".to_string());
    }
    let forbidden = ['|', ';', '&', '$', '`', '>', '<', '!'];
    if path.chars().any(|c| forbidden.contains(&c)) {
        return Err("Path contains forbidden character".to_string());
    }
    Ok(PathBuf::from(path))
}

/// Validate that a path is safe to access. Rejects paths that:
/// - Contain `..` components after canonicalization
/// - Resolve to sensitive system directories
fn sanitize_path(path: &Path) -> Result<PathBuf, String> {
    // Canonicalize to resolve symlinks, `.`, and `..`
    // If path doesn't exist yet (e.g. write target), canonicalize the parent
    let canonical = if path.exists() {
        path.canonicalize()
    } else if let Some(parent) = path.parent() {
        if parent.as_os_str().is_empty() || !parent.exists() {
            // For deeply nested new paths, just check string components
            let path_str = path.to_string_lossy();
            for sensitive in SENSITIVE_DIRS {
                if path_str.starts_with(&format!("{}/", sensitive)) || path_str == *sensitive {
                    return Err(format!(
                        "Access denied: path {} is under sensitive directory {}",
                        path.display(),
                        sensitive
                    ));
                }
            }
            return Ok(path.to_path_buf());
        } else {
            parent
                .canonicalize()
                .map(|p| p.join(path.file_name().unwrap_or_default()))
        }
    } else {
        path.canonicalize()
    }
    .map_err(|e| format!("Path validation failed for {}: {}", path.display(), e))?;

    // Check that no `..` component remains (shouldn't after canonicalize, but belt-and-suspenders)
    for component in canonical.components() {
        if let std::path::Component::ParentDir = component {
            return Err(format!("Path traversal rejected: {}", path.display()));
        }
    }

    // Reject sensitive system directories
    let canonical_str = canonical.to_string_lossy();
    for sensitive in SENSITIVE_DIRS {
        if canonical_str == *sensitive || canonical_str.starts_with(&format!("{}/", sensitive)) {
            return Err(format!(
                "Access denied: path {} is under sensitive directory {}",
                path.display(),
                sensitive
            ));
        }
    }

    Ok(canonical)
}

#[derive(Debug, Serialize, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub extension: Option<String>,
}

#[tauri::command]
pub fn read_directory(path: String, show_hidden: Option<bool>) -> Result<Vec<FileEntry>, String> {
    let dir_path = PathBuf::from(&path);

    if !dir_path.exists() {
        return Err(format!("Path does not exist: {}", path));
    }

    if !dir_path.is_dir() {
        return Err(format!("Path is not a directory: {}", path));
    }

    let dir_path = sanitize_path(&dir_path)?;

    let read_dir =
        fs::read_dir(&dir_path).map_err(|e| format!("Failed to read directory: {}", e))?;
    let show = show_hidden.unwrap_or(false);

    // Collect entries efficiently with pre-allocated capacity
    let raw_entries: Vec<_> = read_dir.filter_map(|e| e.ok()).collect();
    let mut entries: Vec<FileEntry> = Vec::with_capacity(raw_entries.len());

    for entry in raw_entries {
        let file_name = entry.file_name().to_string_lossy().to_string();

        // Skip hidden files starting with '.' unless show_hidden is true
        if !show && file_name.starts_with('.') {
            continue;
        }

        // Use symlink_metadata to avoid following symlinks (faster)
        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        let entry_path = entry.path();
        let file_path = entry_path.to_string_lossy().to_string();
        let is_dir = metadata.is_dir();
        let is_symlink = metadata.is_symlink();
        // Only get size for files (dirs report 0 or block size)
        let size = if is_dir { 0 } else { metadata.len() };
        let extension = entry_path
            .extension()
            .map(|e| e.to_string_lossy().to_string());

        entries.push(FileEntry {
            name: file_name,
            path: file_path,
            is_dir,
            is_symlink,
            size,
            extension,
        });
    }

    // Sort: directories first, then files, both alphabetically
    entries.sort_by(|a, b| {
        if a.is_dir == b.is_dir {
            a.name.to_lowercase().cmp(&b.name.to_lowercase())
        } else if a.is_dir {
            std::cmp::Ordering::Less
        } else {
            std::cmp::Ordering::Greater
        }
    });

    Ok(entries)
}

#[tauri::command]
pub fn read_file_content(path: String) -> Result<String, String> {
    sanitize_path_str(&path)?;
    let file_path = PathBuf::from(&path);

    if !file_path.exists() {
        return Err(format!("File does not exist: {}", path));
    }

    if !file_path.is_file() {
        return Err(format!("Path is not a file: {}", path));
    }

    let file_path = sanitize_path(&file_path)?;

    // Check file size - limit to 50MB for safety
    let metadata =
        fs::metadata(&file_path).map_err(|e| format!("Failed to read metadata: {}", e))?;
    if metadata.len() > 50 * 1024 * 1024 {
        return Err(
            "File is too large (> 50MB). Use read_file_chunk for partial reading.".to_string(),
        );
    }

    fs::read_to_string(&file_path).map_err(|e| format!("Failed to read file: {}", e))
}

#[tauri::command]
pub fn write_file_content(path: String, content: String) -> Result<(), String> {
    sanitize_path_str(&path)?;
    let file_path = PathBuf::from(&path);

    // Reject directory paths early with a clear, actionable message
    if file_path.is_dir() {
        let suggestion = path.trim_end_matches('/');
        return Err(format!(
            "Failed to write '{}': Is a directory (os error 21). \
             The path points to a directory, not a file. \
             Specify a filename inside it, e.g. '{}/PLAN.md' or '{}/README.md'.",
            path, suggestion, suggestion
        ));
    }

    // Ensure parent directory exists
    if let Some(parent) = file_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create parent directories: {}", e))?;
        }
    }

    // Warn on very large writes (> 500KB) — likely a mistake
    const MAX_WRITE_SIZE: usize = 512 * 1024;
    if content.len() > MAX_WRITE_SIZE {
        return Err(format!(
            "Content too large to write in one call ({} bytes, limit {}KB). \
             Split into multiple write_file or patch_file calls.",
            content.len(),
            MAX_WRITE_SIZE / 1024
        ));
    }

    // Validate after ensuring parents exist so canonicalize can resolve
    let file_path = sanitize_path(&file_path)?;

    fs::write(&file_path, content).map_err(|e| format!("Failed to write '{}': {}", path, e))
}

#[tauri::command]
pub fn create_file_with_template(path: String, template: String) -> Result<String, String> {
    let file_path = PathBuf::from(&path);

    // Determine extension from template
    let ext = match template.as_str() {
        "rust" => "rs",
        "typescript" => "ts",
        "python" => "py",
        "markdown" => "md",
        "cpp" => "cpp",
        "go" => "go",
        _ => "txt",
    };

    // If path is a directory (or ends with '/'), create untitled.<ext> inside it
    let actual_path = if file_path.is_dir() || path.ends_with('/') {
        file_path.join(format!("untitled.{}", ext))
    } else {
        file_path.clone()
    };

    // Template content
    let content = match template.as_str() {
        "rust" => "fn main() {\n    println!(\"Hello, world!\");\n}\n",
        "typescript" => "export {};\n",
        "python" => "def main():\n    pass\n\nif __name__ == '__main__':\n    main()\n",
        "markdown" => "# Title\n\n",
        "cpp" => "#include <iostream>\n\nint main() {\n    std::cout << \"Hello, world!\" << std::endl;\n    return 0;\n}\n",
        "go" => "package main\n\nimport \"fmt\"\n\nfunc main() {\n    fmt.Println(\"Hello, world!\")\n}\n",
        _ => "",
    };

    // Ensure parent directory exists
    if let Some(parent) = actual_path.parent() {
        if !parent.as_os_str().is_empty() && !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create parent directories: {}", e))?;
        }
    }

    let safe_path = sanitize_path(&actual_path)?;
    fs::write(&safe_path, content).map_err(|e| format!("Failed to write template file: {}", e))?;
    Ok(safe_path.to_string_lossy().to_string())
}

/// Apply a line-level patch: replace lines `start_line..=end_line` with `new_content`.
/// Lines are 1-based. If new_content is empty, the specified lines are deleted.
/// If start_line > total lines, new_content is appended.
pub fn patch_file_lines(
    path: String,
    start_line: usize,
    end_line: usize,
    new_content: &str,
) -> Result<(), String> {
    if start_line == 0 {
        return Err("start_line must be >= 1".to_string());
    }
    if end_line < start_line {
        return Err("end_line must be >= start_line".to_string());
    }

    let file_path = PathBuf::from(&path);
    let content =
        fs::read_to_string(&file_path).map_err(|e| format!("Failed to read file: {}", e))?;

    let mut lines: Vec<&str> = content.lines().collect();
    // Preserve trailing newline info
    let had_trailing_newline = content.ends_with('\n');

    let total = lines.len();
    // Clamp indices to valid range
    let start_idx = (start_line - 1).min(total);
    let end_idx = end_line.min(total);

    // Build replacement lines
    let replacement: Vec<&str> = if new_content.is_empty() {
        Vec::new()
    } else {
        new_content.lines().collect()
    };

    // Splice: remove old range, insert new content
    lines.splice(start_idx..end_idx, replacement);

    // Reconstruct file content
    let mut result = lines.join("\n");
    if had_trailing_newline || result.is_empty() {
        result.push('\n');
    }

    fs::write(&file_path, result).map_err(|e| format!("Failed to write file: {}", e))
}

#[tauri::command]
pub fn get_home_dir() -> Result<String, String> {
    dirs_next::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .ok_or_else(|| "Could not determine home directory".to_string())
}

#[tauri::command]
pub fn create_directory(path: String) -> Result<(), String> {
    fs::create_dir_all(&path).map_err(|e| format!("Failed to create directory: {}", e))
}

#[tauri::command]
pub fn delete_entry(path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if p.is_dir() {
        fs::remove_dir_all(&p).map_err(|e| format!("Failed to delete directory: {}", e))
    } else {
        fs::remove_file(&p).map_err(|e| format!("Failed to delete file: {}", e))
    }
}

#[tauri::command]
pub fn rename_entry(old_path: String, new_path: String) -> Result<(), String> {
    fs::rename(&old_path, &new_path).map_err(|e| format!("Failed to rename: {}", e))
}

#[derive(Debug, Serialize)]
pub struct FileInfo {
    pub size: u64,
    pub is_binary: bool,
    pub line_count: Option<u64>,
}

#[tauri::command]
pub fn get_file_info(path: String) -> Result<FileInfo, String> {
    let file_path = PathBuf::from(&path);
    if !file_path.exists() {
        return Err(format!("File does not exist: {}", path));
    }
    let metadata =
        fs::metadata(&file_path).map_err(|e| format!("Failed to read metadata: {}", e))?;
    let size = metadata.len();

    // Check if binary by reading first 8KB and looking for null bytes
    let mut is_binary = false;
    let mut line_count = None;
    if metadata.is_file() && size > 0 {
        let mut file =
            fs::File::open(&file_path).map_err(|e| format!("Failed to open file: {}", e))?;
        let mut buf = vec![0u8; 8192.min(size as usize)];
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("Failed to read file: {}", e))?;
        is_binary = buf[..n].contains(&0);

        // Count lines for text files under 50MB
        if !is_binary && size < 50 * 1024 * 1024 {
            file.seek(SeekFrom::Start(0))
                .map_err(|e| format!("Failed to seek: {}", e))?;
            let mut content = Vec::new();
            file.read_to_end(&mut content)
                .map_err(|e| format!("Failed to read file: {}", e))?;
            line_count = Some(bytecount_lines(&content));
        }
    }

    Ok(FileInfo {
        size,
        is_binary,
        line_count,
    })
}

fn bytecount_lines(data: &[u8]) -> u64 {
    data.iter().filter(|&&b| b == b'\n').count() as u64
}

#[tauri::command]
pub fn read_file_chunk(path: String, offset: u64, length: u64) -> Result<String, String> {
    let file_path = PathBuf::from(&path);
    if !file_path.exists() {
        return Err(format!("File does not exist: {}", path));
    }

    let file_path = sanitize_path(&file_path)?;

    // Cap chunk size at 1MB
    let length = length.min(1024 * 1024);

    let mut file = fs::File::open(&file_path).map_err(|e| format!("Failed to open file: {}", e))?;
    file.seek(SeekFrom::Start(offset))
        .map_err(|e| format!("Failed to seek: {}", e))?;

    let len = usize::try_from(length)
        .map_err(|_| format!("Chunk size {} exceeds platform limit", length))?;
    let mut buf = vec![0u8; len];
    let n = file
        .read(&mut buf)
        .map_err(|e| format!("Failed to read file: {}", e))?;
    buf.truncate(n);

    String::from_utf8(buf).map_err(|_| "File chunk contains invalid UTF-8".to_string())
}

// ===== Search Commands =====

#[tauri::command]
pub fn search_files_by_name(root: String, pattern: String) -> Result<Vec<FileEntry>, String> {
    let pattern_lower = pattern.to_lowercase();
    let mut results = Vec::new();
    search_name_recursive(Path::new(&root), &pattern_lower, &mut results, 0);
    Ok(results)
}

const SKIP_DIRS: &[&str] = &["node_modules", ".git", "target", "dist", "build"];
const MAX_SEARCH_DEPTH: usize = 8;
const MAX_NAME_RESULTS: usize = 50;

fn search_name_recursive(dir: &Path, pattern: &str, results: &mut Vec<FileEntry>, depth: usize) {
    if depth > MAX_SEARCH_DEPTH || results.len() >= MAX_NAME_RESULTS {
        return;
    }

    let read_dir = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };

    for entry in read_dir {
        if results.len() >= MAX_NAME_RESULTS {
            return;
        }

        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let file_name = entry.file_name().to_string_lossy().to_string();

        // Skip ignored directories
        if SKIP_DIRS.contains(&file_name.as_str()) {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        let is_dir = metadata.is_dir();

        if file_name.to_lowercase().contains(pattern) {
            let file_path = entry.path().to_string_lossy().to_string();
            let is_symlink = metadata.is_symlink();
            let size = metadata.len();
            let extension = entry
                .path()
                .extension()
                .map(|e| e.to_string_lossy().to_string());

            results.push(FileEntry {
                name: file_name.clone(),
                path: file_path,
                is_dir,
                is_symlink,
                size,
                extension,
            });
        }

        if is_dir {
            search_name_recursive(&entry.path(), pattern, results, depth + 1);
        }
    }
}

#[derive(Debug, Serialize, Clone)]
pub struct ContentSearchResult {
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub text: String,
    pub match_text: String,
}

const MAX_CONTENT_RESULTS: usize = 200;
const MAX_FILE_SIZE_FOR_SEARCH: u64 = 512 * 1024; // 512KB

#[tauri::command]
pub fn search_in_files(
    root: String,
    pattern: String,
    extensions: Option<String>,
) -> Result<Vec<ContentSearchResult>, String> {
    let pattern_lower = pattern.to_lowercase();
    let ext_filter: Option<Vec<String>> = extensions.map(|exts| {
        exts.split(',')
            .map(|e| e.trim().to_lowercase())
            .filter(|e| !e.is_empty())
            .collect()
    });
    let mut results = Vec::new();
    search_content_recursive(
        Path::new(&root),
        &pattern_lower,
        &pattern,
        &ext_filter,
        &mut results,
        0,
    );
    Ok(results)
}

fn search_content_recursive(
    dir: &Path,
    pattern_lower: &str,
    pattern_original: &str,
    ext_filter: &Option<Vec<String>>,
    results: &mut Vec<ContentSearchResult>,
    depth: usize,
) {
    if depth > MAX_SEARCH_DEPTH || results.len() >= MAX_CONTENT_RESULTS {
        return;
    }

    let read_dir = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return,
    };

    for entry in read_dir {
        if results.len() >= MAX_CONTENT_RESULTS {
            return;
        }

        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        let file_name = entry.file_name().to_string_lossy().to_string();

        if SKIP_DIRS.contains(&file_name.as_str()) {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        if metadata.is_dir() {
            search_content_recursive(
                &entry.path(),
                pattern_lower,
                pattern_original,
                ext_filter,
                results,
                depth + 1,
            );
            continue;
        }

        if !metadata.is_file() {
            continue;
        }

        // Skip large files
        if metadata.len() > MAX_FILE_SIZE_FOR_SEARCH {
            continue;
        }

        // Check extension filter
        if let Some(ref exts) = ext_filter {
            let file_ext = entry
                .path()
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if !exts.contains(&file_ext) {
                continue;
            }
        }

        // Read file and search
        let content = match fs::read_to_string(entry.path()) {
            Ok(c) => c,
            Err(_) => continue, // Skip binary/unreadable files
        };

        let file_path = entry.path().to_string_lossy().to_string();

        for (line_idx, line) in content.lines().enumerate() {
            if results.len() >= MAX_CONTENT_RESULTS {
                return;
            }

            let line_lower = line.to_lowercase();
            if let Some(col) = line_lower.find(pattern_lower) {
                let match_end = col + pattern_lower.len();
                let match_text = line[col..match_end].to_string();
                results.push(ContentSearchResult {
                    file: file_path.clone(),
                    line: line_idx + 1,
                    column: col + 1,
                    text: line.to_string(),
                    match_text,
                });
            }
        }
    }
}

#[tauri::command]
pub fn replace_in_files(
    root: String,
    search: String,
    replace: String,
    file_paths: Vec<String>,
) -> Result<usize, String> {
    let _ = &root; // root provided for context, file_paths are absolute
    let mut total_replacements: usize = 0;

    for file_path in &file_paths {
        let path = Path::new(file_path);
        if !path.is_file() {
            continue;
        }

        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let count = content.matches(&search).count();
        if count == 0 {
            continue;
        }

        let new_content = content.replace(&search, &replace);
        if let Err(e) = fs::write(path, &new_content) {
            return Err(format!("Failed to write {}: {}", file_path, e));
        }

        total_replacements += count;
    }

    Ok(total_replacements)
}

// ===== Workspace File Watcher =====

pub struct WatcherState {
    watcher: Mutex<Option<RecommendedWatcher>>,
    watched_path: Mutex<Option<String>>,
}

impl WatcherState {
    pub fn new() -> Self {
        Self {
            watcher: Mutex::new(None),
            watched_path: Mutex::new(None),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
struct FsChangeEvent {
    kind: String, // "create", "modify", "remove", "rename"
    paths: Vec<String>,
    dir: String, // parent directory that changed
}

/// Directories to skip watching (reduces inotify/ReadDirectoryChanges pressure)
const WATCH_SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "target",
    "dist",
    "build",
    "__pycache__",
    ".next",
    ".cache",
    ".venv",
    "venv",
    ".svn",
    ".hg",
    "vendor",
    "bower_components",
    ".gradle",
    ".idea",
    ".vs",
    "obj",
    "bin",
    "Debug",
    "Release",
    ".tox",
    ".mypy_cache",
    ".pytest_cache",
    "coverage",
    ".nyc_output",
    ".turbo",
];

fn should_skip_path(path: &Path) -> bool {
    for component in path.components() {
        if let std::path::Component::Normal(name) = component {
            let name_str = name.to_string_lossy();
            if WATCH_SKIP_DIRS.contains(&name_str.as_ref()) {
                return true;
            }
        }
    }
    false
}

#[tauri::command]
pub fn watch_workspace(
    root_path: String,
    app: tauri::AppHandle,
    state: tauri::State<'_, WatcherState>,
) -> Result<(), String> {
    let root = PathBuf::from(&root_path);
    if !root.exists() || !root.is_dir() {
        return Err(format!("Invalid workspace path: {}", root_path));
    }

    // Stop existing watcher
    if let Ok(mut w) = state.watcher.lock() {
        *w = None;
    }

    // Use a channel + debounce thread to batch rapid filesystem events.
    // Without this, Windows ReadDirectoryChangesW floods the frontend.
    let (tx, rx) = std::sync::mpsc::channel::<notify::Event>();

    let root_clone = root_path.clone();
    std::thread::spawn(move || {
        use std::collections::HashMap;
        let debounce = std::time::Duration::from_millis(250);
        loop {
            // Wait for at least one event
            let first = match rx.recv() {
                Ok(e) => e,
                Err(_) => break, // channel closed
            };

            // Batch events for debounce period
            let mut batch: HashMap<String, String> = HashMap::new();
            let mut process_event = |event: notify::Event| {
                let kind = match event.kind {
                    notify::EventKind::Create(_) => "create",
                    notify::EventKind::Modify(_) => "modify",
                    notify::EventKind::Remove(_) => "remove",
                    _ => return,
                };
                for p in &event.paths {
                    if !should_skip_path(p) {
                        batch.insert(p.to_string_lossy().to_string(), kind.to_string());
                    }
                }
            };
            process_event(first);

            // Drain more events within the debounce window
            let deadline = std::time::Instant::now() + debounce;
            loop {
                let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match rx.recv_timeout(remaining) {
                    Ok(e) => process_event(e),
                    Err(_) => break,
                }
            }

            if batch.is_empty() {
                continue;
            }

            // Group by kind and emit batched events
            let mut by_kind: HashMap<String, Vec<String>> = HashMap::new();
            for (path, kind) in batch {
                by_kind.entry(kind).or_default().push(path);
            }
            for (kind, paths) in by_kind {
                let dir = paths
                    .first()
                    .and_then(|p| Path::new(p).parent())
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| root_clone.clone());
                let _ = app.emit("workspace-fs-changed", FsChangeEvent { kind, paths, dir });
            }
        }
    });

    let mut watcher =
        notify::recommended_watcher(move |res: Result<notify::Event, notify::Error>| match res {
            Ok(event) => {
                let _ = tx.send(event);
            }
            Err(e) => {
                log::warn!("File watcher error: {}", e);
            }
        })
        .map_err(|e| format!("Failed to create watcher: {}", e))?;

    watcher
        .watch(&root, RecursiveMode::Recursive)
        .map_err(|e| format!("Failed to watch {}: {}", root_path, e))?;

    if let Ok(mut w) = state.watcher.lock() {
        *w = Some(watcher);
    }
    if let Ok(mut wp) = state.watched_path.lock() {
        *wp = Some(root_path);
    }

    Ok(())
}

#[tauri::command]
pub fn unwatch_workspace(state: tauri::State<'_, WatcherState>) -> Result<(), String> {
    if let Ok(mut w) = state.watcher.lock() {
        *w = None;
    }
    if let Ok(mut wp) = state.watched_path.lock() {
        *wp = None;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs as stdfs;

    #[test]
    fn test_read_directory_basic() {
        let dir = tempfile::tempdir().unwrap();
        stdfs::write(dir.path().join("file.txt"), "hello").unwrap();
        stdfs::create_dir(dir.path().join("subdir")).unwrap();
        let result = read_directory(dir.path().to_str().unwrap().to_string(), None);
        assert!(result.is_ok());
        let entries = result.unwrap();
        assert_eq!(entries.len(), 2);
        // Directories should come first
        assert!(entries[0].is_dir);
        assert!(!entries[1].is_dir);
    }

    #[test]
    fn test_read_directory_hides_dotfiles_by_default() {
        let dir = tempfile::tempdir().unwrap();
        stdfs::write(dir.path().join("visible.txt"), "").unwrap();
        stdfs::write(dir.path().join(".hidden"), "").unwrap();
        let result = read_directory(dir.path().to_str().unwrap().to_string(), None).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "visible.txt");
    }

    #[test]
    fn test_read_directory_shows_dotfiles_when_requested() {
        let dir = tempfile::tempdir().unwrap();
        stdfs::write(dir.path().join("visible.txt"), "").unwrap();
        stdfs::write(dir.path().join(".hidden"), "").unwrap();
        let result = read_directory(dir.path().to_str().unwrap().to_string(), Some(true)).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_read_directory_sorted_alphabetically() {
        let dir = tempfile::tempdir().unwrap();
        stdfs::write(dir.path().join("zebra.txt"), "").unwrap();
        stdfs::write(dir.path().join("apple.txt"), "").unwrap();
        stdfs::write(dir.path().join("mango.txt"), "").unwrap();
        let result = read_directory(dir.path().to_str().unwrap().to_string(), None).unwrap();
        assert_eq!(result[0].name, "apple.txt");
        assert_eq!(result[1].name, "mango.txt");
        assert_eq!(result[2].name, "zebra.txt");
    }

    #[test]
    fn test_read_directory_nonexistent() {
        let result = read_directory("/nonexistent/path".to_string(), None);
        assert!(result.is_err());
    }

    #[test]
    fn test_bytecount_lines() {
        assert_eq!(bytecount_lines(b"hello\nworld\n"), 2);
        assert_eq!(bytecount_lines(b"single line"), 0);
        assert_eq!(bytecount_lines(b""), 0);
        assert_eq!(bytecount_lines(b"\n\n\n"), 3);
    }

    #[test]
    fn test_read_file_content_basic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        stdfs::write(&path, "file contents").unwrap();
        let result = read_file_content(path.to_str().unwrap().to_string());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "file contents");
    }

    #[test]
    fn test_read_file_content_nonexistent() {
        let result = read_file_content("/nonexistent/file.txt".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_write_file_content_basic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("output.txt");
        let result = write_file_content(path.to_str().unwrap().to_string(), "written".to_string());
        assert!(result.is_ok());
        assert_eq!(stdfs::read_to_string(&path).unwrap(), "written");
    }

    #[test]
    fn test_write_file_content_creates_parents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("dir").join("file.txt");
        let result = write_file_content(path.to_str().unwrap().to_string(), "nested".to_string());
        assert!(result.is_ok());
        assert_eq!(stdfs::read_to_string(&path).unwrap(), "nested");
    }

    #[test]
    fn test_get_file_info_text_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        stdfs::write(&path, "line1\nline2\nline3\n").unwrap();
        let info = get_file_info(path.to_str().unwrap().to_string()).unwrap();
        assert!(!info.is_binary);
        assert_eq!(info.line_count, Some(3));
    }

    #[test]
    fn test_get_file_info_binary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("binary.bin");
        stdfs::write(&path, &[0u8, 1, 2, 0, 255, 0, 128]).unwrap();
        let info = get_file_info(path.to_str().unwrap().to_string()).unwrap();
        assert!(info.is_binary);
        assert_eq!(info.line_count, None);
    }

    #[test]
    fn test_search_files_by_name_basic() {
        let dir = tempfile::tempdir().unwrap();
        stdfs::write(dir.path().join("main.rs"), "").unwrap();
        stdfs::write(dir.path().join("lib.rs"), "").unwrap();
        stdfs::write(dir.path().join("readme.md"), "").unwrap();
        let result =
            search_files_by_name(dir.path().to_str().unwrap().to_string(), ".rs".to_string())
                .unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_search_in_files_basic() {
        let dir = tempfile::tempdir().unwrap();
        stdfs::write(
            dir.path().join("a.txt"),
            "hello world\ngoodbye\nhello again\n",
        )
        .unwrap();
        let result = search_in_files(
            dir.path().to_str().unwrap().to_string(),
            "hello".to_string(),
            None,
        )
        .unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].line, 1);
        assert_eq!(result[1].line, 3);
    }

    #[test]
    fn test_replace_in_files_basic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        stdfs::write(&path, "foo bar foo baz foo").unwrap();
        let count = replace_in_files(
            dir.path().to_str().unwrap().to_string(),
            "foo".to_string(),
            "qux".to_string(),
            vec![path.to_str().unwrap().to_string()],
        )
        .unwrap();
        assert_eq!(count, 3);
        assert_eq!(stdfs::read_to_string(&path).unwrap(), "qux bar qux baz qux");
    }
}
