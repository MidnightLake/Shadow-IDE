use std::path::{Path, PathBuf};
use uuid::Uuid;

// ── Processed asset types ─────────────────────────────────────────────

/// A single glTF mesh primitive extracted to CPU arrays.
#[derive(Debug, Clone)]
pub struct MeshPrimitive {
    /// Index within the source file (0-based).
    pub index: usize,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    /// Vertex-colors RGBA8 scaled to [0,1].  Empty if not present in source.
    pub colors: Vec<[f32; 4]>,
}

/// All primitives extracted from a single mesh source file.
#[derive(Debug, Clone)]
pub struct ProcessedMesh {
    pub asset_id: Uuid,
    /// Bare file name, e.g. `"player.glb"`.
    pub source_name: String,
    pub primitives: Vec<MeshPrimitive>,
}

/// RGBA8 image decoded from a texture source file.
#[derive(Debug, Clone)]
pub struct ProcessedTexture {
    pub asset_id: Uuid,
    /// Bare file name, e.g. `"albedo.png"`.
    pub source_name: String,
    pub width: u32,
    pub height: u32,
    /// Raw RGBA8 pixels, row-major.
    pub rgba: Vec<u8>,
}

/// Outcome of importing one asset file.
#[derive(Debug, Default)]
pub struct ImportResult {
    pub mesh: Option<ProcessedMesh>,
    pub texture: Option<ProcessedTexture>,
    pub errors: Vec<String>,
}

/// Summary returned by `AssetDatabase::import_all`.
#[derive(Debug, Default)]
pub struct ImportReport {
    pub meshes: Vec<ProcessedMesh>,
    pub textures: Vec<ProcessedTexture>,
    pub errors: Vec<String>,
}

// ── glTF importer ────────────────────────────────────────────────────

pub fn import_gltf(path: &Path, asset_id: Uuid) -> Result<ProcessedMesh, String> {
    let (doc, buffers, _images) = gltf::import(path)
        .map_err(|e| format!("gltf import failed for {}: {e}", path.display()))?;

    let source_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown.glb")
        .to_string();

    let mut primitives = Vec::new();

    for mesh in doc.meshes() {
        for (prim_idx, prim) in mesh.primitives().enumerate() {
            let reader = prim.reader(|buf| buffers.get(buf.index()).map(|b| b.0.as_slice()));

            let positions: Vec<[f32; 3]> = reader
                .read_positions()
                .map(|iter| iter.collect())
                .unwrap_or_default();

            if positions.is_empty() {
                continue;
            }

            let normals: Vec<[f32; 3]> = reader
                .read_normals()
                .map(|iter| iter.collect())
                .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; positions.len()]);

            let colors: Vec<[f32; 4]> = reader
                .read_colors(0)
                .map(|c| c.into_rgba_f32().collect())
                .unwrap_or_default();

            let indices: Vec<u32> = reader
                .read_indices()
                .map(|iter| iter.into_u32().collect())
                .unwrap_or_else(|| (0..positions.len() as u32).collect());

            primitives.push(MeshPrimitive {
                index: prim_idx,
                positions,
                normals,
                indices,
                colors,
            });
        }
    }

    if primitives.is_empty() {
        return Err(format!("no mesh primitives found in {}", path.display()));
    }

    Ok(ProcessedMesh { asset_id, source_name, primitives })
}

// ── Image / texture importer ─────────────────────────────────────────

pub fn import_image_file(path: &Path, asset_id: Uuid) -> Result<ProcessedTexture, String> {
    let img = image::open(path)
        .map_err(|e| format!("image load failed for {}: {e}", path.display()))?;
    let rgba = img.to_rgba8();
    let (width, height) = rgba.dimensions();
    let source_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown.png")
        .to_string();
    Ok(ProcessedTexture {
        asset_id,
        source_name,
        width,
        height,
        rgba: rgba.into_raw(),
    })
}

// ── AssetDatabase import methods ─────────────────────────────────────

#[derive(Debug, Clone)]
pub struct AssetDescriptor {
    pub id: Uuid,
    pub name: String,
    pub kind: String,
    pub source: PathBuf,
    pub size_bytes: u64,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AssetDatabase {
    pub cache_root: PathBuf,
    pub items: Vec<AssetDescriptor>,
}

impl AssetDatabase {
    /// Scan a ShadowEditor project root for assets in the `assets/` subdirectory.
    pub fn scan(project_root: impl AsRef<Path>) -> Self {
        let project_root = project_root.as_ref();
        let cache_root = project_root.join(".shadoweditor/cache");
        let assets_dir = project_root.join("assets");

        let mut items = Vec::new();
        if assets_dir.exists() {
            walk_assets(&assets_dir, &mut items);
        }

        // Always include at least the scene files from scenes/
        let scenes_dir = project_root.join("scenes");
        if scenes_dir.exists() {
            walk_assets(&scenes_dir, &mut items);
        }

        Self { cache_root, items }
    }

    /// Fallback demo data when no project root is available.
    pub fn seed_demo(cache_root: impl AsRef<Path>) -> Self {
        Self {
            cache_root: cache_root.as_ref().to_path_buf(),
            items: vec![
                AssetDescriptor {
                    id: Uuid::new_v4(),
                    name: "player.glb".into(),
                    kind: "Mesh".into(),
                    source: PathBuf::from("assets/player.glb"),
                    size_bytes: 0,
                    tags: vec!["character".into(), "hero".into()],
                },
                AssetDescriptor {
                    id: Uuid::new_v4(),
                    name: "ground.glb".into(),
                    kind: "Mesh".into(),
                    source: PathBuf::from("assets/ground.glb"),
                    size_bytes: 0,
                    tags: vec!["terrain".into()],
                },
                AssetDescriptor {
                    id: Uuid::new_v4(),
                    name: "sunny_sky.ktx2".into(),
                    kind: "Texture".into(),
                    source: PathBuf::from("assets/sky/sunny_sky.ktx2"),
                    size_bytes: 0,
                    tags: vec!["sky".into(), "hdr".into()],
                },
            ],
        }
    }

    pub fn semantic_index_state(&self) -> String {
        let counts = self.kind_counts();
        let summary: Vec<String> = counts
            .iter()
            .map(|(k, n)| format!("{n} {k}"))
            .collect();
        if summary.is_empty() {
            "No assets found".into()
        } else {
            summary.join(", ")
        }
    }

    fn kind_counts(&self) -> Vec<(String, usize)> {
        let mut map: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for item in &self.items {
            *map.entry(item.kind.clone()).or_insert(0) += 1;
        }
        map.into_iter().collect()
    }

    /// Import a single asset file, returning parsed CPU-side data.
    pub fn import_asset(&self, desc: &AssetDescriptor) -> ImportResult {
        let mut result = ImportResult::default();
        match desc.kind.as_str() {
            "Mesh" => match import_gltf(&desc.source, desc.id) {
                Ok(mesh) => result.mesh = Some(mesh),
                Err(e) => result.errors.push(e),
            },
            "Texture" => match import_image_file(&desc.source, desc.id) {
                Ok(tex) => result.texture = Some(tex),
                Err(e) => result.errors.push(e),
            },
            _ => {}
        }
        result
    }

    /// Import every Mesh and Texture in the database that exists on disk.
    pub fn import_all(&self) -> ImportReport {
        let mut report = ImportReport::default();
        for desc in &self.items {
            if !desc.source.exists() {
                continue;
            }
            if !matches!(desc.kind.as_str(), "Mesh" | "Texture") {
                continue;
            }
            let result = self.import_asset(desc);
            if let Some(m) = result.mesh {
                report.meshes.push(m);
            }
            if let Some(t) = result.texture {
                report.textures.push(t);
            }
            report.errors.extend(result.errors);
        }
        report
    }
}

fn walk_assets(dir: &Path, out: &mut Vec<AssetDescriptor>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden directories and .import caches
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with('.') {
                walk_assets(&path, out);
            }
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        let kind = classify_asset(&ext);
        if kind == "other" {
            continue;
        }
        let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        let tags = auto_tags(&name, &kind);
        out.push(AssetDescriptor {
            id: Uuid::new_v4(),
            name,
            kind: kind.to_string(),
            source: path,
            size_bytes,
            tags,
        });
    }
}

fn classify_asset(ext: &str) -> &'static str {
    match ext {
        "gltf" | "glb" | "obj" | "fbx" | "dae" | "blend" => "Mesh",
        "png" | "jpg" | "jpeg" | "webp" | "exr" | "hdr" | "ktx2" | "dds" | "tga" | "bmp" => {
            "Texture"
        }
        "wav" | "ogg" | "mp3" | "flac" => "Audio",
        "ttf" | "otf" | "woff" | "woff2" => "Font",
        "shadow" => "Scene",
        "shadow_mat" => "Material",
        "wgsl" | "glsl" | "vert" | "frag" | "gdshader" => "Shader",
        "json" if ext == "json" => "Data",
        _ => "other",
    }
}

fn auto_tags(name: &str, kind: &str) -> Vec<String> {
    let mut tags = vec![kind.to_lowercase()];
    let lower = name.to_lowercase();
    for keyword in &["player", "enemy", "ground", "sky", "water", "terrain", "ui", "effect"] {
        if lower.contains(keyword) {
            tags.push(keyword.to_string());
        }
    }
    tags
}
