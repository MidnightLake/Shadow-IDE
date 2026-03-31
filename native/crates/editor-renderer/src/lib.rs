pub mod camera;
pub mod mesh;
pub mod primitives;

use bytemuck::Zeroable;
use camera::{CameraUniforms, OrbitCamera};
use editor_core::{EntityRecord, PlayState, ViewMode, ViewportState};
use egui::{Align2, Color32, FontId, Pos2, Rect, Stroke, Ui, vec2};
use glam::{Mat4, Vec3};
use mesh::{GpuMesh, PbrVertex};
use std::collections::HashMap;
use std::sync::Arc;
use wgpu::util::DeviceExt;

// ── Gizmo types (Task 3) ──────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GizmoMode {
    Translate,
    Rotate,
    Scale,
}

impl Default for GizmoMode {
    fn default() -> Self {
        Self::Translate
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GizmoAxis {
    X,
    Y,
    Z,
}

#[derive(Debug, Clone)]
pub struct GizmoState {
    pub mode: GizmoMode,
    pub active_axis: Option<GizmoAxis>,
    pub snap_enabled: bool,
    pub snap_translate: f32,
    pub snap_rotate: f32,
    pub snap_scale: f32,
}

impl Default for GizmoState {
    fn default() -> Self {
        Self {
            mode: GizmoMode::Translate,
            active_axis: None,
            snap_enabled: false,
            snap_translate: 0.5,
            snap_rotate: 15.0,
            snap_scale: 0.1,
        }
    }
}

// ── Model uniforms ─────────────────────────────────────────────────────

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ModelUniforms {
    model: [[f32; 4]; 4],
    color: [f32; 4],
    selected: f32,
    _pad: [f32; 3],
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct GizmoUniforms {
    model: [[f32; 4]; 4],
    color: [f32; 4],
}

// ── Draw commands snapshot ─────────────────────────────────────────────

#[derive(Debug, Clone)]
struct EntityDraw {
    mesh_key: String,
    model_matrix: Mat4,
    color: [f32; 4],
    selected: bool,
}

#[derive(Debug, Clone)]
struct GizmoDraw {
    axis: GizmoAxis,
    model_matrix: Mat4,
    color: [f32; 4],
}

// ── GPU resources stored in CallbackResources ─────────────────────────

struct GpuResources {
    camera_buffer: wgpu::Buffer,
    camera_bind_group_layout: wgpu::BindGroupLayout,
    camera_bind_group: wgpu::BindGroup,
    model_buffer: wgpu::Buffer,
    model_bind_group_layout: wgpu::BindGroupLayout,
    model_bind_group: wgpu::BindGroup,
    gizmo_buffer: wgpu::Buffer,
    gizmo_bind_group_layout: wgpu::BindGroupLayout,
    gizmo_bind_group: wgpu::BindGroup,
    pbr_pipeline: wgpu::RenderPipeline,
    grid_pipeline: wgpu::RenderPipeline,
    gizmo_pipeline: wgpu::RenderPipeline,
    depth_texture: wgpu::TextureView,
    depth_size: (u32, u32),
    meshes: HashMap<String, GpuMesh>,
}

impl GpuResources {
    fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        // Camera bind group
        let camera_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("camera_uniform"),
            contents: bytemuck::bytes_of(&CameraUniforms::zeroed()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("camera_bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera_bg"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buffer.as_entire_binding(),
            }],
        });

        // Model bind group
        let model_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("model_uniform"),
            contents: bytemuck::bytes_of(&ModelUniforms {
                model: Mat4::IDENTITY.to_cols_array_2d(),
                color: [1.0; 4],
                selected: 0.0,
                _pad: [0.0; 3],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let model_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("model_bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let model_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("model_bg"),
            layout: &model_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: model_buffer.as_entire_binding(),
            }],
        });

        // Gizmo bind group
        let gizmo_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("gizmo_uniform"),
            contents: bytemuck::bytes_of(&GizmoUniforms {
                model: Mat4::IDENTITY.to_cols_array_2d(),
                color: [1.0, 0.0, 0.0, 1.0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let gizmo_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("gizmo_bgl"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
        let gizmo_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gizmo_bg"),
            layout: &gizmo_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: gizmo_buffer.as_entire_binding(),
            }],
        });

        let depth_format = wgpu::TextureFormat::Depth32Float;

        // PBR pipeline
        let pbr_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("pbr_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/pbr.wgsl").into()),
        });
        let pbr_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("pbr_layout"),
            bind_group_layouts: &[&camera_bind_group_layout, &model_bind_group_layout],
            push_constant_ranges: &[],
        });
        let pbr_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("pbr_pipeline"),
            layout: Some(&pbr_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &pbr_shader,
                entry_point: Some("vs_main"),
                buffers: &[PbrVertex::layout()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &pbr_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        // Grid pipeline
        let grid_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grid_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/grid.wgsl").into()),
        });
        let grid_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("grid_layout"),
                bind_group_layouts: &[&camera_bind_group_layout],
                push_constant_ranges: &[],
            });
        let grid_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grid_pipeline"),
            layout: Some(&grid_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &grid_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &grid_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        // Gizmo pipeline (no depth test — always on top)
        let gizmo_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gizmo_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/gizmo.wgsl").into()),
        });
        let gizmo_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("gizmo_layout"),
                bind_group_layouts: &[&camera_bind_group_layout, &gizmo_bind_group_layout],
                push_constant_ranges: &[],
            });
        let gizmo_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gizmo_pipeline"),
            layout: Some(&gizmo_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &gizmo_shader,
                entry_point: Some("vs_main"),
                buffers: &[PbrVertex::layout()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &gizmo_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: Default::default(),
            multiview: None,
            cache: None,
        });

        // Default meshes
        let mut meshes = HashMap::new();
        let (v, i) = primitives::cube_vertices([0.7, 0.75, 0.8, 1.0]);
        meshes.insert("cube".into(), GpuMesh::from_data(device, &v, &i));
        let (v, i) = primitives::sphere_vertices([0.8, 0.6, 0.3, 1.0], 16, 24);
        meshes.insert("sphere".into(), GpuMesh::from_data(device, &v, &i));
        let (v, i) = primitives::plane_vertices([0.4, 0.5, 0.4, 1.0], 10.0);
        meshes.insert("plane".into(), GpuMesh::from_data(device, &v, &i));
        // Gizmo arrows
        let (v, i) = primitives::arrow_vertices([1.0, 0.2, 0.2, 1.0], 1.5, 0.04);
        meshes.insert("arrow_x".into(), GpuMesh::from_data(device, &v, &i));
        let (v, i) = primitives::arrow_vertices([0.2, 1.0, 0.2, 1.0], 1.5, 0.04);
        meshes.insert("arrow_y".into(), GpuMesh::from_data(device, &v, &i));
        let (v, i) = primitives::arrow_vertices([0.2, 0.3, 1.0, 1.0], 1.5, 0.04);
        meshes.insert("arrow_z".into(), GpuMesh::from_data(device, &v, &i));

        let depth_texture = create_depth_texture(device, 64, 64);

        Self {
            camera_buffer,
            camera_bind_group_layout,
            camera_bind_group,
            model_buffer,
            model_bind_group_layout,
            model_bind_group,
            gizmo_buffer,
            gizmo_bind_group_layout,
            gizmo_bind_group,
            pbr_pipeline,
            grid_pipeline,
            gizmo_pipeline,
            depth_texture,
            depth_size: (64, 64),
            meshes,
        }
    }

    fn ensure_depth_size(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.depth_size != (width, height) && width > 0 && height > 0 {
            self.depth_texture = create_depth_texture(device, width, height);
            self.depth_size = (width, height);
        }
    }
}

fn create_depth_texture(device: &wgpu::Device, width: u32, height: u32) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewport_depth"),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    texture.create_view(&Default::default())
}

// ── Viewport paint callback ────────────────────────────────────────────

struct ViewportCallback {
    entity_draws: Vec<EntityDraw>,
    gizmo_draws: Vec<GizmoDraw>,
    camera_uniforms: CameraUniforms,
    viewport_pixels: (u32, u32),
    /// Meshes to upload to the GPU before this frame is rendered.
    pending_mesh_uploads: Vec<(String, Vec<PbrVertex>, Vec<u32>)>,
}

impl egui_wgpu::CallbackTrait for ViewportCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let gpu = resources.get_mut::<GpuResources>().unwrap();

        // Upload any imported meshes that arrived since the last frame.
        for (key, verts, inds) in &self.pending_mesh_uploads {
            gpu.meshes.insert(key.clone(), GpuMesh::from_data(device, verts, inds));
        }

        gpu.ensure_depth_size(device, self.viewport_pixels.0, self.viewport_pixels.1);
        queue.write_buffer(
            &gpu.camera_buffer,
            0,
            bytemuck::bytes_of(&self.camera_uniforms),
        );
        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let gpu = resources.get::<GpuResources>().unwrap();
        let rect = info.viewport_in_pixels();
        render_pass.set_viewport(
            rect.left_px as f32,
            rect.top_px as f32,
            rect.width_px as f32,
            rect.height_px as f32,
            0.0,
            1.0,
        );
        render_pass.set_scissor_rect(
            rect.left_px as u32,
            rect.top_px as u32,
            rect.width_px as u32,
            rect.height_px as u32,
        );

        // Grid
        render_pass.set_pipeline(&gpu.grid_pipeline);
        render_pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        render_pass.draw(0..6, 0..1);

        // Entity meshes
        render_pass.set_pipeline(&gpu.pbr_pipeline);
        render_pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
        render_pass.set_bind_group(1, &gpu.model_bind_group, &[]);
        for draw in &self.entity_draws {
            if let Some(mesh) = gpu.meshes.get(&draw.mesh_key) {
                // We can't write uniforms per-draw in the paint callback (no queue access).
                // For the MVP, all entities share the same model uniform written in prepare().
                // A proper impl would use a dynamic uniform buffer. For now, draw all entities.
                render_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                render_pass.set_index_buffer(mesh.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                render_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
            }
        }

        // Gizmos (no depth test pipeline)
        if !self.gizmo_draws.is_empty() {
            render_pass.set_pipeline(&gpu.gizmo_pipeline);
            render_pass.set_bind_group(0, &gpu.camera_bind_group, &[]);
            render_pass.set_bind_group(1, &gpu.gizmo_bind_group, &[]);
            for gizmo_draw in &self.gizmo_draws {
                let mesh_key = match gizmo_draw.axis {
                    GizmoAxis::X => "arrow_x",
                    GizmoAxis::Y => "arrow_y",
                    GizmoAxis::Z => "arrow_z",
                };
                if let Some(mesh) = gpu.meshes.get(mesh_key) {
                    render_pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
                    render_pass.set_index_buffer(
                        mesh.index_buffer.slice(..),
                        wgpu::IndexFormat::Uint32,
                    );
                    render_pass.draw_indexed(0..mesh.index_count, 0, 0..1);
                }
            }
        }
    }
}

// ── Public viewport renderer ───────────────────────────────────────────

#[derive(Debug)]
pub struct ViewportRenderer {
    pub camera: OrbitCamera,
    pub gizmo: GizmoState,
    initialized: bool,
    /// Meshes queued for GPU upload on the next paint pass.
    /// Each entry: (mesh_key, vertices[pos+normal+color], indices)
    pending_uploads: Vec<(String, Vec<PbrVertex>, Vec<u32>)>,
}

impl Default for ViewportRenderer {
    fn default() -> Self {
        Self {
            camera: OrbitCamera::default(),
            gizmo: GizmoState::default(),
            initialized: false,
            pending_uploads: Vec::new(),
        }
    }
}

impl ViewportRenderer {
    /// Queue a mesh for upload to the GPU on the next frame.
    ///
    /// `key` — unique name used to look it up at draw time (e.g. `"player_0"`).
    /// `positions` / `normals` are parallel arrays; `colors` may be empty.
    pub fn queue_mesh_upload(
        &mut self,
        key: String,
        positions: Vec<[f32; 3]>,
        normals: Vec<[f32; 3]>,
        colors: Vec<[f32; 4]>,
        indices: Vec<u32>,
    ) {
        let default_color = [0.8_f32, 0.8, 0.8, 1.0];
        let vertices: Vec<PbrVertex> = positions
            .iter()
            .enumerate()
            .map(|(i, pos)| PbrVertex {
                position: *pos,
                normal: normals.get(i).copied().unwrap_or([0.0, 1.0, 0.0]),
                color: colors.get(i).copied().unwrap_or(default_color),
            })
            .collect();
        self.pending_uploads.push((key, vertices, indices));
    }
}

impl ViewportRenderer {
    pub fn init_gpu(&mut self, cc: &eframe::CreationContext<'_>) {
        let Some(wgpu_state) = cc.wgpu_render_state.as_ref() else {
            eprintln!("[editor-renderer] eframe wgpu render state not available");
            return;
        };
        let device = &wgpu_state.device;
        let target_format = wgpu_state.target_format;
        let resources = GpuResources::new(device, target_format);
        wgpu_state
            .renderer
            .write()
            .callback_resources
            .insert(resources);
        self.initialized = true;
    }

    pub fn paint(
        &mut self,
        ui: &mut Ui,
        viewport: &ViewportState,
        entities: &[EntityRecord],
        selected: Option<editor_core::EntityId>,
    ) {
        let available = ui.available_size_before_wrap();
        let size = vec2(available.x.max(100.0), available.y.max(100.0));
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());

        // Handle camera input
        if response.dragged_by(egui::PointerButton::Middle)
            || (response.dragged_by(egui::PointerButton::Primary)
                && ui.input(|i| i.modifiers.alt))
        {
            let delta = response.drag_delta();
            if ui.input(|i| i.modifiers.shift) {
                self.camera.pan(glam::Vec2::new(delta.x, delta.y));
            } else {
                self.camera.rotate(-delta.x * 0.005, -delta.y * 0.005);
            }
        }

        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            if scroll != 0.0 {
                self.camera.zoom(scroll * 0.05);
            }
        }

        if !self.initialized {
            // Fallback: paint placeholder
            paint_placeholder(ui, rect, viewport);
            return;
        }

        let aspect = size.x / size.y;
        let camera_uniforms = self.camera.uniforms(aspect, viewport.mode);
        let pixels_per_point = ui.ctx().pixels_per_point();
        let viewport_pixels = (
            (size.x * pixels_per_point) as u32,
            (size.y * pixels_per_point) as u32,
        );

        // Build draw list from entities
        let mut entity_draws = Vec::new();
        for entity in entities {
            let pos = parse_entity_position(entity);
            // Prefer an imported mesh referenced by the MeshRenderer component.
            // Key convention: "{stem}_{prim_index}" e.g. "player_0" for "assets/player.glb#Mesh0"
            let mesh_key = resolve_mesh_key(entity);
            entity_draws.push(EntityDraw {
                mesh_key,
                model_matrix: Mat4::from_translation(pos),
                color: entity_color(entity),
                selected: Some(entity.id) == selected,
            });
        }

        // Build gizmo draws for selected entity
        let mut gizmo_draws = Vec::new();
        if viewport.play_state == PlayState::Edit {
            if let Some(sel_id) = selected {
                if let Some(sel_entity) = entities.iter().find(|e| e.id == sel_id) {
                    let pos = parse_entity_position(sel_entity);
                    let base = Mat4::from_translation(pos);
                    // X arrow (points along +X): rotate -90 around Z
                    gizmo_draws.push(GizmoDraw {
                        axis: GizmoAxis::X,
                        model_matrix: base * Mat4::from_rotation_z(-std::f32::consts::FRAC_PI_2),
                        color: [1.0, 0.2, 0.2, 1.0],
                    });
                    // Y arrow (points along +Y): identity
                    gizmo_draws.push(GizmoDraw {
                        axis: GizmoAxis::Y,
                        model_matrix: base,
                        color: [0.2, 1.0, 0.2, 1.0],
                    });
                    // Z arrow (points along +Z): rotate 90 around X
                    gizmo_draws.push(GizmoDraw {
                        axis: GizmoAxis::Z,
                        model_matrix: base * Mat4::from_rotation_x(std::f32::consts::FRAC_PI_2),
                        color: [0.2, 0.3, 1.0, 1.0],
                    });
                }
            }
        }

        let callback = ViewportCallback {
            entity_draws,
            gizmo_draws,
            camera_uniforms,
            viewport_pixels,
            pending_mesh_uploads: std::mem::take(&mut self.pending_uploads),
        };

        let paint_callback = egui::PaintCallback {
            rect,
            callback: Arc::new(egui_wgpu::Callback::new_paint_callback(rect, callback)),
        };
        ui.painter().add(paint_callback);

        // Paint 2D overlay on top
        paint_overlay(ui, rect, viewport);
    }
}

fn paint_overlay(ui: &Ui, rect: Rect, viewport: &ViewportState) {
    let painter = ui.painter_at(rect);
    let text_color = Color32::from_rgba_unmultiplied(236, 241, 244, 200);
    let accent = Color32::from_rgb(233, 170, 95);

    let mode = match viewport.mode {
        ViewMode::Perspective3D => "Perspective",
        ViewMode::Orthographic2D => "Ortho 2D",
    };
    let play_state = match viewport.play_state {
        PlayState::Edit => "Edit",
        PlayState::Playing => "Playing",
        PlayState::Paused => "Paused",
    };

    painter.text(
        rect.left_top() + vec2(12.0, 10.0),
        Align2::LEFT_TOP,
        format!(
            "{mode}  |  {play_state}  |  {} FPS  |  {:.1}ms",
            viewport.stats.fps, viewport.stats.gpu_time_ms
        ),
        FontId::proportional(13.0),
        text_color,
    );

    painter.text(
        Pos2::new(rect.right() - 12.0, rect.top() + 10.0),
        Align2::RIGHT_TOP,
        format!(
            "{} draws  |  {} entities",
            viewport.stats.draw_calls, viewport.stats.visible_entities
        ),
        FontId::proportional(13.0),
        accent,
    );

    // Live Preview badge
    if viewport.live_preview {
        let live_color = Color32::from_rgb(235, 64, 64);
        let label = "● LIVE";
        let badge_pos = rect.left_top() + vec2(12.0, 32.0);
        painter.text(badge_pos, Align2::LEFT_TOP, label, FontId::proportional(13.0), live_color);
    }
}

fn paint_placeholder(ui: &Ui, rect: Rect, viewport: &ViewportState) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, Color32::from_rgb(15, 25, 31));

    let grid_color = Color32::from_rgba_unmultiplied(139, 174, 184, 28);
    let spacing = 32.0;
    let mut x = rect.left();
    while x <= rect.right() {
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.0, grid_color),
        );
        x += spacing;
    }
    let mut y = rect.top();
    while y <= rect.bottom() {
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0, grid_color),
        );
        y += spacing;
    }

    painter.text(
        rect.center(),
        Align2::CENTER_CENTER,
        "wgpu not initialized — placeholder viewport",
        FontId::proportional(18.0),
        Color32::from_rgb(236, 241, 244),
    );
    paint_overlay(ui, rect, viewport);
}

fn parse_entity_position(entity: &EntityRecord) -> Vec3 {
    entity
        .components
        .iter()
        .find(|c| c.type_name == "Transform")
        .and_then(|c| c.field_value("position"))
        .and_then(parse_vec3)
        .unwrap_or(Vec3::ZERO)
}

fn parse_vec3(s: &str) -> Option<Vec3> {
    let s = s.trim().trim_start_matches('[').trim_end_matches(']');
    let parts: Vec<f32> = s.split(',').filter_map(|p| p.trim().parse().ok()).collect();
    if parts.len() >= 3 {
        Some(Vec3::new(parts[0], parts[1], parts[2]))
    } else {
        None
    }
}

fn entity_color(entity: &EntityRecord) -> [f32; 4] {
    match entity.kind.as_str() {
        "Light" => [1.0, 0.95, 0.6, 1.0],
        "StaticMesh" => [0.5, 0.6, 0.5, 1.0],
        _ => [0.7, 0.75, 0.8, 1.0],
    }
}

/// Map an entity to the GPU mesh key to draw.
///
/// - Terrain entities use key `"terrain_{scene_id}"` (populated by RegenerateTerrain).
/// - MeshRenderer.mesh values like `"assets/player.glb#Mesh0"` → `"player_0"`.
/// - Falls back to built-in primitive names based on entity name/kind.
fn resolve_mesh_key(entity: &EntityRecord) -> String {
    // Terrain entities get their own procedurally-generated mesh.
    if entity.components.iter().any(|c| c.type_name == "Terrain") {
        return format!("terrain_{}", entity.scene_id);
    }

    // Look for a MeshRenderer component with a "mesh" field.
    let mesh_ref = entity
        .components
        .iter()
        .find(|c| c.type_name == "MeshRenderer")
        .and_then(|c| c.field_value("mesh"));

    if let Some(mesh_ref) = mesh_ref {
        // mesh_ref = "assets/player.glb#Mesh0"
        // Extract the part after the last '/' (or whole string) as the file stem.
        let filename = mesh_ref.rsplit('/').next().unwrap_or(mesh_ref);
        // Split at '#' to get optional primitive index suffix.
        let (file_part, prim_suffix) = filename.split_once('#').unwrap_or((filename, "Mesh0"));
        // Strip file extension to get stem.
        let stem = file_part.rsplit('.').nth(1).unwrap_or(file_part);
        // Parse prim index from suffix like "Mesh0", "Mesh1", etc.
        let prim_idx: usize = prim_suffix
            .trim_start_matches(|c: char| !c.is_ascii_digit())
            .parse()
            .unwrap_or(0);
        return format!("{}_{}", stem, prim_idx);
    }

    // Fall back to built-in primitive based on entity kind/name.
    let name_lower = entity.name.to_lowercase();
    if name_lower.contains("sphere") || entity.kind == "Sphere" {
        "sphere".to_string()
    } else if name_lower.contains("cube") || name_lower.contains("box") || entity.kind == "Cube" {
        "cube".to_string()
    } else {
        "plane".to_string()
    }
}

// ── Procedural terrain mesh generator ────────────────────────────────
//
// Generates a sin/cos heightmap grid suitable for live preview.
// Called from main.rs whenever Live Preview is active and terrain params change.

/// Build a procedural terrain mesh from noise parameters.
///
/// Returns `(positions, normals, colors, indices)`.
///
/// - `cols` / `rows` — vertex grid resolution (clamped ≥ 2)
/// - `scale` — world-space width/depth of the terrain patch
/// - `height_scale` — maximum vertical displacement
/// - `frequency` — noise frequency (higher = more hills per unit)
pub fn generate_terrain_mesh(
    cols: u32,
    rows: u32,
    scale: f32,
    height_scale: f32,
    frequency: f32,
) -> (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 4]>, Vec<u32>) {
    let cols = cols.max(2);
    let rows = rows.max(2);

    let height_at = |x: f32, z: f32| -> f32 {
        let f = frequency * 0.3;
        height_scale
            * ((x * f).sin() * 0.5
                + (z * f).cos() * 0.5
                + (x * f * 2.1 + 1.3).sin() * (z * f * 1.7).cos() * 0.3)
    };

    let mut positions = Vec::with_capacity((cols * rows) as usize);
    let mut colors = Vec::with_capacity((cols * rows) as usize);

    for row in 0..rows {
        for col in 0..cols {
            let x = (col as f32 / (cols - 1) as f32 - 0.5) * scale;
            let z = (row as f32 / (rows - 1) as f32 - 0.5) * scale;
            let y = height_at(x, z);
            positions.push([x, y, z]);
            // Color: greenish valleys, rocky peaks
            let t = ((y / height_scale.max(0.001)) * 0.5 + 0.5).clamp(0.0, 1.0);
            colors.push([0.18 + t * 0.35, 0.38 + t * 0.22, 0.12 + t * 0.08, 1.0]);
        }
    }

    // Normals via finite differences
    let dx = scale / (cols - 1) as f32;
    let dz = scale / (rows - 1) as f32;
    let mut normals = vec![[0.0f32, 1.0, 0.0]; (cols * rows) as usize];
    for row in 0..rows as usize {
        for col in 0..cols as usize {
            let x = (col as f32 / (cols - 1) as f32 - 0.5) * scale;
            let z = (row as f32 / (rows - 1) as f32 - 0.5) * scale;
            let yl = height_at(x - dx, z);
            let yr = height_at(x + dx, z);
            let yd = height_at(x, z - dz);
            let yu = height_at(x, z + dz);
            let n = Vec3::new(yl - yr, 2.0 * dx.min(dz), yd - yu).normalize();
            normals[row * cols as usize + col] = n.into();
        }
    }

    // Quad strip indices (two triangles per cell)
    let mut indices = Vec::with_capacity(((cols - 1) * (rows - 1) * 6) as usize);
    for row in 0..(rows - 1) as usize {
        for col in 0..(cols - 1) as usize {
            let i = (row * cols as usize + col) as u32;
            let r = i + 1;
            let d = i + cols;
            let dr = d + 1;
            indices.extend_from_slice(&[i, d, r, r, d, dr]);
        }
    }

    (positions, normals, colors, indices)
}
