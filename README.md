# ShadowIDE

A lightweight, Rust-powered IDE built with Tauri v2.

The repo now also contains a native editor foundation under [`native/`](/home/shadowbunny/Documents/CLI/shadow-ide/native/README.md) that follows the `planengine.md` architecture while keeping the current app intact.

## Features

- **File Explorer** -- Tree-view file browser with directory watching via `notify`
- **Monaco Editor** -- Full-featured code editor with syntax highlighting and multi-tab support
- **Integrated Terminal** -- xterm.js frontend backed by `portable-pty` for native shell access
- **AI Integration** -- Chat sidebar and inline code completion powered by LM Studio (OpenAI-compatible API)
- **Tool Calling** -- 7 built-in tools (read_file, write_file, list_directory, search_files, search_content, run_command, get_diagnostics) with multi-turn execution loop
- **Token Optimizer** -- LM cache (SHA-256 keyed, 200 entries, 5min TTL), token cleaning modes (none/trim/strip), and smart truncation
- **TODO Scanner** -- Scans for TODO, FIXME, HACK, BUG, and other markers with priority sorting and filtering
- **Project Manager** -- Auto-saves editor state (open files, layout), restores on reopen, recent projects menu
- **Remote Access** -- TLS WebSocket server for secure remote connectivity
- **QR Pairing** -- Pair remote devices by scanning a generated QR code

## Tech Stack

| Layer | Technology |
| :--- | :--- |
| Backend | Rust, Tauri v2 |
| Frontend | React 19, TypeScript, Vite |
| Editor | Monaco Editor |
| Terminal | portable-pty (backend), xterm.js (frontend) |

## Build Instructions

### Prerequisites

- [Rust](https://rustup.rs/) (stable toolchain)
- [Node.js](https://nodejs.org/) (v18+)
- [Tauri CLI](https://v2.tauri.app/start/create-project/) (`cargo install tauri-cli`)

### Development

```sh
npm install
npm run tauri dev
```

### Native Editor Foundation

```sh
npm run dev:native
```

This launches the in-repo native Rust editor track built under `shadow-ide/native`, while the existing Tauri app and web frontend continue to work as before.

### Production Build

```sh
npm run tauri build
```

The release binary and platform packages (.deb, .rpm) will be generated in `src-tauri/target/release/bundle/`.

## Keyboard Shortcuts

| Shortcut | Action |
| :--- | :--- |
| Ctrl+S | Save current file |
| Ctrl+` | Toggle terminal |
| Ctrl+Shift+A | Toggle AI chat |
| Ctrl+Shift+T | Toggle TODO panel |
| Ctrl+Shift+R | Toggle remote settings |
| Ctrl+Shift+F | Search files |

## License

MIT
