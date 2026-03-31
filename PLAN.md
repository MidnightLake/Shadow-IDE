# Rust-Based IDE Architecture Plan (Codename: "ShadowIDE")

> Note: `shadow-ide/native` is the current integration point for the standalone native editor track defined in `/home/shadowbunny/Documents/CLI/planengine.md`. The existing Tauri/mobile plan remains in-repo and intact.

## 1. High-Level Architecture
ShadowIDE will follow a **Headless Core + Multi-Frontend** architecture. This allows the heavy lifting (file indexing, LSP, terminal PTY) to run on a powerful machine while the UI can run locally on the desktop or remotely on an iPhone.

- **ShadowCore (Rust)**: The backend service responsible for file systems, process management (CLI), and LSP orchestration.
- **ShadowUI (Tauri v2)**: The frontend layer. Tauri v2 is chosen because it supports cross-platform Desktop (Rust/Webview) and Mobile (iOS/Android) using a shared codebase.

## 2. Component Breakdown

### A. Integrated CLI (Terminal)
- **Library**: `portable-pty` (by the author of WezTerm) for cross-platform PTY handling.
- **Frontend**: `xterm.js` for the terminal UI, communicating with the Rust backend via WebSockets or Tauri IPC.
- **Features**: Shell integration (Zsh/Bash/PowerShell), color support, and persistent sessions.

### B. Secure Local Bridge (LM Studio)
- **Interface**: LM Studio exposes an OpenAI-compatible API at `localhost:1234`.
- **Implementation**:
    - Use `reqwest` in the Rust backend to proxy requests to LM Studio.
    - **Security**: The bridge only listens on `localhost` by default. For remote access, the ShadowCore will encrypt and forward these requests securely to the iPhone client.
    - **AI Features**: Inline code completion, chat sidebar, and "Explain CLI Error" button.

### C. iPhone Remote Access ("Remote Write & Check")
- **Technology**: Native iOS app built on Linux via `xtool` (embedding the compiled web frontend) and a PWA fallback.
- **Secure Tunnel**: 
    - **Option 1 (P2P)**: Use `libp2p` or `noise-protocol` for an end-to-end encrypted (E2EE) connection between the iPhone and the Desktop.
    - **Option 2 (VPN)**: Recommend/Integrate with Tailscale (WireGuard) for a secure "Local" network.
- **Functionality**:
    - **Remote FS**: Browse and edit files on the desktop from the iPhone.
    - **Remote PTY**: Run build/test commands from the iPhone and see live output.
    - **State Sync**: Open files and cursor positions synced across devices.

## 3. Tech Stack
| Layer | Technology |
| :--- | :--- |
| **Language** | Rust (Primary) |
| **UI Framework** | Tauri v2 (React + Rust + Vanilla CSS) |
| **Editor Core** | Monaco Editor or CodeMirror 6 (Web-based) |
| **Terminal** | `portable-pty` (Backend) + `xterm.js` (Frontend) |
| **LSP** | `tower-lsp` for language server integration |
| **Networking** | `tokio-tungstenite` (WebSockets) + `rustls` (mTLS) |
| **Serialization** | `serde` (JSON/MessagePack) |

## 4. Security Model
1.  **Local-Only by Default**: The ShadowCore only accepts local connections unless "Remote Access" is explicitly enabled.
2.  **Authentication**: iPhone pairing via QR Code (exchanging public keys for mTLS).
3.  **Encryption**: All remote traffic is encrypted via TLS 1.3 or the Noise Protocol.
4.  **Sandboxing**: Tauri provides a restricted environment for the UI, protecting the host system.

## 5. Implementation Roadmap
### Phase 1: The Core (Desktop)
- [ ] Initialize Tauri v2 project.
- [ ] Implement File Explorer (Rust `std::fs` + `notify`).
- [ ] Integrate `portable-pty` for a working terminal.
- [ ] Basic Monaco Editor integration.

### Phase 2: AI & LM Studio
- [ ] Create `ai-bridge` module to talk to `localhost:1234`.
- [ ] Implement code-completion logic.

### Phase 3: Remote Connectivity
- [ ] Implement a WebSocket server with mTLS in the Rust backend.
- [ ] Build the Tauri iOS frontend.
- [ ] Create the pairing/handshake mechanism.

### Phase 4: Polish
- [ ] Performance optimization for large files.
- [ ] UI/UX refinements for mobile (touch-friendly editing).
