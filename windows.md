# Windows 10/11 Build Guide — ShadowIDE

## Prerequisites

### Required Software

| Tool | Version | Notes |
|------|---------|-------|
| **Windows 10/11** | 1809+ | x86_64 only |
| **Visual Studio Build Tools** | 2022 | Select "Desktop development with C++" workload |
| **Rust** | 1.75+ | `rustup default stable-x86_64-pc-windows-msvc` |
| **Node.js** | 20 LTS+ | For Vite/React frontend |
| **pnpm** or **npm** | latest | Package manager |
| **Git** | 2.40+ | With Git Bash |

### Install Commands (PowerShell as Admin)

```powershell
# Rust
winget install Rustlang.Rustup
rustup default stable-x86_64-pc-windows-msvc

# Node.js
winget install OpenJS.NodeJS.LTS

# Visual Studio Build Tools (C++ workload)
winget install Microsoft.VisualStudio.2022.BuildTools --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"

# Git
winget install Git.Git

# Tauri CLI
cargo install tauri-cli
```

### Optional (GPU Support)

| Tool | Version | Notes |
|------|---------|-------|
| **CUDA Toolkit** | 12.x+ | For NVIDIA GPU acceleration with llama.cpp |
| **Vulkan SDK** | 1.3+ | Alternative GPU backend |
| **CMake** | 3.25+ | Required if building llama.cpp from source |

```powershell
# CUDA (if NVIDIA GPU)
winget install Nvidia.CUDA

# Vulkan SDK
winget install KhronosGroup.VulkanSDK

# CMake
winget install Kitware.CMake
```

---

## Known Build Blockers

The codebase has several Unix-specific sections that must be addressed before a clean Windows build. These are the current issues and their fixes:

### 1. `bluer` crate — Linux-only Bluetooth dependency

**File:** `src-tauri/Cargo.toml`

The `bluer` crate (BlueZ Bluetooth) only compiles on Linux. It's unconditionally included.

**Fix:** Make it platform-conditional:

```toml
# Before
bluer = { version = "0.17", features = ["bluetoothd", "l2cap"] }

# After
[target.'cfg(target_os = "linux")'.dependencies]
bluer = { version = "0.17", features = ["bluetoothd", "l2cap"] }
```

And gate the module in `src-tauri/src/lib.rs`:

```rust
#[cfg(target_os = "linux")]
mod bluetooth_server;
```

### 2. `std::os::unix::process::CommandExt` — Unix-only import

**File:** `src-tauri/src/llm_loader.rs:5`

```rust
// Before
use std::os::unix::process::CommandExt;

// After
#[cfg(unix)]
use std::os::unix::process::CommandExt;
```

Any usage of `.exec()` from CommandExt must also be gated:

```rust
#[cfg(unix)]
{ cmd.exec(); }
#[cfg(windows)]
{ cmd.spawn().expect("Failed to spawn process"); }
```

### 3. Archive extraction — uses `tar` / `unzip` shell commands

**File:** `src-tauri/src/llm_loader.rs` (lines ~1524, ~1660)

The code calls `tar -xzf` and `unzip` which aren't available on stock Windows.

**Fix:** Use the `zip` and `flate2`/`tar` Rust crates instead, or use PowerShell:

```rust
#[cfg(windows)]
{
    std::process::Command::new("powershell")
        .args(["-Command", &format!(
            "Expand-Archive -Path '{}' -DestinationPath '{}'",
            archive_path.display(), extract_dir.display()
        )])
        .status()
}
```

### 4. GPU detection — Linux-specific tool paths

**File:** `src-tauri/src/llm_loader.rs`

GPU detection uses `lspci`, `nvidia-smi` at Linux paths, `rocm-smi`, etc.

**Fix (Windows):**

```rust
#[cfg(windows)]
fn detect_gpu() -> Option<GpuInfo> {
    // Use DXGI or WMI to enumerate GPUs
    let output = std::process::Command::new("wmic")
        .args(["path", "win32_VideoController", "get", "name"])
        .output().ok()?;
    // Parse output for NVIDIA/AMD/Intel
}
```

Or use `nvidia-smi.exe` which is at `C:\Windows\System32\nvidia-smi.exe` on NVIDIA systems.

### 5. File permissions — `chmod 0o755`

**File:** `src-tauri/src/llm_loader.rs` (multiple locations)

Already gated with `#[cfg(unix)]` — no action needed, but Windows needs no equivalent since executables don't need +x.

### 6. Shell detection

**File:** `src-tauri/src/terminal.rs`

Already partially Windows-aware (checks `COMSPEC`). Should also detect PowerShell:

```rust
#[cfg(windows)]
{
    shells.push("powershell.exe".to_string());
    shells.push("pwsh.exe".to_string()); // PowerShell Core
    if let Ok(comspec) = std::env::var("COMSPEC") {
        shells.push(comspec); // cmd.exe
    }
}
```

---

## Build Steps

### 1. Clone and setup

```powershell
git clone <repo-url> shadow-ide
cd shadow-ide
npm install
```

### 2. Apply Windows fixes

Apply the fixes from the "Known Build Blockers" section above. The minimum changes needed:

```powershell
# These files MUST be patched before building on Windows:
# - src-tauri/Cargo.toml          (gate bluer dependency)
# - src-tauri/src/lib.rs          (gate bluetooth_server module)
# - src-tauri/src/llm_loader.rs   (gate unix imports + archive commands)
```

### 3. Build

```powershell
# Development build
cargo tauri dev

# Release build
cargo tauri build

# CLI only
cd cli
cargo build --release
# Binary at: cli\target\release\shadowai.exe
```

### 4. Output locations

```
src-tauri\target\release\shadow-ide.exe          # Main app binary
src-tauri\target\release\bundle\nsis\*.exe        # NSIS installer
cli\target\release\shadowai.exe                   # CLI binary
```

---

## llama.cpp on Windows

The app downloads pre-built llama.cpp binaries. For Windows, it needs:

- **CUDA build:** `llama-server.exe` built with `-DGGML_CUDA=ON`
- **Vulkan build:** `llama-server.exe` built with `-DGGML_VULKAN=ON`
- **CPU build:** `llama-server.exe` (default, no GPU)

Pre-built Windows binaries are available from the [llama.cpp releases](https://github.com/ggerganov/llama.cpp/releases).

The app spawns llama.cpp as a child process:

```
llama-server.exe --model <path> --port 8080 --ctx-size 8192
```

### Windows-specific llama.cpp notes

- Use `.exe` suffix in spawn commands
- CUDA requires `cudart64_*.dll` on PATH or next to the binary
- Vulkan requires the Vulkan SDK or `vulkan-1.dll` on PATH
- The `--timeout 0` flag prevents idle disconnects (same as Linux)

---

## Environment Variables

| Variable | Purpose | Example |
|----------|---------|---------|
| `SHADOWAI_STATE_DIR` | State files directory | `C:\Users\You\.shadowai\state` |
| `SHADOWAI_HOST` | Server host for CLI | `localhost` |
| `RUST_LOG` | Log level | `info` |

---

## Troubleshooting

### "LINK : fatal error LNK1181: cannot open input file 'dbus-1.lib'"
The `bluer` crate requires D-Bus (Linux only). Apply the Cargo.toml fix from blocker #1.

### "error: process didn't exit successfully" during `cargo tauri build`
Ensure Visual Studio Build Tools C++ workload is installed. Run from a "Developer PowerShell for VS 2022".

### WebView2 missing
Windows 10 older versions may not have WebView2. Install it:
```powershell
winget install Microsoft.EdgeWebView2Runtime
```

### llama-server.exe won't start
Check that the correct DLLs are present (CUDA/Vulkan). Run manually to see error:
```powershell
.\llama-server.exe --model model.gguf --port 8080
```
