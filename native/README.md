# ShadowIDE Native Foundation

This workspace is the in-repo start of the `planengine.md` architecture, integrated into `shadow-ide` without removing the existing Tauri/React app.

What lives here:

- `crates/shadow-editor`: native desktop shell for the planned editor
- `crates/editor-*`: planned service crates for core/editor subsystems
- `runtime-sdk/`: C++23 ABI boundary and starter runtime target
- `templates/`: starter project layout matching the planned `.shadow_project.toml` flow

Run it from the `shadow-ide` root:

```sh
npm run dev:native
```

Or directly:

```sh
cargo run --manifest-path native/Cargo.toml -p shadow-editor
```

The existing `npm run dev` Tauri app remains available and unchanged.
