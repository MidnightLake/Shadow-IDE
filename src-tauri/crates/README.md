# FerrumChat Crates

Internal Rust workspace crates powering the AI chat features of ShadowIDE.

## Crates

### `ferrum-core`
Shared types and configuration.

- **`types`** — Core data types: `Message`, `ToolCall`, `Usage`, `FinishReason`, `ConnectionStatus`, `TokenBarState`/`TokenLevel`
- **`config`** — TOML-based configuration with profiles (`Config`, `Profile`, `Defaults`). Default config path: `~/.config/ferrum-chat/config.toml`
- **`error`** — `FerrumError` enum (Config, Io, Db, Api, Parse) with `From` impls for `io::Error`, `toml::de::Error`, `serde_json::Error`

### `ferrum-llm`
LLM client and caching layer.

- **`client`** — `LlmClient` for OpenAI-compatible APIs. Constructed from a `Profile`. Methods: `check_connection()`, `list_models()`
- **`stream`** — SSE streaming types: `Chunk` enum (Token, Think, ToolCall, Done), plus OpenAI SSE wire types
- **`cache`** — `ExactCache` with SHA-256 prompt hashing, TTL expiration, and LRU eviction. Thread-safe (`Mutex<HashMap>`)

### `ferrum-sessions`
Conversation persistence and compaction.

- **`store`** — `SessionStore` backed by SQLite (WAL mode). Full CRUD for sessions and messages, plus token counting, markdown export, and pin/sort support. DB path: `~/.local/share/ferrum-chat/ferrum.db`
- **`compact`** — Conversation compaction: `should_compact()` threshold check, `compaction_prompt()` for LLM-driven summarization (preserves code blocks), `compact_messages()` to replace history with a summary

## Architecture

```
ferrum-core (types, config, errors)
    |
    +-- ferrum-llm (API client, streaming, cache)
    +-- ferrum-sessions (SQLite persistence, compaction)
```

Both `ferrum-llm` and `ferrum-sessions` depend on `ferrum-core`. They do not depend on each other.

## Testing

```sh
cargo test -p ferrum-core -p ferrum-llm -p ferrum-sessions
```

Session store tests use an in-memory SQLite database (`SessionStore::open_in_memory()`).
