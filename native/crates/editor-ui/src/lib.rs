// ── Syntax highlighting ───────────────────────────────────────────────
mod syntax {
    use egui::text::{LayoutJob, TextFormat};
    use egui::{Color32, FontId};
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    use std::ops::Range;

    // VSCode Dark+ inspired palette
    const COL_KEYWORD:     Color32 = Color32::from_rgb(86,  156, 214); // blue
    const COL_CONTROL:     Color32 = Color32::from_rgb(197, 134, 192); // purple
    const COL_TYPE:        Color32 = Color32::from_rgb(78,  201, 176); // teal
    const COL_STRING:      Color32 = Color32::from_rgb(206, 145, 120); // orange
    const COL_NUMBER:      Color32 = Color32::from_rgb(181, 206, 168); // light green
    const COL_COMMENT:     Color32 = Color32::from_rgb(106, 153, 85);  // green
    const COL_PREPROC:     Color32 = Color32::from_rgb(197, 134, 192); // purple

    #[derive(Clone)]
    pub struct Span {
        pub byte_range: Range<usize>,
        pub color: Color32,
    }

    pub struct Highlighter {
        parser: Option<tree_sitter::Parser>,
        cache_hash: u64,
        cache_spans: Vec<Span>,
    }

    impl std::fmt::Debug for Highlighter {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("Highlighter")
                .field("cache_spans", &self.cache_spans.len())
                .finish_non_exhaustive()
        }
    }

    impl Default for Highlighter {
        fn default() -> Self {
            let mut p = tree_sitter::Parser::new();
            let ok = p.set_language(&tree_sitter_cpp::language()).is_ok();
            Self {
                parser: if ok { Some(p) } else { None },
                cache_hash: 0,
                cache_spans: Vec::new(),
            }
        }
    }

    impl Highlighter {
        pub fn layout_job(
            &mut self,
            source: &str,
            font_id: FontId,
            default_color: Color32,
        ) -> LayoutJob {
            let hash = {
                let mut h = DefaultHasher::new();
                source.hash(&mut h);
                h.finish()
            };
            if hash != self.cache_hash || self.cache_spans.is_empty() && !source.is_empty() {
                self.cache_hash = hash;
                self.cache_spans.clear();
                if let Some(parser) = &mut self.parser {
                    if let Some(tree) = parser.parse(source.as_bytes(), None) {
                        collect_spans(tree.root_node(), source.as_bytes(), &mut self.cache_spans);
                    }
                }
            }
            build_job(source, &self.cache_spans, font_id, default_color)
        }
    }

    fn collect_spans(node: tree_sitter::Node<'_>, src: &[u8], out: &mut Vec<Span>) {
        // Non-leaf preprocessor nodes get colored as a whole block.
        if let Some(col) = preproc_color(node.kind()) {
            out.push(Span { byte_range: node.byte_range(), color: col });
            return;
        }
        if node.child_count() == 0 {
            if let Some(col) = leaf_color(node.kind(), src, node.byte_range()) {
                out.push(Span { byte_range: node.byte_range(), color: col });
            }
            return;
        }
        let mut cur = node.walk();
        for child in node.children(&mut cur) {
            collect_spans(child, src, out);
        }
    }

    fn preproc_color(kind: &str) -> Option<Color32> {
        match kind {
            "preproc_include" | "preproc_def" | "preproc_function_def" | "preproc_if"
            | "preproc_ifdef" | "preproc_else" | "preproc_endif" | "preproc_call"
            | "preproc_directive" => Some(COL_PREPROC),
            _ => None,
        }
    }

    fn leaf_color(kind: &str, src: &[u8], range: Range<usize>) -> Option<Color32> {
        match kind {
            // Comments
            "comment" => Some(COL_COMMENT),
            // Strings / chars
            "string_literal" | "char_literal" | "raw_string_literal"
            | "concatenated_string" => Some(COL_STRING),
            // Numbers
            "number_literal" => Some(COL_NUMBER),
            // Booleans / null
            "true" | "false" | "null" | "nullptr" | "NULL" | "this" => Some(COL_KEYWORD),
            // Control-flow keywords (purple)
            "if" | "else" | "for" | "while" | "do" | "switch" | "case"
            | "break" | "continue" | "return" | "goto" | "default"
            | "try" | "catch" | "throw" => Some(COL_CONTROL),
            // Declaration keywords (blue)
            "class" | "struct" | "enum" | "union" | "namespace" | "template"
            | "typename" | "typedef" | "using" | "public" | "private" | "protected"
            | "virtual" | "override" | "final" | "explicit" | "inline" | "static"
            | "extern" | "const" | "constexpr" | "consteval" | "constinit"
            | "volatile" | "mutable" | "friend" | "operator" | "new" | "delete"
            | "sizeof" | "alignof" | "decltype" | "noexcept" | "auto" => Some(COL_KEYWORD),
            // Primitive types
            "primitive_type" => Some(COL_KEYWORD),
            // Type identifiers (teal)
            "type_identifier" => Some(COL_TYPE),
            // For plain "identifier" leaf nodes: look up the text — known type names get teal.
            "identifier" => {
                if let Ok(text) = std::str::from_utf8(&src[range]) {
                    if is_type_like(text) { return Some(COL_TYPE); }
                }
                None
            }
            _ => None,
        }
    }

    fn is_type_like(s: &str) -> bool {
        // Heuristic: PascalCase or ends with _t typically denotes a type.
        if s.len() < 2 { return false; }
        let first = s.chars().next().unwrap();
        (first.is_uppercase() && s.chars().any(|c| c.is_lowercase()))
            || s.ends_with("_t")
            || s.ends_with("_type")
    }

    fn build_job(src: &str, spans: &[Span], font_id: FontId, default: Color32) -> LayoutJob {
        let mut job = LayoutJob::default();
        let bytes = src.as_bytes();

        if spans.is_empty() {
            job.append(src, 0.0, fmt(font_id, default));
            return job;
        }

        // Sort by start so we can walk left-to-right filling gaps.
        let mut sorted: Vec<&Span> = spans.iter().collect();
        sorted.sort_unstable_by_key(|s| s.byte_range.start);

        let mut cursor = 0usize;
        for span in sorted {
            let start = span.byte_range.start;
            let end   = span.byte_range.end;
            if end <= cursor { continue; }
            // Gap before this span
            if start > cursor {
                if let Ok(t) = std::str::from_utf8(&bytes[cursor..start]) {
                    job.append(t, 0.0, fmt(font_id.clone(), default));
                }
            }
            let actual_start = start.max(cursor);
            if let Ok(t) = std::str::from_utf8(&bytes[actual_start..end]) {
                job.append(t, 0.0, fmt(font_id.clone(), span.color));
            }
            cursor = end;
        }
        // Remaining text
        if cursor < bytes.len() {
            if let Ok(t) = std::str::from_utf8(&bytes[cursor..]) {
                job.append(t, 0.0, fmt(font_id, default));
            }
        }
        job
    }

    fn fmt(font_id: FontId, color: Color32) -> TextFormat {
        TextFormat { font_id, color, ..Default::default() }
    }
}

// ─────────────────────────────────────────────────────────────────────
use editor_ai::AiService;
use editor_assets::AssetDatabase;
use editor_build::BuildOrchestrator;
use editor_core::{ComponentDefinition, ComponentRecord, EditorState, EntityId, EntityRecord, PlayState};
use editor_hot_reload::HotReloadHost;
use editor_lsp_client::ClangdClient;
use editor_renderer::{GizmoMode, ViewportRenderer};
use egui::{Align, Color32, Layout, RichText, ScrollArea, TextEdit};
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditorCommand {
    None,
    Play,
    Pause,
    Stop,
    Build,
    SaveScene,
    SaveFiles,
    GenerateCompileCommands,
    GenerateReflection,
    ReloadProject,
    Undo,
    Redo,
    // Task 3: Gizmo
    SetGizmoTranslate,
    SetGizmoRotate,
    SetGizmoScale,
    ToggleSnap,
    // Task 8: Menu actions
    DeleteEntity,
    DuplicateEntity,
    FocusSelected,
    // AI chat: send the user's message to the AI service
    AiChatSend(String),
    // AI chat: clear conversation history
    AiChatClear,
    // Asset browser: trigger import of all on-disk assets into GPU
    ImportAssets,
    // Toggle live preview mode (LIVE badge + instant terrain regen on inspector edits)
    ToggleLivePreview,
    // Regenerate the selected/active terrain entity's GPU mesh from its Terrain component
    RegenerateTerrain,
}

#[derive(Debug, Default)]
pub struct EditorUi {
    /// Tree-sitter C++ syntax highlighter — kept here so the parse tree cache
    /// survives across frames without touching EditorState.
    syntax: std::cell::RefCell<syntax::Highlighter>,
}

impl EditorUi {
    pub fn configure_style(ctx: &egui::Context) {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = Color32::from_rgb(15, 20, 24);
        visuals.window_fill = Color32::from_rgb(18, 24, 29);
        visuals.extreme_bg_color = Color32::from_rgb(9, 13, 16);
        visuals.selection.bg_fill = Color32::from_rgb(43, 99, 123);
        visuals.selection.stroke.color = Color32::from_rgb(233, 170, 95);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(31, 43, 50);
        visuals.widgets.active.bg_fill = Color32::from_rgb(43, 99, 123);
        visuals.widgets.open.bg_fill = Color32::from_rgb(24, 34, 40);
        visuals.override_text_color = Some(Color32::from_rgb(236, 241, 244));
        ctx.set_visuals(visuals);

        let mut style = (*ctx.style()).clone();
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 8.0);
        style.spacing.window_margin = egui::Margin::same(12);
        ctx.set_style(style);
    }

    // ── Task 4: Keyboard shortcuts ─────────────────────────────────────
    fn process_shortcuts(ctx: &egui::Context) -> EditorCommand {
        let shortcuts: &[(egui::Modifiers, egui::Key, EditorCommand)] = &[
            (egui::Modifiers::CTRL, egui::Key::S, EditorCommand::SaveScene),
            (egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::S, EditorCommand::SaveFiles),
            (egui::Modifiers::CTRL, egui::Key::B, EditorCommand::Build),
            (egui::Modifiers::CTRL, egui::Key::Z, EditorCommand::Undo),
            (egui::Modifiers::CTRL | egui::Modifiers::SHIFT, egui::Key::Z, EditorCommand::Redo),
            (egui::Modifiers::CTRL, egui::Key::Y, EditorCommand::Redo),
            (egui::Modifiers::NONE, egui::Key::W, EditorCommand::SetGizmoTranslate),
            (egui::Modifiers::NONE, egui::Key::E, EditorCommand::SetGizmoRotate),
            (egui::Modifiers::NONE, egui::Key::R, EditorCommand::SetGizmoScale),
            (egui::Modifiers::NONE, egui::Key::Delete, EditorCommand::DeleteEntity),
            (egui::Modifiers::CTRL, egui::Key::D, EditorCommand::DuplicateEntity),
            (egui::Modifiers::NONE, egui::Key::F, EditorCommand::FocusSelected),
            (egui::Modifiers::NONE, egui::Key::F5, EditorCommand::Play),
            (egui::Modifiers::NONE, egui::Key::F6, EditorCommand::Pause),
            (egui::Modifiers::NONE, egui::Key::Escape, EditorCommand::Stop),
        ];

        for (mods, key, command) in shortcuts {
            let shortcut = egui::KeyboardShortcut::new(*mods, *key);
            if ctx.input_mut(|i| i.consume_shortcut(&shortcut)) {
                return command.clone();
            }
        }
        EditorCommand::None
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        ctx: &egui::Context,
        state: &mut EditorState,
        renderer: &mut ViewportRenderer,
        build: &BuildOrchestrator,
        hot_reload: &HotReloadHost,
        assets: &AssetDatabase,
        ai: &AiService,
        lsp: &ClangdClient,
    ) -> EditorCommand {
        // Process keyboard shortcuts first
        let shortcut_cmd = Self::process_shortcuts(ctx);

        let mut command = self.top_bar(ctx, state, build, hot_reload, ai, lsp, renderer);
        self.left_panel(ctx, state);
        self.right_panel(ctx, state, ai, &mut command);
        self.bottom_panel(ctx, state, assets, ai, &mut command);

        let viewport_command = self.viewport(ctx, state, renderer);
        if matches!(command, EditorCommand::None) {
            command = viewport_command;
        }
        if matches!(command, EditorCommand::None) {
            command = shortcut_cmd;
        }

        command
    }

    // ── Task 8: Menu system ────────────────────────────────────────────
    fn top_bar(
        &self,
        ctx: &egui::Context,
        state: &mut EditorState,
        build: &BuildOrchestrator,
        hot_reload: &HotReloadHost,
        ai: &AiService,
        lsp: &ClangdClient,
        _renderer: &ViewportRenderer,
    ) -> EditorCommand {
        let accent = Color32::from_rgb(233, 170, 95);
        let mut command = EditorCommand::None;

        egui::TopBottomPanel::top("shadow_editor_top_bar").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                // Real menus instead of dead buttons
                egui::menu::bar(ui, |ui| {
                    ui.menu_button("File", |ui| {
                        if ui.button("Save Scene  Ctrl+S").clicked() {
                            set_command_once(&mut command, EditorCommand::SaveScene);
                            ui.close();
                        }
                        if ui.button("Save All Files  Ctrl+Shift+S").clicked() {
                            set_command_once(&mut command, EditorCommand::SaveFiles);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Reload Project").clicked() {
                            set_command_once(&mut command, EditorCommand::ReloadProject);
                            ui.close();
                        }
                    });
                    ui.menu_button("Edit", |ui| {
                        if ui.button("Undo  Ctrl+Z").clicked() {
                            set_command_once(&mut command, EditorCommand::Undo);
                            ui.close();
                        }
                        if ui.button("Redo  Ctrl+Shift+Z").clicked() {
                            set_command_once(&mut command, EditorCommand::Redo);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Delete Entity  Del").clicked() {
                            set_command_once(&mut command, EditorCommand::DeleteEntity);
                            ui.close();
                        }
                        if ui.button("Duplicate  Ctrl+D").clicked() {
                            set_command_once(&mut command, EditorCommand::DuplicateEntity);
                            ui.close();
                        }
                    });
                    ui.menu_button("Build", |ui| {
                        if ui.button("Build  Ctrl+B").clicked() {
                            set_command_once(&mut command, EditorCommand::Build);
                            ui.close();
                        }
                        ui.separator();
                        if ui.button("Generate compile_commands.json").clicked() {
                            set_command_once(&mut command, EditorCommand::GenerateCompileCommands);
                            ui.close();
                        }
                        if ui.button("Generate Reflection").clicked() {
                            set_command_once(&mut command, EditorCommand::GenerateReflection);
                            ui.close();
                        }
                    });
                    ui.menu_button("Play", |ui| {
                        if ui.button("Play  F5").clicked() {
                            set_command_once(&mut command, EditorCommand::Play);
                            ui.close();
                        }
                        if ui.button("Pause  F6").clicked() {
                            set_command_once(&mut command, EditorCommand::Pause);
                            ui.close();
                        }
                        if ui.button("Stop  Esc").clicked() {
                            set_command_once(&mut command, EditorCommand::Stop);
                            ui.close();
                        }
                    });
                });
                ui.separator();

                if ui.button(RichText::new("Build").color(accent)).clicked() {
                    set_command_once(&mut command, EditorCommand::Build);
                }

                ui.separator();
                ui.label(
                    RichText::new(format!("{}  |  {}", state.project_name, state.runtime))
                        .strong()
                        .size(18.0),
                );
                ui.label(
                    RichText::new(state.project_root.display().to_string())
                        .small()
                        .color(Color32::from_rgb(142, 181, 193)),
                );
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} FPS  |  {:.1}ms GPU",
                            state.viewport.stats.fps, state.viewport.stats.gpu_time_ms
                        ))
                        .color(accent),
                    );
                });
            });

            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                self.status_chip(ui, "Build", &build.status_line());
                self.status_chip(ui, "Hot Reload", &hot_reload.status_line());
                self.status_chip(ui, "LSP", &lsp.status_line());
                self.status_chip(ui, "AI", &ai.routing_summary());
            });
        });

        command
    }

    fn left_panel(&self, ctx: &egui::Context, state: &mut EditorState) {
        let mut pending_selection = None;

        egui::SidePanel::left("shadow_editor_hierarchy")
            .default_width(230.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("Hierarchy");
                ui.label(format!("{} entities in scene", state.entities.len()));
                ui.separator();

                ScrollArea::vertical().show(ui, |ui| {
                    for entity in &state.entities {
                        let selected = state.selection == Some(entity.id);
                        let label = format!("{} {}", icon_for(entity), entity.name);
                        if ui.selectable_label(selected, label).clicked() {
                            pending_selection = Some(entity.id);
                        }
                    }
                });
            });

        if let Some(entity_id) = pending_selection {
            state.select_entity(entity_id);
        }
    }

    fn right_panel(
        &self,
        ctx: &egui::Context,
        state: &mut EditorState,
        ai: &AiService,
        command: &mut EditorCommand,
    ) {
        let selected_entity = state.selected_entity().cloned();
        let reflection = state.reflection.clone();
        let live_preview = state.viewport.live_preview;
        let mut pending_edits: Vec<(EntityId, String, String, String)> = Vec::new();

        egui::SidePanel::right("shadow_editor_inspector")
            .default_width(340.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.heading("Inspector");
                ui.separator();

                if let Some(entity) = selected_entity.as_ref() {
                    ui.label(RichText::new(&entity.name).strong().size(20.0));
                    ui.label(
                        RichText::new(format!("{}  |  scene id: {}", entity.kind, entity.scene_id))
                            .small()
                            .color(Color32::from_rgb(142, 181, 193)),
                    );
                    ui.add_space(8.0);

                    for component in &entity.components {
                        let definition = reflection
                            .iter()
                            .find(|candidate| candidate.name == component.type_name)
                            .cloned();
                        egui::CollapsingHeader::new(&component.type_name)
                            .default_open(true)
                            .show(ui, |ui| {
                                for field in ordered_fields(component, definition.as_ref()) {
                                    let mut value = field.value;
                                    ui.horizontal(|ui| {
                                        let label = ui.label(field.display_name);
                                        if !field.metadata.is_empty() {
                                            label.on_hover_text(field.metadata.clone());
                                        }
                                        let response = ui.add(
                                            TextEdit::singleline(&mut value)
                                                .desired_width(170.0)
                                                .font(egui::TextStyle::Monospace),
                                        );
                                        if response.changed() {
                                            pending_edits.push((
                                                entity.id,
                                                component.type_name.clone(),
                                                field.name,
                                                value,
                                            ));
                                        }
                                    });
                                }
                            });
                    }
                } else {
                    ui.label("No entity selected.");
                }

                ui.separator();
                ui.heading("AI Suggestions");
                for suggestion in ai.inspector_suggestions() {
                    ui.group(|ui| {
                        ui.label(RichText::new(&suggestion.title).strong());
                        ui.label(&suggestion.rationale);
                        ui.label(
                            RichText::new(&suggestion.action)
                                .small()
                                .color(Color32::from_rgb(233, 170, 95)),
                        );
                    });
                }
            });

        let terrain_touched = pending_edits.iter().any(|(_, comp, _, _)| comp == "Terrain");
        for (entity_id, component, field, value) in pending_edits {
            state.set_component_field(entity_id, &component, &field, value);
        }
        // If live preview is active and any Terrain field changed, schedule regen.
        if live_preview && terrain_touched {
            set_command_once(command, EditorCommand::RegenerateTerrain);
        }
    }

    fn viewport(
        &self,
        ctx: &egui::Context,
        state: &mut EditorState,
        renderer: &mut ViewportRenderer,
    ) -> EditorCommand {
        let accent = Color32::from_rgb(233, 170, 95);
        let mut command = EditorCommand::None;

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                // Gizmo mode buttons (Task 3)
                let gizmo = &renderer.gizmo;
                let is_translate = matches!(gizmo.mode, GizmoMode::Translate);
                let is_rotate = matches!(gizmo.mode, GizmoMode::Rotate);
                let is_scale = matches!(gizmo.mode, GizmoMode::Scale);

                if ui.selectable_label(is_translate, "W Translate").clicked() {
                    command = EditorCommand::SetGizmoTranslate;
                }
                if ui.selectable_label(is_rotate, "E Rotate").clicked() {
                    command = EditorCommand::SetGizmoRotate;
                }
                if ui.selectable_label(is_scale, "R Scale").clicked() {
                    command = EditorCommand::SetGizmoScale;
                }
                if ui.selectable_label(gizmo.snap_enabled, "Snap").clicked() {
                    command = EditorCommand::ToggleSnap;
                }
                ui.separator();

                let is_playing = matches!(state.viewport.play_state, PlayState::Playing);
                let is_paused = matches!(state.viewport.play_state, PlayState::Paused);
                let is_editing = matches!(state.viewport.play_state, PlayState::Edit);

                let play_btn = ui.add_enabled(
                    is_editing || is_paused,
                    egui::Button::new(
                        RichText::new("▶ Play").color(if is_playing {
                            accent
                        } else {
                            Color32::WHITE
                        }),
                    ),
                );
                if play_btn.clicked() {
                    command = EditorCommand::Play;
                }

                let pause_btn = ui.add_enabled(
                    is_playing,
                    egui::Button::new(
                        RichText::new("⏸ Pause").color(if is_paused {
                            accent
                        } else {
                            Color32::WHITE
                        }),
                    ),
                );
                if pause_btn.clicked() {
                    command = EditorCommand::Pause;
                }

                let stop_btn = ui.add_enabled(
                    is_playing || is_paused,
                    egui::Button::new("⏹ Stop"),
                );
                if stop_btn.clicked() {
                    command = EditorCommand::Stop;
                }

                ui.separator();
                if ui.button(RichText::new("Build").color(accent)).clicked() {
                    command = EditorCommand::Build;
                }
                ui.separator();
                let live_on = state.viewport.live_preview;
                let live_label = RichText::new("● Live")
                    .color(if live_on { Color32::from_rgb(235, 64, 64) } else { Color32::GRAY });
                if ui.selectable_label(live_on, live_label).clicked() {
                    command = EditorCommand::ToggleLivePreview;
                }
                if live_on {
                    if ui.button("⟳ Regen").on_hover_text("Regenerate terrain mesh now").clicked() {
                        command = EditorCommand::RegenerateTerrain;
                    }
                }

                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let play_state_label = match state.viewport.play_state {
                        PlayState::Edit => "Edit",
                        PlayState::Playing => "Playing",
                        PlayState::Paused => "Paused",
                    };
                    ui.label(
                        RichText::new(format!("{} | Scene: {}", play_state_label, state.scene_name))
                            .small(),
                    );
                });
            });
            ui.separator();

            let selection = state.selection;
            renderer.paint(ui, &state.viewport, &state.entities, selection);
        });

        command
    }

    fn bottom_panel(
        &self,
        ctx: &egui::Context,
        state: &mut EditorState,
        assets: &AssetDatabase,
        ai: &AiService,
        command: &mut EditorCommand,
    ) {
        let mut activate_buffer = None;

        egui::TopBottomPanel::bottom("shadow_editor_bottom_dock")
            .default_height(300.0)
            .resizable(true)
            .show(ctx, |ui| {
                ui.columns(4, |columns| {
                    columns[0].heading("Asset Browser");
                    columns[0].label(assets.semantic_index_state());
                    columns[0].label(format!("Cache root: {}", assets.cache_root.display()));
                    if columns[0].button("Import Assets").clicked() {
                        set_command_once(command, EditorCommand::ImportAssets);
                    }
                    columns[0].separator();
                    ScrollArea::vertical().show(&mut columns[0], |ui| {
                        for asset in &assets.items {
                            ui.group(|ui| {
                                ui.label(RichText::new(&asset.name).strong());
                                ui.label(format!(
                                    "{}  |  {}  |  {} bytes",
                                    asset.kind,
                                    asset.source.display(),
                                    asset.size_bytes
                                ));
                                ui.label(RichText::new(asset.tags.join(", ")).small());
                            });
                        }
                    });

                    columns[1].heading("Code Editor");
                    columns[1].horizontal_wrapped(|ui| {
                        if ui.button("Save Files").clicked() {
                            set_command_once(command, EditorCommand::SaveFiles);
                        }
                        if ui.button("Build").clicked() {
                            set_command_once(command, EditorCommand::Build);
                        }
                    });
                    columns[1].separator();
                    ScrollArea::horizontal().show(&mut columns[1], |ui| {
                        ui.horizontal_wrapped(|ui| {
                            for (index, buffer) in state.code_buffers.iter().enumerate() {
                                let label = if buffer.dirty {
                                    format!("• {}", buffer.file_name())
                                } else {
                                    buffer.file_name()
                                };
                                if ui
                                    .selectable_label(index == state.active_buffer, label)
                                    .clicked()
                                {
                                    activate_buffer = Some(index);
                                }
                            }
                        });
                    });
                    columns[1].separator();
                    if let Some(buffer) = state.active_buffer_mut() {
                        let lines = buffer.line_count();
                        let chars = buffer.char_count();
                        columns[1].label(
                            RichText::new(format!(
                                "{}  |  {}  |  {} lines  |  {} chars",
                                buffer.language,
                                buffer.path.display(),
                                lines,
                                chars,
                            ))
                            .small()
                            .color(Color32::from_rgb(142, 181, 193)),
                        );

                        // Syntax-highlighted layouter via tree-sitter C++ parser.
                        // RefCell gives us interior mutability inside the egui closure.
                        let syn = &self.syntax;
                        let font_id = egui::FontId::monospace(13.0);
                        let default_color = Color32::from_rgb(212, 212, 212);
                        let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, _wrap: f32| {
                            let job = syn.borrow_mut().layout_job(
                                buf.as_str(),
                                font_id.clone(),
                                default_color,
                            );
                            ui.fonts(|f| f.layout_job(job))
                        };

                        let response = columns[1].add(
                            TextEdit::multiline(&mut buffer.contents)
                                .code_editor()
                                .layouter(&mut layouter)
                                .desired_rows(16)
                                .lock_focus(true)
                                .desired_width(f32::INFINITY),
                        );
                        if response.changed() {
                            buffer.dirty = true;
                            buffer.invalidate_rope();
                        }
                    } else {
                        columns[1].label("No C++ source files found.");
                    }

                    columns[2].heading("Console");
                    columns[2].separator();
                    ScrollArea::vertical().show(&mut columns[2], |ui| {
                        for entry in &state.console {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(
                                    RichText::new(format!("[{}]", entry.channel))
                                        .monospace()
                                        .color(Color32::from_rgb(142, 181, 193)),
                                );
                                ui.label(&entry.message);
                            });
                        }
                    });

                    // ── AI Chat panel ─────────────────────────────────
                    columns[3].heading("AI Chat");
                    columns[3].label(
                        RichText::new(ai.routing_summary())
                            .small()
                            .color(Color32::from_rgb(142, 181, 193)),
                    );
                    columns[3].separator();

                    // Message history
                    let history_height = columns[3].available_height() - 72.0;
                    ScrollArea::vertical()
                        .id_salt("ai_chat_history")
                        .max_height(history_height)
                        .auto_shrink([false, false])
                        .stick_to_bottom(true)
                        .show(&mut columns[3], |ui| {
                            if state.ai_chat_messages.is_empty() {
                                ui.label(
                                    RichText::new(
                                        "Ask me anything about your C++23 game project.\n\
                                         Slash commands: /create /component /system /shader /debug /scene /prefab",
                                    )
                                    .small()
                                    .color(Color32::from_rgb(142, 181, 193)),
                                );
                            } else {
                                for entry in &state.ai_chat_messages {
                                    let (label_color, prefix) = match entry.role.as_str() {
                                        "user"      => (Color32::from_rgb(233, 170, 95),  "You"),
                                        "assistant" => (Color32::from_rgb(126, 212, 167), "AI"),
                                        _           => (Color32::from_rgb(248, 113, 113), "Err"),
                                    };
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(
                                            RichText::new(format!("{prefix}:"))
                                                .small()
                                                .strong()
                                                .color(label_color),
                                        );
                                        ui.label(RichText::new(&entry.text).small());
                                    });
                                    ui.add_space(4.0);
                                }
                            }
                            if state.ai_chat_loading {
                                ui.label(
                                    RichText::new("Thinking…")
                                        .small()
                                        .italics()
                                        .color(Color32::from_rgb(142, 181, 193)),
                                );
                            }
                        });

                    columns[3].separator();

                    // Input row
                    let mut send_clicked = false;
                    {
                        let ui = &mut columns[3];
                        ui.horizontal(|ui| {
                            let input_resp = ui.add(
                                TextEdit::singleline(&mut state.ai_chat_input)
                                    .hint_text("Ask about your game…")
                                    .desired_width(ui.available_width() - 56.0),
                            );
                            let enter_pressed =
                                input_resp.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            let btn_clicked = ui
                                .add_enabled(
                                    !state.ai_chat_loading && !state.ai_chat_input.trim().is_empty(),
                                    egui::Button::new("Send"),
                                )
                                .clicked();
                            if (enter_pressed || btn_clicked)
                                && !state.ai_chat_loading
                                && !state.ai_chat_input.trim().is_empty()
                            {
                                send_clicked = true;
                            }
                        });
                        if !state.ai_chat_loading {
                            ui.horizontal_wrapped(|ui| {
                                if ui.small_button("Clear").clicked() {
                                    set_command_once(command, EditorCommand::AiChatClear);
                                }
                            });
                        }
                    }

                    if send_clicked {
                        let msg = std::mem::take(&mut state.ai_chat_input);
                        set_command_once(command, EditorCommand::AiChatSend(msg));
                    }
                });
            });

        if let Some(index) = activate_buffer {
            state.set_active_buffer(index);
        }
    }

    fn status_chip(&self, ui: &mut egui::Ui, label: &str, value: &str) {
        ui.group(|ui| {
            ui.label(RichText::new(label).small().strong());
            ui.label(RichText::new(value).small());
        });
    }
}

#[derive(Debug, Clone)]
struct EditableField {
    name: String,
    display_name: String,
    value: String,
    metadata: String,
}

fn ordered_fields(
    component: &ComponentRecord,
    definition: Option<&ComponentDefinition>,
) -> Vec<EditableField> {
    let mut fields = Vec::new();
    let mut seen = HashSet::new();

    if let Some(definition) = definition {
        for property in &definition.properties {
            seen.insert(property.name.clone());
            fields.push(EditableField {
                name: property.name.clone(),
                display_name: property.display_name(),
                value: component.field_value(&property.name).unwrap_or("").to_string(),
                metadata: property.metadata.clone(),
            });
        }
    }

    for field in &component.fields {
        if seen.insert(field.name.clone()) {
            fields.push(EditableField {
                name: field.name.clone(),
                display_name: field.name.clone(),
                value: field.value.clone(),
                metadata: String::new(),
            });
        }
    }

    fields
}

fn set_command_once(target: &mut EditorCommand, next: EditorCommand) {
    if matches!(target, EditorCommand::None) {
        *target = next;
    }
}

fn icon_for(entity: &EntityRecord) -> &'static str {
    match entity.kind.as_str() {
        "Actor" => "▣",
        "Light" => "◉",
        "StaticMesh" => "▤",
        _ => "•",
    }
}
