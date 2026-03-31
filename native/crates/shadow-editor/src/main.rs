use editor_ai::AiService;
use editor_assets::AssetDatabase;
use editor_build::BuildOrchestrator;
use editor_core::{AiChatEntry, ConsoleEntry, EditorState, EntityRecord, PlayState};
use editor_hot_reload::HotReloadHost;
use editor_lsp_client::ClangdClient;
use editor_renderer::ViewportRenderer;
use editor_ui::{EditorCommand, EditorUi};
use eframe::egui;
use glam;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::env;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use uuid;

fn main() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ShadowEditor")
            .with_inner_size([1600.0, 960.0])
            .with_min_inner_size([1200.0, 720.0]),
        ..Default::default()
    };

    eframe::run_native(
        "ShadowEditor",
        native_options,
        Box::new(|creation_context| Ok(Box::new(ShadowEditorApp::new(creation_context)))),
    )
}

struct ShadowEditorApp {
    state: EditorState,
    ui: EditorUi,
    renderer: ViewportRenderer,
    build: BuildOrchestrator,
    hot_reload: HotReloadHost,
    assets: AssetDatabase,
    ai: AiService,
    lsp: ClangdClient,
    _watcher: Option<RecommendedWatcher>,
    watcher_rx: Option<mpsc::Receiver<notify::Result<notify::Event>>>,
    build_pending: bool,
    reload_pending: bool,
    last_change_at: Option<Instant>,
}

impl ShadowEditorApp {
    fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        EditorUi::configure_style(&creation_context.egui_ctx);

        let native_root = native_workspace_root();
        let (project_root, project_note) = resolve_project_root(&native_root);

        let build = BuildOrchestrator::from_project_root(&project_root)
            .unwrap_or_else(|_| BuildOrchestrator::example(&project_root));
        let _ = build.generate_compile_commands();
        let _ = build.generate_reflection();

        let assets = AssetDatabase::scan(&project_root);
        let ai = AiService::default();

        let mut lsp = ClangdClient::new(build.compile_commands_path());
        let _ = lsp.spawn();

        let hot_reload = HotReloadHost::new(build.runtime_library_path());
        let mut renderer = ViewportRenderer::default();
        renderer.init_gpu(creation_context);

        // Auto-import on-disk assets into the pending GPU upload queue.
        let startup_report = assets.import_all();
        for mesh in &startup_report.meshes {
            for prim in &mesh.primitives {
                renderer.queue_mesh_upload(
                    format!("{}_{}", mesh.source_name.rsplit('.').nth(1).unwrap_or(&mesh.source_name), prim.index),
                    prim.positions.clone(),
                    prim.normals.clone(),
                    prim.colors.clone(),
                    prim.indices.clone(),
                );
            }
        }
        let mut state = EditorState::load_project(
            &project_root,
            build.config.name.clone(),
            build.config.runtime.clone(),
            build.entry_scene_path(),
            build.reflection_output_path(),
        )
        .unwrap_or_else(|error| {
            let mut demo = EditorState::demo();
            demo.console.push(ConsoleEntry::new(
                "project",
                format!("Project load fallback: {error}"),
            ));
            demo
        });

        state.project_name = build.config.name.clone();
        state.runtime = build.config.runtime.clone();
        state.console.push(ConsoleEntry::new(
            "project",
            format!("Project root: {}", project_root.display()),
        ));
        if let Some(note) = project_note {
            state.console.push(ConsoleEntry::new("project", note));
        }
        state.console.push(ConsoleEntry::new(
            "build",
            format!(
                "{} {} | scene {}",
                build.config.build.compiler,
                build.config.build.standard,
                build.entry_scene_path().display()
            ),
        ));
        state.console.push(ConsoleEntry::new(
            "reflection",
            format!("Reflection JSON: {}", build.reflection_output_path().display()),
        ));
        state.console.push(ConsoleEntry::new("lsp", lsp.status_line()));

        let (tx, rx) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |result| {
            let _ = tx.send(result);
        })
        .ok();

        for watch_dir in [build.source_root(), project_root.join("scenes")] {
            if watch_dir.exists() {
                if let Some(instance) = watcher.as_mut() {
                    if instance.watch(&watch_dir, RecursiveMode::Recursive).is_ok() {
                        state.console.push(ConsoleEntry::new(
                            "watcher",
                            format!("Watching {} for live changes...", watch_dir.display()),
                        ));
                    }
                }
            }
        }

        Self {
            state,
            ui: EditorUi::default(),
            renderer,
            build,
            hot_reload,
            assets,
            ai,
            lsp,
            _watcher: watcher,
            watcher_rx: Some(rx),
            build_pending: false,
            reload_pending: false,
            last_change_at: None,
        }
    }

    fn push_console(&mut self, channel: &str, message: impl Into<String>) {
        self.state.console.push(ConsoleEntry::new(channel, message));
    }

    fn reload_project_from_disk(&mut self) {
        match self.state.refresh_from_disk() {
            Ok(()) => {
                self.assets = AssetDatabase::scan(&self.build.project_root);
                self.push_console("project", "Reloaded scene, reflection, and code buffers from disk.");
            }
            Err(error) => {
                self.push_console("project", format!("[ERROR] {error}"));
            }
        }
    }

    fn save_scene(&mut self) {
        match self.state.save_scene() {
            Ok(()) => self.push_console(
                "scene",
                format!("Saved {}", self.build.entry_scene_path().display()),
            ),
            Err(error) => self.push_console("scene", format!("[ERROR] {error}")),
        }
    }

    fn save_code_buffers(&mut self) -> usize {
        match self.state.save_dirty_buffers() {
            Ok(saved) => {
                if saved > 0 {
                    self.push_console("code", format!("Saved {saved} source files."));
                }
                saved
            }
            Err(error) => {
                self.push_console("code", format!("[ERROR] {error}"));
                0
            }
        }
    }

    fn hot_reload_snapshot_path(&self) -> PathBuf {
        let scene_path = self.build.entry_scene_path();
        let scene_stem = scene_path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("live_runtime_state");
        self.build
            .project_root
            .join(".shadoweditor")
            .join("runtime_state")
            .join(format!("{}_hot_reload.shadow", scene_stem))
    }

    fn run_build(&mut self) {
        self.save_code_buffers();
        self.save_scene();
        let preserve_live_state =
            matches!(self.state.viewport.play_state, PlayState::Playing | PlayState::Paused)
                && self.hot_reload.is_live();
        let restore_snapshot = if preserve_live_state {
            let snapshot_path = self.hot_reload_snapshot_path();
            if let Some(parent) = snapshot_path.parent() {
                if let Err(error) = std::fs::create_dir_all(parent) {
                    self.push_console("runtime", format!("[ERROR] {error}"));
                    None
                } else {
                    match self.hot_reload.save_scene(&snapshot_path) {
                        Ok(()) => {
                            self.push_console(
                                "runtime",
                                format!(
                                    "Preserved live runtime state before rebuild: {}",
                                    snapshot_path.display()
                                ),
                            );
                            Some(snapshot_path)
                        }
                        Err(error) => {
                            self.push_console("runtime", format!("[ERROR] {error}"));
                            None
                        }
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        match self.build.trigger_build() {
            Ok(output) => {
                let status = if output.success { "OK" } else { "FAILED" };
                let summary_line = output.output.trim().lines().last().unwrap_or("done");
                self.push_console(
                    "build",
                    format!("[{status}] {}ms — {summary_line}", output.duration_ms),
                );

                if let Err(error) = self.build.generate_compile_commands() {
                    self.push_console("lsp", format!("[ERROR] {error}"));
                }
                match self.build.generate_reflection() {
                    Ok(path) => self.push_console(
                        "reflection",
                        format!("Generated {}", path.display()),
                    ),
                    Err(error) => self.push_console("reflection", format!("[ERROR] {error}")),
                }

                self.reload_project_from_disk();

                if output.success
                    && matches!(self.state.viewport.play_state, PlayState::Playing | PlayState::Paused)
                {
                    match self.hot_reload.load_if_present() {
                        Ok(()) => {
                            let restored = if let Some(snapshot_path) =
                                restore_snapshot.as_ref().filter(|path| path.exists())
                            {
                                match self.hot_reload.load_scene(snapshot_path) {
                                    Ok(()) => {
                                        self.push_console(
                                            "runtime",
                                            format!(
                                                "Hot-reload swapped the C++23 runtime and restored live state from {}.",
                                                snapshot_path.display()
                                            ),
                                        );
                                        true
                                    }
                                    Err(error) => {
                                        self.push_console("runtime", format!("[ERROR] {error}"));
                                        false
                                    }
                                }
                            } else {
                                false
                            };

                            if !restored {
                                let _ = self.hot_reload.load_scene(self.build.entry_scene_path());
                                self.push_console(
                                    "runtime",
                                    "Hot-reload swapped the C++23 runtime and reloaded the scene.",
                                );
                            }
                        }
                        Err(error) => self.push_console("runtime", format!("[ERROR] {error}")),
                    }
                }
            }
            Err(error) => {
                self.push_console("build", format!("[ERROR] {error}"));
            }
        }
    }

    fn handle_command(&mut self, command: EditorCommand) {
        match command {
            EditorCommand::None => {}
            EditorCommand::Build => {
                self.push_console("build", "Manual build triggered.");
                self.run_build();
            }
            EditorCommand::SaveScene => self.save_scene(),
            EditorCommand::SaveFiles => {
                let _ = self.save_code_buffers();
            }
            EditorCommand::GenerateCompileCommands => match self.build.generate_compile_commands() {
                Ok(()) => self.push_console("lsp", "compile_commands.json regenerated."),
                Err(error) => self.push_console("lsp", format!("[ERROR] {error}")),
            },
            EditorCommand::GenerateReflection => match self.build.generate_reflection() {
                Ok(path) => {
                    self.push_console("reflection", format!("Generated {}", path.display()));
                    self.reload_project_from_disk();
                }
                Err(error) => self.push_console("reflection", format!("[ERROR] {error}")),
            },
            EditorCommand::ReloadProject => self.reload_project_from_disk(),
            EditorCommand::Undo => {
                if self.state.undo() {
                    self.push_console("edit", "Undo.");
                }
            }
            EditorCommand::Redo => {
                if self.state.redo() {
                    self.push_console("edit", "Redo.");
                }
            }
            EditorCommand::Play => {
                self.save_code_buffers();
                self.save_scene();
                if !self.build.runtime_library_path().exists() {
                    self.push_console("runtime", "Runtime library missing — building before Play.");
                    self.run_build();
                }
                match self.hot_reload.load_if_present() {
                    Ok(()) => {
                        if let Err(error) = self.hot_reload.load_scene(self.build.entry_scene_path()) {
                            self.push_console("runtime", format!("[ERROR] {error}"));
                        }
                        self.state.viewport.play_state = PlayState::Playing;
                        self.push_console("runtime", "Play-in-Editor started.");
                    }
                    Err(error) => self.push_console("runtime", format!("[ERROR] {error}")),
                }
            }
            EditorCommand::Pause => {
                self.state.viewport.play_state = PlayState::Paused;
                self.push_console("runtime", "Paused.");
            }
            EditorCommand::Stop => {
                let _ = self.hot_reload.save_scene(self.build.entry_scene_path());
                self.hot_reload.stop_session();
                self.state.viewport.play_state = PlayState::Edit;
                self.reload_project_from_disk();
                self.push_console("runtime", "Stopped — returning to edit mode.");
            }
            // Task 3: Gizmo mode switching
            EditorCommand::SetGizmoTranslate => {
                self.renderer.gizmo.mode = editor_renderer::GizmoMode::Translate;
            }
            EditorCommand::SetGizmoRotate => {
                self.renderer.gizmo.mode = editor_renderer::GizmoMode::Rotate;
            }
            EditorCommand::SetGizmoScale => {
                self.renderer.gizmo.mode = editor_renderer::GizmoMode::Scale;
            }
            EditorCommand::ToggleSnap => {
                self.renderer.gizmo.snap_enabled = !self.renderer.gizmo.snap_enabled;
            }
            // Task 8: Entity operations
            EditorCommand::DeleteEntity => {
                if let Some(sel) = self.state.selection {
                    if let Some(entity) = self.state.entities.iter().find(|e| e.id == sel).cloned() {
                        self.state.push_action(editor_core::EditorAction::RemoveEntity { entity });
                        self.state.entities.retain(|e| e.id != sel);
                        self.state.selection = self.state.entities.first().map(|e| e.id);
                        self.push_console("edit", "Deleted entity.");
                    }
                }
            }
            EditorCommand::DuplicateEntity => {
                if let Some(sel) = self.state.selection {
                    if let Some(entity) = self.state.entities.iter().find(|e| e.id == sel).cloned() {
                        let mut dup = entity.clone();
                        dup.id = uuid::Uuid::new_v4();
                        dup.name = format!("{} (copy)", dup.name);
                        dup.scene_id = format!("{}_copy", dup.scene_id);
                        self.state.push_action(editor_core::EditorAction::AddEntity { entity: dup.clone() });
                        self.state.entities.push(dup.clone());
                        self.state.selection = Some(dup.id);
                        self.push_console("edit", format!("Duplicated as '{}'.", dup.name));
                    }
                }
            }
            EditorCommand::AiChatClear => {
                self.ai.clear_history();
                self.state.ai_chat_messages.clear();
                self.push_console("ai", "AI chat history cleared.");
            }
            EditorCommand::AiChatSend(message) => {
                self.state.ai_chat_messages.push(AiChatEntry::user(&message));
                self.state.ai_chat_loading = true;

                let context = build_ai_context(&self.state);
                match self.ai.chat_with_context(&message, &context) {
                    Ok(reply) => {
                        self.state.ai_chat_messages.push(AiChatEntry::assistant(reply));
                    }
                    Err(error) => {
                        self.state.ai_chat_messages.push(AiChatEntry::error(error.to_string()));
                        self.push_console("ai", format!("[ERROR] {error}"));
                    }
                }
                self.state.ai_chat_loading = false;
            }
            EditorCommand::ImportAssets => {
                let report = self.assets.import_all();
                let mesh_count = report.meshes.len();
                let tex_count = report.textures.len();
                for mesh in &report.meshes {
                    for prim in &mesh.primitives {
                        self.renderer.queue_mesh_upload(
                            format!("{}_{}", mesh.source_name.rsplit('.').nth(1).unwrap_or(&mesh.source_name), prim.index),
                            prim.positions.clone(),
                            prim.normals.clone(),
                            prim.colors.clone(),
                            prim.indices.clone(),
                        );
                    }
                }
                for err in &report.errors {
                    self.push_console("assets", format!("[ERROR] {err}"));
                }
                self.push_console(
                    "assets",
                    format!("Imported {mesh_count} mesh(es), {tex_count} texture(s)."),
                );
            }
            EditorCommand::ToggleLivePreview => {
                self.state.viewport.live_preview = !self.state.viewport.live_preview;
                let status = if self.state.viewport.live_preview { "ON" } else { "OFF" };
                self.push_console("live", format!("Live Preview {status}"));
                // Immediately generate all terrain meshes when enabling.
                if self.state.viewport.live_preview {
                    self.regenerate_all_terrain();
                }
            }
            EditorCommand::RegenerateTerrain => {
                // Regen the selected entity if it has a Terrain component, otherwise regen all.
                let target = self.state.selected_entity()
                    .filter(|e| e.components.iter().any(|c| c.type_name == "Terrain"))
                    .cloned();
                if let Some(entity) = target {
                    self.regenerate_terrain_entity(&entity);
                } else {
                    self.regenerate_all_terrain();
                }
            }
            EditorCommand::FocusSelected => {
                // Focus camera on selected entity
                if let Some(sel) = self.state.selection {
                    if let Some(entity) = self.state.entities.iter().find(|e| e.id == sel) {
                        if let Some(pos) = entity.components.iter()
                            .find(|c| c.type_name == "Transform")
                            .and_then(|c| c.field_value("position"))
                        {
                            let parts: Vec<f32> = pos.trim().trim_start_matches('[').trim_end_matches(']')
                                .split(',').filter_map(|p| p.trim().parse().ok()).collect();
                            if parts.len() >= 3 {
                                self.renderer.camera.target = glam::Vec3::new(parts[0], parts[1], parts[2]);
                                self.renderer.camera.distance = 8.0;
                            }
                        }
                    }
                }
            }
        }
    }

    fn process_file_watcher(&mut self) {
        let mut pending_messages = Vec::new();
        if let Some(rx) = &self.watcher_rx {
            while let Ok(Ok(event)) = rx.try_recv() {
                let relevant = matches!(
                    event.kind,
                    EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
                );
                if !relevant {
                    continue;
                }

                let mut saw_source_change = false;
                let mut saw_scene_change = false;
                for path in &event.paths {
                    let ext = path.extension().and_then(|value| value.to_str()).unwrap_or("");
                    if matches!(ext, "cpp" | "cc" | "cxx" | "h" | "hpp" | "hh" | "hxx" | "ixx" | "inl")
                    {
                        saw_source_change = true;
                    }
                    if matches!(ext, "shadow" | "json" | "toml") {
                        saw_scene_change = true;
                    }
                }

                if saw_source_change {
                    self.build_pending = true;
                }
                if saw_scene_change {
                    self.reload_pending = true;
                }
                if saw_source_change || saw_scene_change {
                    self.last_change_at = Some(Instant::now());
                    pending_messages.push(
                        event
                            .paths
                            .first()
                            .and_then(|path| path.file_name())
                            .and_then(|name| name.to_str())
                            .unwrap_or("?")
                            .to_string(),
                    );
                }
            }
        }

        for file_name in pending_messages {
            self.push_console("watcher", format!("Changed: {file_name}"));
        }
    }

    fn process_debounced_actions(&mut self) {
        let elapsed = self
            .last_change_at
            .map(|time| time.elapsed())
            .unwrap_or(Duration::MAX);
        if elapsed < Duration::from_millis(400) {
            return;
        }

        if self.build_pending {
            self.build_pending = false;
            self.push_console("build", "Auto-rebuilding after source change...");
            self.run_build();
        } else if self.reload_pending {
            self.reload_pending = false;
            self.reload_project_from_disk();
        }
    }

    fn regenerate_terrain_entity(&mut self, entity: &EntityRecord) {
        let Some(terrain) = entity.components.iter().find(|c| c.type_name == "Terrain") else {
            return;
        };
        let resolution: u32 = terrain.field_value("resolution").and_then(|v| v.parse().ok()).unwrap_or(32);
        let scale: f32 = terrain.field_value("scale").and_then(|v| v.parse().ok()).unwrap_or(20.0);
        let height_scale: f32 = terrain.field_value("height_scale").and_then(|v| v.parse().ok()).unwrap_or(3.0);
        let frequency: f32 = terrain.field_value("frequency").and_then(|v| v.parse().ok()).unwrap_or(1.0);
        let (positions, normals, colors, indices) =
            editor_renderer::generate_terrain_mesh(resolution, resolution, scale, height_scale, frequency);
        self.renderer.queue_mesh_upload(
            format!("terrain_{}", entity.scene_id),
            positions,
            normals,
            colors,
            indices,
        );
        self.push_console(
            "live",
            format!(
                "Terrain '{}' regenerated — {}×{} verts, scale={}, h={}",
                entity.name, resolution, resolution, scale, height_scale
            ),
        );
    }

    fn regenerate_all_terrain(&mut self) {
        let terrain_entities: Vec<_> = self.state.entities
            .iter()
            .filter(|e| e.components.iter().any(|c| c.type_name == "Terrain"))
            .cloned()
            .collect();
        for entity in terrain_entities {
            self.regenerate_terrain_entity(&entity);
        }
    }

    fn tick_runtime(&mut self) {
        if matches!(self.state.viewport.play_state, PlayState::Playing) && self.hot_reload.is_live() {
            if let Err(error) = self.hot_reload.update(1.0 / 60.0) {
                self.push_console("runtime", format!("[ERROR] {error}"));
                self.state.viewport.play_state = PlayState::Edit;
                self.hot_reload.stop_session();
                return;
            }
            self.state.viewport.stats.visible_entities = self.hot_reload.entity_count().max(1);
            self.state.viewport.stats.draw_calls = 120 + self.hot_reload.entity_count() * 3;
            self.state.viewport.stats.gpu_time_ms =
                (self.state.viewport.stats.draw_calls as f32 / 180.0).max(0.8);
            self.state.viewport.stats.fps =
                (1000.0 / self.state.viewport.stats.gpu_time_ms).round() as u32;
        }
    }
}

/// Build a compact context string for the AI from the current editor state.
///
/// Includes: project name, selected entity + components, active code buffer
/// (truncated), and the last 10 console lines.
fn build_ai_context(state: &EditorState) -> String {
    let mut parts: Vec<String> = Vec::new();

    parts.push(format!(
        "Project: {} | Scene: {} | Runtime: {}",
        state.project_name, state.scene_name, state.runtime
    ));

    if let Some(entity) = state.selected_entity() {
        let comps: Vec<String> = entity
            .components
            .iter()
            .map(|c| {
                let fields: Vec<String> = c
                    .fields
                    .iter()
                    .map(|f| format!("{}={}", f.name, f.value))
                    .collect();
                if fields.is_empty() {
                    c.type_name.clone()
                } else {
                    format!("{}({})", c.type_name, fields.join(", "))
                }
            })
            .collect();
        parts.push(format!(
            "Selected entity: {} [{}]  components: {}",
            entity.name,
            entity.kind,
            comps.join(", ")
        ));
    }

    if let Some(buffer) = state.active_buffer() {
        let preview: String = buffer
            .contents
            .lines()
            .take(60)
            .collect::<Vec<_>>()
            .join("\n");
        parts.push(format!(
            "Active file: {}\n```cpp\n{}\n```",
            buffer.path.display(),
            preview
        ));
    }

    if !state.console.is_empty() {
        let recent: Vec<String> = state
            .console
            .iter()
            .rev()
            .take(10)
            .rev()
            .map(|e| format!("[{}] {}", e.channel, e.message))
            .collect();
        parts.push(format!("Recent console:\n{}", recent.join("\n")));
    }

    parts.join("\n\n")
}

fn native_workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."))
}

fn resolve_project_root(native_root: &Path) -> (PathBuf, Option<String>) {
    let fallback = native_root.join("templates/empty_3d");

    let mut args = env::args_os().skip(1);
    let mut requested = None;
    while let Some(arg) = args.next() {
        if arg == "--project" {
            requested = args.next().map(PathBuf::from);
            break;
        }
        if requested.is_none() {
            requested = Some(PathBuf::from(arg));
            break;
        }
    }

    let Some(requested) = requested else {
        return (fallback, None);
    };

    let requested = if requested.is_absolute() {
        requested
    } else {
        env::current_dir()
            .map(|dir| dir.join(&requested))
            .unwrap_or(requested)
    };

    let resolved = requested.canonicalize().unwrap_or(requested.clone());
    if resolved.join(".shadow_project.toml").exists() {
        return (
            resolved.clone(),
            Some(format!("Opened project from CLI: {}", resolved.display())),
        );
    }

    (
        fallback.clone(),
        Some(format!(
            "Requested project '{}' is missing .shadow_project.toml — falling back to {}",
            resolved.display(),
            fallback.display()
        )),
    )
}

impl eframe::App for ShadowEditorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_file_watcher();
        self.process_debounced_actions();
        self.tick_runtime();

        let command = self.ui.render(
            ctx,
            &mut self.state,
            &mut self.renderer,
            &self.build,
            &self.hot_reload,
            &self.assets,
            &self.ai,
            &self.lsp,
        );
        self.handle_command(command);

        ctx.request_repaint_after(Duration::from_millis(16));
    }
}
