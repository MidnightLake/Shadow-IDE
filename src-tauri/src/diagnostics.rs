use std::collections::HashMap;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct ComplexityResult {
    pub file: String,
    pub function: String,
    pub complexity: u32,
    pub line: u32,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct DuplicateBlock {
    pub hash: String,
    pub occurrences: Vec<DuplicateLocation>,
    pub line_count: u32,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct DuplicateLocation {
    pub file: String,
    pub start_line: u32,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct SecurityIssue {
    pub severity: String,
    pub title: String,
    pub package: String,
    pub advisory: String,
    pub fixed_in: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct LicenseEntry {
    pub name: String,
    pub version: String,
    pub license: String,
    pub compatible: bool,
}

// ===== Complexity Analysis =====

/// Analyze cyclomatic complexity of source files in a directory.
#[tauri::command]
pub async fn analyze_complexity(project_path: String) -> Result<Vec<ComplexityResult>, String> {
    let root = Path::new(&project_path);
    if !root.exists() {
        return Err(format!("Project path does not exist: {}", project_path));
    }

    let mut results = Vec::new();
    walk_for_complexity(root, &mut results);
    Ok(results)
}

fn walk_for_complexity(dir: &Path, results: &mut Vec<ComplexityResult>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden directories and common noise dirs
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.')
                || name == "target"
                || name == "node_modules"
                || name == "__pycache__"
            {
                continue;
            }
            walk_for_complexity(&path, results);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if matches!(ext, "rs" | "ts" | "tsx" | "js" | "jsx" | "py") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let file_str = path.to_string_lossy().to_string();
                    analyze_file_complexity(&file_str, &content, ext, results);
                }
            }
        }
    }
}

fn analyze_file_complexity(
    file: &str,
    content: &str,
    ext: &str,
    results: &mut Vec<ComplexityResult>,
) {
    let lines: Vec<&str> = content.lines().collect();

    // Find function boundaries and measure complexity within each
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();

        if let Some(func_name) = extract_function_name(line, ext) {
            let start_line = i as u32 + 1;
            // Count decision points until we find the matching closing brace or end of function
            let mut complexity: u32 = 1; // base complexity
            let mut depth: i32 = 0;
            let mut in_func = false;
            let mut j = i;

            while j < lines.len() {
                let fl = lines[j];
                for ch in fl.chars() {
                    match ch {
                        '{' => {
                            depth += 1;
                            in_func = true;
                        }
                        '}' => {
                            depth -= 1;
                            if in_func && depth <= 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                }

                if in_func {
                    complexity += count_decision_points(fl);
                }

                if in_func && depth <= 0 {
                    break;
                }
                j += 1;
            }

            // For Python: use indentation-based scope ending
            if ext == "py" && !in_func {
                let func_indent = lines[i].len() - lines[i].trim_start().len();
                complexity = 1;
                let mut k = i + 1;
                while k < lines.len() {
                    let kl = lines[k];
                    if kl.trim().is_empty() {
                        k += 1;
                        continue;
                    }
                    let k_indent = kl.len() - kl.trim_start().len();
                    if k_indent <= func_indent && !kl.trim().is_empty() {
                        break;
                    }
                    complexity += count_decision_points(kl);
                    k += 1;
                }
            }

            if complexity > 1 || in_func {
                results.push(ComplexityResult {
                    file: file.to_string(),
                    function: func_name,
                    complexity,
                    line: start_line,
                });
            }
        }

        i += 1;
    }
}

fn extract_function_name(line: &str, ext: &str) -> Option<String> {
    match ext {
        "rs" => {
            // Match: (pub )?(async )?fn name
            if let Some(pos) = line.find("fn ") {
                let after = &line[pos + 3..];
                let name: String = after
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    return Some(name);
                }
            }
            None
        }
        "py" => {
            if let Some(pos) = line.find("def ") {
                let after = &line[pos + 4..];
                let name: String = after
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    return Some(name);
                }
            }
            None
        }
        "ts" | "tsx" | "js" | "jsx" => {
            // function name(...) or const name = ... => or name(...) {
            if let Some(pos) = line.find("function ") {
                let after = &line[pos + 9..];
                let name: String = after
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect();
                if !name.is_empty() {
                    return Some(name);
                }
            }
            // Arrow functions: const/let/var name = (...) =>
            if line.contains("=>") {
                if let Some(pos) = line
                    .find("const ")
                    .or_else(|| line.find("let "))
                    .or_else(|| line.find("var "))
                {
                    let keyword_len = if line[pos..].starts_with("const ") {
                        6
                    } else if line[pos..].starts_with("let ") {
                        4
                    } else {
                        4
                    };
                    let after = &line[pos + keyword_len..];
                    let name: String = after
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() {
                        return Some(name);
                    }
                }
            }
            None
        }
        _ => None,
    }
}

fn count_decision_points(line: &str) -> u32 {
    let mut count = 0u32;
    // Keywords
    for kw in &[
        "if ", "if(", "} else", "for ", "for(", "while ", "while(", "match ", "case ",
    ] {
        if line.contains(kw) {
            count += 1;
        }
    }
    // Logical operators: && and ||
    count += line.matches("&&").count() as u32;
    count += line.matches("||").count() as u32;
    // Ternary
    count += line.matches('?').count() as u32;
    count
}

// ===== Duplicate Detection =====

/// Find duplicate code blocks across the project.
#[tauri::command]
pub async fn find_duplicates(
    project_path: String,
    min_lines: u32,
) -> Result<Vec<DuplicateBlock>, String> {
    let root = Path::new(&project_path);
    if !root.exists() {
        return Err(format!("Project path does not exist: {}", project_path));
    }
    let min = min_lines.max(3) as usize;

    // Collect all source files
    let mut files: Vec<String> = Vec::new();
    walk_source_files(root, &mut files);

    // Map from hash -> list of (file, start_line)
    let mut hash_map: HashMap<u64, Vec<(String, usize)>> = HashMap::new();

    for file_path in &files {
        if let Ok(content) = std::fs::read_to_string(file_path) {
            let lines: Vec<&str> = content.lines().collect();
            if lines.len() < min {
                continue;
            }
            // Slide a window of min_lines over the file
            for start in 0..=(lines.len() - min) {
                let block: Vec<&str> = lines[start..start + min]
                    .iter()
                    .map(|l| l.trim())
                    .filter(|l| !l.is_empty() && !l.starts_with("//") && !l.starts_with('#'))
                    .collect();
                if block.len() < min / 2 + 1 {
                    continue; // Skip mostly-empty blocks
                }
                let hash = fnv_hash_lines(&block);
                hash_map
                    .entry(hash)
                    .or_default()
                    .push((file_path.clone(), start + 1));
            }
        }
    }

    let mut results: Vec<DuplicateBlock> = hash_map
        .into_iter()
        .filter(|(_, locs)| locs.len() > 1)
        .map(|(hash, locs)| DuplicateBlock {
            hash: format!("{:016x}", hash),
            line_count: min as u32,
            occurrences: locs
                .into_iter()
                .map(|(file, start_line)| DuplicateLocation {
                    file,
                    start_line: start_line as u32,
                })
                .collect(),
        })
        .collect();

    // Limit to most interesting results
    results.truncate(50);
    Ok(results)
}

fn fnv_hash_lines(lines: &[&str]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for line in lines {
        for byte in line.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= b'\n' as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn walk_source_files(dir: &Path, files: &mut Vec<String>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.')
                || name == "target"
                || name == "node_modules"
                || name == "__pycache__"
            {
                continue;
            }
            walk_source_files(&path, files);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if matches!(
                ext,
                "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "go" | "java" | "c" | "cpp"
            ) {
                files.push(path.to_string_lossy().to_string());
            }
        }
    }
}

// ===== Security Scanning =====

/// Run security scans (cargo audit, npm audit).
#[tauri::command]
pub async fn scan_security(project_path: String) -> Result<Vec<SecurityIssue>, String> {
    let mut issues = Vec::new();

    let cargo_toml = Path::new(&project_path).join("Cargo.toml");
    if cargo_toml.exists() {
        if let Ok(cargo_issues) = run_cargo_audit(&project_path) {
            issues.extend(cargo_issues);
        }
        // Silently ignore if cargo-audit is not installed
    }

    let package_json = Path::new(&project_path).join("package.json");
    if package_json.exists() {
        if let Ok(npm_issues) = run_npm_audit(&project_path) {
            issues.extend(npm_issues);
        }
    }

    Ok(issues)
}

fn run_cargo_audit(project_path: &str) -> Result<Vec<SecurityIssue>, String> {
    let out = {
        let mut cmd = Command::new("cargo");
        cmd.args(["audit", "--json"]).current_dir(project_path);
        crate::platform::hide_window(&mut cmd);
        cmd.output().map_err(|e| e.to_string())?
    };

    let body = String::from_utf8_lossy(&out.stdout);
    parse_cargo_audit_json(&body)
}

fn parse_cargo_audit_json(json_str: &str) -> Result<Vec<SecurityIssue>, String> {
    let val: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| format!("Failed to parse cargo audit JSON: {}", e))?;

    let mut issues = Vec::new();
    if let Some(vulns) = val["vulnerabilities"]["list"].as_array() {
        for v in vulns {
            let advisory = &v["advisory"];
            let package = &v["package"];
            let severity = advisory["severity"]
                .as_str()
                .unwrap_or("unknown")
                .to_lowercase();
            let title = advisory["title"]
                .as_str()
                .unwrap_or("Unknown vulnerability")
                .to_string();
            let pkg_name = package["name"].as_str().unwrap_or("unknown").to_string();
            let advisory_id = advisory["id"].as_str().unwrap_or("").to_string();
            let fixed_in = v["versions"]["patched"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|v| v.as_str())
                .map(String::from);

            issues.push(SecurityIssue {
                severity,
                title,
                package: pkg_name,
                advisory: advisory_id,
                fixed_in,
            });
        }
    }
    Ok(issues)
}

fn run_npm_audit(project_path: &str) -> Result<Vec<SecurityIssue>, String> {
    let out = {
        let mut cmd = Command::new("npm");
        cmd.args(["audit", "--json"]).current_dir(project_path);
        crate::platform::hide_window(&mut cmd);
        cmd.output().map_err(|e| e.to_string())?
    };

    let body = String::from_utf8_lossy(&out.stdout);
    parse_npm_audit_json(&body)
}

fn parse_npm_audit_json(json_str: &str) -> Result<Vec<SecurityIssue>, String> {
    let val: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| format!("Failed to parse npm audit JSON: {}", e))?;

    let mut issues = Vec::new();
    // npm audit v2 format: vulnerabilities is an object
    if let Some(vulns) = val["vulnerabilities"].as_object() {
        for (pkg_name, vuln) in vulns {
            let severity = vuln["severity"]
                .as_str()
                .unwrap_or("unknown")
                .to_lowercase();
            let title = vuln["title"]
                .as_str()
                .or_else(|| vuln["name"].as_str())
                .unwrap_or("Unknown vulnerability")
                .to_string();
            let advisory = vuln["url"]
                .as_str()
                .or_else(|| vuln["cwe"].as_str())
                .unwrap_or("")
                .to_string();
            let fixed_in = vuln["fixAvailable"]
                .as_object()
                .and_then(|f| f["version"].as_str())
                .map(String::from);

            issues.push(SecurityIssue {
                severity,
                title,
                package: pkg_name.clone(),
                advisory,
                fixed_in,
            });
        }
    } else if let Some(advisories) = val["advisories"].as_object() {
        // npm audit v1 format
        for (_id, advisory) in advisories {
            let severity = advisory["severity"]
                .as_str()
                .unwrap_or("unknown")
                .to_lowercase();
            let title = advisory["title"]
                .as_str()
                .unwrap_or("Unknown vulnerability")
                .to_string();
            let pkg_name = advisory["module_name"]
                .as_str()
                .unwrap_or("unknown")
                .to_string();
            let advisory_id = advisory["url"].as_str().unwrap_or("").to_string();
            let fixed_in = advisory["patched_versions"].as_str().map(String::from);

            issues.push(SecurityIssue {
                severity,
                title,
                package: pkg_name,
                advisory: advisory_id,
                fixed_in,
            });
        }
    }
    Ok(issues)
}

// ===== License Scanning =====

/// Scan licenses of dependencies.
#[tauri::command]
pub async fn scan_licenses(project_path: String) -> Result<Vec<LicenseEntry>, String> {
    let mut entries = Vec::new();

    let cargo_toml = Path::new(&project_path).join("Cargo.toml");
    if cargo_toml.exists() {
        let cargo_entries = run_cargo_license(&project_path).unwrap_or_default();
        entries.extend(cargo_entries);
    }

    let package_json = Path::new(&project_path).join("package.json");
    if package_json.exists() {
        let npm_entries = scan_npm_licenses(&project_path).unwrap_or_default();
        entries.extend(npm_entries);
    }

    Ok(entries)
}

fn run_cargo_license(project_path: &str) -> Result<Vec<LicenseEntry>, String> {
    // Try `cargo license --json` first
    let out = {
        let mut cmd = Command::new("cargo");
        cmd.args(["license", "--json"]).current_dir(project_path);
        crate::platform::hide_window(&mut cmd);
        cmd.output().map_err(|e| e.to_string())?
    };

    if out.status.success() {
        let body = String::from_utf8_lossy(&out.stdout);
        return parse_cargo_license_json(&body);
    }

    // Fall back to `cargo metadata` to extract package info
    let meta_out = {
        let mut cmd = Command::new("cargo");
        cmd.args(["metadata", "--format-version=1", "--no-deps"])
            .current_dir(project_path);
        crate::platform::hide_window(&mut cmd);
        cmd.output().map_err(|e| e.to_string())?
    };

    if !meta_out.status.success() {
        return Ok(Vec::new());
    }

    let body = String::from_utf8_lossy(&meta_out.stdout);
    let val: serde_json::Value = serde_json::from_str(&body)
        .map_err(|e| format!("Failed to parse cargo metadata: {}", e))?;

    let mut entries = Vec::new();
    if let Some(pkgs) = val["packages"].as_array() {
        for pkg in pkgs {
            let name = pkg["name"].as_str().unwrap_or("unknown").to_string();
            let version = pkg["version"].as_str().unwrap_or("0.0.0").to_string();
            let license = pkg["license"].as_str().unwrap_or("Unknown").to_string();
            let compatible = is_compatible_license(&license);
            entries.push(LicenseEntry {
                name,
                version,
                license,
                compatible,
            });
        }
    }
    Ok(entries)
}

fn parse_cargo_license_json(json_str: &str) -> Result<Vec<LicenseEntry>, String> {
    let val: serde_json::Value = serde_json::from_str(json_str)
        .map_err(|e| format!("Failed to parse cargo license JSON: {}", e))?;

    let mut entries = Vec::new();
    if let Some(pkgs) = val.as_array() {
        for pkg in pkgs {
            let name = pkg["name"].as_str().unwrap_or("unknown").to_string();
            let version = pkg["version"].as_str().unwrap_or("0.0.0").to_string();
            let license = pkg["license"].as_str().unwrap_or("Unknown").to_string();
            let compatible = is_compatible_license(&license);
            entries.push(LicenseEntry {
                name,
                version,
                license,
                compatible,
            });
        }
    }
    Ok(entries)
}

fn scan_npm_licenses(project_path: &str) -> Result<Vec<LicenseEntry>, String> {
    // Try `license-checker --json`
    let out = {
        let mut cmd = Command::new("license-checker");
        cmd.args(["--json"]).current_dir(project_path);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
    };

    if let Ok(output) = out {
        if output.status.success() {
            let body = String::from_utf8_lossy(&output.stdout);
            let val: serde_json::Value =
                serde_json::from_str(&body).unwrap_or(serde_json::Value::Null);
            if let Some(obj) = val.as_object() {
                let mut entries = Vec::new();
                for (pkg_ver, info) in obj {
                    let parts: Vec<&str> = pkg_ver.rsplitn(2, '@').collect();
                    let (version, name) = if parts.len() == 2 {
                        (parts[0].to_string(), parts[1].to_string())
                    } else {
                        ("0.0.0".to_string(), pkg_ver.clone())
                    };
                    let license = info["licenses"].as_str().unwrap_or("Unknown").to_string();
                    let compatible = is_compatible_license(&license);
                    entries.push(LicenseEntry {
                        name,
                        version,
                        license,
                        compatible,
                    });
                }
                return Ok(entries);
            }
        }
    }

    // Fall back: read package.json dependencies and report from node_modules
    let pkg_path = Path::new(project_path).join("package.json");
    let content = std::fs::read_to_string(&pkg_path)
        .map_err(|e| format!("Failed to read package.json: {}", e))?;
    let pkg: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse package.json: {}", e))?;

    let mut entries = Vec::new();
    let dep_keys = ["dependencies", "devDependencies"];
    for key in &dep_keys {
        if let Some(deps) = pkg[key].as_object() {
            for (name, version_val) in deps {
                let version = version_val
                    .as_str()
                    .unwrap_or("*")
                    .trim_start_matches('^')
                    .trim_start_matches('~')
                    .to_string();

                // Try to read license from node_modules/<name>/package.json
                let nm_pkg = Path::new(project_path)
                    .join("node_modules")
                    .join(name)
                    .join("package.json");
                let license = if let Ok(c) = std::fs::read_to_string(&nm_pkg) {
                    if let Ok(p) = serde_json::from_str::<serde_json::Value>(&c) {
                        p["license"]
                            .as_str()
                            .or_else(|| p["license"]["type"].as_str())
                            .unwrap_or("Unknown")
                            .to_string()
                    } else {
                        "Unknown".to_string()
                    }
                } else {
                    "Unknown".to_string()
                };

                let compatible = is_compatible_license(&license);
                entries.push(LicenseEntry {
                    name: name.clone(),
                    version,
                    license,
                    compatible,
                });
            }
        }
    }
    Ok(entries)
}

// ===== Dead Code Finder =====

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct DeadCodeItem {
    pub file: String,
    pub line: u32,
    pub kind: String,
    pub name: String,
    pub reason: String,
}

#[tauri::command]
pub async fn find_dead_code(project_path: String) -> Result<Vec<DeadCodeItem>, String> {
    let mut items = Vec::new();
    let path = Path::new(&project_path);

    // For Rust: run `cargo check --message-format=json` and parse unused warnings
    let cargo_toml = path.join("Cargo.toml");
    if cargo_toml.exists() {
        let out = {
            let mut cmd = Command::new("cargo");
            cmd.args(["check", "--message-format=json"])
                .current_dir(path);
            crate::platform::hide_window(&mut cmd);
            cmd.output().map_err(|e| e.to_string())?
        };

        // Parse JSON messages from stdout for location info
        for msg_line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(msg_line) {
                if v.get("reason").and_then(|r| r.as_str()) == Some("compiler-message") {
                    let msg = &v["message"];
                    if let Some(rendered) = msg.get("rendered").and_then(|r| r.as_str()) {
                        if rendered.contains("unused") || rendered.contains("dead_code") {
                            let level = msg.get("level").and_then(|l| l.as_str()).unwrap_or("");
                            if level == "warning" {
                                if let Some(span) = msg["spans"].as_array().and_then(|s| s.first())
                                {
                                    let file = span
                                        .get("file_name")
                                        .and_then(|f| f.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    let line = span
                                        .get("line_start")
                                        .and_then(|l| l.as_u64())
                                        .unwrap_or(0)
                                        as u32;
                                    let text =
                                        msg.get("message").and_then(|m| m.as_str()).unwrap_or("");
                                    let kind = if text.contains("function") {
                                        "function"
                                    } else if text.contains("import") || text.contains("use ") {
                                        "import"
                                    } else {
                                        "variable"
                                    };
                                    let name = text.split('`').nth(1).unwrap_or("").to_string();
                                    items.push(DeadCodeItem {
                                        file,
                                        line,
                                        kind: kind.to_string(),
                                        name,
                                        reason: text.to_string(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // For TypeScript/JS: run eslint with no-unused-vars rule
    let pkg_json = path.join("package.json");
    if pkg_json.exists() {
        let out = {
            let mut cmd = Command::new("npx");
            cmd.args([
                "eslint",
                "--format=json",
                "--rule",
                "{\"no-unused-vars\":\"warn\"}",
                "src/",
            ])
            .current_dir(path);
            crate::platform::hide_window(&mut cmd);
            cmd.output()
        };

        if let Ok(o) = out {
            if let Ok(results) = serde_json::from_slice::<serde_json::Value>(&o.stdout) {
                if let Some(arr) = results.as_array() {
                    for file_result in arr {
                        let file_path = file_result
                            .get("filePath")
                            .and_then(|f| f.as_str())
                            .unwrap_or("")
                            .to_string();
                        if let Some(messages) =
                            file_result.get("messages").and_then(|m| m.as_array())
                        {
                            for msg in messages {
                                if msg.get("ruleId").and_then(|r| r.as_str())
                                    == Some("no-unused-vars")
                                {
                                    let line = msg.get("line").and_then(|l| l.as_u64()).unwrap_or(0)
                                        as u32;
                                    let name = msg
                                        .get("message")
                                        .and_then(|m| m.as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    items.push(DeadCodeItem {
                                        file: file_path.clone(),
                                        line,
                                        kind: "variable".to_string(),
                                        name,
                                        reason: "no-unused-vars".to_string(),
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    items.dedup_by(|a, b| a.file == b.file && a.line == b.line);
    Ok(items)
}

// ===== Profiling =====

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct ProfilingResult {
    pub tool: String,
    pub output_file: Option<String>,
    pub summary: String,
    pub success: bool,
}

fn executable_available(name: &str) -> bool {
    Command::new("which")
        .arg(name)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn cargo_subcommand_available(subcommand: &str) -> bool {
    Command::new("cargo")
        .arg("--list")
        .output()
        .map(|o| {
            o.status.success()
                && String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .any(|line| line.trim_start().starts_with(subcommand))
        })
        .unwrap_or(false)
}

fn profile_candidate_dirs(path: &Path) -> [PathBuf; 10] {
    [
        path.join("target/debug"),
        path.join("src-tauri/target/debug"),
        path.join("build"),
        path.join("build/bin"),
        path.join("build/debug"),
        path.join("bin"),
        path.join("out"),
        path.join("out/debug"),
        path.join("cmake-build-debug"),
        path.join("cmake-build-release"),
    ]
}

fn looks_like_native_project(path: &Path) -> bool {
    path.join("compile_commands.json").exists()
        || path.join("CMakeLists.txt").exists()
        || path.join("Makefile").exists()
        || path.join("meson.build").exists()
        || std::fs::read_dir(path)
            .ok()
            .map(|dir| {
                dir.flatten().any(|entry| {
                    entry
                        .path()
                        .extension()
                        .and_then(|ext| ext.to_str())
                        .map(|ext| {
                            matches!(ext, "cpp" | "cc" | "cxx" | "hpp" | "hh" | "hxx" | "ixx")
                        })
                        .unwrap_or(false)
                })
            })
            .unwrap_or(false)
}

fn is_profile_executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.starts_with('.') || name.starts_with("lib") {
        return false;
    }

    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    if matches!(
        ext,
        "o" | "obj" | "a" | "so" | "dylib" | "dll" | "pdb" | "d" | "rlib" | "rmeta"
    ) {
        return false;
    }

    #[cfg(unix)]
    {
        path.metadata()
            .map(|meta| meta.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        ext.eq_ignore_ascii_case("exe")
    }
}

fn is_profile_library(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }

    matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("so") | Some("dylib") | Some("dll")
    )
}

fn discover_in_dirs(
    path: &Path,
    predicate: fn(&Path) -> bool,
    preferred_name: Option<&str>,
) -> Option<String> {
    for dir_path in profile_candidate_dirs(path) {
        if !dir_path.exists() {
            continue;
        }

        let mut stack = vec![(dir_path, 0usize)];
        let mut fallback: Option<String> = None;

        while let Some((current_dir, depth)) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&current_dir) else {
                continue;
            };

            for entry in entries.flatten() {
                let entry_path = entry.path();
                if entry_path.is_dir() {
                    let dir_name = entry_path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("");
                    if depth < 2 && !dir_name.starts_with('.') {
                        stack.push((entry_path, depth + 1));
                    }
                    continue;
                }

                if !predicate(&entry_path) {
                    continue;
                }

                let candidate = entry_path.to_string_lossy().to_string();
                let file_name = entry_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if preferred_name
                    .map(|name| file_name == name || file_name == format!("{}.exe", name))
                    .unwrap_or(false)
                {
                    return Some(candidate);
                }

                if fallback.is_none() {
                    fallback = Some(candidate);
                }
            }
        }

        if fallback.is_some() {
            return fallback;
        }
    }

    None
}

fn discover_profile_binary(path: &Path) -> Option<String> {
    let preferred_name = path.file_name().and_then(|n| n.to_str());
    discover_in_dirs(path, is_profile_executable, preferred_name)
}

fn discover_profile_library(path: &Path) -> Option<String> {
    let candidate_dirs = [
        path.join("build"),
        path.join("bin"),
        path.join("out"),
        path.join("cmake-build-debug"),
        path.join("cmake-build-release"),
    ];

    let preferred_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .map(|name| format!("lib{}", name));

    candidate_dirs.iter().find_map(|dir_path| {
        if !dir_path.exists() {
            return None;
        }

        let mut stack = vec![(dir_path.clone(), 0usize)];
        let mut fallback: Option<String> = None;

        while let Some((current_dir, depth)) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&current_dir) else {
                continue;
            };

            for entry in entries.flatten() {
                let entry_path = entry.path();
                if entry_path.is_dir() {
                    if depth < 2 {
                        stack.push((entry_path, depth + 1));
                    }
                    continue;
                }

                if !is_profile_library(&entry_path) {
                    continue;
                }

                let candidate = entry_path.to_string_lossy().to_string();
                let file_name = entry_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if preferred_name
                    .as_deref()
                    .map(|name| file_name.starts_with(name))
                    .unwrap_or(false)
                {
                    return Some(candidate);
                }

                if fallback.is_none() {
                    fallback = Some(candidate);
                }
            }
        }

        fallback
    })
}

#[tauri::command]
pub async fn run_cpu_profiler(
    project_path: String,
    binary: Option<String>,
    duration_secs: Option<u32>,
) -> Result<ProfilingResult, String> {
    let path = Path::new(&project_path);
    let _duration = duration_secs.unwrap_or(10);
    let has_cargo = path.join("Cargo.toml").exists();
    let is_native_project = looks_like_native_project(path);
    let resolved_binary = binary.or_else(|| discover_profile_binary(path));
    let shared_library = discover_profile_library(path);

    // Try cargo flamegraph first
    if has_cargo && cargo_subcommand_available("flamegraph") {
        let out = {
            let mut cmd = Command::new("cargo");
            cmd.args(["flamegraph", "--", "--release"])
                .current_dir(path);
            crate::platform::hide_window(&mut cmd);
            cmd.output()
        };

        if let Ok(o) = out {
            let output_file = path.join("flamegraph.svg");
            return Ok(ProfilingResult {
                tool: "cargo-flamegraph".to_string(),
                output_file: if output_file.exists() {
                    Some(output_file.to_string_lossy().to_string())
                } else {
                    None
                },
                summary: String::from_utf8_lossy(&o.stderr)
                    .chars()
                    .take(500)
                    .collect(),
                success: o.status.success(),
            });
        }
    }

    // Try perf on Linux
    #[cfg(target_os = "linux")]
    {
        let bin = resolved_binary.clone().unwrap_or_default();
        if executable_available("perf") && !bin.is_empty() {
            let out = {
                let mut cmd = Command::new("perf");
                cmd.args(["record", "-g", "-o", "/tmp/perf.data", "--", &bin]);
                crate::platform::hide_window(&mut cmd);
                cmd.output().map_err(|e| e.to_string())?
            };
            return Ok(ProfilingResult {
                tool: "perf".to_string(),
                output_file: Some("/tmp/perf.data".to_string()),
                summary: String::from_utf8_lossy(&out.stderr)
                    .chars()
                    .take(500)
                    .collect(),
                success: out.status.success(),
            });
        }

        if executable_available("valgrind") && !bin.is_empty() {
            let bin_name = std::path::Path::new(&bin)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let callgrind_path = std::env::temp_dir().join(format!("callgrind-{}.out", bin_name));
            let out = {
                let mut cmd = Command::new("valgrind");
                cmd.args(["--tool=callgrind"])
                    .arg(format!(
                        "--callgrind-out-file={}",
                        callgrind_path.to_string_lossy()
                    ))
                    .arg(&bin)
                    .current_dir(path);
                crate::platform::hide_window(&mut cmd);
                cmd.output().map_err(|e| e.to_string())?
            };
            let summary = String::from_utf8_lossy(&out.stderr)
                .chars()
                .take(500)
                .collect::<String>();
            return Ok(ProfilingResult {
                tool: "valgrind-callgrind".to_string(),
                output_file: Some(callgrind_path.to_string_lossy().to_string()),
                summary: if summary.trim().is_empty() {
                    format!(
                        "Callgrind finished. Open {} for the full report.",
                        callgrind_path.to_string_lossy()
                    )
                } else {
                    summary
                },
                success: out.status.success(),
            });
        }
    }

    #[cfg(not(target_os = "linux"))]
    let _ = binary;

    if is_native_project {
        if let Some(lib_path) = shared_library {
            return Ok(ProfilingResult {
                tool: "native-library".to_string(),
                output_file: Some(lib_path.clone()),
                summary: format!(
                    "Detected a native library artifact at {}. CPU profiling needs a host executable that loads this library. Build or configure the runner executable for this C++20/C++23 project, then run profiling again.",
                    lib_path
                ),
                success: false,
            });
        }

        return Ok(ProfilingResult {
            tool: "none".to_string(),
            output_file: None,
            summary: "No runnable native executable was found in build/, bin/, out/, or cmake-build-*. Build the C++ project first or configure a runner executable, then run profiling again."
                .to_string(),
            success: false,
        });
    }

    Ok(ProfilingResult {
        tool: "none".to_string(),
        output_file: None,
        summary: "No CPU profiler found. Install cargo-flamegraph with `cargo install flamegraph`, or configure a runnable binary for perf."
            .to_string(),
        success: false,
    })
}

#[tauri::command]
pub async fn run_memory_profiler(
    project_path: String,
    binary: Option<String>,
) -> Result<ProfilingResult, String> {
    let path = Path::new(&project_path);
    let resolved_binary = binary.or_else(|| discover_profile_binary(path));
    let shared_library = discover_profile_library(path);

    // Check for heaptrack
    let heaptrack_available = executable_available("heaptrack");

    if heaptrack_available {
        let bin = resolved_binary.clone().unwrap_or_default();

        if !bin.is_empty() {
            let out = {
                let mut cmd = Command::new("heaptrack");
                cmd.arg(&bin).current_dir(path);
                crate::platform::hide_window(&mut cmd);
                cmd.output().map_err(|e| e.to_string())?
            };
            let bin_name = std::path::Path::new(&bin)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            return Ok(ProfilingResult {
                tool: "heaptrack".to_string(),
                output_file: Some(format!("heaptrack.{}.gz", bin_name)),
                summary: String::from_utf8_lossy(&out.stderr)
                    .chars()
                    .take(500)
                    .collect(),
                success: out.status.success(),
            });
        }
    }

    let valgrind_available = executable_available("valgrind");
    if valgrind_available {
        let bin = resolved_binary.unwrap_or_default();

        if bin.is_empty() {
            return Ok(ProfilingResult {
                tool: "valgrind".to_string(),
                output_file: shared_library.clone(),
                summary: if let Some(lib_path) = shared_library {
                    format!(
                        "Valgrind is installed, but the detected build artifact is a shared library at {}. Memory profiling needs a host executable that loads this library. Build or configure the runner executable, then run profiling again.",
                        lib_path
                    )
                } else {
                    "Valgrind is installed, but no runnable executable was found in build/, bin/, out/, or target/debug. Build your project first or configure a binary to profile."
                        .to_string()
                },
                success: false,
            });
        }

        let bin_name = std::path::Path::new(&bin)
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let log_path = std::env::temp_dir().join(format!("valgrind-{}.log", bin_name));
        let out = {
            let mut cmd = Command::new("valgrind");
            cmd.args([
                "--tool=memcheck",
                "--leak-check=full",
                "--track-origins=yes",
            ])
            .arg(format!("--log-file={}", log_path.to_string_lossy()))
            .arg(&bin)
            .current_dir(path);
            crate::platform::hide_window(&mut cmd);
            cmd.output().map_err(|e| e.to_string())?
        };

        let summary = std::fs::read_to_string(&log_path)
            .ok()
            .map(|contents| contents.chars().take(500).collect())
            .filter(|contents: &String| !contents.trim().is_empty())
            .unwrap_or_else(|| {
                let stderr = String::from_utf8_lossy(&out.stderr)
                    .chars()
                    .take(500)
                    .collect::<String>();
                if stderr.trim().is_empty() {
                    format!(
                        "Valgrind finished. Open {} for the full report.",
                        log_path.to_string_lossy()
                    )
                } else {
                    stderr
                }
            });

        return Ok(ProfilingResult {
            tool: "valgrind".to_string(),
            output_file: Some(log_path.to_string_lossy().to_string()),
            summary,
            success: out.status.success(),
        });
    }

    Ok(ProfilingResult {
        tool: "none".to_string(),
        output_file: None,
        summary: "No memory profiler found. Install heaptrack or valgrind.".to_string(),
        success: false,
    })
}

// ===== Bundle Analysis =====

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct BundleEntry {
    pub name: String,
    pub size_bytes: u64,
    pub percent: f32,
    pub kind: String, // "js", "css", "asset", "wasm"
}

/// Analyze JS bundle composition
#[tauri::command]
pub async fn analyze_bundle(project_path: String) -> Result<Vec<BundleEntry>, String> {
    let path = std::path::Path::new(&project_path);
    let dist = path.join("dist");
    let build = path.join("build");
    let out_dir = if dist.exists() {
        dist
    } else if build.exists() {
        build
    } else {
        return Err("No dist/ or build/ directory found. Run your build first.".to_string());
    };

    let mut total: u64 = 0;

    fn walk_dir(dir: &std::path::Path, entries: &mut Vec<(String, u64, String)>, total: &mut u64) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    walk_dir(&p, entries, total);
                } else {
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    let ext = p
                        .extension()
                        .and_then(|e| e.to_str())
                        .unwrap_or("")
                        .to_string();
                    let name = p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string();
                    *total += size;
                    entries.push((name, size, ext));
                }
            }
        }
    }

    let mut raw: Vec<(String, u64, String)> = Vec::new();
    walk_dir(&out_dir, &mut raw, &mut total);
    raw.sort_by(|a, b| b.1.cmp(&a.1));

    let mut entries = Vec::new();
    for (name, size, ext) in raw.iter().take(50) {
        let kind = match ext.as_str() {
            "js" | "mjs" | "cjs" => "js",
            "css" => "css",
            "wasm" => "wasm",
            "map" => "sourcemap",
            _ => "asset",
        };
        entries.push(BundleEntry {
            name: name.clone(),
            size_bytes: *size,
            percent: if total > 0 {
                (*size as f32 / total as f32) * 100.0
            } else {
                0.0
            },
            kind: kind.to_string(),
        });
    }
    Ok(entries)
}

/// Returns false for licenses that are GPL/AGPL/LGPL (copyleft) which may be incompatible
/// with proprietary projects.
fn is_compatible_license(license: &str) -> bool {
    let l = license.to_uppercase();
    let incompatible = ["GPL", "AGPL", "LGPL", "EUPL", "OSL", "MPL-2.0"];
    for bad in &incompatible {
        if l.contains(bad) {
            return false;
        }
    }
    true
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct SlowQuery {
    pub query: String,
    pub duration_ms: f64,
    pub count: u32,
    pub file: String,
    pub line: u32,
}

/// Detect N+1 query patterns and slow queries in source code
/// Looks for ORM patterns and repeated queries in loops
#[tauri::command]
pub async fn analyze_db_queries(project_path: String) -> Result<Vec<SlowQuery>, String> {
    let mut findings = Vec::new();

    let extensions = ["rs", "ts", "js", "py", "rb", "go"];

    fn walk(dir: &std::path::Path, findings: &mut Vec<SlowQuery>, exts: &[&str]) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if !["node_modules", "target", ".git"].contains(&name) {
                    walk(&path, findings, exts);
                }
            } else {
                let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
                if exts.contains(&ext) {
                    if let Ok(content) = std::fs::read_to_string(&path) {
                        analyze_file_for_n1(&path.to_string_lossy(), &content, findings);
                    }
                }
            }
        }
    }

    walk(
        std::path::Path::new(&project_path),
        &mut findings,
        &extensions,
    );
    Ok(findings)
}

fn analyze_file_for_n1(file: &str, content: &str, findings: &mut Vec<SlowQuery>) {
    let lines: Vec<&str> = content.lines().collect();

    for (i, line) in lines.iter().enumerate() {
        let lower = line.to_lowercase();

        // Pattern: query inside a loop
        let in_loop = i > 0
            && lines[..i].iter().rev().take(5).any(|l| {
                let ll = l.trim().to_lowercase();
                ll.starts_with("for ")
                    || ll.starts_with("while ")
                    || ll.starts_with(".for_each")
                    || ll.starts_with(".map(")
            });

        if in_loop
            && (lower.contains(".find(")
                || lower.contains(".query(")
                || lower.contains("select ")
                || lower.contains(".get(")
                || lower.contains("db.")
                || lower.contains("conn."))
        {
            findings.push(SlowQuery {
                query: line.trim().to_string(),
                duration_ms: 0.0,
                count: 0,
                file: file.to_string(),
                line: (i + 1) as u32,
            });
        }

        // Pattern: SELECT * (unindexed full scan)
        if lower.contains("select *") || lower.contains("select * from") {
            findings.push(SlowQuery {
                query: line.trim().to_string(),
                duration_ms: 0.0,
                count: 0,
                file: file.to_string(),
                line: (i + 1) as u32,
            });
        }
    }
}

// ===== Snapshot Testing =====

/// Run snapshot tests for the project. Detects Jest/Vitest for JS projects and
/// `cargo test` for Rust projects. Returns a summary with per-snapshot status.
#[tauri::command]
pub async fn run_snapshot_tests(project_dir: String) -> Result<serde_json::Value, String> {
    let root = std::path::Path::new(&project_dir);

    let has_cargo = root.join("Cargo.toml").exists();
    let has_package_json = root.join("package.json").exists();

    if has_package_json {
        // Detect vitest vs jest
        let use_vitest = detect_js_test_runner(&project_dir) == "vitest";
        let (cmd_prog, cmd_args): (&str, Vec<&str>) = if use_vitest {
            ("npx", vec!["vitest", "run", "--reporter=json"])
        } else {
            ("npx", vec!["jest", "--json"])
        };

        let output = tokio::process::Command::new(cmd_prog)
            .args(&cmd_args)
            .current_dir(root)
            .output()
            .await
            .map_err(|e| format!("Failed to run JS test runner: {}", e))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Jest prints JSON to stdout; vitest may print to stdout or stderr
        let json_src =
            if stdout.trim_start().starts_with('{') || stdout.trim_start().starts_with('[') {
                stdout.to_string()
            } else {
                stderr.to_string()
            };

        return parse_js_snapshot_output(&json_src, use_vitest);
    }

    if has_cargo {
        let output = tokio::process::Command::new("cargo")
            .args(["test", "--", "--test-output=immediate"])
            .current_dir(root)
            .output()
            .await
            .map_err(|e| format!("Failed to run cargo test: {}", e))?;

        let combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return parse_cargo_snapshot_output(&combined);
    }

    Err("No recognizable project type found (Cargo.toml or package.json)".to_string())
}

fn detect_js_test_runner(project_dir: &str) -> &'static str {
    let pkg_path = std::path::Path::new(project_dir).join("package.json");
    if let Ok(content) = std::fs::read_to_string(&pkg_path) {
        if content.contains("\"vitest\"") || content.contains("vitest") {
            return "vitest";
        }
    }
    "jest"
}

fn parse_js_snapshot_output(json_src: &str, _is_vitest: bool) -> Result<serde_json::Value, String> {
    let val: serde_json::Value = serde_json::from_str(json_src).unwrap_or(serde_json::Value::Null);

    let mut passed: u32 = 0;
    let mut failed: u32 = 0;
    let mut snapshots: Vec<serde_json::Value> = Vec::new();

    // Jest JSON: { numPassedTests, numFailedTests, testResults: [...] }
    // Vitest JSON: similar structure
    if let Some(num_passed) = val["numPassedTests"].as_u64() {
        passed = num_passed as u32;
    }
    if let Some(num_failed) = val["numFailedTests"].as_u64() {
        failed = num_failed as u32;
    }

    if let Some(test_results) = val["testResults"].as_array() {
        for suite in test_results {
            if let Some(assert_results) = suite["assertionResults"].as_array() {
                for test in assert_results {
                    let name = test["fullName"]
                        .as_str()
                        .or_else(|| test["title"].as_str())
                        .unwrap_or("unknown")
                        .to_string();
                    let status_str = test["status"].as_str().unwrap_or("unknown");
                    // Only include snapshot-related tests
                    let is_snapshot = name.to_lowercase().contains("snapshot")
                        || test["failureMessages"]
                            .as_array()
                            .map(|msgs| {
                                msgs.iter().any(|m| {
                                    m.as_str().map(|s| s.contains("snapshot")).unwrap_or(false)
                                })
                            })
                            .unwrap_or(false);

                    if is_snapshot {
                        let status = match status_str {
                            "passed" => "pass",
                            "failed" => "fail",
                            _ => "unknown",
                        };
                        let diff = test["failureMessages"]
                            .as_array()
                            .and_then(|msgs| msgs.first())
                            .and_then(|m| m.as_str())
                            .map(|s| s.chars().take(2000).collect::<String>());

                        snapshots.push(serde_json::json!({
                            "test": name,
                            "status": status,
                            "diff": diff
                        }));
                    }
                }
            }
        }
    }

    // If snapshotResults exists (jest --json includes snapshot summary)
    if let Some(snap_summary) = val.get("snapshot") {
        let updated = snap_summary["updated"].as_u64().unwrap_or(0) as u32;
        let snap_passed = snap_summary["matched"].as_u64().unwrap_or(0) as u32;
        let snap_failed = snap_summary["unmatched"].as_u64().unwrap_or(0) as u32;
        if snapshots.is_empty() {
            // Build from summary when no individual snapshot tests were found
            if snap_passed > 0 {
                passed = passed.max(snap_passed);
            }
            if snap_failed > 0 {
                failed = failed.max(snap_failed);
            }
            if updated > 0 {
                snapshots.push(serde_json::json!({
                    "test": format!("{} snapshots updated", updated),
                    "status": "updated",
                    "diff": null
                }));
            }
        }
    }

    Ok(serde_json::json!({
        "passed": passed,
        "failed": failed,
        "snapshots": snapshots
    }))
}

fn parse_cargo_snapshot_output(output: &str) -> Result<serde_json::Value, String> {
    let mut passed: u32 = 0;
    let mut failed: u32 = 0;
    let mut snapshots: Vec<serde_json::Value> = Vec::new();

    for line in output.lines() {
        if line.contains("test ") && line.contains(" ... ok") {
            passed += 1;
            if line.to_lowercase().contains("snapshot") {
                snapshots.push(serde_json::json!({
                    "test": line.trim().trim_end_matches(" ... ok").trim_start_matches("test "),
                    "status": "pass",
                    "diff": null
                }));
            }
        } else if line.contains("test ") && line.contains(" ... FAILED") {
            failed += 1;
            if line.to_lowercase().contains("snapshot") {
                snapshots.push(serde_json::json!({
                    "test": line.trim().trim_end_matches(" ... FAILED").trim_start_matches("test "),
                    "status": "fail",
                    "diff": null
                }));
            }
        }
    }

    Ok(serde_json::json!({
        "passed": passed,
        "failed": failed,
        "snapshots": snapshots
    }))
}

/// Update snapshots for the project. Runs the test suite with snapshot update flags.
/// If `test_name` is provided, only updates snapshots for that test (JS only).
#[tauri::command]
pub async fn update_snapshots(
    project_dir: String,
    test_name: Option<String>,
) -> Result<String, String> {
    let root = std::path::Path::new(&project_dir);

    let has_cargo = root.join("Cargo.toml").exists();
    let has_package_json = root.join("package.json").exists();

    if has_package_json {
        let use_vitest = detect_js_test_runner(&project_dir) == "vitest";
        let mut args: Vec<String> = if use_vitest {
            vec!["vitest".into(), "run".into(), "--update-snapshots".into()]
        } else {
            vec!["jest".into(), "--updateSnapshot".into()]
        };

        if let Some(ref name) = test_name {
            if use_vitest {
                args.push("-t".into());
                args.push(name.clone());
            } else {
                args.push("-t".into());
                args.push(name.clone());
            }
        }

        let output = tokio::process::Command::new("npx")
            .args(&args)
            .current_dir(root)
            .output()
            .await
            .map_err(|e| format!("Failed to run snapshot update: {}", e))?;

        let combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        // Parse how many snapshots were updated
        let updated_count = count_updated_snapshots_js(&combined);
        return Ok(format!("{} snapshots updated", updated_count));
    }

    if has_cargo {
        let output = tokio::process::Command::new("cargo")
            .args(["test", "--", "--update-snapshots"])
            .current_dir(root)
            .output()
            .await
            .map_err(|e| format!("Failed to run cargo test --update-snapshots: {}", e))?;

        let combined = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let updated_count = combined
            .lines()
            .filter(|l| l.to_lowercase().contains("snapshot") && l.to_lowercase().contains("updat"))
            .count();
        return Ok(format!("{} snapshots updated", updated_count));
    }

    Err("No recognizable project type found (Cargo.toml or package.json)".to_string())
}

fn count_updated_snapshots_js(output: &str) -> u32 {
    // Jest/Vitest output: "X snapshots updated."
    for line in output.lines() {
        let lower = line.to_lowercase();
        if lower.contains("snapshot") && lower.contains("updated") {
            // Try to extract number from line like "3 snapshots updated."
            let num: u32 = line
                .split_whitespace()
                .find_map(|w| w.parse::<u32>().ok())
                .unwrap_or(0);
            if num > 0 {
                return num;
            }
        }
    }
    0
}

// ===== Mutation Testing =====

/// Run mutation tests for the project using cargo-mutants (Rust) or Stryker (JS/TS).
/// Returns mutation score and per-mutant results.
#[tauri::command]
pub async fn run_mutation_tests(
    project_dir: String,
    target_file: Option<String>,
) -> Result<serde_json::Value, String> {
    let root = std::path::Path::new(&project_dir);

    let has_cargo = root.join("Cargo.toml").exists();
    let has_package_json = root.join("package.json").exists();

    if has_cargo {
        let mut args: Vec<String> = vec!["mutants".into(), "--json".into()];
        if let Some(ref file) = target_file {
            args.push("--file".into());
            args.push(file.clone());
        }

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(300),
            tokio::process::Command::new("cargo")
                .args(&args)
                .current_dir(root)
                .output(),
        )
        .await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if output.status.success() || !stdout.trim().is_empty() {
                    return parse_cargo_mutants_output(&stdout);
                }
                // cargo-mutants not installed
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("no such subcommand") || stderr.contains("error: no such") {
                    return Err(
                        "cargo-mutants is not installed. Install it with: cargo install cargo-mutants"
                            .to_string(),
                    );
                }
                return parse_cargo_mutants_output(&stdout);
            }
            Ok(Err(e)) => {
                if e.kind() == std::io::ErrorKind::NotFound
                    || e.to_string().contains("not found")
                    || e.to_string().contains("No such file")
                {
                    return Err(
                        "cargo-mutants is not installed. Install it with: cargo install cargo-mutants"
                            .to_string(),
                    );
                }
                return Err(format!("Failed to run cargo mutants: {}", e));
            }
            Err(_) => {
                return Err("Mutation testing timed out after 300 seconds".to_string());
            }
        }
    }

    if has_package_json {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(300),
            tokio::process::Command::new("npx")
                .args(["stryker", "run", "--reporters", "json"])
                .current_dir(root)
                .output(),
        )
        .await;

        match result {
            Ok(Ok(output)) => {
                // Stryker writes report to reports/mutation.json
                let report_path = root.join("reports").join("mutation.json");
                if report_path.exists() {
                    let content = tokio::fs::read_to_string(&report_path)
                        .await
                        .map_err(|e| format!("Failed to read Stryker report: {}", e))?;
                    return parse_stryker_output(&content);
                }
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("not found") || stderr.contains("Could not find") {
                    return Err(
                        "Stryker is not installed. Install it with: npm install --save-dev @stryker-mutator/core"
                            .to_string(),
                    );
                }
                // Try to parse from stdout
                return parse_stryker_output(&stdout);
            }
            Ok(Err(e)) => {
                return Err(format!("Failed to run Stryker: {}", e));
            }
            Err(_) => {
                return Err("Mutation testing timed out after 300 seconds".to_string());
            }
        }
    }

    Err("No recognizable project type found (Cargo.toml or package.json)".to_string())
}

fn parse_cargo_mutants_output(json_src: &str) -> Result<serde_json::Value, String> {
    // cargo-mutants --json outputs a JSON array of mutant results:
    // [{outcome: "caught"|"survived"|"timeout"|"unviable", mutant: {file, line, column, op}}, ...]
    let arr: serde_json::Value =
        serde_json::from_str(json_src).unwrap_or(serde_json::Value::Array(vec![]));

    let mut total: u32 = 0;
    let mut caught: u32 = 0;
    let mut survived: u32 = 0;
    let mut timeout: u32 = 0;
    let mut mutants_out: Vec<serde_json::Value> = Vec::new();

    if let Some(items) = arr.as_array() {
        for item in items {
            let outcome = item["outcome"].as_str().unwrap_or("unknown");
            let mutant = &item["mutant"];
            let file = mutant["file"]
                .as_str()
                .or_else(|| mutant["source_file"].as_str())
                .unwrap_or("unknown")
                .to_string();
            let line = mutant["line"].as_u64().unwrap_or(0) as u32;
            let column = mutant["column"].as_u64().unwrap_or(0) as u32;
            let op = mutant["op"]
                .as_str()
                .or_else(|| mutant["kind"].as_str())
                .unwrap_or("unknown")
                .to_string();

            match outcome {
                "caught" => caught += 1,
                "survived" => survived += 1,
                "timeout" => timeout += 1,
                _ => {}
            }
            total += 1;

            mutants_out.push(serde_json::json!({
                "outcome": outcome,
                "file": file,
                "line": line,
                "column": column,
                "op": op
            }));
        }
    }

    let score = if total > 0 {
        (caught as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    Ok(serde_json::json!({
        "total": total,
        "caught": caught,
        "survived": survived,
        "timeout": timeout,
        "score": score,
        "mutants": mutants_out
    }))
}

fn parse_stryker_output(json_src: &str) -> Result<serde_json::Value, String> {
    // Stryker mutation.json format:
    // { files: { "path": { mutants: [{id, status, ...}] } } }
    let val: serde_json::Value = serde_json::from_str(json_src).unwrap_or(serde_json::Value::Null);

    let mut total: u32 = 0;
    let mut caught: u32 = 0;
    let mut survived: u32 = 0;
    let mut timeout: u32 = 0;
    let mut mutants_out: Vec<serde_json::Value> = Vec::new();

    if let Some(files) = val["files"].as_object() {
        for (file_path, file_data) in files {
            if let Some(mutants) = file_data["mutants"].as_array() {
                for mutant in mutants {
                    let status = mutant["status"].as_str().unwrap_or("unknown");
                    let location = &mutant["location"];
                    let line = location["start"]["line"].as_u64().unwrap_or(0) as u32;
                    let column = location["start"]["column"].as_u64().unwrap_or(0) as u32;
                    let op = mutant["mutatorName"]
                        .as_str()
                        .unwrap_or("unknown")
                        .to_string();

                    match status {
                        "Killed" => caught += 1,
                        "Survived" => survived += 1,
                        "Timeout" => timeout += 1,
                        _ => {}
                    }
                    total += 1;

                    mutants_out.push(serde_json::json!({
                        "outcome": match status { "Killed" => "caught", "Survived" => "survived", "Timeout" => "timeout", s => s },
                        "file": file_path,
                        "line": line,
                        "column": column,
                        "op": op
                    }));
                }
            }
        }
    }

    let score = if total > 0 {
        (caught as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    Ok(serde_json::json!({
        "total": total,
        "caught": caught,
        "survived": survived,
        "timeout": timeout,
        "score": score,
        "mutants": mutants_out
    }))
}

// ===== API Docs HTML Export =====

struct DocSymbol {
    file: String,
    name: String,
    signature: String,
    doc: String,
}

/// Walk source files and extract doc comments, then emit a single api-docs.html file.
#[tauri::command]
pub async fn export_api_docs(project_dir: String, output_dir: String) -> Result<String, String> {
    let root = std::path::Path::new(&project_dir);
    let out_dir = std::path::Path::new(&output_dir);

    tokio::fs::create_dir_all(out_dir)
        .await
        .map_err(|e| format!("Failed to create output directory: {}", e))?;

    let mut source_files: Vec<std::path::PathBuf> = Vec::new();
    collect_source_files_for_docs(root, &mut source_files, 0);
    source_files.truncate(200);

    let mut all_symbols: Vec<DocSymbol> = Vec::new();
    for path in &source_files {
        if let Ok(content) = std::fs::read_to_string(path) {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            let file_str = path.to_string_lossy().to_string();
            extract_doc_symbols(&file_str, &content, ext, &mut all_symbols);
        }
    }

    let symbol_count = all_symbols.len();
    let html = build_api_docs_html(&all_symbols, &project_dir);

    let out_path = out_dir.join("api-docs.html");
    tokio::fs::write(&out_path, html)
        .await
        .map_err(|e| format!("Failed to write api-docs.html: {}", e))?;

    Ok(format!(
        "Exported {} symbols to {}",
        symbol_count,
        out_path.display()
    ))
}

fn collect_source_files_for_docs(
    dir: &std::path::Path,
    files: &mut Vec<std::path::PathBuf>,
    depth: usize,
) {
    if depth > 10 || files.len() >= 200 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        if files.len() >= 200 {
            break;
        }
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if ![
                "node_modules",
                "target",
                ".git",
                "dist",
                "build",
                "__pycache__",
            ]
            .contains(&name)
                && !name.starts_with('.')
            {
                collect_source_files_for_docs(&path, files, depth + 1);
            }
        } else {
            let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
            if matches!(ext, "rs" | "ts" | "js" | "py") {
                files.push(path);
            }
        }
    }
}

fn extract_doc_symbols(file: &str, content: &str, ext: &str, symbols: &mut Vec<DocSymbol>) {
    match ext {
        "rs" => extract_rust_doc_symbols(file, content, symbols),
        "ts" | "js" => extract_js_doc_symbols(file, content, symbols),
        "py" => extract_python_doc_symbols(file, content, symbols),
        _ => {}
    }
}

fn extract_rust_doc_symbols(file: &str, content: &str, symbols: &mut Vec<DocSymbol>) {
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        // Collect `///` doc comment block
        if line.starts_with("///") || line.starts_with("//!") {
            let mut doc_lines: Vec<&str> = Vec::new();
            while i < lines.len()
                && (lines[i].trim().starts_with("///") || lines[i].trim().starts_with("//!"))
            {
                let stripped = lines[i]
                    .trim()
                    .trim_start_matches("///")
                    .trim_start_matches("//!")
                    .trim();
                doc_lines.push(stripped);
                i += 1;
            }
            // The next non-attribute line should be the signature
            while i < lines.len()
                && (lines[i].trim().starts_with("#[") || lines[i].trim().is_empty())
            {
                i += 1;
            }
            if i < lines.len() {
                let sig_line = lines[i].trim();
                if let Some(name) = extract_rust_symbol_name(sig_line) {
                    symbols.push(DocSymbol {
                        file: file.to_string(),
                        name,
                        signature: sig_line.chars().take(200).collect(),
                        doc: doc_lines.join("\n"),
                    });
                }
            }
        }
        i += 1;
    }
}

fn extract_rust_symbol_name(line: &str) -> Option<String> {
    for keyword in &[
        "pub async fn ",
        "pub fn ",
        "async fn ",
        "fn ",
        "pub struct ",
        "struct ",
        "pub enum ",
        "enum ",
        "pub trait ",
        "trait ",
    ] {
        if let Some(pos) = line.find(keyword) {
            let after = &line[pos + keyword.len()..];
            let name: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

fn extract_js_doc_symbols(file: &str, content: &str, symbols: &mut Vec<DocSymbol>) {
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i].trim();
        if line.starts_with("/**") {
            let mut doc_lines: Vec<&str> = Vec::new();
            // Collect until closing */
            while i < lines.len() {
                let l = lines[i].trim();
                if l.ends_with("*/") || l == "*/" {
                    // strip leading * from this line
                    let stripped = l
                        .trim_start_matches("/**")
                        .trim_end_matches("*/")
                        .trim_start_matches('*')
                        .trim();
                    if !stripped.is_empty() {
                        doc_lines.push(stripped);
                    }
                    i += 1;
                    break;
                }
                let stripped = l.trim_start_matches("/**").trim_start_matches('*').trim();
                if !stripped.is_empty() {
                    doc_lines.push(stripped);
                }
                i += 1;
            }
            // Next non-empty line is signature
            while i < lines.len() && lines[i].trim().is_empty() {
                i += 1;
            }
            if i < lines.len() {
                let sig_line = lines[i].trim();
                if let Some(name) = extract_js_symbol_name(sig_line) {
                    symbols.push(DocSymbol {
                        file: file.to_string(),
                        name,
                        signature: sig_line.chars().take(200).collect(),
                        doc: doc_lines.join("\n"),
                    });
                }
            }
        }
        i += 1;
    }
}

fn extract_js_symbol_name(line: &str) -> Option<String> {
    // export function name / export async function name / export const name / function name
    if let Some(pos) = line.find("function ") {
        let after = &line[pos + 9..];
        let name: String = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
            .collect();
        if !name.is_empty() {
            return Some(name);
        }
    }
    // class Foo
    if let Some(pos) = line.find("class ") {
        let after = &line[pos + 6..];
        let name: String = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            return Some(name);
        }
    }
    // const/let/var name =
    for kw in &["const ", "let ", "var "] {
        if let Some(pos) = line.find(kw) {
            let after = &line[pos + kw.len()..];
            let name: String = after
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '$')
                .collect();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

fn extract_python_doc_symbols(file: &str, content: &str, symbols: &mut Vec<DocSymbol>) {
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Look for def or class with a docstring on the next line
        if trimmed.starts_with("def ")
            || trimmed.starts_with("class ")
            || trimmed.starts_with("async def ")
        {
            let sig_line = trimmed;
            let name = if let Some(pos) = sig_line.find("def ") {
                let after = &sig_line[pos + 4..];
                after
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect::<String>()
            } else if let Some(pos) = sig_line.find("class ") {
                let after = &sig_line[pos + 6..];
                after
                    .chars()
                    .take_while(|c| c.is_alphanumeric() || *c == '_')
                    .collect::<String>()
            } else {
                String::new()
            };

            if name.is_empty() {
                i += 1;
                continue;
            }

            // Look ahead for docstring
            let next_i = i + 1;
            if next_i < lines.len() {
                let next_trimmed = lines[next_i].trim();
                let (doc, advance) = if next_trimmed.starts_with("\"\"\"")
                    || next_trimmed.starts_with("'''")
                {
                    let quote = if next_trimmed.starts_with("\"\"\"") {
                        "\"\"\""
                    } else {
                        "'''"
                    };
                    // Find closing quote
                    let first_content = next_trimmed.trim_start_matches(quote);
                    if first_content.contains(quote) {
                        // Single-line docstring
                        let doc = first_content
                            .split(quote)
                            .next()
                            .unwrap_or("")
                            .trim()
                            .to_string();
                        (doc, 1)
                    } else {
                        // Multi-line docstring
                        let mut doc_lines = vec![first_content.to_string()];
                        let mut j = next_i + 1;
                        while j < lines.len() {
                            let l = lines[j].trim();
                            if l.contains(quote) {
                                let part = l.split(quote).next().unwrap_or("").trim().to_string();
                                if !part.is_empty() {
                                    doc_lines.push(part);
                                }
                                break;
                            }
                            doc_lines.push(l.to_string());
                            j += 1;
                        }
                        let doc = doc_lines.join("\n");
                        let advance = j - i;
                        (doc, advance)
                    }
                } else {
                    (String::new(), 0)
                };

                symbols.push(DocSymbol {
                    file: file.to_string(),
                    name,
                    signature: sig_line.chars().take(200).collect(),
                    doc,
                });
                i += advance;
            } else {
                symbols.push(DocSymbol {
                    file: file.to_string(),
                    name,
                    signature: sig_line.chars().take(200).collect(),
                    doc: String::new(),
                });
            }
        }
        i += 1;
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn build_api_docs_html(symbols: &[DocSymbol], project_dir: &str) -> String {
    // Group symbols by file
    let mut by_file: std::collections::BTreeMap<String, Vec<&DocSymbol>> =
        std::collections::BTreeMap::new();
    for sym in symbols {
        by_file.entry(sym.file.clone()).or_default().push(sym);
    }

    let mut toc = String::new();
    let mut content = String::new();

    for (file, syms) in &by_file {
        // Shorten file path relative to project dir
        let display_file = file
            .strip_prefix(project_dir)
            .unwrap_or(file)
            .trim_start_matches('/')
            .trim_start_matches('\\');

        toc.push_str(&format!(
            "<li class=\"toc-file\"><span>{}</span><ul>",
            html_escape(display_file)
        ));
        content.push_str(&format!(
            "<section class=\"file-section\"><h2 class=\"file-header\">{}</h2>",
            html_escape(display_file)
        ));

        for sym in syms {
            let anchor = format!(
                "sym-{}",
                sym.name
                    .chars()
                    .map(|c| if c.is_alphanumeric() { c } else { '-' })
                    .collect::<String>()
            );
            toc.push_str(&format!(
                "<li><a href=\"#{}\">{}</a></li>",
                anchor,
                html_escape(&sym.name)
            ));
            content.push_str(&format!(
                "<section id=\"{}\" class=\"symbol\"><h3>{}</h3><pre class=\"signature\">{}</pre>",
                anchor,
                html_escape(&sym.name),
                html_escape(&sym.signature)
            ));
            if !sym.doc.is_empty() {
                content.push_str(&format!(
                    "<div class=\"doc-comment\">{}</div>",
                    html_escape(&sym.doc).replace('\n', "<br>")
                ));
            }
            content.push_str("</section>");
        }

        toc.push_str("</ul></li>");
        content.push_str("</section>");
    }

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>API Documentation</title>
<style>
:root {{
  --bg: #ffffff;
  --fg: #1a1a1a;
  --border: #e0e0e0;
  --accent: #3b82f6;
  --code-bg: #f4f4f5;
  --sidebar-bg: #f9fafb;
}}
@media (prefers-color-scheme: dark) {{
  :root {{
    --bg: #0f172a;
    --fg: #e2e8f0;
    --border: #1e293b;
    --accent: #60a5fa;
    --code-bg: #1e293b;
    --sidebar-bg: #0f172a;
  }}
}}
* {{ box-sizing: border-box; margin: 0; padding: 0; }}
body {{ font-family: system-ui, sans-serif; background: var(--bg); color: var(--fg); display: flex; min-height: 100vh; }}
#sidebar {{
  width: 260px; min-width: 200px; background: var(--sidebar-bg); border-right: 1px solid var(--border);
  padding: 1rem; overflow-y: auto; position: sticky; top: 0; height: 100vh;
}}
#sidebar h1 {{ font-size: 1rem; font-weight: 700; color: var(--accent); margin-bottom: 1rem; }}
#toc {{ list-style: none; }}
#toc li {{ margin: 0.2rem 0; }}
#toc .toc-file {{ font-weight: 600; font-size: 0.8rem; color: var(--fg); opacity: 0.7; margin-top: 0.6rem; }}
#toc a {{ color: var(--accent); text-decoration: none; font-size: 0.85rem; }}
#toc a:hover {{ text-decoration: underline; }}
#toc ul {{ list-style: none; padding-left: 0.8rem; }}
#main {{ flex: 1; padding: 2rem; max-width: 900px; overflow-x: hidden; }}
.file-section {{ margin-bottom: 3rem; }}
.file-header {{ font-size: 1rem; font-weight: 700; color: var(--fg); opacity: 0.6; border-bottom: 1px solid var(--border); padding-bottom: 0.5rem; margin-bottom: 1.5rem; font-family: monospace; }}
.symbol {{ margin-bottom: 2rem; padding: 1rem; border: 1px solid var(--border); border-radius: 6px; }}
.symbol h3 {{ font-size: 1.1rem; color: var(--accent); margin-bottom: 0.5rem; }}
.signature {{ background: var(--code-bg); padding: 0.6rem 0.8rem; border-radius: 4px; font-size: 0.85rem; overflow-x: auto; margin-bottom: 0.6rem; white-space: pre-wrap; word-break: break-all; }}
.doc-comment {{ font-size: 0.9rem; line-height: 1.6; color: var(--fg); opacity: 0.85; }}
</style>
</head>
<body>
<nav id="sidebar">
  <h1>API Docs</h1>
  <ul id="toc">{toc}</ul>
</nav>
<main id="main">
  <h1 style="margin-bottom:2rem;font-size:1.5rem;">API Documentation</h1>
  {content}
</main>
</body>
</html>"#,
        toc = toc,
        content = content
    )
}
