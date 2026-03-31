use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, hash_map::DefaultHasher};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use toml::Value as TomlValue;
use uuid::Uuid;

pub type EntityId = Uuid;

// ── AI Chat ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiChatEntry {
    /// "user", "assistant", or "error"
    pub role: String,
    pub text: String,
}

impl AiChatEntry {
    pub fn user(text: impl Into<String>) -> Self {
        Self { role: "user".into(), text: text.into() }
    }
    pub fn assistant(text: impl Into<String>) -> Self {
        Self { role: "assistant".into(), text: text.into() }
    }
    pub fn error(text: impl Into<String>) -> Self {
        Self { role: "error".into(), text: text.into() }
    }
}

// ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EditorAction {
    SelectEntity { previous: Option<EntityId>, next: Option<EntityId> },
    SetComponentField {
        entity: EntityId,
        component: String,
        field: String,
        old_value: String,
        new_value: String,
    },
    AddEntity { entity: EntityRecord },
    RemoveEntity { entity: EntityRecord },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditorState {
    pub project_name: String,
    pub project_root: PathBuf,
    pub scene_name: String,
    pub runtime: String,
    pub entry_scene_path: PathBuf,
    pub reflection_path: PathBuf,
    pub entities: Vec<EntityRecord>,
    pub selection: Option<EntityId>,
    pub assets: Vec<AssetSummary>,
    pub console: Vec<ConsoleEntry>,
    pub viewport: ViewportState,
    pub reflection: Vec<ComponentDefinition>,
    pub code_buffers: Vec<CodeBuffer>,
    pub active_buffer: usize,
    #[serde(skip)]
    pub undo_stack: Vec<EditorAction>,
    #[serde(skip)]
    pub redo_stack: Vec<EditorAction>,
    // AI Chat panel state (not serialized — session-local)
    #[serde(skip)]
    pub ai_chat_messages: Vec<AiChatEntry>,
    #[serde(skip)]
    pub ai_chat_input: String,
    #[serde(skip)]
    pub ai_chat_loading: bool,
}

impl Default for EditorState {
    fn default() -> Self {
        Self::demo()
    }
}

impl EditorState {
    pub fn demo() -> Self {
        let player_id = Uuid::new_v4();
        let sun_id = Uuid::new_v4();
        let ground_id = Uuid::new_v4();
        let terrain_id = Uuid::new_v4();

        let entities = vec![
            EntityRecord {
                id: player_id,
                scene_id: "player".into(),
                name: "Player".into(),
                kind: "Actor".into(),
                children: Vec::new(),
                components: vec![
                    ComponentRecord::new(
                        "Transform",
                        [("position", "[0.0, 1.0, 0.0]"), ("scale", "[1.0, 1.0, 1.0]")],
                    ),
                    ComponentRecord::new(
                        "PlayerController",
                        [("speed", "5.0"), ("jump_force", "8.0")],
                    ),
                    ComponentRecord::new("Health", [("current", "100.0"), ("max", "100.0")]),
                ],
            },
            EntityRecord {
                id: sun_id,
                scene_id: "sun".into(),
                name: "DirectionalLight".into(),
                kind: "Light".into(),
                children: Vec::new(),
                components: vec![
                    ComponentRecord::new("Transform", [("rotation", "[-0.52, 0.18, 0.0]")]),
                    ComponentRecord::new("Light", [("lux", "110000"), ("temperature", "5600K")]),
                ],
            },
            EntityRecord {
                id: ground_id,
                scene_id: "ground".into(),
                name: "Ground".into(),
                kind: "StaticMesh".into(),
                children: Vec::new(),
                components: vec![
                    ComponentRecord::new("Transform", [("position", "[0.0, 0.0, 0.0]")]),
                    ComponentRecord::new(
                        "MeshRenderer",
                        [
                            ("mesh", "assets/ground.glb#Mesh0"),
                            ("material", "materials/terrain.shadow_mat"),
                        ],
                    ),
                ],
            },
            EntityRecord {
                id: terrain_id,
                scene_id: "terrain_main".into(),
                name: "ProceduralTerrain".into(),
                kind: "Terrain".into(),
                children: Vec::new(),
                components: vec![
                    ComponentRecord::new("Transform", [("position", "[0.0, 0.0, 0.0]")]),
                    ComponentRecord::new(
                        "Terrain",
                        [
                            ("resolution", "32"),
                            ("scale", "20.0"),
                            ("height_scale", "3.0"),
                            ("frequency", "1.0"),
                        ],
                    ),
                ],
            },
        ];

        Self {
            project_name: "Empty3D".into(),
            project_root: PathBuf::new(),
            scene_name: "MainLevel".into(),
            runtime: "cpp23".into(),
            entry_scene_path: PathBuf::new(),
            reflection_path: PathBuf::new(),
            selection: Some(player_id),
            entities,
            assets: vec![
                AssetSummary::new("player.glb", "Mesh", "assets/player.glb"),
                AssetSummary::new("ground.glb", "Mesh", "assets/ground.glb"),
                AssetSummary::new("sunny_sky.ktx2", "Texture", "assets/sky/sunny_sky.ktx2"),
            ],
            console: vec![
                ConsoleEntry::new("build", "Native editor foundation loaded inside shadow-ide."),
                ConsoleEntry::new("runtime", "Waiting for C++23 game runtime build output."),
                ConsoleEntry::new("lsp", "clangd sidecar initialized with compile_commands.json."),
            ],
            viewport: ViewportState::default(),
            reflection: vec![
                ComponentDefinition::demo(
                    "Transform",
                    [
                        ("position", "float[3]", "display_name=Position, step=0.1"),
                        ("scale", "float[3]", "display_name=Scale, min=0.001, step=0.01"),
                    ],
                ),
                ComponentDefinition::demo(
                    "PlayerController",
                    [
                        ("speed", "float", "display_name=Move Speed, min=0, max=50"),
                        ("jump_force", "float", "display_name=Jump Force"),
                    ],
                ),
            ],
            code_buffers: vec![
                CodeBuffer::demo(
                    "src/components.h",
                    "#pragma once\n\n#include \"shadow/shadow_reflect.h\"\n\nSHADOW_COMPONENT()\nstruct Health {\n    SHADOW_PROPERTY(float, \"display_name=Current HP, min=0\")\n    float current = 100.0f;\n};\n",
                ),
                CodeBuffer::demo(
                    "src/terrain.h",
                    "#pragma once\n\n#include \"shadow/shadow_reflect.h\"\n\n/// Procedural terrain component.\n/// Adjust these values and press Live Preview to see changes in the viewport.\nSHADOW_COMPONENT()\nstruct Terrain {\n    SHADOW_PROPERTY(int, \"display_name=Resolution, min=4, max=256\")\n    int resolution = 32;\n\n    SHADOW_PROPERTY(float, \"display_name=World Scale, min=1, max=500\")\n    float scale = 20.0f;\n\n    SHADOW_PROPERTY(float, \"display_name=Height Scale, min=0, max=50\")\n    float height_scale = 3.0f;\n\n    SHADOW_PROPERTY(float, \"display_name=Noise Frequency, min=0.1, max=10\")\n    float frequency = 1.0f;\n};\n",
                ),
                CodeBuffer::demo(
                    "src/game.cpp",
                    "#include \"shadow/game_api.h\"\n\nextern \"C\" void shadow_update(float dt) {\n    (void)dt;\n}\n",
                ),
            ],
            active_buffer: 0,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            ai_chat_messages: Vec::new(),
            ai_chat_input: String::new(),
            ai_chat_loading: false,
        }
    }

    pub fn load_project(
        project_root: impl AsRef<Path>,
        project_name: impl Into<String>,
        runtime: impl Into<String>,
        entry_scene_path: impl AsRef<Path>,
        reflection_path: impl AsRef<Path>,
    ) -> Result<Self> {
        let project_root = project_root.as_ref().to_path_buf();
        let entry_scene_path = entry_scene_path.as_ref().to_path_buf();
        let reflection_path = reflection_path.as_ref().to_path_buf();
        let (scene_name, entities) = load_scene_entities(&entry_scene_path)?;
        let reflection = load_reflection(&reflection_path).unwrap_or_default();
        let code_buffers = load_code_buffers(&project_root.join("src"))?;

        let selection = entities.first().map(|entity| entity.id);
        Ok(Self {
            project_name: project_name.into(),
            project_root,
            scene_name,
            runtime: runtime.into(),
            entry_scene_path,
            reflection_path,
            entities,
            selection,
            assets: Vec::new(),
            console: Vec::new(),
            viewport: ViewportState::default(),
            reflection,
            active_buffer: 0,
            code_buffers,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            ai_chat_messages: Vec::new(),
            ai_chat_input: String::new(),
            ai_chat_loading: false,
        })
    }

    pub fn refresh_from_disk(&mut self) -> Result<()> {
        let previous_selection = self.selection;
        let (scene_name, entities) = load_scene_entities(&self.entry_scene_path)?;
        self.scene_name = scene_name;
        self.entities = entities;
        self.reflection = load_reflection(&self.reflection_path).unwrap_or_default();
        self.code_buffers = load_code_buffers(&self.project_root.join("src"))?;
        if self.active_buffer >= self.code_buffers.len() {
            self.active_buffer = 0;
        }
        self.selection = previous_selection
            .and_then(|selected| self.entities.iter().find(|entity| entity.id == selected).map(|entity| entity.id))
            .or_else(|| self.entities.first().map(|entity| entity.id));
        Ok(())
    }

    pub fn save_scene(&self) -> Result<()> {
        if let Some(parent) = self.entry_scene_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.entry_scene_path, self.render_scene_file())
            .with_context(|| format!("failed to write {}", self.entry_scene_path.display()))
    }

    pub fn save_dirty_buffers(&mut self) -> Result<usize> {
        let mut saved = 0usize;
        for buffer in &mut self.code_buffers {
            if !buffer.dirty {
                continue;
            }
            if let Some(parent) = buffer.path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&buffer.path, &buffer.contents)
                .with_context(|| format!("failed to write {}", buffer.path.display()))?;
            buffer.dirty = false;
            saved += 1;
        }
        Ok(saved)
    }

    pub fn select_entity(&mut self, entity_id: EntityId) {
        let previous = self.selection;
        let action = EditorAction::SelectEntity {
            previous,
            next: Some(entity_id),
        };
        self.push_action(action);
        self.selection = Some(entity_id);
    }

    pub fn set_component_field(
        &mut self,
        entity_id: EntityId,
        component: &str,
        field: &str,
        new_value: impl Into<String>,
    ) -> bool {
        let new_value = new_value.into();
        let Some((component_record, field_record)) = self
            .entities
            .iter()
            .find(|entity| entity.id == entity_id)
            .and_then(|entity| {
                entity
                    .components
                    .iter()
                    .find(|record| record.type_name == component)
                    .and_then(|component_record| {
                        component_record
                            .fields
                            .iter()
                            .find(|field_record| field_record.name == field)
                            .map(|field_record| (component_record, field_record))
                    })
            })
        else {
            return false;
        };

        if field_record.value == new_value {
            return false;
        }

        let action = EditorAction::SetComponentField {
            entity: entity_id,
            component: component_record.type_name.clone(),
            field: field_record.name.clone(),
            old_value: field_record.value.clone(),
            new_value: new_value.clone(),
        };

        self.set_component_field_raw(&entity_id, component, field, &new_value);
        self.push_action(action);
        true
    }

    pub fn set_active_buffer(&mut self, index: usize) {
        if index < self.code_buffers.len() {
            self.active_buffer = index;
        }
    }

    pub fn active_buffer(&self) -> Option<&CodeBuffer> {
        self.code_buffers.get(self.active_buffer)
    }

    pub fn active_buffer_mut(&mut self) -> Option<&mut CodeBuffer> {
        self.code_buffers.get_mut(self.active_buffer)
    }

    pub fn reflection_for_component(&self, name: &str) -> Option<&ComponentDefinition> {
        self.reflection.iter().find(|component| component.name == name)
    }

    pub fn selected_entity(&self) -> Option<&EntityRecord> {
        let selected = self.selection?;
        self.entities.iter().find(|entity| entity.id == selected)
    }

    pub fn push_action(&mut self, action: EditorAction) {
        self.undo_stack.push(action);
        self.redo_stack.clear();
    }

    pub fn undo(&mut self) -> bool {
        let Some(action) = self.undo_stack.pop() else {
            return false;
        };
        match &action {
            EditorAction::SelectEntity { previous, .. } => {
                self.selection = *previous;
            }
            EditorAction::SetComponentField {
                entity,
                component,
                field,
                old_value,
                ..
            } => {
                self.set_component_field_raw(entity, component, field, old_value);
            }
            EditorAction::AddEntity { entity } => {
                self.entities.retain(|candidate| candidate.id != entity.id);
            }
            EditorAction::RemoveEntity { entity } => {
                self.entities.push(entity.clone());
            }
        }
        self.redo_stack.push(action);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(action) = self.redo_stack.pop() else {
            return false;
        };
        match &action {
            EditorAction::SelectEntity { next, .. } => {
                self.selection = *next;
            }
            EditorAction::SetComponentField {
                entity,
                component,
                field,
                new_value,
                ..
            } => {
                self.set_component_field_raw(entity, component, field, new_value);
            }
            EditorAction::AddEntity { entity } => {
                self.entities.push(entity.clone());
            }
            EditorAction::RemoveEntity { entity } => {
                self.entities.retain(|candidate| candidate.id != entity.id);
            }
        }
        self.undo_stack.push(action);
        true
    }

    fn set_component_field_raw(
        &mut self,
        entity_id: &EntityId,
        component: &str,
        field: &str,
        value: &str,
    ) {
        if let Some(entity) = self.entities.iter_mut().find(|entity| &entity.id == entity_id) {
            if let Some(component_record) = entity
                .components
                .iter_mut()
                .find(|record| record.type_name == component)
            {
                if let Some(field_record) = component_record
                    .fields
                    .iter_mut()
                    .find(|record| record.name == field)
                {
                    field_record.value = value.to_string();
                } else {
                    component_record.fields.push(ComponentField {
                        name: field.to_string(),
                        value: value.to_string(),
                    });
                }
            }
        }
    }

    fn render_scene_file(&self) -> String {
        let mut out = String::new();
        out.push_str("[scene]\n");
        out.push_str(&format!("name = {}\n", quoted(&self.scene_name)));
        out.push_str("version = \"1.0\"\n");
        out.push_str(&format!("runtime = {}\n", quoted(&self.runtime)));

        for entity in &self.entities {
            let rendered_scene_id = if entity.scene_id.is_empty() {
                entity.id.to_string()
            } else {
                entity.scene_id.clone()
            };
            out.push_str("\n[[entity]]\n");
            out.push_str(&format!("id = {}\n", quoted(&rendered_scene_id)));
            out.push_str(&format!("name = {}\n", quoted(&entity.name)));
            if !entity.children.is_empty() {
                out.push_str("children = [");
                for (index, child) in entity.children.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&quoted(child));
                }
                out.push_str("]\n");
            }

            for component in &entity.components {
                out.push_str("\n  [[entity.component]]\n");
                out.push_str(&format!("  type = {}\n", quoted(&component.type_name)));
                for field in &component.fields {
                    out.push_str(&format!(
                        "  {} = {}\n",
                        field.name,
                        render_field_value(&field.value)
                    ));
                }
            }
        }

        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityRecord {
    pub id: EntityId,
    pub scene_id: String,
    pub name: String,
    pub kind: String,
    pub children: Vec<String>,
    pub components: Vec<ComponentRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentRecord {
    pub type_name: String,
    pub fields: Vec<ComponentField>,
}

impl ComponentRecord {
    pub fn new<const N: usize>(type_name: impl Into<String>, fields: [(&str, &str); N]) -> Self {
        Self {
            type_name: type_name.into(),
            fields: fields
                .into_iter()
                .map(|(name, value)| ComponentField {
                    name: name.into(),
                    value: value.into(),
                })
                .collect(),
        }
    }

    pub fn field_value(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|field| field.name == name)
            .map(|field| field.value.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentField {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentDefinition {
    pub name: String,
    pub properties: Vec<PropertyDefinition>,
}

impl ComponentDefinition {
    pub fn demo<const N: usize>(
        name: impl Into<String>,
        properties: [(&str, &str, &str); N],
    ) -> Self {
        Self {
            name: name.into(),
            properties: properties
                .into_iter()
                .map(|(property, ty, metadata)| PropertyDefinition::new(property, ty, metadata))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PropertyDefinition {
    pub name: String,
    pub ty: String,
    pub metadata: String,
    pub attributes: BTreeMap<String, String>,
}

impl PropertyDefinition {
    pub fn new(
        name: impl Into<String>,
        ty: impl Into<String>,
        metadata: impl Into<String>,
    ) -> Self {
        let metadata = metadata.into();
        Self {
            name: name.into(),
            ty: ty.into(),
            attributes: parse_metadata_map(&metadata),
            metadata,
        }
    }

    pub fn display_name(&self) -> String {
        self.attributes
            .get("display_name")
            .cloned()
            .unwrap_or_else(|| self.name.clone())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CodeBuffer {
    pub path: PathBuf,
    pub language: String,
    pub contents: String,
    pub dirty: bool,
    /// Rope backing for efficient O(log n) line/byte queries.
    /// Rebuilt lazily from `contents` on first access after a write.
    #[serde(skip)]
    rope: Option<ropey::Rope>,
}

impl CodeBuffer {
    pub fn demo(path: impl Into<PathBuf>, contents: impl Into<String>) -> Self {
        let path = path.into();
        let contents = contents.into();
        let rope = Some(ropey::Rope::from_str(&contents));
        Self {
            language: detect_language(&path),
            path,
            contents,
            dirty: false,
            rope,
        }
    }

    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("untitled")
            .to_string()
    }

    /// Number of lines in the buffer (O(log n) via rope).
    pub fn line_count(&mut self) -> usize {
        self.ensure_rope().len_lines().max(1)
    }

    /// Number of Unicode scalar values (O(1) via rope).
    pub fn char_count(&mut self) -> usize {
        self.ensure_rope().len_chars()
    }

    /// Returns all line numbers (0-based) that contain `query` (case-insensitive).
    pub fn search_lines(&self, query: &str) -> Vec<usize> {
        let lower = query.to_lowercase();
        self.contents
            .lines()
            .enumerate()
            .filter_map(|(i, line)| {
                if line.to_lowercase().contains(&lower) { Some(i) } else { None }
            })
            .collect()
    }

    /// Call after mutating `contents` so the rope cache is rebuilt on next use.
    pub fn invalidate_rope(&mut self) {
        self.rope = None;
    }

    fn ensure_rope(&mut self) -> &ropey::Rope {
        if self.rope.is_none() {
            self.rope = Some(ropey::Rope::from_str(&self.contents));
        }
        self.rope.as_ref().unwrap()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetSummary {
    pub name: String,
    pub kind: String,
    pub source: String,
}

impl AssetSummary {
    pub fn new(
        name: impl Into<String>,
        kind: impl Into<String>,
        source: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            kind: kind.into(),
            source: source.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsoleEntry {
    pub channel: String,
    pub message: String,
}

impl ConsoleEntry {
    pub fn new(channel: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            channel: channel.into(),
            message: message.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewportState {
    pub mode: ViewMode,
    pub play_state: PlayState,
    pub stats: ViewportStats,
    pub camera_yaw: f32,
    pub camera_pitch: f32,
    pub camera_distance: f32,
    pub camera_target: [f32; 3],
    /// When true the viewport shows a "LIVE" badge and terrain regenerates
    /// immediately on each inspector field change.
    #[serde(skip)]
    pub live_preview: bool,
}

impl Default for ViewportState {
    fn default() -> Self {
        Self {
            mode: ViewMode::Perspective3D,
            play_state: PlayState::Edit,
            stats: ViewportStats {
                fps: 144,
                draw_calls: 287,
                visible_entities: 64,
                gpu_time_ms: 1.8,
            },
            camera_yaw: std::f32::consts::FRAC_PI_4,
            camera_pitch: 0.4,
            camera_distance: 12.0,
            camera_target: [0.0, 0.0, 0.0],
            live_preview: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum ViewMode {
    Perspective3D,
    Orthographic2D,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlayState {
    Edit,
    Playing,
    Paused,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewportStats {
    pub fps: u32,
    pub draw_calls: u32,
    pub visible_entities: u32,
    pub gpu_time_ms: f32,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct ReflectionFile {
    #[serde(default)]
    components: Vec<ReflectionComponentFile>,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct ReflectionComponentFile {
    name: String,
    #[serde(default)]
    properties: Vec<ReflectionPropertyFile>,
}

#[derive(Debug, Default, Clone, Deserialize)]
struct ReflectionPropertyFile {
    name: String,
    ty: String,
    #[serde(default)]
    metadata: String,
}

fn load_reflection(path: &Path) -> Result<Vec<ComponentDefinition>> {
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read reflection doc {}", path.display()))?;
    let reflection_file: ReflectionFile = serde_json::from_str(&content)
        .with_context(|| format!("failed to parse reflection doc {}", path.display()))?;

    Ok(reflection_file
        .components
        .into_iter()
        .map(|component| ComponentDefinition {
            name: component.name,
            properties: component
                .properties
                .into_iter()
                .map(|property| {
                    PropertyDefinition::new(
                        normalize_reflection_field_name(&property.name),
                        property.ty,
                        property.metadata,
                    )
                })
                .collect(),
        })
        .collect())
}

fn normalize_reflection_field_name(name: &str) -> String {
    name.split('[').next().unwrap_or(name).trim().to_string()
}

fn load_code_buffers(source_root: &Path) -> Result<Vec<CodeBuffer>> {
    if !source_root.exists() {
        return Ok(Vec::new());
    }

    let mut paths = Vec::new();
    collect_code_files(source_root, &mut paths)?;
    paths.sort();

    let mut buffers = Vec::with_capacity(paths.len());
    for path in paths {
        let contents = fs::read_to_string(&path)
            .with_context(|| format!("failed to read code file {}", path.display()))?;
        let rope = Some(ropey::Rope::from_str(&contents));
        buffers.push(CodeBuffer {
            language: detect_language(&path),
            path,
            contents,
            dirty: false,
            rope,
        });
    }
    Ok(buffers)
}

fn collect_code_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("failed to read {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("");
            if !name.starts_with('.') && name != "build" {
                collect_code_files(&path, out)?;
            }
            continue;
        }
        let ext = path.extension().and_then(|value| value.to_str()).unwrap_or("");
        if matches!(ext, "cpp" | "cc" | "cxx" | "h" | "hpp" | "hh" | "hxx" | "ixx" | "inl") {
            out.push(path);
        }
    }
    Ok(())
}

fn detect_language(path: &Path) -> String {
    match path.extension().and_then(|value| value.to_str()).unwrap_or("") {
        "h" | "hpp" | "hh" | "hxx" | "ixx" | "inl" => "C++ Header".into(),
        _ => "C++23 Source".into(),
    }
}

fn load_scene_entities(path: &Path) -> Result<(String, Vec<EntityRecord>)> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read scene {}", path.display()))?;
    let value: TomlValue = toml::from_str(&content)
        .with_context(|| format!("failed to parse scene {}", path.display()))?;
    let scene_name = value
        .get("scene")
        .and_then(|scene| scene.get("name"))
        .and_then(TomlValue::as_str)
        .unwrap_or("MainLevel")
        .to_string();

    let entities = value
        .get("entity")
        .and_then(TomlValue::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|entity_value| parse_entity_value(&entity_value))
        .collect::<Result<Vec<_>>>()?;

    Ok((scene_name, entities))
}

fn parse_entity_value(value: &TomlValue) -> Result<EntityRecord> {
    let table = value
        .as_table()
        .context("entity entry was not a TOML table")?;
    let scene_id = table
        .get("id")
        .and_then(TomlValue::as_str)
        .unwrap_or("entity")
        .to_string();
    let name = table
        .get("name")
        .and_then(TomlValue::as_str)
        .unwrap_or(&scene_id)
        .to_string();
    let children = table
        .get("children")
        .and_then(TomlValue::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(TomlValue::as_str)
                .map(ToString::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let components = table
        .get("component")
        .and_then(TomlValue::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .map(|component_value| parse_component_value(&component_value))
        .collect::<Result<Vec<_>>>()?;

    Ok(EntityRecord {
        id: parse_entity_id(&scene_id),
        scene_id,
        name,
        kind: infer_entity_kind(&components),
        children,
        components,
    })
}

fn parse_component_value(value: &TomlValue) -> Result<ComponentRecord> {
    let table = value
        .as_table()
        .context("component entry was not a TOML table")?;
    let type_name = table
        .get("type")
        .and_then(TomlValue::as_str)
        .unwrap_or("Component")
        .to_string();
    let mut fields = Vec::new();
    for (key, value) in table {
        if key == "type" {
            continue;
        }
        fields.push(ComponentField {
            name: key.clone(),
            value: toml_value_to_field_string(value),
        });
    }

    Ok(ComponentRecord { type_name, fields })
}

fn parse_entity_id(raw: &str) -> EntityId {
    Uuid::parse_str(raw).unwrap_or_else(|_| synthetic_uuid(raw))
}

fn synthetic_uuid(raw: &str) -> EntityId {
    let mut high = DefaultHasher::new();
    "shadow-editor-hi".hash(&mut high);
    raw.hash(&mut high);
    let mut low = DefaultHasher::new();
    "shadow-editor-lo".hash(&mut low);
    raw.hash(&mut low);
    let value = ((high.finish() as u128) << 64) | (low.finish() as u128);
    Uuid::from_u128(value)
}

fn infer_entity_kind(components: &[ComponentRecord]) -> String {
    if components.iter().any(|component| component.type_name == "Light") {
        "Light".into()
    } else if components
        .iter()
        .any(|component| component.type_name == "MeshRenderer")
    {
        "StaticMesh".into()
    } else {
        "Actor".into()
    }
}

fn toml_value_to_field_string(value: &TomlValue) -> String {
    match value {
        TomlValue::String(string) => string.clone(),
        TomlValue::Integer(integer) => integer.to_string(),
        TomlValue::Float(float) => {
            let rendered = float.to_string();
            if rendered.contains('.') {
                rendered
            } else {
                format!("{rendered}.0")
            }
        }
        TomlValue::Boolean(boolean) => boolean.to_string(),
        TomlValue::Array(array) => {
            let parts = array
                .iter()
                .map(toml_value_to_field_string)
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{parts}]")
        }
        _ => value.to_string(),
    }
}

fn parse_metadata_map(metadata: &str) -> BTreeMap<String, String> {
    let mut attributes = BTreeMap::new();
    for item in metadata.split(',') {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((key, value)) = trimmed.split_once('=') {
            attributes.insert(key.trim().to_string(), value.trim().to_string());
        } else {
            attributes.insert(trimmed.to_string(), "true".into());
        }
    }
    attributes
}

fn quoted(value: &str) -> String {
    format!("{:?}", value)
}

fn render_field_value(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.starts_with('[') && trimmed.ends_with(']') {
        return trimmed.to_string();
    }
    if matches!(trimmed, "true" | "false") {
        return trimmed.to_string();
    }
    if trimmed.parse::<i64>().is_ok() {
        return trimmed.to_string();
    }
    if trimmed.parse::<f64>().is_ok()
        && (trimmed.contains('.') || trimmed.contains('e') || trimmed.contains('E'))
    {
        return trimmed.to_string();
    }
    quoted(trimmed)
}
