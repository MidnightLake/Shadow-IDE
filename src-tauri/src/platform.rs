//! Cross-platform helpers for process spawning and shell commands.

/// On Windows, sets CREATE_NO_WINDOW (0x08000000) to prevent CMD popups.
/// On other platforms, this is a no-op.
#[cfg(windows)]
pub fn hide_window(cmd: &mut std::process::Command) -> &mut std::process::Command {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x08000000) // CREATE_NO_WINDOW
}

#[cfg(not(windows))]
pub fn hide_window(cmd: &mut std::process::Command) -> &mut std::process::Command {
    cmd
}

/// Same as hide_window but for tokio::process::Command.
#[cfg(windows)]
pub fn hide_window_async(cmd: &mut tokio::process::Command) -> &mut tokio::process::Command {
    #[allow(unused_imports)]
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(0x08000000)
}

#[cfg(not(windows))]
pub fn hide_window_async(cmd: &mut tokio::process::Command) -> &mut tokio::process::Command {
    cmd
}

/// Returns the command to check if a binary exists in PATH.
/// "which" on Unix, "where" on Windows.
pub fn which_cmd() -> &'static str {
    if cfg!(windows) {
        "where"
    } else {
        "which"
    }
}

/// Check if a command is available in PATH.
pub fn is_command_available(cmd: &str) -> bool {
    let mut check = std::process::Command::new(which_cmd());
    check
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    hide_window(&mut check);
    check.status().map(|s| s.success()).unwrap_or(false)
}

/// Run a shell command string cross-platform.
/// Uses "sh -c" on Unix, "cmd /c" on Windows.
pub fn shell_command(cmd_str: &str) -> std::process::Command {
    let mut cmd = if cfg!(windows) {
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", cmd_str]);
        c
    } else {
        let mut c = std::process::Command::new("sh");
        c.args(["-c", cmd_str]);
        c
    };
    hide_window(&mut cmd);
    cmd
}
