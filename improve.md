# Shadow IDE Improvement Plan

## 🛡️ Security & Stability
- [ ] **Transition from Blacklist to Whitelist:** Replace the `BLOCKED_COMMANDS` approach in `tool_calling.rs` with a strict whitelist of permitted commands.
- [ ] **User Confirmation for Risky Tools:** Implement a UI prompt to require manual approval before the AI executes commands that modify the system (e.g., `rm`, `npm install`, `git push`).
- [ ] **Robust Error Handling:** Replace `.unwrap()` calls in `src-tauri/src/lib.rs` and other backend modules with proper `Result` propagation and frontend error reporting.
- [ ] **Command Sandboxing:** Investigate running terminal commands within an isolated environment or a dedicated pseudo-terminal (PTY).

## 🏗️ Backend Architecture
- [ ] **Module Decomposition:** Split `src-tauri/src/ai_bridge.rs` and `src-tauri/src/tool_calling.rs` into smaller sub-modules (e.g., `parsers.rs`, `executor.rs`, `bridge_logic.rs`).
- [ ] **JSON Repair Testing:** Add comprehensive unit tests for the `repair_json` logic to ensure it handles various malformed outputs from different local LLM models.
- [ ] **LLM Fallback Logic:** Enhance `llm_loader.rs` to provide clearer diagnostics and automatic CPU fallback if GPU initialization fails.
- [ ] **Resource Optimization:** Refine the 50MB file-read cap and recursive search depth limits based on real-world performance benchmarks.

## ⚛️ Frontend Refactoring
- [ ] **Decompose App.tsx:** Break down the monolithic `App.tsx` into smaller, functional components.
- [ ] **State Management (Hooks):** Extract complex state logic (AI streaming, file tree management, tab state) into custom React hooks (e.g., `useAiStream`, `useFileSystem`).
- [ ] **Component Consolidation:** Refactor `AiChat.tsx` to separate the message rendering logic from the tool-execution and streaming orchestration.
- [ ] **Improved Type Safety:** Audit and strengthen TypeScript interfaces between the Tauri backend and the React frontend to prevent runtime errors.

## 🚀 Performance & UX
- [x] **Streaming UI Polish:** Improve the visual feedback during the `<think>` phase and tool execution blocks in the chat.
- [x] **File Explorer Optimization:** Implement virtualization for large directory structures to maintain UI responsiveness.
- [x] **Telemetry & Logging:** Add a "Developer Console" or detailed log view in the UI to help users debug local model loading and tool execution issues.
