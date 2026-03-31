use serde::Serialize;
use serde_json::json;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Serialize, Clone)]
pub struct GitFileStatus {
    pub path: String,
    pub status: String, // "M", "A", "D", "?", "R", etc.
}

#[derive(Debug, Serialize, Clone)]
pub struct BlameLine {
    pub commit: String,
    pub author: String,
    pub line_num: usize,
    pub content: String,
}

#[tauri::command]
pub fn git_worktree_list(root: String) -> Result<Vec<String>, String> {
    let output = {
        let mut cmd = Command::new("git");
        cmd.args(["worktree", "list", "--porcelain"])
            .current_dir(&root);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
            .map_err(|e| format!("Failed to run git worktree list: {}", e))?
    };
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut results = Vec::new();
    let mut current_path = String::new();
    let mut current_branch = String::new();
    for line in stdout.lines() {
        if let Some(p) = line.strip_prefix("worktree ") {
            current_path = p.to_string();
            current_branch = String::new();
        } else if let Some(b) = line.strip_prefix("branch refs/heads/") {
            current_branch = b.to_string();
        } else if line.is_empty() && !current_path.is_empty() {
            let label = if current_branch.is_empty() {
                current_path.clone()
            } else {
                format!("{} ({})", current_path, current_branch)
            };
            results.push(label);
            current_path.clear();
            current_branch.clear();
        }
    }
    if !current_path.is_empty() {
        let label = if current_branch.is_empty() {
            current_path.clone()
        } else {
            format!("{} ({})", current_path, current_branch)
        };
        results.push(label);
    }
    Ok(results)
}

#[tauri::command]
pub fn git_worktree_add(root: String, path: String, branch: String) -> Result<String, String> {
    // Try `git worktree add <path> <branch>` first; if branch doesn't exist, create it with -b
    let output = {
        let mut cmd = Command::new("git");
        cmd.args(["worktree", "add", &path, &branch])
            .current_dir(&root);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
            .map_err(|e| format!("Failed to run git worktree add: {}", e))?
    };
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }
    // Try creating a new branch
    let output2 = {
        let mut cmd = Command::new("git");
        cmd.args(["worktree", "add", "-b", &branch, &path])
            .current_dir(&root);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
            .map_err(|e| format!("Failed to run git worktree add -b: {}", e))?
    };
    if !output2.status.success() {
        return Err(String::from_utf8_lossy(&output2.stderr).to_string());
    }
    Ok(String::from_utf8_lossy(&output2.stdout).to_string())
}

#[tauri::command]
pub fn git_worktree_remove(root: String, path: String) -> Result<String, String> {
    let output = {
        let mut cmd = Command::new("git");
        cmd.args(["worktree", "remove", &path]).current_dir(&root);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
            .map_err(|e| format!("Failed to run git worktree remove: {}", e))?
    };
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[tauri::command]
pub fn git_stash_list(root: String) -> Result<Vec<serde_json::Value>, String> {
    let output = {
        let mut cmd = Command::new("git");
        cmd.args(["stash", "list", "--format=%gd|%s|%cr"])
            .current_dir(&root);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
            .map_err(|e| format!("Failed to run git stash list: {}", e))?
    };
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let results = stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.splitn(3, '|').collect();
            json!({
                "id": parts.first().copied().unwrap_or(""),
                "message": parts.get(1).copied().unwrap_or(""),
                "date": parts.get(2).copied().unwrap_or(""),
            })
        })
        .collect();
    Ok(results)
}

#[tauri::command]
pub fn git_stash_show(root: String, stash_ref: String) -> Result<String, String> {
    let output = {
        let mut cmd = Command::new("git");
        cmd.args(["stash", "show", "-p", &stash_ref])
            .current_dir(&root);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
            .map_err(|e| format!("Failed to run git stash show: {}", e))?
    };
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[tauri::command]
pub fn git_cherry_pick(root: String, commit: String) -> Result<String, String> {
    let output = {
        let mut cmd = Command::new("git");
        cmd.args(["cherry-pick", &commit]).current_dir(&root);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
            .map_err(|e| format!("Failed to run git cherry-pick: {}", e))?
    };
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[tauri::command]
pub fn git_commit_details(root: String, hash: String) -> Result<serde_json::Value, String> {
    let output = {
        let mut cmd = Command::new("git");
        cmd.args([
            "show",
            "--format=%H|%an|%ae|%at|%s|%b",
            "--name-status",
            &hash,
        ])
        .current_dir(&root);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
            .map_err(|e| format!("Failed to run git show: {}", e))?
    };
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut lines = stdout.lines();

    // First line is the format
    let header = lines.next().unwrap_or("");
    let parts: Vec<&str> = header.splitn(6, '|').collect();
    let hash_val = parts.first().copied().unwrap_or("");
    let author = parts.get(1).copied().unwrap_or("");
    let email = parts.get(2).copied().unwrap_or("");
    let timestamp: i64 = parts.get(3).copied().unwrap_or("0").parse().unwrap_or(0);
    let subject = parts.get(4).copied().unwrap_or("");
    let body = parts.get(5).copied().unwrap_or("");

    // Skip blank line after header
    let mut files: Vec<serde_json::Value> = Vec::new();
    let mut found_blank = false;
    for line in lines {
        if line.is_empty() && !found_blank {
            found_blank = true;
            continue;
        }
        if line.is_empty() {
            continue;
        }
        // Name-status lines: "M\tpath" or "A\tpath" etc.
        let mut parts2 = line.splitn(2, '\t');
        let status = parts2.next().unwrap_or("").trim().to_string();
        let fpath = parts2.next().unwrap_or("").trim().to_string();
        if !status.is_empty() && !fpath.is_empty() {
            files.push(json!({ "status": status, "path": fpath }));
        }
    }

    Ok(json!({
        "hash": hash_val,
        "author": author,
        "email": email,
        "timestamp": timestamp,
        "subject": subject,
        "body": body,
        "files": files,
    }))
}

#[tauri::command]
pub fn git_branch_graph(
    root: String,
    max_commits: Option<u32>,
) -> Result<Vec<serde_json::Value>, String> {
    let limit = max_commits.unwrap_or(200).min(500);
    let limit_str = format!("-{}", limit);
    let output = {
        let mut cmd = Command::new("git");
        cmd.args(["log", "--all", "--format=%H|%P|%an|%at|%s|%D", &limit_str])
            .current_dir(&root);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
            .map_err(|e| format!("Failed to run git log: {}", e))?
    };
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let results = stdout
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let parts: Vec<&str> = line.splitn(6, '|').collect();
            let hash = parts.first().copied().unwrap_or("");
            let parents_str = parts.get(1).copied().unwrap_or("");
            let author = parts.get(2).copied().unwrap_or("");
            let timestamp: i64 = parts.get(3).copied().unwrap_or("0").parse().unwrap_or(0);
            let subject = parts.get(4).copied().unwrap_or("");
            let refs_str = parts.get(5).copied().unwrap_or("");
            let parents: Vec<&str> = parents_str.split_whitespace().collect();
            let refs: Vec<&str> = refs_str
                .split(", ")
                .map(|r| r.trim())
                .filter(|r| !r.is_empty())
                .collect();
            json!({
                "hash": hash,
                "parents": parents,
                "author": author,
                "timestamp": timestamp,
                "subject": subject,
                "refs": refs,
            })
        })
        .collect();
    Ok(results)
}

/// Get git blame for a file — returns per-line blame info
#[tauri::command]
pub async fn git_blame(repo_path: String, file_path: String) -> Result<Vec<BlameEntry>, String> {
    let out = {
        let mut cmd = Command::new("git");
        cmd.args(["blame", "--porcelain", &file_path])
            .current_dir(&repo_path);
        crate::platform::hide_window(&mut cmd);
        cmd.output().map_err(|e| e.to_string())?
    };

    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }

    let text = String::from_utf8_lossy(&out.stdout);
    let mut entries: Vec<BlameEntry> = Vec::new();
    let mut current_hash = String::new();
    let mut current_author = String::new();
    let mut current_time: i64 = 0;
    let mut current_summary = String::new();
    let mut current_line: u32 = 0;

    for line in text.lines() {
        if line.len() >= 40 && line.chars().take(40).all(|c| c.is_ascii_hexdigit()) {
            let parts: Vec<&str> = line.splitn(4, ' ').collect();
            current_hash = parts[0].to_string();
            current_line = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("author ") {
            current_author = v.to_string();
        } else if let Some(v) = line.strip_prefix("author-time ") {
            current_time = v.parse().unwrap_or(0);
        } else if let Some(v) = line.strip_prefix("summary ") {
            current_summary = v.to_string();
        } else if line.starts_with('\t') {
            entries.push(BlameEntry {
                line: current_line,
                commit_hash: current_hash[..8.min(current_hash.len())].to_string(),
                author: current_author.clone(),
                timestamp: current_time,
                summary: current_summary.clone(),
                content: line[1..].to_string(),
            });
        }
    }
    Ok(entries)
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct BlameEntry {
    pub line: u32,
    pub commit_hash: String,
    pub author: String,
    pub timestamp: i64,
    pub summary: String,
    pub content: String,
}

/// Get diff hunks for a file (staged + unstaged)
#[tauri::command]
pub async fn git_diff_hunks(
    repo_path: String,
    file_path: String,
    staged: bool,
) -> Result<Vec<DiffHunk>, String> {
    let mut args = vec!["diff", "--unified=3"];
    if staged {
        args.push("--staged");
    }
    args.push(&file_path);

    let out = {
        let mut cmd = Command::new("git");
        cmd.args(&args).current_dir(&repo_path);
        crate::platform::hide_window(&mut cmd);
        cmd.output().map_err(|e| e.to_string())?
    };

    let text = String::from_utf8_lossy(&out.stdout);
    let mut hunks: Vec<DiffHunk> = Vec::new();
    let mut current_header = String::new();
    let mut current_lines: Vec<String> = Vec::new();
    let mut old_start = 0u32;
    let mut new_start = 0u32;

    for line in text.lines() {
        if line.starts_with("@@") {
            if !current_header.is_empty() {
                hunks.push(DiffHunk {
                    header: current_header.clone(),
                    lines: current_lines.clone(),
                    old_start,
                    new_start,
                });
                current_lines.clear();
            }
            current_header = line.to_string();
            // Parse @@ -old_start,... +new_start,...
            let parts: Vec<&str> = line.split_whitespace().collect();
            old_start = parts
                .get(1)
                .and_then(|s| {
                    s.trim_start_matches('-')
                        .split(',')
                        .next()
                        .and_then(|n| n.parse().ok())
                })
                .unwrap_or(0);
            new_start = parts
                .get(2)
                .and_then(|s| {
                    s.trim_start_matches('+')
                        .split(',')
                        .next()
                        .and_then(|n| n.parse().ok())
                })
                .unwrap_or(0);
        } else if !current_header.is_empty() {
            current_lines.push(line.to_string());
        }
    }
    if !current_header.is_empty() {
        hunks.push(DiffHunk {
            header: current_header,
            lines: current_lines,
            old_start,
            new_start,
        });
    }
    Ok(hunks)
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct DiffHunk {
    pub header: String,
    pub lines: Vec<String>,
    pub old_start: u32,
    pub new_start: u32,
}

/// Stage a specific hunk (index into the hunk list)
#[tauri::command]
pub async fn git_stage_hunk(
    repo_path: String,
    file_path: String,
    hunk_header: String,
    hunk_lines: Vec<String>,
) -> Result<String, String> {
    // Build a minimal patch and apply it with `git apply --cached`
    let patch = format!(
        "--- a/{}\n+++ b/{}\n{}\n{}\n",
        file_path,
        file_path,
        hunk_header,
        hunk_lines.join("\n")
    );

    // Write patch to temp file
    let tmp = std::env::temp_dir().join("shadow_ide_hunk.patch");
    std::fs::write(&tmp, &patch).map_err(|e| e.to_string())?;

    let out = {
        let mut cmd = Command::new("git");
        cmd.args([
            "apply",
            "--cached",
            "--whitespace=nowarn",
            &tmp.to_string_lossy(),
        ])
        .current_dir(&repo_path);
        crate::platform::hide_window(&mut cmd);
        cmd.output().map_err(|e| e.to_string())?
    };

    let _ = std::fs::remove_file(&tmp);

    if out.status.success() {
        Ok("Hunk staged".to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

/// Unstage a specific hunk
#[tauri::command]
pub async fn git_unstage_hunk(
    repo_path: String,
    file_path: String,
    hunk_header: String,
    hunk_lines: Vec<String>,
) -> Result<String, String> {
    let patch = format!(
        "--- a/{}\n+++ b/{}\n{}\n{}\n",
        file_path,
        file_path,
        hunk_header,
        hunk_lines.join("\n")
    );
    let tmp = std::env::temp_dir().join("shadow_ide_unstage_hunk.patch");
    std::fs::write(&tmp, &patch).map_err(|e| e.to_string())?;

    let out = {
        let mut cmd = Command::new("git");
        cmd.args([
            "apply",
            "--cached",
            "--reverse",
            "--whitespace=nowarn",
            &tmp.to_string_lossy(),
        ])
        .current_dir(&repo_path);
        crate::platform::hide_window(&mut cmd);
        cmd.output().map_err(|e| e.to_string())?
    };
    let _ = std::fs::remove_file(&tmp);

    if out.status.success() {
        Ok("Hunk unstaged".to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_is_repo_with_git_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        assert!(git_is_repo(dir.path().to_string_lossy().to_string()));
    }

    #[test]
    fn test_git_is_repo_without_git_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!git_is_repo(dir.path().to_string_lossy().to_string()));
    }

    #[test]
    fn test_git_status_non_repo() {
        let dir = tempfile::tempdir().unwrap();
        let result = git_status(dir.path().to_string_lossy().to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_git_status_real_repo() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().to_string();
        // Init a git repo
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&root)
            .output()
            .unwrap();
        // Configure git for the test
        std::process::Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(&root)
            .output()
            .unwrap();

        // Create an untracked file
        std::fs::write(dir.path().join("new.txt"), "hello").unwrap();

        let result = git_status(root).unwrap();
        assert!(!result.is_empty());
        assert!(result
            .iter()
            .any(|s| s.path == "new.txt" && s.status == "??"));
    }

    #[test]
    fn test_git_file_diff_non_repo() {
        let dir = tempfile::tempdir().unwrap();
        let result = git_file_diff(
            "test.txt".to_string(),
            dir.path().to_string_lossy().to_string(),
        );
        // git diff in non-repo returns empty or error depending on git version
        // Just verify it doesn't panic
        let _ = result;
    }

    #[test]
    fn test_blame_line_struct() {
        let line = BlameLine {
            commit: "abc12345".to_string(),
            author: "John".to_string(),
            line_num: 42,
            content: "let x = 1;".to_string(),
        };
        assert_eq!(line.commit, "abc12345");
        assert_eq!(line.line_num, 42);
    }

    #[test]
    fn test_git_file_blame_non_repo() {
        let dir = tempfile::tempdir().unwrap();
        let result = git_file_blame(
            "test.txt".to_string(),
            dir.path().to_string_lossy().to_string(),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_git_file_blame_real_repo() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_string_lossy().to_string();
        // Init repo and commit a file
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(&root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.email", "t@t.com"])
            .current_dir(&root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["config", "user.name", "T"])
            .current_dir(&root)
            .output()
            .unwrap();
        std::fs::write(dir.path().join("file.txt"), "line one\nline two\n").unwrap();
        std::process::Command::new("git")
            .args(["add", "file.txt"])
            .current_dir(&root)
            .output()
            .unwrap();
        std::process::Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(&root)
            .output()
            .unwrap();

        let result = git_file_blame("file.txt".to_string(), root).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].content, "line one");
        assert_eq!(result[1].content, "line two");
        assert_eq!(result[0].line_num, 1);
        assert_eq!(result[1].line_num, 2);
    }

    #[test]
    fn test_git_file_status_serializes() {
        let status = GitFileStatus {
            path: "src/main.rs".to_string(),
            status: "M".to_string(),
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.contains("src/main.rs"));
        assert!(json.contains("\"M\""));
    }
}

#[tauri::command]
pub fn git_is_repo(root: String) -> bool {
    Path::new(&root).join(".git").exists()
}

#[tauri::command]
pub fn git_status(root: String) -> Result<Vec<GitFileStatus>, String> {
    let output = {
        let mut cmd = Command::new("git");
        cmd.args(["status", "--porcelain", "-uall"])
            .current_dir(&root);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
            .map_err(|e| format!("Failed to run git: {}", e))?
    };

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut results = Vec::new();
    for line in stdout.lines() {
        if line.len() < 4 || !line.is_char_boundary(2) || !line.is_char_boundary(3) {
            continue;
        }
        let status = line[..2].trim().to_string();
        let path = line[3..].to_string();
        results.push(GitFileStatus { path, status });
    }
    Ok(results)
}

#[tauri::command]
pub fn git_file_diff(path: String, root: String) -> Result<String, String> {
    let output = {
        let mut cmd = Command::new("git");
        cmd.args(["diff", "--", &path]).current_dir(&root);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
            .map_err(|e| format!("Failed to run git diff: {}", e))?
    };

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[tauri::command]
pub fn git_file_blame(path: String, root: String) -> Result<Vec<BlameLine>, String> {
    let output = {
        let mut cmd = Command::new("git");
        cmd.args(["blame", "--line-porcelain", &path])
            .current_dir(&root);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
            .map_err(|e| format!("Failed to run git blame: {}", e))?
    };

    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).to_string());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut results = Vec::new();
    let mut current_commit = String::new();
    let mut current_author = String::new();
    let mut current_line: usize = 0;

    for line in stdout.lines() {
        if line.len() >= 40 && line.chars().take(40).all(|c| c.is_ascii_hexdigit()) {
            // This is a commit line: <hash> <orig_line> <final_line> [<num_lines>]
            let parts: Vec<&str> = line.split_whitespace().collect();
            current_commit = parts.first().unwrap_or(&"").chars().take(8).collect();
            current_line = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
        } else if let Some(author) = line.strip_prefix("author ") {
            current_author = author.to_string();
        } else if let Some(content) = line.strip_prefix('\t') {
            results.push(BlameLine {
                commit: current_commit.clone(),
                author: current_author.clone(),
                line_num: current_line,
                content: content.to_string(),
            });
        }
    }
    Ok(results)
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct PullRequest {
    pub number: u64,
    pub title: String,
    pub state: String,
    pub url: String,
    pub author: String,
    pub created_at: String,
    pub body: String,
    pub labels: Vec<String>,
    pub draft: bool,
}

/// List open PRs for current repo using `gh` CLI
#[tauri::command]
pub async fn git_list_prs(
    repo_path: String,
    state: Option<String>,
) -> Result<Vec<PullRequest>, String> {
    let state_filter = state.as_deref().unwrap_or("open");
    let out = std::process::Command::new("gh")
        .args([
            "pr",
            "list",
            "--state",
            state_filter,
            "--json",
            "number,title,state,url,author,createdAt,body,labels,isDraft",
        ])
        .current_dir(&repo_path)
        .output()
        .map_err(|e| format!("gh CLI not found: {}", e))?;

    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }

    let prs: Vec<serde_json::Value> =
        serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;

    Ok(prs
        .iter()
        .map(|pr| PullRequest {
            number: pr["number"].as_u64().unwrap_or(0),
            title: pr["title"].as_str().unwrap_or("").to_string(),
            state: pr["state"].as_str().unwrap_or("").to_string(),
            url: pr["url"].as_str().unwrap_or("").to_string(),
            author: pr["author"]["login"].as_str().unwrap_or("").to_string(),
            created_at: pr["createdAt"].as_str().unwrap_or("").to_string(),
            body: pr["body"].as_str().unwrap_or("").to_string(),
            labels: pr["labels"]
                .as_array()
                .unwrap_or(&vec![])
                .iter()
                .filter_map(|l| l["name"].as_str().map(str::to_string))
                .collect(),
            draft: pr["isDraft"].as_bool().unwrap_or(false),
        })
        .collect())
}

/// Create a PR using `gh` CLI
#[tauri::command]
pub async fn git_create_pr(
    repo_path: String,
    title: String,
    body: String,
    base: Option<String>,
    draft: Option<bool>,
) -> Result<String, String> {
    let base_ref = base.unwrap_or_else(|| "main".to_string());
    let mut args = vec![
        "pr".to_string(),
        "create".to_string(),
        "--title".to_string(),
        title.clone(),
        "--body".to_string(),
        body.clone(),
        "--base".to_string(),
        base_ref,
    ];
    if draft.unwrap_or(false) {
        args.push("--draft".to_string());
    }

    let out = std::process::Command::new("gh")
        .args(&args)
        .current_dir(&repo_path)
        .output()
        .map_err(|e| format!("gh CLI not found: {}", e))?;

    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).to_string())
    }
}

/// Get PR details + review comments
#[tauri::command]
pub async fn git_get_pr(repo_path: String, pr_number: u64) -> Result<serde_json::Value, String> {
    let out = std::process::Command::new("gh")
        .args([
            "pr",
            "view",
            &pr_number.to_string(),
            "--json",
            "number,title,body,state,url,reviews,comments,files,statusCheckRollup",
        ])
        .current_dir(&repo_path)
        .output()
        .map_err(|e| format!("gh CLI not found: {}", e))?;

    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
}

/// Get CI status for a commit
#[tauri::command]
pub async fn git_ci_status(
    repo_path: String,
    commit_sha: Option<String>,
) -> Result<serde_json::Value, String> {
    let sha = commit_sha.unwrap_or_else(|| "HEAD".to_string());
    let endpoint = format!("repos/{{owner}}/{{repo}}/commits/{}/check-runs", sha);
    let out = std::process::Command::new("gh")
        .args(["api", &endpoint])
        .current_dir(&repo_path)
        .output()
        .map_err(|e| format!("gh CLI not found: {}", e))?;

    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).to_string());
    }
    serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())
}
