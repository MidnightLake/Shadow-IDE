use crate::config::ConfigState;
use crate::tool_calling::security::{parse_command_argv, ALLOWED_COMMANDS};
use std::collections::BTreeSet;
use std::path::Path;
use std::process::{Command, Stdio};

fn ui_allowed_commands(extra_allowed_commands: &[String]) -> BTreeSet<String> {
    let mut allowed = BTreeSet::new();
    for command in ALLOWED_COMMANDS {
        allowed.insert((*command).to_string());
    }
    for command in [
        "gh",
        "glab",
        "xdg-open",
        "open",
        "explorer",
        "explorer.exe",
        "cmd",
        "powershell",
        "pwsh",
    ] {
        allowed.insert(command.to_string());
    }
    for command in extra_allowed_commands {
        if !command.trim().is_empty() {
            allowed.insert(command.trim().to_string());
        }
    }
    allowed
}

#[tauri::command]
pub async fn shell_exec(
    command: Option<String>,
    cmd: Option<String>,
    cwd: Option<String>,
    working_dir: Option<String>,
    timeout_secs: Option<u64>,
    shadow_config: tauri::State<'_, ConfigState>,
) -> Result<String, String> {
    let raw_command = command
        .or(cmd)
        .ok_or_else(|| "Missing command".to_string())?;
    if raw_command.trim().is_empty() {
        return Err("Empty command".to_string());
    }

    let (default_timeout_secs, extra_allowed_commands) = shadow_config
        .lock()
        .map(|config| {
            (
                config.tools.shell_timeout_secs,
                config.tools.extra_allowed_commands.clone(),
            )
        })
        .unwrap_or((30, Vec::new()));

    let timeout_secs = timeout_secs.unwrap_or(default_timeout_secs).max(1);
    let working_dir = cwd.or(working_dir).unwrap_or_else(|| ".".to_string());
    if !Path::new(&working_dir).exists() {
        return Err(format!("Working directory does not exist: {}", working_dir));
    }

    let argv = parse_command_argv(&raw_command)?;
    let base_name = Path::new(&argv[0])
        .file_name()
        .map(|file| file.to_string_lossy().to_string())
        .unwrap_or_else(|| argv[0].clone());
    let allowed_commands = ui_allowed_commands(&extra_allowed_commands);
    if !allowed_commands.contains(&base_name) {
        let mut sorted_allowed: Vec<String> = allowed_commands.into_iter().collect();
        sorted_allowed.sort();
        return Err(format!(
            "Command '{}' is not allowed. Allowed commands: {}",
            base_name,
            sorted_allowed.join(", ")
        ));
    }

    let mut process = Command::new(&argv[0]);
    process
        .args(&argv[1..])
        .current_dir(&working_dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    crate::platform::hide_window(&mut process);

    let mut child = process
        .spawn()
        .map_err(|e| format!("Failed to start '{}': {}", raw_command, e))?;

    let stdout_pipe = child.stdout.take();
    let stderr_pipe = child.stderr.take();
    let (stdout_tx, stdout_rx) = std::sync::mpsc::channel::<String>();
    let (stderr_tx, stderr_rx) = std::sync::mpsc::channel::<String>();

    let stdout_handle = stdout_pipe.map(|pipe| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut reader = std::io::BufReader::new(pipe);
            let mut buffer = String::new();
            let _ = reader.read_to_string(&mut buffer);
            let _ = stdout_tx.send(buffer);
        })
    });

    let stderr_handle = stderr_pipe.map(|pipe| {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut reader = std::io::BufReader::new(pipe);
            let mut buffer = String::new();
            let _ = reader.read_to_string(&mut buffer);
            let _ = stderr_tx.send(buffer);
        })
    });

    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(timeout_secs);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    return Err(format!(
                        "Command '{}' timed out after {} seconds",
                        raw_command, timeout_secs
                    ));
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(e) => return Err(format!("Failed while waiting for '{}': {}", raw_command, e)),
        }
    };

    if let Some(handle) = stdout_handle {
        let _ = handle.join();
    }
    if let Some(handle) = stderr_handle {
        let _ = handle.join();
    }

    let stdout = stdout_rx.recv().unwrap_or_default();
    let stderr = stderr_rx.recv().unwrap_or_default();

    if status.success() {
        if !stdout.is_empty() {
            return Ok(stdout);
        }
        return Ok(stderr);
    }

    if !stderr.trim().is_empty() {
        Err(stderr)
    } else if !stdout.trim().is_empty() {
        Err(stdout)
    } else {
        Err(format!(
            "Command '{}' exited with status {}",
            raw_command,
            status.code().unwrap_or(-1)
        ))
    }
}
