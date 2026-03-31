use editor_hot_reload::{EntityId, HotReloadHost, RuntimeComponentInfo};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Mutex;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct ShaderSnippet {
    pub name: String,
    pub language: String,
    pub description: String,
    pub code: String,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct OptimizeResult {
    pub original_size: u64,
    pub optimized_size: u64,
    pub output_path: String,
    pub savings_percent: f32,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct GodotAsset {
    pub path: String,
    pub asset_type: String,
    pub size: u64,
}

#[derive(Default)]
pub struct ShadowRuntimeState {
    sessions: Mutex<HashMap<String, ShadowRuntimeSession>>,
}

struct ShadowRuntimeSession {
    host: HotReloadHost,
    last_error: Option<String>,
    last_scene_path: Option<String>,
}

#[derive(Debug, Clone)]
enum LiveSceneMutation {
    AddEntity {
        entity_id: String,
    },
    RenameEntity {
        entity_id: String,
    },
    RemoveEntity {
        entity_id: String,
    },
    AddComponent {
        entity_id: String,
        component_type: String,
    },
    RemoveComponent {
        entity_id: String,
        component_type: String,
    },
    SetComponentField {
        entity_id: String,
        component_type: String,
    },
}

// The runtime host is only accessed behind ShadowRuntimeState's Mutex.
unsafe impl Send for ShadowRuntimeSession {}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct ShadowRuntimeStatus {
    pub project_path: String,
    pub library_path: String,
    pub library_exists: bool,
    pub is_live: bool,
    pub status_line: String,
    pub frame_index: u64,
    pub component_count: u32,
    pub entity_count: u32,
    pub entry_scene_path: String,
    pub last_scene_path: String,
    pub last_error: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct ShadowPlanengineDocs {
    pub plan_path: String,
    pub finish_path: String,
    pub plan_markdown: String,
    pub finish_markdown: String,
    pub finish_available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShadowCompilerStyle {
    GnuLike,
    Msvc,
}

#[derive(Debug, Clone)]
struct ShadowCompilerResolution {
    executable: PathBuf,
    display_name: String,
    style: ShadowCompilerStyle,
    setup_script: Option<PathBuf>,
}

const SHADOW_RUNTIME_EXPORTS: &[&str] = &[
    "shadow_init",
    "shadow_update",
    "shadow_shutdown",
    "shadow_component_count",
    "shadow_component_meta",
    "shadow_get_component",
    "shadow_set_component",
    "shadow_remove_component",
    "shadow_create_entity",
    "shadow_destroy_entity",
    "shadow_set_entity_name",
    "shadow_set_entity_scene_id",
    "shadow_find_entity_by_scene_id",
    "shadow_entity_count",
    "shadow_entity_list",
    "shadow_load_scene",
    "shadow_save_scene",
];

// ===== Godot LSP =====

/// Connect to Godot's built-in LSP server on the given port (default 6005).
#[tauri::command]
pub async fn connect_godot_lsp(port: u16) -> Result<String, String> {
    use std::net::TcpStream;

    let addr = format!("127.0.0.1:{}", port);
    match TcpStream::connect(&addr) {
        Ok(_) => Ok(format!("Connected to Godot LSP at {}", addr)),
        Err(e) => Err(format!("Failed to connect to Godot LSP at {}: {}", addr, e)),
    }
}

// ===== Run Godot Project =====

/// Run a Godot project in play mode.
#[tauri::command]
pub async fn run_godot_project(
    project_path: String,
    godot_binary: Option<String>,
) -> Result<String, String> {
    let binary = if let Some(b) = godot_binary {
        b
    } else {
        find_godot_binary()?
    };

    let mut cmd = Command::new(&binary);
    cmd.args(["--path", &project_path, "--"]);
    crate::platform::hide_window(&mut cmd);

    let child = cmd
        .spawn()
        .map_err(|e| format!("Failed to launch Godot ({}): {}", binary, e))?;

    Ok(format!("Godot project launched (PID {})", child.id()))
}

fn find_godot_binary() -> Result<String, String> {
    for name in &["godot4", "godot3", "godot"] {
        let check = Command::new("which").arg(name).output();
        if let Ok(out) = check {
            if out.status.success() {
                let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if !path.is_empty() {
                    return Ok(path);
                }
            }
        }
    }
    Err("Godot binary not found in PATH. Install Godot or provide the binary path.".to_string())
}

#[tauri::command]
pub async fn shadow_launch_native_editor(project_path: String) -> Result<String, String> {
    let project_root = PathBuf::from(&project_path);
    if !project_root.join(".shadow_project.toml").exists() {
        return Err(format!(
            "No .shadow_project.toml found in {}",
            project_root.display()
        ));
    }

    let tauri_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ide_root = tauri_root
        .parent()
        .ok_or_else(|| "Failed to resolve shadow-ide root from src-tauri".to_string())?
        .to_path_buf();
    let native_manifest = ide_root.join("native").join("Cargo.toml");
    let project_root = project_root.canonicalize().unwrap_or(project_root);

    let binary_name = if cfg!(windows) {
        "shadow-editor.exe"
    } else {
        "shadow-editor"
    };
    let binary_candidates = [
        ide_root
            .join("native")
            .join("target")
            .join("debug")
            .join(binary_name),
        ide_root
            .join("native")
            .join("target")
            .join("release")
            .join(binary_name),
    ];

    for candidate in &binary_candidates {
        if !candidate.exists() {
            continue;
        }

        let mut cmd = Command::new(candidate);
        cmd.current_dir(&ide_root)
            .arg("--project")
            .arg(&project_root);
        crate::platform::hide_window(&mut cmd);

        let child = cmd.spawn().map_err(|e| {
            format!(
                "Failed to launch native ShadowEditor binary {}: {}",
                candidate.display(),
                e
            )
        })?;

        return Ok(format!(
            "Opened native ShadowEditor for {} (PID {})",
            project_root.display(),
            child.id()
        ));
    }

    if !crate::platform::is_command_available("cargo") {
        return Err(format!(
            "Native ShadowEditor binary not found. Build it first with `npm run build:native` from {}",
            ide_root.display()
        ));
    }

    let mut cmd = Command::new("cargo");
    cmd.current_dir(&ide_root)
        .arg("run")
        .arg("--manifest-path")
        .arg(&native_manifest)
        .arg("-p")
        .arg("shadow-editor")
        .arg("--")
        .arg("--project")
        .arg(&project_root);
    crate::platform::hide_window(&mut cmd);

    let child = cmd.spawn().map_err(|e| {
        format!(
            "Failed to launch native ShadowEditor via cargo run using {}: {}",
            native_manifest.display(),
            e
        )
    })?;

    Ok(format!(
        "Building and opening native ShadowEditor for {} (PID {})",
        project_root.display(),
        child.id()
    ))
}

// ===== Shader Snippets =====

/// Get built-in shader snippets.
#[tauri::command]
pub fn get_shader_snippets() -> Vec<ShaderSnippet> {
    vec![
        ShaderSnippet {
            name: "PBR Fragment".to_string(),
            language: "glsl".to_string(),
            description:
                "Physically-based rendering fragment shader with metallic/roughness workflow"
                    .to_string(),
            code: r#"// PBR Fragment Shader (metallic/roughness workflow)
uniform sampler2D albedoMap;
uniform sampler2D normalMap;
uniform sampler2D metallicRoughnessMap;
uniform sampler2D aoMap;
uniform float metallic;
uniform float roughness;

const float PI = 3.14159265359;

vec3 fresnelSchlick(float cosTheta, vec3 F0) {
    return F0 + (1.0 - F0) * pow(clamp(1.0 - cosTheta, 0.0, 1.0), 5.0);
}

float distributionGGX(vec3 N, vec3 H, float roughness) {
    float a = roughness * roughness;
    float a2 = a * a;
    float NdotH = max(dot(N, H), 0.0);
    float denom = (NdotH * NdotH * (a2 - 1.0) + 1.0);
    return a2 / (PI * denom * denom);
}

float geometrySmith(float NdotV, float NdotL, float roughness) {
    float r = roughness + 1.0;
    float k = (r * r) / 8.0;
    float g1 = NdotV / (NdotV * (1.0 - k) + k);
    float g2 = NdotL / (NdotL * (1.0 - k) + k);
    return g1 * g2;
}

void fragment() {
    vec3 albedo = texture(albedoMap, UV).rgb;
    float metallic_val = texture(metallicRoughnessMap, UV).b * metallic;
    float roughness_val = texture(metallicRoughnessMap, UV).g * roughness;
    float ao = texture(aoMap, UV).r;
    vec3 N = normalize(NORMAL);
    vec3 V = normalize(VIEW);
    vec3 F0 = mix(vec3(0.04), albedo, metallic_val);
    vec3 Lo = vec3(0.0);
    vec3 L = normalize(LIGHT);
    vec3 H = normalize(V + L);
    float NdotV = max(dot(N, V), 0.0);
    float NdotL = max(dot(N, L), 0.0);
    vec3 F = fresnelSchlick(max(dot(H, V), 0.0), F0);
    float NDF = distributionGGX(N, H, roughness_val);
    float G = geometrySmith(NdotV, NdotL, roughness_val);
    vec3 numerator = NDF * G * F;
    float denominator = 4.0 * NdotV * NdotL + 0.0001;
    vec3 specular = numerator / denominator;
    vec3 kS = F;
    vec3 kD = (vec3(1.0) - kS) * (1.0 - metallic_val);
    Lo += (kD * albedo / PI + specular) * LIGHT_COLOR * NdotL;
    vec3 ambient = vec3(0.03) * albedo * ao;
    ALBEDO = ambient + Lo;
}"#
            .to_string(),
        },
        ShaderSnippet {
            name: "Perlin Noise".to_string(),
            language: "glsl".to_string(),
            description: "Classic Perlin noise function for procedural textures and terrain"
                .to_string(),
            code: r#"// Perlin Noise GLSL
vec2 fade(vec2 t) { return t * t * t * (t * (t * 6.0 - 15.0) + 10.0); }

float grad(int hash, float x, float y) {
    int h = hash & 7;
    float u = h < 4 ? x : y;
    float v = h < 4 ? y : x;
    return ((h & 1) == 0 ? u : -u) + ((h & 2) == 0 ? v : -v);
}

float perlin(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = fade(f);
    int ix = int(i.x) & 255;
    int iy = int(i.y) & 255;
    // Permutation table (simplified)
    int aa = (ix + iy) & 255;
    int ab = (ix + iy + 1) & 255;
    int ba = (ix + 1 + iy) & 255;
    int bb = (ix + 1 + iy + 1) & 255;
    float res = mix(
        mix(grad(aa, f.x, f.y), grad(ba, f.x - 1.0, f.y), u.x),
        mix(grad(ab, f.x, f.y - 1.0), grad(bb, f.x - 1.0, f.y - 1.0), u.x),
        u.y
    );
    return res * 0.5 + 0.5;
}

// Fractal Brownian Motion
float fbm(vec2 p, int octaves) {
    float value = 0.0;
    float amplitude = 0.5;
    float frequency = 1.0;
    for (int i = 0; i < octaves; i++) {
        value += amplitude * perlin(p * frequency);
        frequency *= 2.0;
        amplitude *= 0.5;
    }
    return value;
}

void fragment() {
    float noise = fbm(UV * 8.0, 6);
    ALBEDO = vec3(noise);
}"#
            .to_string(),
        },
        ShaderSnippet {
            name: "Screen-Space Blur".to_string(),
            language: "glsl".to_string(),
            description: "Gaussian blur post-processing effect".to_string(),
            code: r#"// Screen-Space Gaussian Blur (post-processing)
uniform sampler2D SCREEN_TEXTURE : hint_screen_texture, filter_linear_mipmap;
uniform float blur_amount : hint_range(0.0, 10.0) = 2.0;

void fragment() {
    vec2 size = 1.0 / vec2(textureSize(SCREEN_TEXTURE, 0));
    vec4 color = vec4(0.0);
    float total_weight = 0.0;

    // 3x3 Gaussian kernel
    float kernel[9] = float[9](
        1.0, 2.0, 1.0,
        2.0, 4.0, 2.0,
        1.0, 2.0, 1.0
    );

    int idx = 0;
    for (int x = -1; x <= 1; x++) {
        for (int y = -1; y <= 1; y++) {
            vec2 offset = vec2(float(x), float(y)) * size * blur_amount;
            float w = kernel[idx++];
            color += texture(SCREEN_TEXTURE, SCREEN_UV + offset) * w;
            total_weight += w;
        }
    }
    COLOR = color / total_weight;
}"#
            .to_string(),
        },
        ShaderSnippet {
            name: "Chromatic Aberration".to_string(),
            language: "gdshader".to_string(),
            description: "Chromatic aberration lens distortion effect for cameras".to_string(),
            code: r#"// Chromatic Aberration (GDShader)
shader_type canvas_item;

uniform sampler2D SCREEN_TEXTURE : hint_screen_texture, filter_linear_mipmap;
uniform float aberration_amount : hint_range(0.0, 0.02) = 0.005;

void fragment() {
    vec2 uv = SCREEN_UV;
    vec2 dir = uv - vec2(0.5);

    float r = texture(SCREEN_TEXTURE, uv + dir * aberration_amount).r;
    float g = texture(SCREEN_TEXTURE, uv).g;
    float b = texture(SCREEN_TEXTURE, uv - dir * aberration_amount).b;
    float a = texture(SCREEN_TEXTURE, uv).a;

    COLOR = vec4(r, g, b, a);
}"#
            .to_string(),
        },
        ShaderSnippet {
            name: "Cel Shading".to_string(),
            language: "gdshader".to_string(),
            description: "Cel/toon shading with hard light bands and outline".to_string(),
            code: r#"// Cel Shading (GDShader - spatial)
shader_type spatial;
render_mode unshaded;

uniform vec4 color : source_color = vec4(1.0, 0.5, 0.1, 1.0);
uniform vec4 outline_color : source_color = vec4(0.0, 0.0, 0.0, 1.0);
uniform float outline_width : hint_range(0.0, 0.1) = 0.02;
uniform int bands : hint_range(2, 8) = 4;

void vertex() {
    // Expand vertices slightly along normal for outline pass
    // (Use a second mesh pass for actual outline)
    VERTEX += NORMAL * outline_width;
}

void fragment() {
    vec3 normal = normalize(NORMAL);
    vec3 light_dir = normalize(vec3(0.5, 1.0, 0.3));
    float NdotL = max(dot(normal, light_dir), 0.0);

    // Quantize lighting into bands
    float band = floor(NdotL * float(bands)) / float(bands);

    // Add rim lighting
    vec3 view = normalize(VIEW);
    float rim = 1.0 - max(dot(normal, view), 0.0);
    rim = pow(rim, 3.0);
    rim = step(0.5, rim);

    ALBEDO = color.rgb * (band + 0.1) + vec3(rim) * 0.3;
}"#
            .to_string(),
        },
        ShaderSnippet {
            name: "Water Surface".to_string(),
            language: "gdshader".to_string(),
            description: "Animated water surface with normals and Fresnel".to_string(),
            code: r#"// Water Surface (GDShader - spatial)
shader_type spatial;

uniform vec4 water_color : source_color = vec4(0.0, 0.4, 0.7, 0.8);
uniform float wave_speed = 1.0;
uniform float wave_scale = 2.0;
uniform float wave_height = 0.3;
uniform sampler2D normal_map : hint_normal;

void vertex() {
    float time = TIME * wave_speed;
    float wave1 = sin(VERTEX.x * wave_scale + time) * wave_height;
    float wave2 = sin(VERTEX.z * wave_scale * 0.7 + time * 1.3) * wave_height * 0.5;
    VERTEX.y += wave1 + wave2;
    NORMAL = normalize(vec3(
        -cos(VERTEX.x * wave_scale + time) * wave_scale * wave_height,
        1.0,
        -cos(VERTEX.z * wave_scale * 0.7 + time * 1.3) * wave_scale * wave_height * 0.5
    ));
}

void fragment() {
    vec3 view = normalize(VIEW);
    float fresnel = pow(1.0 - max(dot(NORMAL, view), 0.0), 3.0);
    vec3 n = texture(normal_map, UV * 4.0 + vec2(TIME * 0.05)).rgb * 2.0 - 1.0;
    NORMAL_MAP = n;
    ALBEDO = water_color.rgb;
    ALPHA = mix(water_color.a * 0.4, 1.0, fresnel);
    ROUGHNESS = 0.05;
    METALLIC = 0.1;
}"#
            .to_string(),
        },
    ]
}

// ===== Texture Optimization =====

/// Optimize a texture file using oxipng (PNG) or cwebp (other formats).
#[tauri::command]
pub async fn optimize_texture(
    file_path: String,
    format: Option<String>,
) -> Result<OptimizeResult, String> {
    let path = Path::new(&file_path);
    if !path.exists() {
        return Err(format!("File not found: {}", file_path));
    }

    let original_size = std::fs::metadata(&file_path)
        .map_err(|e| format!("Failed to stat file: {}", e))?
        .len();

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let output_format = format.as_deref().unwrap_or(&ext);

    let (output_path, optimized_size) = match output_format {
        "png" => optimize_png(&file_path, original_size)?,
        _ => convert_to_webp(&file_path, path)?,
    };

    let savings_percent = if original_size > 0 {
        (1.0 - (optimized_size as f32 / original_size as f32)) * 100.0
    } else {
        0.0
    };

    Ok(OptimizeResult {
        original_size,
        optimized_size,
        output_path,
        savings_percent,
    })
}

fn optimize_png(file_path: &str, original_size: u64) -> Result<(String, u64), String> {
    // Try oxipng in-place
    let out = {
        let mut cmd = Command::new("oxipng");
        cmd.args(["-o", "4", "--strip", "all", file_path]);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
    };

    match out {
        Ok(o) if o.status.success() => {
            let new_size = std::fs::metadata(file_path)
                .map(|m| m.len())
                .unwrap_or(original_size);
            Ok((file_path.to_string(), new_size))
        }
        _ => {
            // oxipng not available — return as-is
            Ok((file_path.to_string(), original_size))
        }
    }
}

fn convert_to_webp(file_path: &str, path: &Path) -> Result<(String, u64), String> {
    let out_path = path.with_extension("webp");
    let out_path_str = out_path.to_string_lossy().to_string();

    let out = {
        let mut cmd = Command::new("cwebp");
        cmd.args(["-q", "80", file_path, "-o", &out_path_str]);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
    };

    match out {
        Ok(o) if o.status.success() => {
            let new_size = std::fs::metadata(&out_path_str)
                .map(|m| m.len())
                .unwrap_or(0);
            Ok((out_path_str, new_size))
        }
        _ => {
            // cwebp not available — return original
            let size = std::fs::metadata(file_path).map(|m| m.len()).unwrap_or(0);
            Ok((file_path.to_string(), size))
        }
    }
}

// ===== Godot Asset Listing =====

/// List Godot assets in a project directory, classified by type.
#[tauri::command]
pub async fn list_godot_assets(project_path: String) -> Result<Vec<GodotAsset>, String> {
    let root = Path::new(&project_path);
    if !root.exists() {
        return Err(format!("Project path does not exist: {}", project_path));
    }

    let mut assets = Vec::new();
    walk_godot_assets(root, root, &mut assets);
    Ok(assets)
}

fn walk_godot_assets(base: &Path, dir: &Path, assets: &mut Vec<GodotAsset>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || name == ".import" {
                continue;
            }
            walk_godot_assets(base, &path, assets);
        } else {
            let size = path.metadata().map(|m| m.len()).unwrap_or(0);
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_lowercase();

            let asset_type = classify_godot_asset(&ext);

            // Build res:// path relative to project root
            let rel = path
                .strip_prefix(base)
                .map(|p| format!("res://{}", p.to_string_lossy().replace('\\', "/")))
                .unwrap_or_else(|_| path.to_string_lossy().to_string());

            assets.push(GodotAsset {
                path: rel,
                asset_type,
                size,
            });
        }
    }
}

/// Connect to Godot's DAP-compatible debugger (default port 6007)
#[tauri::command]
pub async fn connect_godot_debugger(port: Option<u16>) -> Result<String, String> {
    let port = port.unwrap_or(6007);
    match std::net::TcpStream::connect(format!("127.0.0.1:{}", port)) {
        Ok(_) => Ok(format!("Connected to Godot debugger on port {}", port)),
        Err(e) => Err(format!("Cannot connect to Godot debugger on port {}: {}. Make sure Godot is running with --debug flag", port, e)),
    }
}

/// Validate a WGSL shader using naga-cli or wgsl-validator
#[tauri::command]
pub async fn validate_wgsl(shader_code: String) -> Result<WgslValidationResult, String> {
    // Write to temp file
    let tmp = std::env::temp_dir().join("shadow_ide_shader.wgsl");
    std::fs::write(&tmp, &shader_code).map_err(|e| e.to_string())?;

    // Try naga CLI
    let out = std::process::Command::new("naga")
        .args([tmp.to_str().unwrap_or("")])
        .output();

    let _ = std::fs::remove_file(&tmp);

    match out {
        Ok(o) => Ok(WgslValidationResult {
            valid: o.status.success(),
            errors: if o.status.success() {
                vec![]
            } else {
                String::from_utf8_lossy(&o.stderr)
                    .lines()
                    .map(str::to_string)
                    .collect()
            },
            tool: "naga".to_string(),
        }),
        Err(_) => {
            // naga not installed — do basic syntax check
            let errors = basic_wgsl_lint(&shader_code);
            Ok(WgslValidationResult {
                valid: errors.is_empty(),
                errors,
                tool: "basic-lint".to_string(),
            })
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct WgslValidationResult {
    pub valid: bool,
    pub errors: Vec<String>,
    pub tool: String,
}

fn basic_wgsl_lint(code: &str) -> Vec<String> {
    let mut errors = Vec::new();
    let mut brace_depth: i32 = 0;
    for (i, line) in code.lines().enumerate() {
        brace_depth += line.chars().filter(|&c| c == '{').count() as i32;
        brace_depth -= line.chars().filter(|&c| c == '}').count() as i32;
        if brace_depth < 0 {
            errors.push(format!("line {}: unexpected closing brace", i + 1));
            brace_depth = 0;
        }
    }
    if brace_depth != 0 {
        errors.push(format!("unclosed brace (depth {})", brace_depth));
    }
    // Check for required WGSL keywords
    if !code.contains("@fragment") && !code.contains("@vertex") && !code.contains("@compute") {
        errors.push(
            "WGSL shader must have at least one @fragment, @vertex, or @compute entry point"
                .to_string(),
        );
    }
    errors
}

/// Parse a .tscn Godot scene file and return basic info
#[tauri::command]
pub async fn parse_tscn_preview(file_path: String) -> Result<TscnPreview, String> {
    let content = std::fs::read_to_string(&file_path).map_err(|e| e.to_string())?;

    let mut node_count = 0;
    let mut root_type = String::new();
    let mut root_name = String::new();
    let mut script_path = None;

    for line in content.lines() {
        if line.starts_with("[node") {
            node_count += 1;
            if node_count == 1 {
                if let Some(name) = extract_attr(line, "name") {
                    root_name = name;
                }
                if let Some(ty) = extract_attr(line, "type") {
                    root_type = ty;
                }
            }
        }
        if line.contains("script = ExtResource") || line.starts_with("script =") {
            script_path = Some(line.to_string());
        }
    }

    Ok(TscnPreview {
        root_name,
        root_type,
        node_count,
        has_script: script_path.is_some(),
    })
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct TscnPreview {
    pub root_name: String,
    pub root_type: String,
    pub node_count: u32,
    pub has_script: bool,
}

fn extract_attr(line: &str, attr: &str) -> Option<String> {
    let needle = format!("{}=\"", attr);
    let start = line.find(&needle)? + needle.len();
    let end = line[start..].find('"')? + start;
    Some(line[start..end].to_string())
}

fn classify_godot_asset(ext: &str) -> String {
    match ext {
        "tscn" | "scn" => "scene",
        "gd" | "gdscript" => "script",
        "cs" => "script",
        "png" | "jpg" | "jpeg" | "webp" | "svg" | "bmp" | "tga" | "exr" | "hdr" => "texture",
        "wav" | "ogg" | "mp3" | "flac" => "audio",
        "gdshader" | "shader" | "glsl" | "vert" | "frag" => "shader",
        "tres" => "resource",
        "import" => "import",
        "ttf" | "otf" | "woff" | "woff2" | "fnt" => "font",
        "gltf" | "glb" | "obj" | "fbx" | "dae" | "blend" => "mesh",
        "godot" | "project" => "project",
        _ => "other",
    }
    .to_string()
}

// ===== ShadowEditor Project =====

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct ShadowProjectInfo {
    pub name: String,
    pub runtime: String,
    pub entry_scene: String,
    pub entry_scene_path: String,
    pub game_library_name: String,
    pub game_library_path: String,
    pub game_library_exists: bool,
    pub compiler: String,
    pub standard: String,
    pub include_dirs: Vec<String>,
    pub defines: Vec<String>,
    pub link_libs: Vec<String>,
    pub scenes: Vec<String>,
    pub source_file_count: usize,
    pub header_file_count: usize,
    pub has_reflection_json: bool,
    pub has_reflection_generated_cpp: bool,
    pub has_compile_commands: bool,
    pub build_system: String,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct ShadowSourceFile {
    pub path: String,
    pub kind: String,
    pub size_bytes: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct ShadowSceneComponent {
    pub component_type: String,
    pub fields: Vec<(String, String)>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct ShadowSceneEntity {
    pub id: String,
    pub name: String,
    pub components: Vec<ShadowSceneComponent>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct ShadowScene {
    pub scene_name: String,
    pub version: String,
    pub runtime: String,
    pub entities: Vec<ShadowSceneEntity>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct ShadowBuildResult {
    pub success: bool,
    pub output: String,
    pub duration_ms: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct ShadowSceneValidationIssue {
    pub severity: String,
    pub entity: String,
    pub component_type: String,
    pub message: String,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct ShadowSceneValidationReport {
    pub scene_path: String,
    pub issue_count: usize,
    pub issues: Vec<ShadowSceneValidationIssue>,
}

impl ShadowRuntimeSession {
    fn new(library_path: PathBuf) -> Self {
        Self {
            host: HotReloadHost::new(library_path),
            last_error: None,
            last_scene_path: None,
        }
    }
}

fn project_runtime_library_path(root: &Path, value: &toml::Value) -> PathBuf {
    let game_library_name = value
        .get("game_library_name")
        .and_then(|v| v.as_str())
        .unwrap_or("libgame.so");
    root.join("build").join(game_library_name)
}

fn project_entry_scene_path(root: &Path, value: &toml::Value) -> Option<PathBuf> {
    let entry_scene = value
        .get("entry_scene")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim();
    if entry_scene.is_empty() {
        None
    } else {
        Some(root.join(entry_scene))
    }
}

fn runtime_hot_reload_snapshot_path(root: &Path, scene_path: Option<&Path>) -> PathBuf {
    let scene_stem = scene_path
        .and_then(|path| path.file_stem())
        .and_then(|value| value.to_str())
        .unwrap_or("live_runtime_state");
    root.join(".shadoweditor")
        .join("runtime_state")
        .join(format!(
            "{}_hot_reload.shadow",
            slugify_scene_identifier(scene_stem)
        ))
}

fn runtime_preview_snapshot_path(root: &Path, scene_path: Option<&Path>) -> PathBuf {
    let scene_stem = scene_path
        .and_then(|path| path.file_stem())
        .and_then(|value| value.to_str())
        .unwrap_or("live_runtime_preview");
    root.join(".shadoweditor")
        .join("runtime_state")
        .join(format!(
            "{}_live_preview.shadow",
            slugify_scene_identifier(scene_stem)
        ))
}

fn workspace_doc_path(file_name: &str) -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for ancestor in manifest_dir.ancestors() {
        let candidate = ancestor.join(file_name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn command_exists_on_path(program: &str) -> Option<PathBuf> {
    let trimmed = program.trim();
    if trimmed.is_empty() {
        return None;
    }

    let explicit = PathBuf::from(trimmed);
    if explicit.is_absolute() || trimmed.contains('\\') || trimmed.contains('/') {
        if explicit.is_file() {
            return Some(explicit);
        }
        #[cfg(target_os = "windows")]
        {
            if explicit.extension().is_none() {
                let with_exe = PathBuf::from(format!("{trimmed}.exe"));
                if with_exe.is_file() {
                    return Some(with_exe);
                }
            }
        }
        return None;
    }

    let path_value = std::env::var_os("PATH")?;
    let candidates: Vec<String> = {
        #[cfg(target_os = "windows")]
        {
            if Path::new(trimmed).extension().is_some() {
                vec![trimmed.to_string()]
            } else {
                vec![
                    trimmed.to_string(),
                    format!("{trimmed}.exe"),
                    format!("{trimmed}.cmd"),
                    format!("{trimmed}.bat"),
                ]
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            vec![trimmed.to_string()]
        }
    };

    for dir in std::env::split_paths(&path_value) {
        for candidate in &candidates {
            let path = dir.join(candidate);
            if path.is_file() {
                return Some(path);
            }
        }
    }

    None
}

fn find_latest_subdirectory(root: &Path) -> Option<PathBuf> {
    let mut children: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    children.sort();
    children.pop()
}

fn find_visual_studio_vcvars64() -> Option<PathBuf> {
    if !cfg!(target_os = "windows") {
        return None;
    }

    let roots = [
        r"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools",
        r"C:\Program Files\Microsoft Visual Studio\2022\BuildTools",
        r"C:\Program Files (x86)\Microsoft Visual Studio\2022\Community",
        r"C:\Program Files\Microsoft Visual Studio\2022\Community",
        r"C:\Program Files (x86)\Microsoft Visual Studio\2022\Professional",
        r"C:\Program Files\Microsoft Visual Studio\2022\Professional",
        r"C:\Program Files (x86)\Microsoft Visual Studio\2022\Enterprise",
        r"C:\Program Files\Microsoft Visual Studio\2022\Enterprise",
        r"C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools",
        r"C:\Program Files\Microsoft Visual Studio\2019\BuildTools",
        r"C:\Program Files (x86)\Microsoft Visual Studio\2019\Community",
        r"C:\Program Files\Microsoft Visual Studio\2019\Community",
    ];

    for root in roots {
        let candidate = PathBuf::from(root).join("VC/Auxiliary/Build/vcvars64.bat");
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    None
}

fn find_visual_studio_cl() -> Option<(PathBuf, PathBuf)> {
    if !cfg!(target_os = "windows") {
        return None;
    }

    let installs = [
        r"C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools",
        r"C:\Program Files\Microsoft Visual Studio\2022\BuildTools",
        r"C:\Program Files (x86)\Microsoft Visual Studio\2022\Community",
        r"C:\Program Files\Microsoft Visual Studio\2022\Community",
        r"C:\Program Files (x86)\Microsoft Visual Studio\2022\Professional",
        r"C:\Program Files\Microsoft Visual Studio\2022\Professional",
        r"C:\Program Files (x86)\Microsoft Visual Studio\2022\Enterprise",
        r"C:\Program Files\Microsoft Visual Studio\2022\Enterprise",
        r"C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools",
        r"C:\Program Files\Microsoft Visual Studio\2019\BuildTools",
        r"C:\Program Files (x86)\Microsoft Visual Studio\2019\Community",
        r"C:\Program Files\Microsoft Visual Studio\2019\Community",
    ];

    for install in installs {
        let root = PathBuf::from(install);
        let vcvars = root.join("VC/Auxiliary/Build/vcvars64.bat");
        let msvc_root = root.join("VC/Tools/MSVC");
        let Some(version_dir) = find_latest_subdirectory(&msvc_root) else {
            continue;
        };
        let cl = version_dir.join("bin/Hostx64/x64/cl.exe");
        if cl.is_file() && vcvars.is_file() {
            return Some((cl, vcvars));
        }
    }

    None
}

fn resolve_shadow_compiler(requested: &str) -> Result<ShadowCompilerResolution, String> {
    let requested = requested.trim();

    let resolution_from_path = |path: PathBuf| ShadowCompilerResolution {
        display_name: path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string()),
        style: if path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| {
                name.eq_ignore_ascii_case("cl.exe")
                    || name.eq_ignore_ascii_case("cl")
                    || name.eq_ignore_ascii_case("clang-cl.exe")
                    || name.eq_ignore_ascii_case("clang-cl")
            })
            .unwrap_or(false)
        {
            ShadowCompilerStyle::Msvc
        } else {
            ShadowCompilerStyle::GnuLike
        },
        setup_script: find_visual_studio_vcvars64(),
        executable: path,
    };

    if let Some(path) = command_exists_on_path(requested) {
        let mut resolved = resolution_from_path(path);
        if resolved.style != ShadowCompilerStyle::Msvc {
            resolved.setup_script = None;
        }
        return Ok(resolved);
    }

    if cfg!(target_os = "windows") {
        for fallback in ["clang++", "clang-cl", "g++"] {
            if let Some(path) = command_exists_on_path(fallback) {
                let mut resolved = resolution_from_path(path);
                if fallback.eq_ignore_ascii_case("clang-cl") {
                    resolved.style = ShadowCompilerStyle::Msvc;
                    resolved.setup_script = find_visual_studio_vcvars64();
                } else if resolved.style != ShadowCompilerStyle::Msvc {
                    resolved.setup_script = None;
                }
                return Ok(resolved);
            }
        }

        if let Some((cl, vcvars)) = find_visual_studio_cl() {
            return Ok(ShadowCompilerResolution {
                executable: cl,
                display_name: "cl.exe (Visual Studio Build Tools)".to_string(),
                style: ShadowCompilerStyle::Msvc,
                setup_script: Some(vcvars),
            });
        }
    } else {
        for fallback in ["clang++", "c++", "g++"] {
            if let Some(path) = command_exists_on_path(fallback) {
                let mut resolved = resolution_from_path(path);
                resolved.setup_script = None;
                return Ok(resolved);
            }
        }
    }

    Err(format!(
        "No usable C++ compiler was found. Requested `{}` but it is not available on PATH, and no supported fallback compiler was detected.",
        if requested.is_empty() { "clang++" } else { requested }
    ))
}

fn msvc_standard_flag(standard: &str) -> &'static str {
    match standard.trim().to_ascii_lowercase().as_str() {
        "c++17" | "gnu++17" => "/std:c++17",
        "c++20" | "gnu++20" => "/std:c++20",
        _ => "/std:c++latest",
    }
}

fn quote_cmd_arg(argument: &str) -> String {
    if argument.is_empty() {
        return "\"\"".to_string();
    }
    if argument
        .chars()
        .any(|ch| ch.is_whitespace() || matches!(ch, '"' | '&' | '(' | ')'))
    {
        format!("\"{}\"", argument.replace('"', "\\\""))
    } else {
        argument.to_string()
    }
}

fn project_build_system(root: &Path) -> String {
    if root.join("build/build.ninja").exists() {
        "ninja".to_string()
    } else {
        "direct-native".to_string()
    }
}

fn ensure_runtime_session<'a>(
    sessions: &'a mut HashMap<String, ShadowRuntimeSession>,
    project_path: &str,
    library_path: PathBuf,
) -> &'a mut ShadowRuntimeSession {
    let recreate = sessions
        .get(project_path)
        .map(|session| session.host.library_path() != library_path.as_path())
        .unwrap_or(true);
    if recreate {
        sessions.insert(
            project_path.to_string(),
            ShadowRuntimeSession::new(library_path),
        );
    }
    sessions
        .get_mut(project_path)
        .expect("runtime session inserted")
}

fn runtime_status_from_session(
    project_path: &str,
    entry_scene_path: Option<&Path>,
    session: &ShadowRuntimeSession,
) -> ShadowRuntimeStatus {
    ShadowRuntimeStatus {
        project_path: project_path.to_string(),
        library_path: session
            .host
            .library_path()
            .to_string_lossy()
            .replace('\\', "/"),
        library_exists: session.host.library_path().exists(),
        is_live: session.host.is_live(),
        status_line: session.host.status_line(),
        frame_index: session.host.frame_index(),
        component_count: session.host.component_count(),
        entity_count: session.host.entity_count(),
        entry_scene_path: entry_scene_path
            .map(|path| path.to_string_lossy().replace('\\', "/"))
            .unwrap_or_default(),
        last_scene_path: session.last_scene_path.clone().unwrap_or_default(),
        last_error: session.last_error.clone(),
    }
}

#[tauri::command]
pub async fn shadow_runtime_status(
    project_path: String,
    runtime_state: tauri::State<'_, ShadowRuntimeState>,
) -> Result<ShadowRuntimeStatus, String> {
    let root = Path::new(&project_path);
    let value = load_shadow_project_config(root)?;
    let library_path = project_runtime_library_path(root, &value);
    let entry_scene_path = project_entry_scene_path(root, &value);
    let mut sessions = runtime_state
        .inner()
        .sessions
        .lock()
        .map_err(|_| "runtime state lock poisoned".to_string())?;
    let session = ensure_runtime_session(&mut sessions, &project_path, library_path);
    Ok(runtime_status_from_session(
        &project_path,
        entry_scene_path.as_deref(),
        session,
    ))
}

#[tauri::command]
pub async fn shadow_runtime_load(
    project_path: String,
    load_entry_scene: Option<bool>,
    runtime_state: tauri::State<'_, ShadowRuntimeState>,
) -> Result<ShadowRuntimeStatus, String> {
    let root = Path::new(&project_path);
    let value = load_shadow_project_config(root)?;
    let library_path = project_runtime_library_path(root, &value);
    let entry_scene_path = project_entry_scene_path(root, &value);
    let load_entry_scene = load_entry_scene.unwrap_or(true);

    let mut sessions = runtime_state
        .inner()
        .sessions
        .lock()
        .map_err(|_| "runtime state lock poisoned".to_string())?;
    let session = ensure_runtime_session(&mut sessions, &project_path, library_path);

    session.last_error = None;
    if let Err(err) = session.host.load_if_present() {
        session.last_error = Some(err.to_string());
        return Ok(runtime_status_from_session(
            &project_path,
            entry_scene_path.as_deref(),
            session,
        ));
    }

    if load_entry_scene {
        if let Some(scene_path) = entry_scene_path.as_ref().filter(|path| path.exists()) {
            match session.host.load_scene(scene_path) {
                Ok(()) => {
                    session.last_scene_path = Some(scene_path.to_string_lossy().replace('\\', "/"));
                }
                Err(err) => {
                    session.last_error = Some(err.to_string());
                }
            }
        }
    }

    Ok(runtime_status_from_session(
        &project_path,
        entry_scene_path.as_deref(),
        session,
    ))
}

#[tauri::command]
pub async fn shadow_runtime_stop(
    project_path: String,
    runtime_state: tauri::State<'_, ShadowRuntimeState>,
) -> Result<ShadowRuntimeStatus, String> {
    let root = Path::new(&project_path);
    let value = load_shadow_project_config(root)?;
    let library_path = project_runtime_library_path(root, &value);
    let entry_scene_path = project_entry_scene_path(root, &value);

    let mut sessions = runtime_state
        .inner()
        .sessions
        .lock()
        .map_err(|_| "runtime state lock poisoned".to_string())?;
    let session = ensure_runtime_session(&mut sessions, &project_path, library_path);
    session.host.stop_session();
    session.last_error = None;
    Ok(runtime_status_from_session(
        &project_path,
        entry_scene_path.as_deref(),
        session,
    ))
}

#[tauri::command]
pub async fn shadow_runtime_step(
    project_path: String,
    delta_time: Option<f32>,
    runtime_state: tauri::State<'_, ShadowRuntimeState>,
) -> Result<ShadowRuntimeStatus, String> {
    let root = Path::new(&project_path);
    let value = load_shadow_project_config(root)?;
    let library_path = project_runtime_library_path(root, &value);
    let entry_scene_path = project_entry_scene_path(root, &value);
    let delta_time = delta_time.unwrap_or(1.0 / 60.0).max(0.0);

    let mut sessions = runtime_state
        .inner()
        .sessions
        .lock()
        .map_err(|_| "runtime state lock poisoned".to_string())?;
    let session = ensure_runtime_session(&mut sessions, &project_path, library_path);
    if !session.host.is_live() {
        session.last_error = Some("Runtime is not loaded. Load it before stepping.".to_string());
        return Ok(runtime_status_from_session(
            &project_path,
            entry_scene_path.as_deref(),
            session,
        ));
    }

    session.last_error = None;
    if let Err(err) = session.host.update(delta_time) {
        session.last_error = Some(err.to_string());
    }
    Ok(runtime_status_from_session(
        &project_path,
        entry_scene_path.as_deref(),
        session,
    ))
}

#[tauri::command]
pub async fn shadow_runtime_save_scene(
    project_path: String,
    scene_path: Option<String>,
    runtime_state: tauri::State<'_, ShadowRuntimeState>,
) -> Result<ShadowRuntimeStatus, String> {
    let root = Path::new(&project_path);
    let value = load_shadow_project_config(root)?;
    let library_path = project_runtime_library_path(root, &value);
    let entry_scene_path = project_entry_scene_path(root, &value);
    let target_scene_path = if let Some(path) = scene_path.filter(|path| !path.trim().is_empty()) {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            root.join(path)
        }
    } else {
        entry_scene_path
            .clone()
            .ok_or_else(|| "No entry scene configured in .shadow_project.toml".to_string())?
    };

    let mut sessions = runtime_state
        .inner()
        .sessions
        .lock()
        .map_err(|_| "runtime state lock poisoned".to_string())?;
    let session = ensure_runtime_session(&mut sessions, &project_path, library_path);
    if !session.host.is_live() {
        session.last_error =
            Some("Runtime is not loaded. Load it before saving scene state.".to_string());
        return Ok(runtime_status_from_session(
            &project_path,
            entry_scene_path.as_deref(),
            session,
        ));
    }

    session.last_error = None;
    match session.host.save_scene(&target_scene_path) {
        Ok(()) => {
            session.last_scene_path = Some(target_scene_path.to_string_lossy().replace('\\', "/"));
        }
        Err(err) => {
            session.last_error = Some(err.to_string());
        }
    }
    Ok(runtime_status_from_session(
        &project_path,
        entry_scene_path.as_deref(),
        session,
    ))
}

#[tauri::command]
pub async fn shadow_runtime_capture_scene(
    project_path: String,
    runtime_state: tauri::State<'_, ShadowRuntimeState>,
) -> Result<ShadowScene, String> {
    let root = Path::new(&project_path);
    let value = load_shadow_project_config(root)?;
    let library_path = project_runtime_library_path(root, &value);
    let entry_scene_path = project_entry_scene_path(root, &value);
    let snapshot_path = runtime_preview_snapshot_path(root, entry_scene_path.as_deref());
    if let Some(parent) = snapshot_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("Failed to create runtime preview directory: {}", error))?;
    }

    let mut sessions = runtime_state
        .inner()
        .sessions
        .lock()
        .map_err(|_| "runtime state lock poisoned".to_string())?;
    let session = ensure_runtime_session(&mut sessions, &project_path, library_path);
    if !session.host.is_live() {
        session.last_error = Some(
            "Runtime is not loaded. Load it before capturing a live scene snapshot.".to_string(),
        );
        return Err("Runtime is not loaded.".to_string());
    }

    session.last_error = None;
    session.host.save_scene(&snapshot_path).map_err(|error| {
        session.last_error = Some(error.to_string());
        error.to_string()
    })?;

    let content = std::fs::read_to_string(&snapshot_path).map_err(|error| {
        let message = format!(
            "Failed to read runtime preview scene {}: {}",
            snapshot_path.display(),
            error
        );
        session.last_error = Some(message.clone());
        message
    })?;
    let value: toml::Value = toml::from_str(&content).map_err(|error| {
        let message = format!(
            "Failed to parse runtime preview scene {}: {}",
            snapshot_path.display(),
            error
        );
        session.last_error = Some(message.clone());
        message
    })?;
    Ok(parse_shadow_scene_value(&value))
}

#[tauri::command]
pub async fn shadow_load_planengine_docs() -> Result<ShadowPlanengineDocs, String> {
    let plan_path = workspace_doc_path("planengine.md")
        .ok_or_else(|| "Could not locate planengine.md from the current workspace.".to_string())?;

    let plan_markdown = std::fs::read_to_string(&plan_path)
        .map_err(|error| format!("Failed to read {}: {}", plan_path.display(), error))?;

    let (finish_available, finish_path, finish_markdown) = match workspace_doc_path("finish.md") {
        Some(path) => {
            let markdown = std::fs::read_to_string(&path)
                .map_err(|error| format!("Failed to read {}: {}", path.display(), error))?;
            (true, path.to_string_lossy().replace('\\', "/"), markdown)
        }
        None => (false, String::new(), String::new()),
    };

    Ok(ShadowPlanengineDocs {
        plan_path: plan_path.to_string_lossy().replace('\\', "/"),
        finish_path,
        plan_markdown,
        finish_markdown,
        finish_available,
    })
}

/// Read a .shadow_project.toml and return project info including all scene files.
#[tauri::command]
pub async fn shadow_get_project_info(project_path: String) -> Result<ShadowProjectInfo, String> {
    let root = Path::new(&project_path);
    let value = load_shadow_project_config(root)?;

    let name = value
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let runtime = value
        .get("runtime")
        .and_then(|v| v.as_str())
        .unwrap_or("cpp23")
        .to_string();
    let entry_scene = value
        .get("entry_scene")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let entry_scene_path = normalize_project_relative_path(root, root.join(&entry_scene));
    let game_library_name = value
        .get("game_library_name")
        .and_then(|v| v.as_str())
        .unwrap_or("libgame.so")
        .to_string();
    let build_output_path = root.join("build").join(&game_library_name);
    let fallback_output_path = root.join(&game_library_name);
    let resolved_output = if build_output_path.exists() {
        build_output_path
    } else {
        fallback_output_path
    };
    let game_library_path = normalize_project_relative_path(root, resolved_output.clone());
    let game_library_exists = resolved_output.exists();

    let build = value.get("build");
    let configured_compiler = build
        .and_then(|b| b.get("compiler"))
        .and_then(|v| v.as_str())
        .unwrap_or("clang++")
        .to_string();
    let compiler = resolve_shadow_compiler(&configured_compiler)
        .map(|resolved| resolved.display_name)
        .unwrap_or(configured_compiler);
    let standard = build
        .and_then(|b| b.get("standard"))
        .and_then(|v| v.as_str())
        .unwrap_or("c++23")
        .to_string();
    let include_dirs = build
        .and_then(|b| b.get("include_dirs"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let defines = build
        .and_then(|b| b.get("defines"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let link_libs = build
        .and_then(|b| b.get("link_libs"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    // Find all .shadow scene files recursively in scenes/
    let scenes = collect_scene_files(root);
    let mut source_files = Vec::new();
    let mut header_files = Vec::new();
    collect_cpp_sources(&root.join("src"), &mut source_files);
    collect_cpp_sources(&root.join("game"), &mut source_files);
    collect_cpp_headers(&root.join("src"), &mut header_files);
    collect_cpp_headers(&root.join("game"), &mut header_files);

    let has_reflection_json = root.join(".shadoweditor/shadow_reflect.json").exists();
    let has_reflection_generated_cpp = root
        .join(".shadoweditor/shadow_reflect_generated.cpp")
        .exists();
    let has_compile_commands = root.join("compile_commands.json").exists();
    let build_system = project_build_system(root);

    Ok(ShadowProjectInfo {
        name,
        runtime,
        entry_scene,
        entry_scene_path,
        game_library_name,
        game_library_path,
        game_library_exists,
        compiler,
        standard,
        include_dirs,
        defines,
        link_libs,
        scenes,
        source_file_count: source_files.len(),
        header_file_count: header_files.len(),
        has_reflection_json,
        has_reflection_generated_cpp,
        has_compile_commands,
        build_system,
    })
}

fn collect_scene_files(root: &Path) -> Vec<String> {
    let scenes_dir = root.join("scenes");
    let mut scene_files = Vec::new();
    collect_files_with_extensions(&scenes_dir, &["shadow"], &mut scene_files);
    scene_files.sort();
    scene_files
        .into_iter()
        .map(|path| normalize_project_relative_path(root, path))
        .collect()
}

fn parse_shadow_scene_value(value: &toml::Value) -> ShadowScene {
    let scene_table = value.get("scene");
    let scene_name = scene_table
        .and_then(|s| s.get("name"))
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_string();
    let version = scene_table
        .and_then(|s| s.get("version"))
        .and_then(|v| v.as_str())
        .unwrap_or("1.0")
        .to_string();
    let runtime = scene_table
        .and_then(|s| s.get("runtime"))
        .and_then(|v| v.as_str())
        .unwrap_or("cpp23")
        .to_string();

    let entities = value
        .get("entity")
        .and_then(|e| e.as_array())
        .map(|arr| arr.iter().map(parse_scene_entity).collect())
        .unwrap_or_default();

    ShadowScene {
        scene_name,
        version,
        runtime,
        entities,
    }
}

/// Parse a .shadow scene file (TOML) and return its entities and components.
#[tauri::command]
pub async fn shadow_parse_scene(scene_path: String) -> Result<ShadowScene, String> {
    let content =
        std::fs::read_to_string(&scene_path).map_err(|e| format!("Failed to read scene: {}", e))?;
    let value: toml::Value =
        toml::from_str(&content).map_err(|e| format!("Failed to parse scene TOML: {}", e))?;
    Ok(parse_shadow_scene_value(&value))
}

fn parse_scene_entity(entity: &toml::Value) -> ShadowSceneEntity {
    let id = entity
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let name = entity
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let components = entity
        .get("component")
        .and_then(|c| c.as_array())
        .map(|arr| arr.iter().map(parse_scene_component).collect())
        .unwrap_or_default();
    ShadowSceneEntity {
        id,
        name,
        components,
    }
}

fn parse_scene_component(comp: &toml::Value) -> ShadowSceneComponent {
    let component_type = comp
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_string();
    let fields = if let Some(table) = comp.as_table() {
        table
            .iter()
            .filter(|(k, _)| k.as_str() != "type")
            .map(|(k, v)| (k.clone(), toml_value_to_string(v)))
            .collect()
    } else {
        Vec::new()
    };
    ShadowSceneComponent {
        component_type,
        fields,
    }
}

fn toml_value_to_string(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        toml::Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(toml_value_to_string).collect();
            format!("[{}]", items.join(", "))
        }
        _ => value.to_string(),
    }
}

#[tauri::command]
pub async fn shadow_validate_scene(
    project_path: String,
    scene_path: Option<String>,
) -> Result<ShadowSceneValidationReport, String> {
    let root = Path::new(&project_path);
    let config = load_shadow_project_config(root)?;
    let resolved_scene_path = if let Some(path) = scene_path.filter(|path| !path.trim().is_empty())
    {
        let path = PathBuf::from(path);
        if path.is_absolute() {
            path
        } else {
            root.join(path)
        }
    } else {
        project_entry_scene_path(root, &config)
            .ok_or_else(|| "No entry scene configured in .shadow_project.toml".to_string())?
    };

    let content = std::fs::read_to_string(&resolved_scene_path).map_err(|e| {
        format!(
            "Failed to read scene {}: {}",
            resolved_scene_path.display(),
            e
        )
    })?;
    let value: toml::Value =
        toml::from_str(&content).map_err(|e| format!("Failed to parse scene TOML: {}", e))?;

    let mut issues = Vec::new();
    let reflection_schema = load_reflection_schema(root)?;
    if reflection_schema.is_empty() {
        issues.push(ShadowSceneValidationIssue {
            severity: "info".to_string(),
            entity: "Scene".to_string(),
            component_type: "Reflection".to_string(),
            message:
                "No reflection metadata found. Run the header tool to validate component fields."
                    .to_string(),
        });
        return Ok(ShadowSceneValidationReport {
            scene_path: resolved_scene_path.to_string_lossy().replace('\\', "/"),
            issue_count: issues.len(),
            issues,
        });
    }

    if let Some(entities) = value.get("entity").and_then(|items| items.as_array()) {
        for entity in entities {
            let entity_name = entity
                .get("name")
                .and_then(|item| item.as_str())
                .or_else(|| entity.get("id").and_then(|item| item.as_str()))
                .unwrap_or("(unnamed)")
                .to_string();

            if let Some(components) = entity.get("component").and_then(|items| items.as_array()) {
                for component in components {
                    let component_type = component
                        .get("type")
                        .and_then(|item| item.as_str())
                        .unwrap_or("Unknown")
                        .to_string();
                    let Some(component_table) = component.as_table() else {
                        continue;
                    };
                    let Some(expected_properties) = reflection_schema.get(&component_type) else {
                        issues.push(ShadowSceneValidationIssue {
                            severity: "error".to_string(),
                            entity: entity_name.clone(),
                            component_type: component_type.clone(),
                            message: format!(
                                "Component '{}' is not present in shadow_reflect.json.",
                                component_type
                            ),
                        });
                        continue;
                    };

                    for property_name in expected_properties {
                        if !component_table.contains_key(property_name) {
                            // Transform fields have sensible defaults, so missing ones are just info
                            let severity = if component_type == "Transform" {
                                "info"
                            } else {
                                "warning"
                            };
                            issues.push(ShadowSceneValidationIssue {
                                severity: severity.to_string(),
                                entity: entity_name.clone(),
                                component_type: component_type.clone(),
                                message: format!(
                                    "Missing field '{}' (will use default).",
                                    property_name
                                ),
                            });
                        }
                    }

                    for key in component_table.keys() {
                        if key == "type" {
                            continue;
                        }
                        if !expected_properties.contains(key) {
                            issues.push(ShadowSceneValidationIssue {
                                severity: "info".to_string(),
                                entity: entity_name.clone(),
                                component_type: component_type.clone(),
                                message: format!(
                                    "Scene contains extra field '{}' which will be preserved as raw TOML.",
                                    key
                                ),
                            });
                        }
                    }
                }
            }
        }
    }

    Ok(ShadowSceneValidationReport {
        scene_path: resolved_scene_path.to_string_lossy().replace('\\', "/"),
        issue_count: issues.len(),
        issues,
    })
}

#[tauri::command]
pub async fn shadow_scene_add_entity(
    project_path: String,
    name: Option<String>,
    scene_path: Option<String>,
    runtime_state: tauri::State<'_, ShadowRuntimeState>,
) -> Result<ShadowScene, String> {
    let root = Path::new(&project_path);
    let (resolved_scene_path, mut doc) = load_scene_document(root, scene_path)?;
    let entities = scene_entities_mut(&mut doc)?;
    let existing_ids = entities
        .iter()
        .filter_map(|entity| {
            entity
                .get("id")
                .and_then(|value| value.as_str())
                .map(String::from)
        })
        .collect::<HashSet<_>>();
    let entity_name = name
        .unwrap_or_else(|| format!("Entity {}", entities.len() + 1))
        .trim()
        .to_string();
    let entity_id = generate_scene_entity_id(&entity_name, &existing_ids);

    let mut entity = toml::map::Map::new();
    entity.insert("id".to_string(), toml::Value::String(entity_id.clone()));
    entity.insert("name".to_string(), toml::Value::String(entity_name));

    let default_components = if reflection_component_names(root).contains("Transform") {
        vec![build_component_value(root, "Transform")]
    } else {
        Vec::new()
    };
    if !default_components.is_empty() {
        entity.insert(
            "component".to_string(),
            toml::Value::Array(default_components),
        );
    }

    entities.push(toml::Value::Table(entity));
    save_scene_document(&resolved_scene_path, &doc)?;
    sync_live_scene_if_needed(
        &project_path,
        root,
        &resolved_scene_path,
        &doc,
        &runtime_state,
        LiveSceneMutation::AddEntity {
            entity_id: entity_id.clone(),
        },
    );
    Ok(parse_shadow_scene_value(&doc))
}

#[tauri::command]
pub async fn shadow_scene_remove_entity(
    project_path: String,
    entity_id: String,
    scene_path: Option<String>,
    runtime_state: tauri::State<'_, ShadowRuntimeState>,
) -> Result<ShadowScene, String> {
    let root = Path::new(&project_path);
    let (resolved_scene_path, mut doc) = load_scene_document(root, scene_path)?;
    let entities = scene_entities_mut(&mut doc)?;
    let original_len = entities.len();
    entities.retain(|entity| {
        entity.get("id").and_then(|value| value.as_str()) != Some(entity_id.as_str())
    });
    if entities.len() == original_len {
        return Err(format!(
            "Entity '{}' was not found in the scene.",
            entity_id
        ));
    }
    save_scene_document(&resolved_scene_path, &doc)?;
    sync_live_scene_if_needed(
        &project_path,
        root,
        &resolved_scene_path,
        &doc,
        &runtime_state,
        LiveSceneMutation::RemoveEntity {
            entity_id: entity_id.clone(),
        },
    );
    Ok(parse_shadow_scene_value(&doc))
}

#[tauri::command]
pub async fn shadow_scene_set_entity_name(
    project_path: String,
    entity_id: String,
    name: String,
    scene_path: Option<String>,
    runtime_state: tauri::State<'_, ShadowRuntimeState>,
) -> Result<ShadowScene, String> {
    let root = Path::new(&project_path);
    let (resolved_scene_path, mut doc) = load_scene_document(root, scene_path)?;
    let entities = scene_entities_mut(&mut doc)?;
    let entity = find_scene_entity_mut(entities, &entity_id)?;
    entity.insert(
        "name".to_string(),
        toml::Value::String(name.trim().to_string()),
    );
    save_scene_document(&resolved_scene_path, &doc)?;
    sync_live_scene_if_needed(
        &project_path,
        root,
        &resolved_scene_path,
        &doc,
        &runtime_state,
        LiveSceneMutation::RenameEntity {
            entity_id: entity_id.clone(),
        },
    );
    Ok(parse_shadow_scene_value(&doc))
}

#[tauri::command]
pub async fn shadow_scene_add_component(
    project_path: String,
    entity_id: String,
    component_type: String,
    scene_path: Option<String>,
    runtime_state: tauri::State<'_, ShadowRuntimeState>,
) -> Result<ShadowScene, String> {
    let root = Path::new(&project_path);
    let (resolved_scene_path, mut doc) = load_scene_document(root, scene_path)?;
    let entities = scene_entities_mut(&mut doc)?;
    let entity = find_scene_entity_mut(entities, &entity_id)?;
    let components = component_values_mut(entity)?;
    if components.iter().any(|component| {
        component
            .get("type")
            .and_then(|value| value.as_str())
            .map(|value| value == component_type)
            .unwrap_or(false)
    }) {
        return Err(format!(
            "Entity '{}' already has a '{}' component.",
            entity_id, component_type
        ));
    }
    components.push(build_component_value(root, &component_type));
    save_scene_document(&resolved_scene_path, &doc)?;
    sync_live_scene_if_needed(
        &project_path,
        root,
        &resolved_scene_path,
        &doc,
        &runtime_state,
        LiveSceneMutation::AddComponent {
            entity_id: entity_id.clone(),
            component_type: component_type.clone(),
        },
    );
    Ok(parse_shadow_scene_value(&doc))
}

#[tauri::command]
pub async fn shadow_scene_remove_component(
    project_path: String,
    entity_id: String,
    component_type: String,
    scene_path: Option<String>,
    runtime_state: tauri::State<'_, ShadowRuntimeState>,
) -> Result<ShadowScene, String> {
    let root = Path::new(&project_path);
    let (resolved_scene_path, mut doc) = load_scene_document(root, scene_path)?;
    let entities = scene_entities_mut(&mut doc)?;
    let entity = find_scene_entity_mut(entities, &entity_id)?;
    let components = component_values_mut(entity)?;
    let original_len = components.len();
    components.retain(|component| {
        component.get("type").and_then(|value| value.as_str()) != Some(component_type.as_str())
    });
    if components.len() == original_len {
        return Err(format!(
            "Entity '{}' does not contain a '{}' component.",
            entity_id, component_type
        ));
    }
    save_scene_document(&resolved_scene_path, &doc)?;
    sync_live_scene_if_needed(
        &project_path,
        root,
        &resolved_scene_path,
        &doc,
        &runtime_state,
        LiveSceneMutation::RemoveComponent {
            entity_id: entity_id.clone(),
            component_type: component_type.clone(),
        },
    );
    Ok(parse_shadow_scene_value(&doc))
}

#[tauri::command]
pub async fn shadow_scene_set_component_field(
    project_path: String,
    entity_id: String,
    component_type: String,
    field_name: String,
    value: String,
    scene_path: Option<String>,
    runtime_state: tauri::State<'_, ShadowRuntimeState>,
) -> Result<ShadowScene, String> {
    let root = Path::new(&project_path);
    let (resolved_scene_path, mut doc) = load_scene_document(root, scene_path)?;
    let entities = scene_entities_mut(&mut doc)?;
    let entity = find_scene_entity_mut(entities, &entity_id)?;
    let components = component_values_mut(entity)?;
    let component = find_scene_component_mut(components, &component_type)?;
    component.insert(field_name.to_string(), parse_editor_field_value(&value));
    save_scene_document(&resolved_scene_path, &doc)?;
    sync_live_scene_if_needed(
        &project_path,
        root,
        &resolved_scene_path,
        &doc,
        &runtime_state,
        LiveSceneMutation::SetComponentField {
            entity_id: entity_id.clone(),
            component_type: component_type.clone(),
        },
    );
    Ok(parse_shadow_scene_value(&doc))
}

/// Build a rich AI context string from a ShadowEditor project for injection into LLM prompts.
/// Includes: project config, entry scene entities/components, reflection JSON, and last build output.
#[tauri::command]
pub async fn shadow_get_ai_context(root_path: String) -> Result<String, String> {
    let root = Path::new(&root_path);
    let config_path = root.join(".shadow_project.toml");

    if !config_path.exists() {
        return Err(format!("No .shadow_project.toml found at {}", root_path));
    }

    let mut ctx = String::new();

    // Project config
    if let Ok(config_content) = std::fs::read_to_string(&config_path) {
        ctx.push_str("## ShadowEditor Project Config\n```toml\n");
        ctx.push_str(&config_content);
        ctx.push_str("```\n\n");
    }

    // Entry scene
    if let Ok(config_str) = std::fs::read_to_string(&config_path) {
        if let Ok(value) = toml::from_str::<toml::Value>(&config_str) {
            if let Some(entry) = value.get("entry_scene").and_then(|v| v.as_str()) {
                let scene_path = root.join(entry);
                if let Ok(scene_content) = std::fs::read_to_string(&scene_path) {
                    ctx.push_str(&format!("## Entry Scene: {}\n```toml\n", entry));
                    ctx.push_str(&scene_content);
                    ctx.push_str("```\n\n");
                }
            }
        }
    }

    // Component reflection metadata (from editor-header-tool output)
    let reflect_path = root.join(".shadoweditor/shadow_reflect.json");
    if let Ok(reflect_content) = std::fs::read_to_string(&reflect_path) {
        ctx.push_str("## C++ Component Reflection\n```json\n");
        ctx.push_str(&reflect_content);
        ctx.push_str("```\n\n");
    }

    // C++ source headers (up to 3, up to 2000 chars each)
    let src_dir = root.join("src");
    let game_dir = root.join("game");
    if src_dir.exists() || game_dir.exists() {
        let mut headers = Vec::new();
        collect_cpp_headers(&src_dir, &mut headers);
        collect_cpp_headers(&game_dir, &mut headers);
        headers.sort();
        headers.dedup();

        if !headers.is_empty() {
            ctx.push_str("## C++ Component Headers\n");
            for entry in headers.iter().take(3) {
                let path = entry;
                let short = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("header.h");
                if let Ok(content) = std::fs::read_to_string(&path) {
                    ctx.push_str(&format!("### {}\n```cpp\n", short));
                    ctx.push_str(&content.chars().take(2000).collect::<String>());
                    ctx.push_str("\n```\n\n");
                }
            }
        }
    }

    // Last build log (last 30 lines)
    let build_log = root.join(".shadoweditor/last_build.log");
    if let Ok(log) = std::fs::read_to_string(&build_log) {
        let lines: Vec<&str> = log.lines().collect();
        let tail: Vec<&str> = lines
            .iter()
            .rev()
            .take(30)
            .copied()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        ctx.push_str("## Last Build Output\n```\n");
        ctx.push_str(&tail.join("\n"));
        ctx.push_str("\n```\n\n");
    }

    Ok(ctx)
}

/// List assets inside a ShadowEditor project's assets/ and scenes/ directories.
#[tauri::command]
pub async fn shadow_list_assets(project_path: String) -> Result<Vec<ShadowAssetItem>, String> {
    let root = Path::new(&project_path);
    let mut items = Vec::new();
    for sub in ["assets", "scenes"] {
        let dir = root.join(sub);
        if dir.exists() {
            walk_shadow_assets(&dir, &dir, sub, &mut items);
        }
    }
    Ok(items)
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct ShadowAssetItem {
    pub name: String,
    pub path: String,
    pub kind: String,
    pub size_bytes: u64,
    pub sub_dir: String,
}

#[tauri::command]
pub async fn shadow_import_assets(
    project_path: String,
    source_paths: Vec<String>,
) -> Result<Vec<ShadowAssetItem>, String> {
    let root = Path::new(&project_path);
    let assets_root = root.join("assets");
    std::fs::create_dir_all(&assets_root)
        .map_err(|e| format!("Failed to create assets directory: {}", e))?;

    let mut imported = Vec::new();
    for source_path in source_paths {
        let source = PathBuf::from(&source_path);
        if !source.is_file() {
            continue;
        }
        let ext = source
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_lowercase();
        let kind = classify_shadow_asset(&ext).to_string();
        let target_subdir = asset_target_subdir(&kind);
        let target_dir = assets_root.join(target_subdir);
        std::fs::create_dir_all(&target_dir)
            .map_err(|e| format!("Failed to create {}: {}", target_dir.display(), e))?;

        let file_name = source
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or_else(|| format!("Invalid file name: {}", source.display()))?;
        let target_path = unique_target_path(&target_dir, file_name);
        std::fs::copy(&source, &target_path)
            .map_err(|e| format!("Failed to import {}: {}", source.display(), e))?;

        let rel_path = normalize_project_relative_path(root, target_path.clone());
        let size_bytes = std::fs::metadata(&target_path)
            .map(|meta| meta.len())
            .unwrap_or(0);
        imported.push(ShadowAssetItem {
            name: target_path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or(file_name)
                .to_string(),
            path: rel_path,
            kind,
            size_bytes,
            sub_dir: "assets".to_string(),
        });
    }

    Ok(imported)
}

fn walk_shadow_assets(base: &Path, dir: &Path, sub_dir: &str, out: &mut Vec<ShadowAssetItem>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if !name.starts_with('.') {
                walk_shadow_assets(base, &path, sub_dir, out);
            }
            continue;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let kind = classify_shadow_asset(&ext);
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let rel_path = path
            .strip_prefix(base)
            .map(|p| format!("{}/{}", sub_dir, p.to_string_lossy().replace('\\', "/")))
            .unwrap_or_else(|_| path.to_string_lossy().to_string());
        let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        out.push(ShadowAssetItem {
            name,
            path: rel_path,
            kind: kind.to_string(),
            size_bytes,
            sub_dir: sub_dir.to_string(),
        });
    }
}

fn classify_shadow_asset(ext: &str) -> &'static str {
    match ext {
        "gltf" | "glb" | "obj" | "fbx" | "dae" | "blend" => "Mesh",
        "png" | "jpg" | "jpeg" | "webp" | "exr" | "hdr" | "ktx2" | "dds" | "tga" => "Texture",
        "wav" | "ogg" | "mp3" | "flac" => "Audio",
        "ttf" | "otf" | "woff" | "woff2" => "Font",
        "tmx" | "tsx" | "ldtk" => "Tilemap",
        "shadow" => "Scene",
        "shadow_mat" => "Material",
        "wgsl" | "glsl" | "vert" | "frag" => "Shader",
        "json" => "Data",
        _ => "Other",
    }
}

fn asset_target_subdir(kind: &str) -> &'static str {
    match kind {
        "Mesh" => "meshes",
        "Texture" => "textures",
        "Audio" => "audio",
        "Font" => "fonts",
        "Tilemap" => "tilemaps",
        "Scene" => "scenes",
        "Material" => "materials",
        "Shader" => "shaders",
        "Data" => "data",
        _ => "imported",
    }
}

fn unique_target_path(dir: &Path, file_name: &str) -> PathBuf {
    let candidate = dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("asset");
    let ext = Path::new(file_name)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default();

    for index in 1..10_000 {
        let numbered = if ext.is_empty() {
            format!("{}_{}", stem, index)
        } else {
            format!("{}_{}.{}", stem, index, ext)
        };
        let path = dir.join(numbered);
        if !path.exists() {
            return path;
        }
    }

    dir.join(file_name)
}

/// Trigger a C++23 build for a ShadowEditor project. Uses ninja if build.ninja exists, otherwise
/// falls back to a native direct compiler build.
#[tauri::command]
pub async fn shadow_trigger_build(
    project_path: String,
    runtime_state: tauri::State<'_, ShadowRuntimeState>,
) -> Result<ShadowBuildResult, String> {
    let root = Path::new(&project_path);
    let value = load_shadow_project_config(root)?;

    let build = value.get("build");
    let compiler = build
        .and_then(|b| b.get("compiler"))
        .and_then(|v| v.as_str())
        .unwrap_or("clang++")
        .to_string();
    let standard = build
        .and_then(|b| b.get("standard"))
        .and_then(|v| v.as_str())
        .unwrap_or("c++23")
        .to_string();
    let game_lib = value
        .get("game_library_name")
        .and_then(|v| v.as_str())
        .unwrap_or("libgame.so")
        .to_string();
    let include_dirs: Vec<String> = build
        .and_then(|b| b.get("include_dirs"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let defines: Vec<String> = build
        .and_then(|b| b.get("defines"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let link_libs: Vec<String> = build
        .and_then(|b| b.get("link_libs"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let resolved_compiler = match resolve_shadow_compiler(&compiler) {
        Ok(resolved) => resolved,
        Err(err) => {
            return Ok(ShadowBuildResult {
                success: false,
                output: err,
                duration_ms: 0,
            })
        }
    };
    let runtime_library_path = project_runtime_library_path(root, &value);
    let entry_scene_path = project_entry_scene_path(root, &value);
    let mut hot_reload_notes = Vec::new();
    let mut build_notes = vec![format!(
        "Compiler: {} ({})",
        resolved_compiler.display_name,
        resolved_compiler.executable.display()
    )];
    let mut hot_reload_restore: Option<(PathBuf, String)> = None;

    if let Ok(mut sessions) = runtime_state.inner().sessions.lock() {
        if let Some(session) = sessions.get_mut(&project_path) {
            if session.host.is_live() {
                let logical_scene_path = session.last_scene_path.clone().or_else(|| {
                    entry_scene_path
                        .as_ref()
                        .map(|path| path.to_string_lossy().replace('\\', "/"))
                });
                if let Some(logical_scene_path) = logical_scene_path {
                    let logical_scene_path_buf = PathBuf::from(&logical_scene_path);
                    let snapshot_path =
                        runtime_hot_reload_snapshot_path(root, Some(&logical_scene_path_buf));
                    match snapshot_path.parent() {
                        Some(parent) => {
                            if let Err(err) = std::fs::create_dir_all(parent) {
                                hot_reload_notes.push(format!(
                                    "Hot reload snapshot skipped: failed to create {}: {}",
                                    parent.display(),
                                    err
                                ));
                            } else {
                                match session.host.save_scene(&snapshot_path) {
                                    Ok(()) => {
                                        hot_reload_restore = Some((
                                            snapshot_path.clone(),
                                            logical_scene_path.clone(),
                                        ));
                                        hot_reload_notes.push(format!(
                                            "Preserved live runtime state before rebuild: {}",
                                            snapshot_path.display()
                                        ));
                                    }
                                    Err(err) => hot_reload_notes
                                        .push(format!("Hot reload snapshot skipped: {}", err)),
                                }
                            }
                        }
                        None => hot_reload_notes.push(
                            "Hot reload snapshot skipped: invalid snapshot path.".to_string(),
                        ),
                    }
                }
            }
            session.host.stop_session();
            session.last_error = None;
        }
    }

    let start = std::time::Instant::now();

    // Prefer ninja if build/build.ninja exists
    let ninja_file = root.join("build/build.ninja");
    let output = if ninja_file.exists() {
        let mut cmd = Command::new("ninja");
        cmd.current_dir(root.join("build"));
        crate::platform::hide_window(&mut cmd);
        cmd.output()
    } else {
        // Fallback: direct native shared-library build of all src/*.cpp files
        let mut cpp_paths = Vec::new();
        collect_cpp_sources(&root.join("src"), &mut cpp_paths);
        collect_cpp_sources(&root.join("game"), &mut cpp_paths);
        let cpp_files: Vec<String> = cpp_paths
            .into_iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect();

        if cpp_files.is_empty() {
            return Ok(ShadowBuildResult {
                success: false,
                output: "No .cpp files found in src/".to_string(),
                duration_ms: 0,
            });
        }

        let _ = std::fs::create_dir_all(root.join("build"));
        let out_lib = root.join("build").join(&game_lib);
        match resolved_compiler.style {
            ShadowCompilerStyle::GnuLike => {
                let mut cmd = Command::new(&resolved_compiler.executable);
                cmd.arg(format!("-std={}", standard))
                    .arg("-shared")
                    .arg("-fPIC")
                    .arg("-O0")
                    .arg("-g");
                cmd.arg(format!("-I{}", root.join("src").display()));
                if root.join("game").exists() {
                    cmd.arg(format!("-I{}", root.join("game").display()));
                }
                if let Some(sdk) = find_sdk_include_path() {
                    cmd.arg(format!("-I{}", sdk));
                }
                for inc in &include_dirs {
                    cmd.arg(format!("-I{}", root.join(inc).display()));
                }
                for def in &defines {
                    cmd.arg(format!("-D{}", def));
                }
                for f in &cpp_files {
                    cmd.arg(f);
                }
                for link in &link_libs {
                    if link.starts_with('-') {
                        cmd.arg(link);
                    } else if link.contains('/') || link.contains('\\') {
                        let path = Path::new(link);
                        let resolved = if path.is_absolute() {
                            path.to_path_buf()
                        } else {
                            root.join(path)
                        };
                        cmd.arg(resolved);
                    } else {
                        cmd.arg(format!("-l{}", link));
                    }
                }
                cmd.arg("-o").arg(&out_lib);
                cmd.current_dir(root);
                crate::platform::hide_window(&mut cmd);
                cmd.output()
            }
            ShadowCompilerStyle::Msvc => {
                let Some(setup_script) = resolved_compiler.setup_script.as_ref() else {
                    return Ok(ShadowBuildResult {
                        success: false,
                        output: "MSVC compiler detected, but Visual Studio environment setup script was not found.".to_string(),
                        duration_ms: 0,
                    });
                };
                let object_dir = root.join("build").join("obj");
                let _ = std::fs::create_dir_all(&object_dir);
                let mut object_dir_arg = object_dir.to_string_lossy().to_string();
                if !object_dir_arg.ends_with('\\') && !object_dir_arg.ends_with('/') {
                    object_dir_arg.push('\\');
                }
                let import_lib = root.join("build").join(format!(
                    "{}.lib",
                    Path::new(&game_lib)
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .unwrap_or("game")
                ));
                let compile_pdb = root.join("build").join("vc140.pdb");

                let mut args = vec![
                    resolved_compiler.executable.to_string_lossy().to_string(),
                    msvc_standard_flag(&standard).to_string(),
                    "/LD".to_string(),
                    "/nologo".to_string(),
                    "/utf-8".to_string(),
                    "/EHsc".to_string(),
                    "/Od".to_string(),
                    "/Zi".to_string(),
                    format!("/Fd{}", compile_pdb.display()),
                    format!("/Fo{}", object_dir_arg),
                    format!("/I{}", root.join("src").display()),
                ];
                if root.join("game").exists() {
                    args.push(format!("/I{}", root.join("game").display()));
                }
                if let Some(sdk) = find_sdk_include_path() {
                    args.push(format!("/I{}", sdk));
                }
                for inc in &include_dirs {
                    args.push(format!("/I{}", root.join(inc).display()));
                }
                for def in &defines {
                    args.push(format!("/D{}", def));
                }
                args.extend(cpp_files.iter().cloned());
                args.push("/link".to_string());
                args.push("/NOLOGO".to_string());
                args.push("/INCREMENTAL:NO".to_string());
                args.push(format!("/OUT:{}", out_lib.display()));
                args.push(format!("/IMPLIB:{}", import_lib.display()));
                args.push(format!(
                    "/PDB:{}",
                    root.join("build").join(format!("{game_lib}.pdb")).display()
                ));
                for export in SHADOW_RUNTIME_EXPORTS {
                    args.push(format!("/EXPORT:{export}"));
                }
                for link in &link_libs {
                    if link.contains('/') || link.contains('\\') {
                        let path = Path::new(link);
                        let resolved = if path.is_absolute() {
                            path.to_path_buf()
                        } else {
                            root.join(path)
                        };
                        args.push(resolved.to_string_lossy().to_string());
                    } else if let Some(stripped) = link.strip_prefix("-l") {
                        args.push(format!("{stripped}.lib"));
                    } else if link.starts_with('-') {
                        build_notes.push(format!("Skipped unsupported MSVC link flag: {link}"));
                    } else if link.to_ascii_lowercase().ends_with(".lib") {
                        args.push(link.clone());
                    } else {
                        args.push(format!("{link}.lib"));
                    }
                }

                let script_body = format!(
                    "@echo off\r\ncall {} >nul\r\nif errorlevel 1 exit /b %errorlevel%\r\n{}\r\nexit /b %errorlevel%\r\n",
                    quote_cmd_arg(setup_script.to_string_lossy().as_ref()),
                    args.iter()
                        .map(|arg| quote_cmd_arg(arg))
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                let build_dir = root.join("build");
                let batch_script = build_dir.join(format!(
                    "shadow_build_{}.cmd",
                    uuid::Uuid::new_v4().simple()
                ));
                if let Err(err) = std::fs::write(&batch_script, script_body) {
                    return Ok(ShadowBuildResult {
                        success: false,
                        output: format!(
                            "Failed to write temporary MSVC build script {}: {}",
                            batch_script.display(),
                            err
                        ),
                        duration_ms: 0,
                    });
                }

                let mut cmd = Command::new("cmd");
                cmd.arg("/C").arg(&batch_script);
                cmd.current_dir(root);
                crate::platform::hide_window(&mut cmd);
                let output = cmd.output();
                let _ = std::fs::remove_file(&batch_script);
                output
            }
        }
    };

    let duration_ms = start.elapsed().as_millis() as u64;

    match output {
        Ok(out) => {
            let mut combined = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
            .trim()
            .to_string();
            if !build_notes.is_empty() {
                let prelude = build_notes.join("\n");
                combined = if combined.is_empty() {
                    prelude
                } else {
                    format!("{prelude}\n\n{combined}")
                };
            }
            if !hot_reload_notes.is_empty() {
                let prelude = hot_reload_notes.join("\n");
                combined = if combined.is_empty() {
                    prelude
                } else {
                    format!("{prelude}\n\n{combined}")
                };
            }

            if out.status.success() {
                if let Ok(cc_msg) = generate_compile_commands_for_project(root, &value) {
                    if !cc_msg.is_empty() {
                        if !combined.is_empty() {
                            combined.push_str("\n\n");
                        }
                        combined.push_str(&cc_msg);
                    }
                }
                match run_header_tool_for_project(root) {
                    Ok(reflect) => {
                        if !combined.is_empty() {
                            combined.push_str("\n\n");
                        }
                        combined.push_str(&format!(
                            "Reflection updated: {} component(s), {} header(s) scanned.",
                            reflect.component_count, reflect.headers_scanned
                        ));
                    }
                    Err(err) => {
                        if !combined.is_empty() {
                            combined.push_str("\n\n");
                        }
                        combined.push_str(&format!("Reflection update skipped: {}", err));
                    }
                }

                if !combined.is_empty() {
                    combined.push_str("\n\n");
                }
                match runtime_state.inner().sessions.lock() {
                    Ok(mut sessions) => {
                        let session = ensure_runtime_session(
                            &mut sessions,
                            &project_path,
                            runtime_library_path.clone(),
                        );
                        session.last_error = None;
                        match session.host.load_if_present() {
                            Ok(()) => {
                                let mut restore_message = None;
                                let mut restored = false;
                                if let Some((snapshot_path, logical_scene_path)) =
                                    hot_reload_restore.as_ref()
                                {
                                    if snapshot_path.exists() {
                                        match session.host.load_scene(snapshot_path) {
                                            Ok(()) => {
                                                session.last_scene_path =
                                                    Some(logical_scene_path.clone());
                                                restore_message = Some(format!(
                                                    "Hot reload host: {}\nRestored live runtime state from {}",
                                                    session.host.status_line(),
                                                    snapshot_path.display()
                                                ));
                                                restored = true;
                                            }
                                            Err(err) => {
                                                combined.push_str(&format!(
                                                    "Hot reload snapshot restore failed: {}\n",
                                                    err
                                                ));
                                            }
                                        }
                                    }
                                }

                                let fallback_scene_path = if restored {
                                    None
                                } else if let Some((_, logical_scene_path)) =
                                    hot_reload_restore.as_ref()
                                {
                                    let path = PathBuf::from(logical_scene_path);
                                    if path.exists() {
                                        Some(path)
                                    } else {
                                        entry_scene_path.clone()
                                    }
                                } else {
                                    entry_scene_path.clone()
                                };

                                if let Some(message) = restore_message {
                                    combined.push_str(&message);
                                } else if let Some(scene_path) =
                                    fallback_scene_path.as_ref().filter(|path| path.exists())
                                {
                                    match session.host.load_scene(scene_path) {
                                        Ok(()) => {
                                            session.last_scene_path = Some(
                                                scene_path.to_string_lossy().replace('\\', "/"),
                                            );
                                            combined.push_str(&format!(
                                                "Hot reload host: {}\nLoaded scene: {}",
                                                session.host.status_line(),
                                                scene_path.display()
                                            ));
                                        }
                                        Err(err) => {
                                            session.last_error = Some(err.to_string());
                                            combined.push_str(&format!(
                                                "Hot reload host loaded runtime, but scene load failed: {}",
                                                err
                                            ));
                                        }
                                    }
                                } else {
                                    combined.push_str(&format!(
                                        "Hot reload host: {}",
                                        session.host.status_line()
                                    ));
                                }
                            }
                            Err(err) => {
                                session.last_error = Some(err.to_string());
                                combined.push_str(&format!("Hot reload failed: {}", err));
                            }
                        }
                    }
                    Err(_) => combined.push_str("Hot reload skipped: runtime state lock poisoned."),
                }
            }

            // Persist build log for AI context
            let log_dir = root.join(".shadoweditor");
            let _ = std::fs::create_dir_all(&log_dir);
            let _ = std::fs::write(log_dir.join("last_build.log"), &combined);

            Ok(ShadowBuildResult {
                success: out.status.success(),
                output: if combined.is_empty() {
                    "Build completed with no output.".to_string()
                } else {
                    combined
                },
                duration_ms,
            })
        }
        Err(e) => Ok(ShadowBuildResult {
            success: false,
            output: format!("Failed to invoke build tool: {}", e),
            duration_ms,
        }),
    }
}

// ===== ShadowEditor Header Tool =====

/// Run editor-header-tool on a project's src/ directory to generate shadow_reflect.json.
/// The parsing is done inline (no external subprocess) using the same regex-free logic
/// as the native editor-header-tool binary.
#[tauri::command]
pub async fn shadow_run_header_tool(project_path: String) -> Result<ShadowReflectResult, String> {
    let root = Path::new(&project_path);
    run_header_tool_for_project(root)
}

#[derive(serde::Serialize, serde::Deserialize, Debug)]
pub struct ShadowReflectResult {
    pub component_count: usize,
    pub headers_scanned: usize,
    pub json: String,
    pub generated_cpp_path: String,
}

fn collect_cpp_headers(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_cpp_headers(&path, out);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if matches!(ext, "h" | "hpp") {
                out.push(path);
            }
        }
    }
}

fn parse_shadow_components(source: &str) -> Vec<serde_json::Value> {
    let mut components = Vec::new();
    let mut current: Option<(String, Vec<serde_json::Value>)> = None;
    let mut waiting_for_name = false;
    let mut pending_prop: Option<(String, String)> = None;

    for line in source.lines() {
        let line = line.trim();
        if line.contains("SHADOW_COMPONENT()") {
            if let Some((name, props)) = current.take() {
                components.push(serde_json::json!({ "name": name, "properties": props }));
            }
            waiting_for_name = true;
            continue;
        }
        if waiting_for_name {
            if let Some(name) = extract_cpp_struct_name(line) {
                current = Some((name, Vec::new()));
                waiting_for_name = false;
                continue;
            }
        }
        if let Some((ty, meta)) = extract_cpp_property_macro(line) {
            pending_prop = Some((ty, meta));
            continue;
        }
        if let Some((ty, meta)) = pending_prop.take() {
            if let Some(field) = extract_cpp_field_name(line) {
                if let Some((_, props)) = current.as_mut() {
                    let meta_items: Vec<String> = meta
                        .split(',')
                        .map(|item| item.trim().to_string())
                        .filter(|item| !item.is_empty())
                        .collect();
                    props.push(serde_json::json!({ "name": field, "ty": ty, "meta": meta_items }));
                }
            } else {
                pending_prop = Some((ty, meta));
            }
        }
        if line.starts_with("};") || line == "}" {
            if let Some((name, props)) = current.take() {
                components.push(serde_json::json!({ "name": name, "properties": props }));
            }
        }
    }
    if let Some((name, props)) = current.take() {
        components.push(serde_json::json!({ "name": name, "properties": props }));
    }
    components
}

fn extract_cpp_struct_name(line: &str) -> Option<String> {
    let remainder = line.trim_start().strip_prefix("struct ")?;
    let name = remainder
        .split(|c: char| c == '{' || c.is_whitespace())
        .find(|s| !s.is_empty())?;
    Some(name.to_string())
}

fn extract_cpp_property_macro(line: &str) -> Option<(String, String)> {
    let start = line.find("SHADOW_PROPERTY(")?;
    let inner = &line[start + "SHADOW_PROPERTY(".len()..line.rfind(')')?];
    let mut parts = inner.splitn(2, ',');
    let ty = parts.next()?.trim().to_string();
    let meta = parts
        .next()
        .map(|s| s.trim().trim_matches('"').to_string())
        .unwrap_or_default();
    Some((ty, meta))
}

fn extract_cpp_field_name(line: &str) -> Option<String> {
    if line.is_empty() || line.starts_with("//") || line.starts_with("SHADOW_") {
        return None;
    }
    let s = line.trim_end_matches(';').split('=').next()?.trim();
    let tok = s
        .split_whitespace()
        .last()?
        .trim_start_matches('*')
        .trim_start_matches('&');
    let tok = tok.split('[').next().unwrap_or(tok);
    if tok.is_empty() {
        None
    } else {
        Some(tok.to_string())
    }
}

// ===== New Project Wizard =====

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct ShadowTemplate {
    pub id: String,
    pub name: String,
    pub description: String,
    pub entities: Vec<String>,
}

#[tauri::command]
pub async fn shadow_list_templates() -> Result<Vec<ShadowTemplate>, String> {
    Ok(vec![
        ShadowTemplate {
            id: "empty_3d".into(),
            name: "Empty 3D".into(),
            description: "Bare project with a ground plane and directional light. Start from scratch.".into(),
            entities: vec!["DirectionalLight".into(), "Ground".into()],
        },
        ShadowTemplate {
            id: "3d_platformer".into(),
            name: "3D Platformer".into(),
            description: "Player with Transform, PlayerController, and Health. Platforms, a camera rig, and a collectible.".into(),
            entities: vec!["Player".into(), "Platform_1".into(), "Platform_2".into(), "Collectible".into(), "DirectionalLight".into()],
        },
        ShadowTemplate {
            id: "empty_2d".into(),
            name: "Empty 2D".into(),
            description: "Orthographic camera, a background sprite, and an empty actor. 2D pixel-perfect mode.".into(),
            entities: vec!["Camera2D".into(), "Background".into()],
        },
        ShadowTemplate {
            id: "2d_rpg".into(),
            name: "2D RPG".into(),
            description: "Top-down player, NPC, spawn point, and camera setup for a 2D RPG starter.".into(),
            entities: vec!["Player2D".into(), "Npc".into(), "SpawnPoint".into(), "Camera2D".into()],
        },
    ])
}

/// Scaffold a new ShadowEditor project from a template.
#[tauri::command]
pub async fn shadow_new_project(
    parent_dir: String,
    name: String,
    template: String,
) -> Result<String, String> {
    if name.trim().is_empty() {
        return Err("Project name cannot be empty".into());
    }
    let safe_name: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let root = Path::new(&parent_dir).join(&safe_name);
    if root.exists() {
        return Err(format!("Directory already exists: {}", root.display()));
    }

    std::fs::create_dir_all(root.join("src")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(root.join("scenes")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(root.join("assets")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(root.join(".shadoweditor")).map_err(|e| e.to_string())?;

    // .shadow_project.toml
    let lib_name = if cfg!(windows) {
        format!("{}.dll", safe_name)
    } else {
        format!("lib{}.so", safe_name)
    };
    let config = format!(
        r#"name = "{name}"
runtime = "cpp23"
entry_scene = "scenes/Main.shadow"
game_library_name = "{lib_name}"

[build]
compiler = "clang++"
standard = "c++23"
include_dirs = []
defines = ["SHADOW_DEBUG", "SHADOW_EDITOR"]
link_libs = []
"#
    );
    std::fs::write(root.join(".shadow_project.toml"), config).map_err(|e| e.to_string())?;

    // scenes/Main.shadow
    let scene = scene_for_template(&template, &name);
    std::fs::write(root.join("scenes/Main.shadow"), scene).map_err(|e| e.to_string())?;

    // src/game.h — annotated component stubs
    let game_h = game_h_for_template(&template, &name);
    std::fs::write(root.join("src/game.h"), game_h).map_err(|e| e.to_string())?;

    // src/game.cpp — entry point stub
    let game_cpp = game_cpp_for_template(&template, &name);
    std::fs::write(root.join("src/game.cpp"), game_cpp).map_err(|e| e.to_string())?;

    Ok(root.to_string_lossy().to_string())
}

fn scene_for_template(template: &str, name: &str) -> String {
    match template {
        "3d_platformer" => format!(
            r#"[scene]
name    = "{name}"
version = "1.0"
runtime = "cpp23"

[[entity]]
id   = "player"
name = "Player"

  [[entity.component]]
  type       = "Transform"
  position   = [0.0, 2.0, 0.0]
  rotation   = [0.0, 0.0, 0.0, 1.0]
  scale      = [1.0, 1.0, 1.0]

  [[entity.component]]
  type       = "PlayerController"
  speed      = 7.0
  jump_force = 10.0

  [[entity.component]]
  type    = "Health"
  current = 100.0
  max     = 100.0

[[entity]]
id   = "platform_1"
name = "Platform_1"

  [[entity.component]]
  type     = "Transform"
  position = [0.0, 0.0, 0.0]
  scale    = [8.0, 0.5, 4.0]

[[entity]]
id   = "platform_2"
name = "Platform_2"

  [[entity.component]]
  type     = "Transform"
  position = [5.0, 2.0, 0.0]
  scale    = [3.0, 0.5, 3.0]

[[entity]]
id   = "light"
name = "DirectionalLight"

  [[entity.component]]
  type     = "Transform"
  rotation = [-0.52, 0.0, 0.0, 0.86]
"#
        ),
        "empty_2d" => format!(
            r#"[scene]
name    = "{name}"
version = "1.0"
runtime = "cpp23"

[[entity]]
id   = "camera"
name = "Camera2D"

  [[entity.component]]
  type     = "Transform"
  position = [0.0, 0.0, 10.0]

[[entity]]
id   = "bg"
name = "Background"

  [[entity.component]]
  type  = "Transform"
  scale = [16.0, 9.0, 1.0]
"#
        ),
        "2d_rpg" => format!(
            r#"[scene]
name    = "{name}"
version = "1.0"
runtime = "cpp23"

[[entity]]
id   = "camera"
name = "Camera2D"

  [[entity.component]]
  type     = "Transform"
  position = [0.0, 0.0, 12.0]

[[entity]]
id   = "spawn"
name = "SpawnPoint"

  [[entity.component]]
  type     = "Transform"
  position = [0.0, 0.0, 0.0]

[[entity]]
id   = "player"
name = "Player2D"

  [[entity.component]]
  type     = "Transform"
  position = [0.0, 0.0, 0.0]
  scale    = [1.0, 1.0, 1.0]

  [[entity.component]]
  type         = "TopDownController"
  move_speed   = 5.0
  interact_range = 1.5

  [[entity.component]]
  type    = "Health"
  current = 100.0
  max     = 100.0

[[entity]]
id   = "npc_merchant"
name = "Merchant"

  [[entity.component]]
  type     = "Transform"
  position = [3.0, 1.0, 0.0]

  [[entity.component]]
  type        = "NpcDialogue"
  display_name = "Merchant"
  dialogue_id = "merchant_intro"
"#
        ),
        _ => format!(
            r#"[scene]
name    = "{name}"
version = "1.0"
runtime = "cpp23"

[[entity]]
id   = "light"
name = "DirectionalLight"

  [[entity.component]]
  type     = "Transform"
  rotation = [-0.52, 0.0, 0.0, 0.86]

[[entity]]
id   = "ground"
name = "Ground"

  [[entity.component]]
  type     = "Transform"
  position = [0.0, 0.0, 0.0]
  scale    = [20.0, 0.5, 20.0]
"#
        ),
    }
}

fn game_h_for_template(template: &str, _name: &str) -> String {
    let extras = if template == "3d_platformer" {
        r#"
// ─── PlayerController ────────────────────────────────────────────────────────

SHADOW_COMPONENT()
struct PlayerController {
    SHADOW_PROPERTY(float, "display_name=Move Speed, min=0, max=50, step=0.5")
    float speed = 7.0f;

    SHADOW_PROPERTY(float, "display_name=Jump Force, min=0, max=30, step=0.5")
    float jump_force = 10.0f;
};

// ─── Health ──────────────────────────────────────────────────────────────────

SHADOW_COMPONENT()
struct Health {
    SHADOW_PROPERTY(float, "display_name=Current HP, min=0, step=1")
    float current = 100.0f;

    SHADOW_PROPERTY(float, "display_name=Max HP, min=1, step=1")
    float max = 100.0f;

    SHADOW_PROPERTY(bool, "display_name=Regenerates")
    bool regen_enabled = false;
};
"#
    } else if template == "2d_rpg" {
        r#"
// ─── TopDownController ───────────────────────────────────────────────────────

SHADOW_COMPONENT()
struct TopDownController {
    SHADOW_PROPERTY(float, "display_name=Move Speed, min=0, max=20, step=0.1")
    float move_speed = 5.0f;

    SHADOW_PROPERTY(float, "display_name=Interact Range, min=0, max=10, step=0.1")
    float interact_range = 1.5f;
};

// ─── NpcDialogue ─────────────────────────────────────────────────────────────

SHADOW_COMPONENT()
struct NpcDialogue {
    SHADOW_PROPERTY(const char*, "display_name=Display Name")
    const char* display_name = "Merchant";

    SHADOW_PROPERTY(const char*, "display_name=Dialogue Id")
    const char* dialogue_id = "merchant_intro";
};

// ─── Health ──────────────────────────────────────────────────────────────────

SHADOW_COMPONENT()
struct Health {
    SHADOW_PROPERTY(float, "display_name=Current HP, min=0, step=1")
    float current = 100.0f;

    SHADOW_PROPERTY(float, "display_name=Max HP, min=1, step=1")
    float max = 100.0f;

    SHADOW_PROPERTY(bool, "display_name=Regenerates")
    bool regen_enabled = false;
};
"#
    } else {
        ""
    };

    format!(
        r#"#pragma once
#include "shadow/shadow_reflect.h"

// ─── Transform ───────────────────────────────────────────────────────────────

SHADOW_COMPONENT()
struct Transform {{
    SHADOW_PROPERTY(float[3], "display_name=Position, step=0.1")
    float position[3] = {{0.0f, 0.0f, 0.0f}};

    SHADOW_PROPERTY(float[4], "display_name=Rotation (Quaternion), step=0.01")
    float rotation[4] = {{0.0f, 0.0f, 0.0f, 1.0f}};

    SHADOW_PROPERTY(float[3], "display_name=Scale, min=0.001, step=0.01")
    float scale[3] = {{1.0f, 1.0f, 1.0f}};
}};
{extras}"#
    )
}

#[derive(Debug, Clone, Copy)]
enum RuntimeTemplateFieldKind {
    Float,
    Bool,
    FloatArray(usize),
    String,
}

#[derive(Debug, Clone, Copy)]
struct RuntimeTemplateField {
    name: &'static str,
    kind: RuntimeTemplateFieldKind,
}

#[derive(Debug, Clone, Copy)]
struct RuntimeTemplateComponent {
    name: &'static str,
    cpp_type: &'static str,
    fields: &'static [RuntimeTemplateField],
}

static TRANSFORM_RUNTIME_FIELDS: [RuntimeTemplateField; 3] = [
    RuntimeTemplateField {
        name: "position",
        kind: RuntimeTemplateFieldKind::FloatArray(3),
    },
    RuntimeTemplateField {
        name: "rotation",
        kind: RuntimeTemplateFieldKind::FloatArray(4),
    },
    RuntimeTemplateField {
        name: "scale",
        kind: RuntimeTemplateFieldKind::FloatArray(3),
    },
];

static PLAYER_CONTROLLER_RUNTIME_FIELDS: [RuntimeTemplateField; 2] = [
    RuntimeTemplateField {
        name: "speed",
        kind: RuntimeTemplateFieldKind::Float,
    },
    RuntimeTemplateField {
        name: "jump_force",
        kind: RuntimeTemplateFieldKind::Float,
    },
];

static HEALTH_RUNTIME_FIELDS: [RuntimeTemplateField; 3] = [
    RuntimeTemplateField {
        name: "current",
        kind: RuntimeTemplateFieldKind::Float,
    },
    RuntimeTemplateField {
        name: "max",
        kind: RuntimeTemplateFieldKind::Float,
    },
    RuntimeTemplateField {
        name: "regen_enabled",
        kind: RuntimeTemplateFieldKind::Bool,
    },
];

static TOP_DOWN_CONTROLLER_RUNTIME_FIELDS: [RuntimeTemplateField; 2] = [
    RuntimeTemplateField {
        name: "move_speed",
        kind: RuntimeTemplateFieldKind::Float,
    },
    RuntimeTemplateField {
        name: "interact_range",
        kind: RuntimeTemplateFieldKind::Float,
    },
];

static NPC_DIALOGUE_RUNTIME_FIELDS: [RuntimeTemplateField; 2] = [
    RuntimeTemplateField {
        name: "display_name",
        kind: RuntimeTemplateFieldKind::String,
    },
    RuntimeTemplateField {
        name: "dialogue_id",
        kind: RuntimeTemplateFieldKind::String,
    },
];

static EMPTY_RUNTIME_COMPONENTS: [RuntimeTemplateComponent; 1] = [RuntimeTemplateComponent {
    name: "Transform",
    cpp_type: "Transform",
    fields: &TRANSFORM_RUNTIME_FIELDS,
}];

static PLATFORMER_RUNTIME_COMPONENTS: [RuntimeTemplateComponent; 3] = [
    RuntimeTemplateComponent {
        name: "Transform",
        cpp_type: "Transform",
        fields: &TRANSFORM_RUNTIME_FIELDS,
    },
    RuntimeTemplateComponent {
        name: "PlayerController",
        cpp_type: "PlayerController",
        fields: &PLAYER_CONTROLLER_RUNTIME_FIELDS,
    },
    RuntimeTemplateComponent {
        name: "Health",
        cpp_type: "Health",
        fields: &HEALTH_RUNTIME_FIELDS,
    },
];

static RPG_RUNTIME_COMPONENTS: [RuntimeTemplateComponent; 4] = [
    RuntimeTemplateComponent {
        name: "Transform",
        cpp_type: "Transform",
        fields: &TRANSFORM_RUNTIME_FIELDS,
    },
    RuntimeTemplateComponent {
        name: "TopDownController",
        cpp_type: "TopDownController",
        fields: &TOP_DOWN_CONTROLLER_RUNTIME_FIELDS,
    },
    RuntimeTemplateComponent {
        name: "NpcDialogue",
        cpp_type: "NpcDialogue",
        fields: &NPC_DIALOGUE_RUNTIME_FIELDS,
    },
    RuntimeTemplateComponent {
        name: "Health",
        cpp_type: "Health",
        fields: &HEALTH_RUNTIME_FIELDS,
    },
];

fn runtime_components_for_template(template: &str) -> &'static [RuntimeTemplateComponent] {
    match template {
        "3d_platformer" => &PLATFORMER_RUNTIME_COMPONENTS,
        "2d_rpg" => &RPG_RUNTIME_COMPONENTS,
        _ => &EMPTY_RUNTIME_COMPONENTS,
    }
}

fn runtime_component_constant(name: &str) -> String {
    let mut out = String::from("COMP_");
    for (index, ch) in name.chars().enumerate() {
        if ch.is_ascii_uppercase() && index > 0 {
            out.push('_');
        } else if !ch.is_ascii_alphanumeric() && !out.ends_with('_') {
            out.push('_');
            continue;
        }
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_uppercase());
        }
    }
    out
}

fn runtime_component_has_string_fields(component: &RuntimeTemplateComponent) -> bool {
    component
        .fields
        .iter()
        .any(|field| matches!(field.kind, RuntimeTemplateFieldKind::String))
}

fn game_cpp_for_template(template: &str, name: &str) -> String {
    let components = runtime_components_for_template(template);
    let has_string_fields = components.iter().any(runtime_component_has_string_fields);
    let default_component = components
        .iter()
        .position(|component| component.name == "Transform");

    let mut out = String::new();
    out.push_str("#include \"game.h\"\n");
    out.push_str("#include \"shadow/game_api.h\"\n\n");
    out.push_str("#include <cstdint>\n");
    out.push_str("#include <cstdio>\n");
    out.push_str("#include <cstring>\n");
    out.push_str("#include <fstream>\n");
    out.push_str("#include <new>\n");
    out.push_str("#include <sstream>\n");
    out.push_str("#include <string>\n");
    if has_string_fields {
        out.push_str("#include <unordered_map>\n");
    }
    out.push_str("#include <vector>\n\n");
    out.push_str(&format!("// {} — ShadowEditor C++23 game runtime\n", name));
    out.push_str(
        "// This starter runtime is component-aware and exports the full live-edit ABI.\n",
    );
    out.push_str(
        "// The editor loads the authored scene after startup and hot-reloads this library in place.\n\n",
    );

    for (index, component) in components.iter().enumerate() {
        out.push_str(&format!(
            "static constexpr uint32_t {} = {};\n",
            runtime_component_constant(component.name),
            index
        ));
    }
    out.push_str(&format!(
        "static constexpr uint32_t COMP_COUNT = {};\n\n",
        components.len()
    ));

    out.push_str("static ComponentMeta COMP_META[COMP_COUNT] = {\n");
    for component in components {
        out.push_str(&format!(
            "    {{\"{}\", sizeof({}), alignof({})}},\n",
            component.name, component.cpp_type, component.cpp_type
        ));
    }
    out.push_str("};\n\n");

    out.push_str("struct RuntimeEntity {\n");
    out.push_str("    EntityId id;\n");
    out.push_str("    std::string scene_id;\n");
    out.push_str("    std::string name;\n");
    if has_string_fields {
        out.push_str("    std::unordered_map<std::string, std::string> string_fields;\n");
    }
    out.push_str("    void* components[COMP_COUNT] = {};\n");
    out.push_str("};\n\n");

    out.push_str("static std::vector<RuntimeEntity> g_entities;\n");
    out.push_str("static std::vector<EntityId>      g_entity_ids;\n");
    out.push_str("static uint64_t                   g_next_entity_id = 1;\n");
    out.push_str("static ShadowEngineCtx*           g_context = nullptr;\n\n");

    out.push_str("static void refresh_entity_cache() {\n");
    out.push_str("    g_entity_ids.clear();\n");
    out.push_str("    g_entity_ids.reserve(g_entities.size());\n");
    out.push_str("    for (const auto& entity : g_entities) {\n");
    out.push_str("        g_entity_ids.push_back(entity.id);\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");

    out.push_str("static RuntimeEntity* find_entity(EntityId id) {\n");
    out.push_str("    for (auto& entity : g_entities) {\n");
    out.push_str("        if (entity.id == id) return &entity;\n");
    out.push_str("    }\n");
    out.push_str("    return nullptr;\n");
    out.push_str("}\n\n");

    out.push_str("static RuntimeEntity* find_entity_by_scene_id(const char* scene_id) {\n");
    out.push_str("    if (scene_id == nullptr || scene_id[0] == '\\0') return nullptr;\n");
    out.push_str("    for (auto& entity : g_entities) {\n");
    out.push_str("        if (entity.scene_id == scene_id) return &entity;\n");
    out.push_str("    }\n");
    out.push_str("    return nullptr;\n");
    out.push_str("}\n\n");

    if has_string_fields {
        out.push_str("static const char* lookup_string_field(const RuntimeEntity& entity, const char* key, const char* fallback = \"\") {\n");
        out.push_str("    auto it = entity.string_fields.find(key);\n");
        out.push_str("    if (it != entity.string_fields.end()) return it->second.c_str();\n");
        out.push_str("    return fallback != nullptr ? fallback : \"\";\n");
        out.push_str("}\n\n");

        out.push_str("static void sync_string_fields(RuntimeEntity& entity, uint32_t type_id, bool capture_existing) {\n");
        out.push_str("    switch (type_id) {\n");
        for component in components
            .iter()
            .filter(|component| runtime_component_has_string_fields(component))
        {
            let const_name = runtime_component_constant(component.name);
            out.push_str(&format!("        case {}: {{\n", const_name));
            out.push_str(&format!(
                "            auto* component = static_cast<{}*>(entity.components[{}]);\n",
                component.cpp_type, const_name
            ));
            out.push_str("            if (component == nullptr) return;\n");
            for field in component
                .fields
                .iter()
                .filter(|field| matches!(field.kind, RuntimeTemplateFieldKind::String))
            {
                let key = format!("{}.{}", component.name, field.name);
                out.push_str(&format!(
                    "            auto& value_{} = entity.string_fields[\"{}\"];\n",
                    field.name, key
                ));
                out.push_str(&format!(
                    "            if (capture_existing && value_{}.empty() && component->{} != nullptr) value_{} = component->{};\n",
                    field.name, field.name, field.name, field.name
                ));
                out.push_str(&format!(
                    "            component->{} = value_{}.c_str();\n",
                    field.name, field.name
                ));
            }
            out.push_str("            return;\n");
            out.push_str("        }\n");
        }
        out.push_str("        default:\n");
        out.push_str("            return;\n");
        out.push_str("    }\n");
        out.push_str("}\n\n");

        out.push_str(
            "static void clear_string_fields(RuntimeEntity& entity, uint32_t type_id) {\n",
        );
        out.push_str("    switch (type_id) {\n");
        for component in components
            .iter()
            .filter(|component| runtime_component_has_string_fields(component))
        {
            let const_name = runtime_component_constant(component.name);
            out.push_str(&format!("        case {}:\n", const_name));
            for field in component
                .fields
                .iter()
                .filter(|field| matches!(field.kind, RuntimeTemplateFieldKind::String))
            {
                out.push_str(&format!(
                    "            entity.string_fields.erase(\"{}.{}\");\n",
                    component.name, field.name
                ));
            }
            out.push_str("            return;\n");
        }
        out.push_str("        default:\n");
        out.push_str("            return;\n");
        out.push_str("    }\n");
        out.push_str("}\n\n");
    }

    out.push_str("static void attach_component(RuntimeEntity& entity, uint32_t type_id) {\n");
    out.push_str(
        "    if (type_id >= COMP_COUNT || entity.components[type_id] != nullptr) return;\n",
    );
    out.push_str("    const size_t size = COMP_META[type_id].size;\n");
    out.push_str("    void* memory = ::operator new(size);\n");
    out.push_str("    std::memset(memory, 0, size);\n");
    out.push_str("    switch (type_id) {\n");
    for component in components {
        let const_name = runtime_component_constant(component.name);
        out.push_str(&format!(
            "        case {}: new (memory) {}{{}}; break;\n",
            const_name, component.cpp_type
        ));
    }
    out.push_str("        default: break;\n");
    out.push_str("    }\n");
    out.push_str("    entity.components[type_id] = memory;\n");
    if has_string_fields {
        out.push_str("    sync_string_fields(entity, type_id, true);\n");
    }
    out.push_str("}\n\n");

    out.push_str("static void detach_component(RuntimeEntity& entity, uint32_t type_id) {\n");
    out.push_str(
        "    if (type_id >= COMP_COUNT || entity.components[type_id] == nullptr) return;\n",
    );
    if has_string_fields {
        out.push_str("    clear_string_fields(entity, type_id);\n");
    }
    out.push_str("    ::operator delete(entity.components[type_id]);\n");
    out.push_str("    entity.components[type_id] = nullptr;\n");
    out.push_str("}\n\n");

    out.push_str("static void detach_all_components(RuntimeEntity& entity) {\n");
    out.push_str("    for (uint32_t index = 0; index < COMP_COUNT; ++index) {\n");
    out.push_str("        detach_component(entity, index);\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");

    out.push_str("extern \"C\" void shadow_init(ShadowEngineCtx* ctx) {\n");
    out.push_str("    g_context = ctx;\n");
    out.push_str("    refresh_entity_cache();\n");
    out.push_str("}\n\n");

    out.push_str("extern \"C\" void shadow_update(float delta_time) {\n");
    out.push_str("    if (g_context != nullptr && delta_time >= 0.0f) {\n");
    out.push_str("        g_context->frame_index += 1;\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");

    out.push_str("extern \"C\" void shadow_shutdown(void) {\n");
    out.push_str("    for (auto& entity : g_entities) {\n");
    out.push_str("        detach_all_components(entity);\n");
    out.push_str("    }\n");
    out.push_str("    g_entities.clear();\n");
    out.push_str("    g_entity_ids.clear();\n");
    out.push_str("    g_next_entity_id = 1;\n");
    out.push_str("    g_context = nullptr;\n");
    out.push_str("}\n\n");

    out.push_str("extern \"C\" uint32_t shadow_component_count(void) {\n");
    out.push_str("    return COMP_COUNT;\n");
    out.push_str("}\n\n");

    out.push_str("extern \"C\" ComponentMeta* shadow_component_meta(uint32_t index) {\n");
    out.push_str("    if (index >= COMP_COUNT) return nullptr;\n");
    out.push_str("    return &COMP_META[index];\n");
    out.push_str("}\n\n");

    out.push_str(
        "extern \"C\" void* shadow_get_component(EntityId id, uint32_t component_type) {\n",
    );
    out.push_str("    if (component_type >= COMP_COUNT) return nullptr;\n");
    out.push_str("    auto* entity = find_entity(id);\n");
    out.push_str("    return entity ? entity->components[component_type] : nullptr;\n");
    out.push_str("}\n\n");

    out.push_str("extern \"C\" void shadow_set_component(EntityId id, uint32_t component_type, void* data) {\n");
    out.push_str("    if (component_type >= COMP_COUNT || data == nullptr) return;\n");
    out.push_str("    auto* entity = find_entity(id);\n");
    out.push_str("    if (!entity) return;\n");
    out.push_str("    if (!entity->components[component_type]) {\n");
    out.push_str("        attach_component(*entity, component_type);\n");
    out.push_str("    }\n");
    out.push_str("    std::memcpy(entity->components[component_type], data, COMP_META[component_type].size);\n");
    if has_string_fields {
        out.push_str("    sync_string_fields(*entity, component_type, false);\n");
    }
    out.push_str("}\n\n");

    out.push_str(
        "extern \"C\" void shadow_remove_component(EntityId id, uint32_t component_type) {\n",
    );
    out.push_str("    if (component_type >= COMP_COUNT) return;\n");
    out.push_str("    auto* entity = find_entity(id);\n");
    out.push_str("    if (!entity) return;\n");
    out.push_str("    detach_component(*entity, component_type);\n");
    out.push_str("}\n\n");

    out.push_str("extern \"C\" EntityId shadow_create_entity(const char* name) {\n");
    out.push_str("    RuntimeEntity entity;\n");
    out.push_str("    entity.id = g_next_entity_id++;\n");
    out.push_str("    entity.scene_id.clear();\n");
    out.push_str("    entity.name = (name != nullptr) ? name : \"Entity\";\n");
    if let Some(index) = default_component {
        out.push_str(&format!(
            "    attach_component(entity, {});\n",
            runtime_component_constant(components[index].name)
        ));
    }
    out.push_str("    g_entities.push_back(std::move(entity));\n");
    out.push_str("    refresh_entity_cache();\n");
    out.push_str("    return g_entities.back().id;\n");
    out.push_str("}\n\n");

    out.push_str("extern \"C\" void shadow_destroy_entity(EntityId id) {\n");
    out.push_str("    for (auto it = g_entities.begin(); it != g_entities.end(); ++it) {\n");
    out.push_str("        if (it->id == id) {\n");
    out.push_str("            detach_all_components(*it);\n");
    out.push_str("            g_entities.erase(it);\n");
    out.push_str("            break;\n");
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("    refresh_entity_cache();\n");
    out.push_str("}\n\n");

    out.push_str("extern \"C\" void shadow_set_entity_name(EntityId id, const char* name) {\n");
    out.push_str("    auto* entity = find_entity(id);\n");
    out.push_str("    if (!entity || name == nullptr) return;\n");
    out.push_str("    entity->name = name;\n");
    out.push_str("}\n\n");

    out.push_str(
        "extern \"C\" void shadow_set_entity_scene_id(EntityId id, const char* scene_id) {\n",
    );
    out.push_str("    auto* entity = find_entity(id);\n");
    out.push_str("    if (!entity || scene_id == nullptr) return;\n");
    out.push_str("    entity->scene_id = scene_id;\n");
    out.push_str("}\n\n");

    out.push_str("extern \"C\" EntityId shadow_find_entity_by_scene_id(const char* scene_id) {\n");
    out.push_str("    auto* entity = find_entity_by_scene_id(scene_id);\n");
    out.push_str("    return entity ? entity->id : 0;\n");
    out.push_str("}\n\n");

    out.push_str("extern \"C\" uint32_t shadow_entity_count(void) {\n");
    out.push_str("    return static_cast<uint32_t>(g_entities.size());\n");
    out.push_str("}\n\n");

    out.push_str("extern \"C\" EntityId* shadow_entity_list(void) {\n");
    out.push_str("    refresh_entity_cache();\n");
    out.push_str("    return g_entity_ids.empty() ? nullptr : g_entity_ids.data();\n");
    out.push_str("}\n\n");

    out.push_str("static std::string fmt_f(float value) {\n");
    out.push_str("    char buffer[32];\n");
    out.push_str("    std::snprintf(buffer, sizeof(buffer), \"%.6g\", value);\n");
    out.push_str("    return buffer;\n");
    out.push_str("}\n\n");

    out.push_str("static std::string fmt_farr(const float* values, int count) {\n");
    out.push_str("    std::string out = \"[\";\n");
    out.push_str("    for (int index = 0; index < count; ++index) {\n");
    out.push_str("        if (index > 0) out += \", \";\n");
    out.push_str("        out += fmt_f(values[index]);\n");
    out.push_str("    }\n");
    out.push_str("    out += \"]\";\n");
    out.push_str("    return out;\n");
    out.push_str("}\n\n");

    out.push_str("static std::string escape_string(const std::string& value) {\n");
    out.push_str("    std::string out;\n");
    out.push_str("    out.reserve(value.size() + 8);\n");
    out.push_str("    for (char ch : value) {\n");
    out.push_str("        switch (ch) {\n");
    out.push_str("            case '\\\\': out += \"\\\\\\\\\"; break;\n");
    out.push_str("            case '\"': out += \"\\\\\\\"\"; break;\n");
    out.push_str("            case '\\n': out += \"\\\\n\"; break;\n");
    out.push_str("            case '\\r': out += \"\\\\r\"; break;\n");
    out.push_str("            case '\\t': out += \"\\\\t\"; break;\n");
    out.push_str("            default: out.push_back(ch); break;\n");
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("    return out;\n");
    out.push_str("}\n\n");

    out.push_str("static std::string fmt_str(const char* value) {\n");
    out.push_str("    return std::string(\"\\\"\") + escape_string(value != nullptr ? value : \"\") + \"\\\"\";\n");
    out.push_str("}\n\n");

    out.push_str("extern \"C\" void shadow_save_scene(const char* path) {\n");
    out.push_str("    if (path == nullptr) return;\n");
    out.push_str("    std::string out;\n");
    out.push_str("    out += \"[scene]\\n\";\n");
    out.push_str(&format!("    out += \"name = \\\"{}\\\"\\n\";\n", name));
    out.push_str("    out += \"version = \\\"1.0\\\"\\n\";\n");
    out.push_str("    out += \"runtime = \\\"cpp23\\\"\\n\";\n");
    out.push_str("    for (const auto& entity : g_entities) {\n");
    out.push_str("        const std::string scene_id = entity.scene_id.empty() ? std::to_string(entity.id) : entity.scene_id;\n");
    out.push_str("        out += \"\\n[[entity]]\\n\";\n");
    out.push_str("        out += \"id = \" + fmt_str(scene_id.c_str()) + \"\\n\";\n");
    out.push_str("        out += \"name = \" + fmt_str(entity.name.c_str()) + \"\\n\";\n");
    for component in components {
        let const_name = runtime_component_constant(component.name);
        out.push_str(&format!(
            "        if (auto* component = static_cast<{}*>(entity.components[{}])) {{\n",
            component.cpp_type, const_name
        ));
        out.push_str("            out += \"\\n  [[entity.component]]\\n\";\n");
        out.push_str(&format!(
            "            out += \"  type = \\\"{}\\\"\\n\";\n",
            component.name
        ));
        for field in component.fields {
            match field.kind {
                RuntimeTemplateFieldKind::Float => out.push_str(&format!(
                    "            out += \"  {} = \" + fmt_f(component->{}) + \"\\n\";\n",
                    field.name, field.name
                )),
                RuntimeTemplateFieldKind::Bool => {
                    out.push_str(&format!("            out += \"  {} = \";\n", field.name));
                    out.push_str(&format!(
                        "            out += component->{} ? \"true\" : \"false\";\n",
                        field.name
                    ));
                    out.push_str("            out += \"\\n\";\n");
                }
                RuntimeTemplateFieldKind::FloatArray(count) => out.push_str(&format!(
                    "            out += \"  {} = \" + fmt_farr(component->{}, {}) + \"\\n\";\n",
                    field.name, field.name, count
                )),
                RuntimeTemplateFieldKind::String => out.push_str(&format!(
                    "            out += \"  {} = \" + fmt_str(lookup_string_field(entity, \"{}.{}\", component->{})) + \"\\n\";\n",
                    field.name, component.name, field.name, field.name
                )),
            }
        }
        out.push_str("        }\n");
    }
    out.push_str("    }\n");
    out.push_str("    std::ofstream file(path);\n");
    out.push_str("    if (file.is_open()) {\n");
    out.push_str("        file << out;\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");

    out.push_str("extern \"C\" void shadow_load_scene(const char* path) {\n");
    out.push_str("    if (path == nullptr) return;\n");
    out.push_str("    std::ifstream file(path);\n");
    out.push_str("    if (!file.is_open()) return;\n");
    out.push_str("    for (auto& entity : g_entities) {\n");
    out.push_str("        detach_all_components(entity);\n");
    out.push_str("    }\n");
    out.push_str("    g_entities.clear();\n");
    out.push_str("    g_next_entity_id = 1;\n");
    out.push_str("    enum class Section { None, Scene, Entity, Component };\n");
    out.push_str("    Section section = Section::None;\n");
    out.push_str("    RuntimeEntity* current_entity = nullptr;\n");
    out.push_str("    uint32_t current_component_type = COMP_COUNT;\n");
    out.push_str("    auto trim = [](const std::string& value) -> std::string {\n");
    out.push_str("        const size_t start = value.find_first_not_of(\" \\t\\r\\n\");\n");
    out.push_str("        if (start == std::string::npos) return \"\";\n");
    out.push_str("        const size_t end = value.find_last_not_of(\" \\t\\r\\n\");\n");
    out.push_str("        return value.substr(start, end - start + 1);\n");
    out.push_str("    };\n");
    out.push_str("    auto strip_quotes = [](const std::string& value) -> std::string {\n");
    out.push_str(
        "        if (value.size() >= 2 && value.front() == '\"' && value.back() == '\"') {\n",
    );
    out.push_str("            return value.substr(1, value.size() - 2);\n");
    out.push_str("        }\n");
    out.push_str("        return value;\n");
    out.push_str("    };\n");
    out.push_str(
        "    auto parse_farr = [&](const std::string& value, float* out_values, int count) {\n",
    );
    out.push_str("        const size_t open = value.find('[');\n");
    out.push_str("        const size_t close = value.find(']');\n");
    out.push_str("        if (open == std::string::npos || close == std::string::npos) return;\n");
    out.push_str("        std::istringstream stream(value.substr(open + 1, close - open - 1));\n");
    out.push_str("        std::string token;\n");
    out.push_str("        int index = 0;\n");
    out.push_str("        while (std::getline(stream, token, ',') && index < count) {\n");
    out.push_str("            token = strip_quotes(trim(token));\n");
    out.push_str("            if (!token.empty()) out_values[index++] = std::stof(token);\n");
    out.push_str("        }\n");
    out.push_str("    };\n");
    out.push_str("    std::string line;\n");
    out.push_str("    while (std::getline(file, line)) {\n");
    out.push_str("        std::string trimmed = trim(line);\n");
    out.push_str("        if (trimmed.empty() || trimmed[0] == '#') continue;\n");
    out.push_str("        if (trimmed == \"[scene]\") {\n");
    out.push_str("            section = Section::Scene;\n");
    out.push_str("            continue;\n");
    out.push_str("        }\n");
    out.push_str("        if (trimmed == \"[[entity]]\") {\n");
    out.push_str("            g_entities.emplace_back();\n");
    out.push_str("            current_entity = &g_entities.back();\n");
    out.push_str("            current_entity->id = g_next_entity_id++;\n");
    out.push_str("            current_component_type = COMP_COUNT;\n");
    out.push_str("            section = Section::Entity;\n");
    out.push_str("            continue;\n");
    out.push_str("        }\n");
    out.push_str("        if (trimmed == \"[[entity.component]]\") {\n");
    out.push_str("            current_component_type = COMP_COUNT;\n");
    out.push_str("            section = Section::Component;\n");
    out.push_str("            continue;\n");
    out.push_str("        }\n");
    out.push_str("        const size_t eq = trimmed.find('=');\n");
    out.push_str("        if (eq == std::string::npos) continue;\n");
    out.push_str("        const std::string key = trim(trimmed.substr(0, eq));\n");
    out.push_str("        const std::string value = trim(trimmed.substr(eq + 1));\n");
    out.push_str("        if (section == Section::Entity && current_entity != nullptr) {\n");
    out.push_str(
        "            if (key == \"id\") current_entity->scene_id = strip_quotes(value);\n",
    );
    out.push_str(
        "            else if (key == \"name\") current_entity->name = strip_quotes(value);\n",
    );
    out.push_str("            continue;\n");
    out.push_str("        }\n");
    out.push_str("        if (section != Section::Component || current_entity == nullptr) {\n");
    out.push_str("            continue;\n");
    out.push_str("        }\n");
    out.push_str("        if (key == \"type\") {\n");
    out.push_str("            const std::string type_name = strip_quotes(value);\n");
    for component in components {
        let const_name = runtime_component_constant(component.name);
        if component.name == components[0].name {
            out.push_str(&format!(
                "            if (type_name == \"{}\") {{\n",
                component.name
            ));
        } else {
            out.push_str(&format!(
                "            else if (type_name == \"{}\") {{\n",
                component.name
            ));
        }
        out.push_str(&format!(
            "                current_component_type = {};\n",
            const_name
        ));
        out.push_str(&format!(
            "                attach_component(*current_entity, {});\n",
            const_name
        ));
        out.push_str("            }\n");
    }
    out.push_str("            else {\n");
    out.push_str("                current_component_type = COMP_COUNT;\n");
    out.push_str("            }\n");
    out.push_str("            continue;\n");
    out.push_str("        }\n");
    for (index, component) in components.iter().enumerate() {
        let const_name = runtime_component_constant(component.name);
        if index == 0 {
            out.push_str(&format!(
                "        if (current_component_type == {}) {{\n",
                const_name
            ));
        } else {
            out.push_str(&format!(
                "        else if (current_component_type == {}) {{\n",
                const_name
            ));
        }
        out.push_str(&format!(
            "            auto* component = static_cast<{}*>(current_entity->components[{}]);\n",
            component.cpp_type, const_name
        ));
        out.push_str("            if (component == nullptr) continue;\n");
        for (field_index, field) in component.fields.iter().enumerate() {
            let prefix = if field_index == 0 {
                "            if"
            } else {
                "            else if"
            };
            match field.kind {
                RuntimeTemplateFieldKind::Float => out.push_str(&format!(
                    "{} (key == \"{}\") component->{} = std::stof(strip_quotes(value));\n",
                    prefix, field.name, field.name
                )),
                RuntimeTemplateFieldKind::Bool => out.push_str(&format!(
                    "{} (key == \"{}\") component->{} = (strip_quotes(value) == \"true\" || strip_quotes(value) == \"1\");\n",
                    prefix, field.name, field.name
                )),
                RuntimeTemplateFieldKind::FloatArray(count) => out.push_str(&format!(
                    "{} (key == \"{}\") parse_farr(value, component->{}, {});\n",
                    prefix, field.name, field.name, count
                )),
                RuntimeTemplateFieldKind::String => {
                    out.push_str(&format!(
                        "{} (key == \"{}\") current_entity->string_fields[\"{}.{}\"] = strip_quotes(value);\n",
                        prefix, field.name, component.name, field.name
                    ));
                    if has_string_fields {
                        out.push_str(&format!(
                            "            if (key == \"{}\") sync_string_fields(*current_entity, {}, false);\n",
                            field.name, const_name
                        ));
                    }
                }
            }
        }
        out.push_str("        }\n");
    }
    out.push_str("    }\n");
    out.push_str("    refresh_entity_cache();\n");
    out.push_str("}\n");

    out
}

// ===== compile_commands.json Generator =====

/// Generate compile_commands.json from .shadow_project.toml for clangd LSP support.
#[tauri::command]
pub async fn shadow_generate_compile_commands(project_path: String) -> Result<String, String> {
    let root = Path::new(&project_path);
    let config = load_shadow_project_config(root)?;
    generate_compile_commands_for_project(root, &config)
}

#[tauri::command]
pub async fn shadow_list_source_files(
    project_path: String,
) -> Result<Vec<ShadowSourceFile>, String> {
    let root = Path::new(&project_path);
    let mut files = Vec::new();
    let mut discovered = Vec::new();
    collect_files_with_extensions(
        &root.join("src"),
        &["h", "hpp", "cpp", "cxx", "cc"],
        &mut discovered,
    );
    collect_files_with_extensions(
        &root.join("game"),
        &["h", "hpp", "cpp", "cxx", "cc"],
        &mut discovered,
    );
    discovered.sort();
    discovered.dedup();

    for path in discovered {
        let ext = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        let kind = match ext {
            "h" | "hpp" => "header",
            "cpp" | "cxx" | "cc" => "source",
            _ => "other",
        };
        let size_bytes = std::fs::metadata(&path).map(|meta| meta.len()).unwrap_or(0);
        files.push(ShadowSourceFile {
            path: normalize_project_relative_path(root, path),
            kind: kind.to_string(),
            size_bytes,
        });
    }

    Ok(files)
}

#[tauri::command]
pub async fn shadow_load_reflection(project_path: String) -> Result<ShadowReflectResult, String> {
    let root = Path::new(&project_path);
    let path = root.join(".shadoweditor/shadow_reflect.json");
    let json = std::fs::read_to_string(&path)
        .map_err(|e| format!("Failed to read {}: {}", path.display(), e))?;
    let component_count = serde_json::from_str::<serde_json::Value>(&json)
        .ok()
        .and_then(|value| {
            value
                .get("components")
                .and_then(|items| items.as_array())
                .map(|items| items.len())
        })
        .unwrap_or(0);
    Ok(ShadowReflectResult {
        component_count,
        headers_scanned: 0,
        json,
        generated_cpp_path: normalize_project_relative_path(
            root,
            root.join(".shadoweditor/shadow_reflect_generated.cpp"),
        ),
    })
}

#[tauri::command]
pub async fn shadow_get_last_build_log(project_path: String) -> Result<String, String> {
    let root = Path::new(&project_path);
    let path = root.join(".shadoweditor/last_build.log");
    if !path.exists() {
        return Ok(String::new());
    }
    std::fs::read_to_string(&path).map_err(|e| format!("Failed to read {}: {}", path.display(), e))
}

/// Locate the ShadowEditor runtime SDK include directory.
/// Checks (in order): SHADOW_SDK_INCLUDE env var → exe-relative install path → dev-mode path.
fn find_sdk_include_path() -> Option<String> {
    // 1. Explicit env override
    if let Ok(p) = std::env::var("SHADOW_SDK_INCLUDE") {
        if std::path::Path::new(&p).exists() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        // 2. Installed layout: shadow-editor.exe → runtime-sdk/include/
        if let Some(dir) = exe.parent() {
            let p = dir.join("runtime-sdk/include");
            if p.exists() {
                return Some(p.to_string_lossy().to_string());
            }
        }
        // 3. Dev layout: src-tauri/target/debug/shadow-ide.exe → ../../../native/runtime-sdk/include/
        let mut cursor = exe.parent();
        for _ in 0..4 {
            if let Some(dir) = cursor {
                let p = dir.join("native/runtime-sdk/include");
                if p.exists() {
                    return Some(p.to_string_lossy().to_string());
                }
                cursor = dir.parent();
            } else {
                break;
            }
        }
    }
    None
}

fn collect_cpp_sources(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_cpp_sources(&path, out);
        } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            if matches!(ext, "cpp" | "cxx" | "cc") {
                out.push(path);
            }
        }
    }
}

fn collect_files_with_extensions(dir: &Path, exts: &[&str], out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files_with_extensions(&path, exts, out);
            continue;
        }
        let ext = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if exts
            .iter()
            .any(|candidate| ext.eq_ignore_ascii_case(candidate))
        {
            out.push(path);
        }
    }
}

fn normalize_project_relative_path(root: &Path, path: PathBuf) -> String {
    path.strip_prefix(root)
        .map(|value| value.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|_| path.to_string_lossy().replace('\\', "/"))
}

fn load_shadow_project_config(root: &Path) -> Result<toml::Value, String> {
    let config_path = root.join(".shadow_project.toml");
    let config_str = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("Cannot read .shadow_project.toml: {}", e))?;
    toml::from_str(&config_str).map_err(|e| format!("Cannot parse config: {}", e))
}

#[derive(Debug, Clone)]
struct ReflectionProperty {
    name: String,
    ty: String,
}

fn resolve_scene_path(root: &Path, scene_path: Option<String>) -> Result<PathBuf, String> {
    let config = load_shadow_project_config(root)?;
    if let Some(path) = scene_path.filter(|path| !path.trim().is_empty()) {
        let path = PathBuf::from(path);
        return Ok(if path.is_absolute() {
            path
        } else {
            root.join(path)
        });
    }
    project_entry_scene_path(root, &config)
        .ok_or_else(|| "No entry scene configured in .shadow_project.toml".to_string())
}

fn load_scene_document(
    root: &Path,
    scene_path: Option<String>,
) -> Result<(PathBuf, toml::Value), String> {
    let resolved_scene_path = resolve_scene_path(root, scene_path)?;
    let content = std::fs::read_to_string(&resolved_scene_path).map_err(|e| {
        format!(
            "Failed to read scene {}: {}",
            resolved_scene_path.display(),
            e
        )
    })?;
    let value = toml::from_str::<toml::Value>(&content)
        .map_err(|e| format!("Failed to parse scene TOML: {}", e))?;
    Ok((resolved_scene_path, value))
}

fn save_scene_document(scene_path: &Path, doc: &toml::Value) -> Result<(), String> {
    if let Some(parent) = scene_path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }
    let content = toml::to_string_pretty(doc)
        .map_err(|e| format!("Failed to serialize scene TOML: {}", e))?;
    std::fs::write(scene_path, content)
        .map_err(|e| format!("Failed to write scene {}: {}", scene_path.display(), e))
}

fn scene_entities_mut(doc: &mut toml::Value) -> Result<&mut Vec<toml::Value>, String> {
    let table = doc
        .as_table_mut()
        .ok_or_else(|| "Scene document root is not a TOML table.".to_string())?;
    if !table.contains_key("entity") {
        table.insert("entity".to_string(), toml::Value::Array(Vec::new()));
    }
    table
        .get_mut("entity")
        .and_then(|value| value.as_array_mut())
        .ok_or_else(|| "Scene document does not contain a valid [[entity]] array.".to_string())
}

fn find_scene_entity_mut<'a>(
    entities: &'a mut [toml::Value],
    entity_id: &str,
) -> Result<&'a mut toml::map::Map<String, toml::Value>, String> {
    entities
        .iter_mut()
        .find_map(|entity| {
            let matches = entity
                .get("id")
                .and_then(|value| value.as_str())
                .map(|value| value == entity_id)
                .unwrap_or(false);
            if matches {
                entity.as_table_mut()
            } else {
                None
            }
        })
        .ok_or_else(|| format!("Entity '{}' was not found in the scene.", entity_id))
}

fn scene_entities(doc: &toml::Value) -> Result<&Vec<toml::Value>, String> {
    doc.get("entity")
        .and_then(|value| value.as_array())
        .ok_or_else(|| "Scene document does not contain a valid [[entity]] array.".to_string())
}

fn find_scene_entity<'a>(
    entities: &'a [toml::Value],
    entity_id: &str,
) -> Result<(usize, &'a toml::map::Map<String, toml::Value>), String> {
    entities
        .iter()
        .enumerate()
        .find_map(|(index, entity)| {
            let matches = entity
                .get("id")
                .and_then(|value| value.as_str())
                .map(|value| value == entity_id)
                .unwrap_or(false);
            if matches {
                entity.as_table().map(|table| (index, table))
            } else {
                None
            }
        })
        .ok_or_else(|| format!("Entity '{}' was not found in the scene.", entity_id))
}

fn component_values_mut(
    entity: &mut toml::map::Map<String, toml::Value>,
) -> Result<&mut Vec<toml::Value>, String> {
    if !entity.contains_key("component") {
        entity.insert("component".to_string(), toml::Value::Array(Vec::new()));
    }
    entity
        .get_mut("component")
        .and_then(|value| value.as_array_mut())
        .ok_or_else(|| "Entity does not contain a valid [[entity.component]] array.".to_string())
}

fn find_scene_component_mut<'a>(
    components: &'a mut [toml::Value],
    component_type: &str,
) -> Result<&'a mut toml::map::Map<String, toml::Value>, String> {
    components
        .iter_mut()
        .find_map(|component| {
            let matches = component
                .get("type")
                .and_then(|value| value.as_str())
                .map(|value| value == component_type)
                .unwrap_or(false);
            if matches {
                component.as_table_mut()
            } else {
                None
            }
        })
        .ok_or_else(|| {
            format!(
                "Component '{}' was not found on the entity.",
                component_type
            )
        })
}

fn scene_component_values(
    entity: &toml::map::Map<String, toml::Value>,
) -> Result<&Vec<toml::Value>, String> {
    entity
        .get("component")
        .and_then(|value| value.as_array())
        .ok_or_else(|| "Entity does not contain a valid [[entity.component]] array.".to_string())
}

fn find_scene_component<'a>(
    components: &'a [toml::Value],
    component_type: &str,
) -> Result<&'a toml::map::Map<String, toml::Value>, String> {
    components
        .iter()
        .find_map(|component| {
            let matches = component
                .get("type")
                .and_then(|value| value.as_str())
                .map(|value| value == component_type)
                .unwrap_or(false);
            if matches {
                component.as_table()
            } else {
                None
            }
        })
        .ok_or_else(|| {
            format!(
                "Component '{}' was not found on the entity.",
                component_type
            )
        })
}

fn parse_editor_field_value(raw: &str) -> toml::Value {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return toml::Value::String(String::new());
    }
    let wrapped = format!("value = {}", trimmed);
    if let Ok(doc) = toml::from_str::<toml::Value>(&wrapped) {
        if let Some(value) = doc.get("value") {
            return value.clone();
        }
    }
    toml::Value::String(trimmed.to_string())
}

fn slugify_scene_identifier(name: &str) -> String {
    let mut slug = String::new();
    let mut previous_was_sep = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            previous_was_sep = false;
        } else if !previous_was_sep {
            slug.push('_');
            previous_was_sep = true;
        }
    }
    let slug = slug.trim_matches('_').to_string();
    if slug.is_empty() {
        "entity".to_string()
    } else {
        slug
    }
}

fn generate_scene_entity_id(name: &str, existing_ids: &HashSet<String>) -> String {
    let base = slugify_scene_identifier(name);
    if !existing_ids.contains(&base) {
        return base;
    }
    for index in 2..10_000 {
        let candidate = format!("{}_{}", base, index);
        if !existing_ids.contains(&candidate) {
            return candidate;
        }
    }
    format!("{}_{}", base, uuid::Uuid::new_v4().simple())
}

fn load_reflection_component_map(
    root: &Path,
) -> Result<BTreeMap<String, Vec<ReflectionProperty>>, String> {
    let reflect_path = root.join(".shadoweditor/shadow_reflect.json");
    if !reflect_path.exists() {
        return Ok(BTreeMap::new());
    }
    let json = std::fs::read_to_string(&reflect_path)
        .map_err(|e| format!("Failed to read {}: {}", reflect_path.display(), e))?;
    let value = serde_json::from_str::<serde_json::Value>(&json)
        .map_err(|e| format!("Failed to parse {}: {}", reflect_path.display(), e))?;
    let mut components = BTreeMap::new();
    if let Some(items) = value.get("components").and_then(|entry| entry.as_array()) {
        for component in items {
            let Some(name) = component.get("name").and_then(|entry| entry.as_str()) else {
                continue;
            };
            let properties = component
                .get("properties")
                .and_then(|entry| entry.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|property| {
                            Some(ReflectionProperty {
                                name: normalize_reflection_field_name(
                                    property.get("name")?.as_str()?,
                                ),
                                ty: property
                                    .get("ty")
                                    .and_then(|entry| entry.as_str())
                                    .unwrap_or("unknown")
                                    .to_string(),
                            })
                        })
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            components.insert(name.to_string(), properties);
        }
    }
    Ok(components)
}

fn reflection_component_names(root: &Path) -> HashSet<String> {
    load_reflection_component_map(root)
        .map(|components| components.keys().cloned().collect())
        .unwrap_or_default()
}

fn normalize_reflection_field_name(name: &str) -> String {
    name.split('[').next().unwrap_or(name).trim().to_string()
}

fn default_value_for_property(component_type: &str, property: &ReflectionProperty) -> toml::Value {
    let field = property.name.as_str();
    let ty = property.ty.trim().to_ascii_lowercase();

    if component_type == "Transform" {
        return match field {
            "position" => toml::Value::Array(
                vec![0.0, 0.0, 0.0]
                    .into_iter()
                    .map(toml::Value::Float)
                    .collect(),
            ),
            "rotation" => toml::Value::Array(
                vec![0.0, 0.0, 0.0, 1.0]
                    .into_iter()
                    .map(toml::Value::Float)
                    .collect(),
            ),
            "scale" => toml::Value::Array(
                vec![1.0, 1.0, 1.0]
                    .into_iter()
                    .map(toml::Value::Float)
                    .collect(),
            ),
            _ => toml::Value::Float(0.0),
        };
    }

    if field.contains("scale") && ty.contains('[') {
        if let Some(count) = parse_cpp_array_len(&ty) {
            return toml::Value::Array((0..count).map(|_| toml::Value::Float(1.0)).collect());
        }
    }

    if let Some(count) = parse_cpp_array_len(&ty) {
        let values = (0..count)
            .map(|index| {
                if field.contains("rotation") && count == 4 && index == 3 {
                    toml::Value::Float(1.0)
                } else if field.contains("scale") {
                    toml::Value::Float(1.0)
                } else {
                    toml::Value::Float(0.0)
                }
            })
            .collect();
        return toml::Value::Array(values);
    }

    if ty.contains("bool") {
        toml::Value::Boolean(false)
    } else if ty.contains("float") || ty.contains("double") {
        toml::Value::Float(0.0)
    } else if ty.contains("int") || ty.contains("size_t") || ty.contains("uint") {
        toml::Value::Integer(0)
    } else if ty.contains("char*") || ty.contains("string") {
        toml::Value::String(String::new())
    } else {
        toml::Value::String(String::new())
    }
}

fn parse_cpp_array_len(ty: &str) -> Option<usize> {
    let start = ty.find('[')?;
    let end = ty[start + 1..].find(']')? + start + 1;
    ty[start + 1..end].trim().parse::<usize>().ok()
}

#[derive(Debug, Clone, Copy)]
enum RuntimeScalarType {
    Bool,
    F32,
    F64,
    I8,
    U8,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
}

impl RuntimeScalarType {
    fn size(self) -> usize {
        match self {
            Self::Bool | Self::I8 | Self::U8 => 1,
            Self::I16 | Self::U16 => 2,
            Self::F32 | Self::I32 | Self::U32 => 4,
            Self::F64 | Self::I64 | Self::U64 => 8,
        }
    }

    fn align(self) -> usize {
        self.size()
    }
}

#[derive(Debug, Clone, Copy)]
struct RuntimeFieldType {
    scalar: RuntimeScalarType,
    len: usize,
}

impl RuntimeFieldType {
    fn field_size(self) -> usize {
        self.scalar.size() * self.len
    }

    fn field_align(self) -> usize {
        self.scalar.align()
    }
}

fn align_up(offset: usize, align: usize) -> usize {
    if align <= 1 {
        offset
    } else {
        let remainder = offset % align;
        if remainder == 0 {
            offset
        } else {
            offset + (align - remainder)
        }
    }
}

fn parse_runtime_field_type(ty: &str) -> Option<RuntimeFieldType> {
    let array_len = parse_cpp_array_len(ty).unwrap_or(1);
    let normalized = ty
        .split('[')
        .next()
        .unwrap_or(ty)
        .trim()
        .to_ascii_lowercase()
        .replace([' ', '\t'], "");

    let scalar = match normalized.as_str() {
        "bool" => RuntimeScalarType::Bool,
        "float" => RuntimeScalarType::F32,
        "double" => RuntimeScalarType::F64,
        "int8_t" | "i8" | "signedchar" => RuntimeScalarType::I8,
        "uint8_t" | "u8" | "unsignedchar" | "char" => RuntimeScalarType::U8,
        "int16_t" | "i16" | "short" | "shortint" => RuntimeScalarType::I16,
        "uint16_t" | "u16" | "unsignedshort" | "unsignedshortint" => RuntimeScalarType::U16,
        "int32_t" | "i32" | "int" | "signed" | "signedint" => RuntimeScalarType::I32,
        "uint32_t" | "u32" | "unsigned" | "unsignedint" => RuntimeScalarType::U32,
        "int64_t" | "i64" | "longlong" | "longlongint" | "ssize_t" | "isize" => {
            RuntimeScalarType::I64
        }
        "uint64_t" | "u64" | "unsignedlonglong" | "unsignedlonglongint" => RuntimeScalarType::U64,
        "size_t" | "usize" => {
            if std::mem::size_of::<usize>() == 8 {
                RuntimeScalarType::U64
            } else {
                RuntimeScalarType::U32
            }
        }
        _ => return None,
    };

    Some(RuntimeFieldType {
        scalar,
        len: array_len,
    })
}

fn parse_toml_bool(value: &toml::Value) -> Result<bool, String> {
    if let Some(value) = value.as_bool() {
        return Ok(value);
    }
    if let Some(value) = value.as_integer() {
        return Ok(value != 0);
    }
    if let Some(value) = value.as_str() {
        return match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" => Ok(true),
            "false" | "0" => Ok(false),
            _ => Err(format!("'{}' is not a valid boolean value.", value)),
        };
    }
    Err(format!(
        "Expected a boolean-compatible value, got {}.",
        value
    ))
}

fn parse_toml_f64(value: &toml::Value) -> Result<f64, String> {
    if let Some(value) = value.as_float() {
        return Ok(value);
    }
    if let Some(value) = value.as_integer() {
        return Ok(value as f64);
    }
    if let Some(value) = value.as_str() {
        return value
            .trim()
            .parse::<f64>()
            .map_err(|_| format!("'{}' is not a valid number.", value));
    }
    Err(format!("Expected a numeric value, got {}.", value))
}

fn parse_toml_i64(value: &toml::Value) -> Result<i64, String> {
    if let Some(value) = value.as_integer() {
        return Ok(value);
    }
    if let Some(value) = value.as_float() {
        return Ok(value as i64);
    }
    if let Some(value) = value.as_str() {
        return value
            .trim()
            .parse::<i64>()
            .map_err(|_| format!("'{}' is not a valid integer.", value));
    }
    Err(format!("Expected an integer value, got {}.", value))
}

fn parse_toml_u64(value: &toml::Value) -> Result<u64, String> {
    if let Some(value) = value.as_integer() {
        return u64::try_from(value)
            .map_err(|_| format!("'{}' cannot be represented as an unsigned integer.", value));
    }
    if let Some(value) = value.as_float() {
        if value < 0.0 {
            return Err(format!(
                "'{}' cannot be represented as an unsigned integer.",
                value
            ));
        }
        return Ok(value as u64);
    }
    if let Some(value) = value.as_str() {
        return value
            .trim()
            .parse::<u64>()
            .map_err(|_| format!("'{}' is not a valid unsigned integer.", value));
    }
    Err(format!(
        "Expected an unsigned integer value, got {}.",
        value
    ))
}

fn write_runtime_scalar(
    bytes: &mut [u8],
    offset: usize,
    scalar: RuntimeScalarType,
    value: &toml::Value,
    field_name: &str,
) -> Result<(), String> {
    let size = scalar.size();
    let target = bytes.get_mut(offset..offset + size).ok_or_else(|| {
        format!(
            "Field '{}' exceeds the runtime component blob size.",
            field_name
        )
    })?;

    match scalar {
        RuntimeScalarType::Bool => {
            target[0] = u8::from(parse_toml_bool(value)?);
        }
        RuntimeScalarType::F32 => {
            target.copy_from_slice(&(parse_toml_f64(value)? as f32).to_ne_bytes());
        }
        RuntimeScalarType::F64 => {
            target.copy_from_slice(&parse_toml_f64(value)?.to_ne_bytes());
        }
        RuntimeScalarType::I8 => {
            let parsed = i8::try_from(parse_toml_i64(value)?)
                .map_err(|_| format!("Field '{}' does not fit into an i8 value.", field_name))?;
            target.copy_from_slice(&parsed.to_ne_bytes());
        }
        RuntimeScalarType::U8 => {
            let parsed = u8::try_from(parse_toml_u64(value)?)
                .map_err(|_| format!("Field '{}' does not fit into a u8 value.", field_name))?;
            target.copy_from_slice(&parsed.to_ne_bytes());
        }
        RuntimeScalarType::I16 => {
            let parsed = i16::try_from(parse_toml_i64(value)?)
                .map_err(|_| format!("Field '{}' does not fit into an i16 value.", field_name))?;
            target.copy_from_slice(&parsed.to_ne_bytes());
        }
        RuntimeScalarType::U16 => {
            let parsed = u16::try_from(parse_toml_u64(value)?)
                .map_err(|_| format!("Field '{}' does not fit into a u16 value.", field_name))?;
            target.copy_from_slice(&parsed.to_ne_bytes());
        }
        RuntimeScalarType::I32 => {
            let parsed = i32::try_from(parse_toml_i64(value)?)
                .map_err(|_| format!("Field '{}' does not fit into an i32 value.", field_name))?;
            target.copy_from_slice(&parsed.to_ne_bytes());
        }
        RuntimeScalarType::U32 => {
            let parsed = u32::try_from(parse_toml_u64(value)?)
                .map_err(|_| format!("Field '{}' does not fit into a u32 value.", field_name))?;
            target.copy_from_slice(&parsed.to_ne_bytes());
        }
        RuntimeScalarType::I64 => {
            target.copy_from_slice(&parse_toml_i64(value)?.to_ne_bytes());
        }
        RuntimeScalarType::U64 => {
            target.copy_from_slice(&parse_toml_u64(value)?.to_ne_bytes());
        }
    }

    Ok(())
}

fn serialize_runtime_component_blob(
    component: &toml::map::Map<String, toml::Value>,
    properties: &[ReflectionProperty],
    runtime_component: &RuntimeComponentInfo,
    base_blob: Option<Vec<u8>>,
) -> Result<Option<Vec<u8>>, String> {
    if properties.is_empty() {
        return Ok(None);
    }

    let mut bytes = base_blob.unwrap_or_else(|| vec![0; runtime_component.size as usize]);
    if bytes.len() != runtime_component.size as usize {
        bytes.resize(runtime_component.size as usize, 0);
    }

    let mut offset = 0usize;
    for property in properties {
        let Some(field_type) = parse_runtime_field_type(&property.ty) else {
            return Ok(None);
        };
        offset = align_up(offset, field_type.field_align());
        let end = offset + field_type.field_size();
        if end > bytes.len() {
            return Err(format!(
                "Reflected field '{}.{}' does not fit inside the runtime component blob ({} bytes).",
                runtime_component.name,
                property.name,
                runtime_component.size
            ));
        }

        let value = component.get(&property.name).ok_or_else(|| {
            format!(
                "Scene component '{}' is missing reflected field '{}'.",
                runtime_component.name, property.name
            )
        })?;

        if field_type.len == 1 {
            write_runtime_scalar(&mut bytes, offset, field_type.scalar, value, &property.name)?;
        } else {
            let values = value.as_array().ok_or_else(|| {
                format!(
                    "Scene field '{}.{}' must be an array with {} elements.",
                    runtime_component.name, property.name, field_type.len
                )
            })?;
            if values.len() != field_type.len {
                return Err(format!(
                    "Scene field '{}.{}' expected {} elements, found {}.",
                    runtime_component.name,
                    property.name,
                    field_type.len,
                    values.len()
                ));
            }
            let scalar_size = field_type.scalar.size();
            for (index, value) in values.iter().enumerate() {
                write_runtime_scalar(
                    &mut bytes,
                    offset + index * scalar_size,
                    field_type.scalar,
                    value,
                    &format!("{}[{}]", property.name, index),
                )?;
            }
        }

        offset = end;
    }

    Ok(Some(bytes))
}

fn runtime_entity_id_for_scene_entity(
    session: &ShadowRuntimeSession,
    scene_entity_id: &str,
    entity_index: usize,
) -> Result<EntityId, String> {
    if let Some(entity_id) = session
        .host
        .find_entity_by_scene_id(scene_entity_id)
        .map_err(|err| err.to_string())?
    {
        return Ok(entity_id);
    }

    session
        .host
        .entity_ids()
        .get(entity_index)
        .copied()
        .ok_or_else(|| {
            format!(
                "Live runtime entity '{}' is not available by scene ID or index {}. Falling back to a full scene reload.",
                scene_entity_id, entity_index
            )
        })
}

fn serialize_runtime_component_for_scene(
    session: &mut ShadowRuntimeSession,
    root: &Path,
    runtime_entity_id: Option<EntityId>,
    component: &toml::map::Map<String, toml::Value>,
) -> Result<Option<(u32, Vec<u8>)>, String> {
    let component_type = component
        .get("type")
        .and_then(|value| value.as_str())
        .ok_or_else(|| "Scene component is missing its 'type' field.".to_string())?;

    let Some(runtime_component) = session.host.component_type_by_name(component_type).cloned()
    else {
        return Ok(None);
    };

    let reflection_components = load_reflection_component_map(root)?;
    let Some(properties) = reflection_components.get(component_type) else {
        return Ok(None);
    };

    let base_blob = match runtime_entity_id {
        Some(entity_id) => session
            .host
            .get_component_bytes(entity_id, runtime_component.type_id)
            .map_err(|err| err.to_string())?,
        None => None,
    };

    let blob =
        serialize_runtime_component_blob(component, properties, &runtime_component, base_blob)?;
    Ok(blob.map(|blob| (runtime_component.type_id, blob)))
}

fn apply_runtime_component_to_entity(
    session: &mut ShadowRuntimeSession,
    root: &Path,
    runtime_entity_id: EntityId,
    component: &toml::map::Map<String, toml::Value>,
) -> Result<bool, String> {
    let Some((component_type, blob)) =
        serialize_runtime_component_for_scene(session, root, Some(runtime_entity_id), component)?
    else {
        return Ok(false);
    };

    session
        .host
        .set_component_bytes(runtime_entity_id, component_type, &blob)
        .map_err(|err| err.to_string())?;
    Ok(true)
}

fn reload_runtime_scene(session: &mut ShadowRuntimeSession, scene_path: &Path) {
    match session.host.load_scene(scene_path) {
        Ok(()) => {
            session.last_error = None;
            session.last_scene_path = Some(scene_path.to_string_lossy().replace('\\', "/"));
        }
        Err(err) => {
            session.last_error = Some(err.to_string());
        }
    }
}

fn try_apply_live_scene_mutation(
    session: &mut ShadowRuntimeSession,
    root: &Path,
    doc: &toml::Value,
    mutation: &LiveSceneMutation,
) -> Result<bool, String> {
    let entities = scene_entities(doc)?;

    match mutation {
        LiveSceneMutation::RenameEntity { entity_id } => {
            let (entity_index, entity) = find_scene_entity(entities, entity_id)?;
            let runtime_entity_id =
                runtime_entity_id_for_scene_entity(session, entity_id, entity_index)?;
            let entity_name = entity
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or("Entity");
            session
                .host
                .set_entity_name(runtime_entity_id, entity_name)
                .map_err(|err| err.to_string())
        }
        LiveSceneMutation::RemoveEntity { entity_id } => {
            let (entity_index, _) = find_scene_entity(entities, entity_id)?;
            let runtime_entity_id =
                runtime_entity_id_for_scene_entity(session, entity_id, entity_index)?;
            session
                .host
                .destroy_entity(runtime_entity_id)
                .map_err(|err| err.to_string())?;
            Ok(true)
        }
        LiveSceneMutation::AddEntity { entity_id } => {
            let (_, entity) = find_scene_entity(entities, entity_id)?;
            let components = scene_component_values(entity)?;
            let mut prepared_components = Vec::with_capacity(components.len());
            for component in components {
                let component_table = component.as_table().ok_or_else(|| {
                    format!(
                        "Entity '{}' contains a component entry that is not a TOML table.",
                        entity_id
                    )
                })?;
                let Some(prepared) =
                    serialize_runtime_component_for_scene(session, root, None, component_table)?
                else {
                    return Ok(false);
                };
                prepared_components.push(prepared);
            }

            let entity_name = entity
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or("Entity");
            let runtime_entity_id = session
                .host
                .create_entity(entity_name)
                .map_err(|err| err.to_string())?;
            session
                .host
                .set_entity_scene_id(runtime_entity_id, entity_id)
                .map_err(|err| err.to_string())?;

            for (component_type, blob) in prepared_components {
                session
                    .host
                    .set_component_bytes(runtime_entity_id, component_type, &blob)
                    .map_err(|err| err.to_string())?;
            }

            Ok(true)
        }
        LiveSceneMutation::AddComponent {
            entity_id,
            component_type,
        }
        | LiveSceneMutation::RemoveComponent {
            entity_id,
            component_type,
        }
        | LiveSceneMutation::SetComponentField {
            entity_id,
            component_type,
        } => {
            let (entity_index, entity) = find_scene_entity(entities, entity_id)?;
            let runtime_entity_id =
                runtime_entity_id_for_scene_entity(session, entity_id, entity_index)?;
            if matches!(mutation, LiveSceneMutation::RemoveComponent { .. }) {
                let Some(component_index) = session.host.component_index_by_name(component_type)
                else {
                    return Ok(false);
                };
                return session
                    .host
                    .remove_component(runtime_entity_id, component_index)
                    .map_err(|err| err.to_string());
            }
            let components = scene_component_values(entity)?;
            let component = find_scene_component(components, component_type)?;
            apply_runtime_component_to_entity(session, root, runtime_entity_id, component)
        }
    }
}

fn fallback_component_fields(component_type: &str) -> Vec<(String, toml::Value)> {
    match component_type {
        "RigidBody" => vec![
            ("mass".to_string(), toml::Value::Float(1.0)),
            ("is_dynamic".to_string(), toml::Value::Boolean(true)),
        ],
        "Lifetime" => vec![("seconds".to_string(), toml::Value::Float(1.0))],
        "PlayerController" => vec![
            ("speed".to_string(), toml::Value::Float(5.0)),
            ("jump_force".to_string(), toml::Value::Float(8.0)),
        ],
        "Transform" => vec![
            (
                "position".to_string(),
                toml::Value::Array(
                    vec![0.0, 0.0, 0.0]
                        .into_iter()
                        .map(toml::Value::Float)
                        .collect(),
                ),
            ),
            (
                "rotation".to_string(),
                toml::Value::Array(
                    vec![0.0, 0.0, 0.0, 1.0]
                        .into_iter()
                        .map(toml::Value::Float)
                        .collect(),
                ),
            ),
            (
                "scale".to_string(),
                toml::Value::Array(
                    vec![1.0, 1.0, 1.0]
                        .into_iter()
                        .map(toml::Value::Float)
                        .collect(),
                ),
            ),
        ],
        _ => Vec::new(),
    }
}

fn build_component_value(root: &Path, component_type: &str) -> toml::Value {
    let mut component = toml::map::Map::new();
    component.insert(
        "type".to_string(),
        toml::Value::String(component_type.to_string()),
    );

    let properties = load_reflection_component_map(root)
        .ok()
        .and_then(|components| components.get(component_type).cloned())
        .unwrap_or_default();

    let field_values = if properties.is_empty() {
        fallback_component_fields(component_type)
    } else {
        properties
            .iter()
            .map(|property| {
                (
                    property.name.clone(),
                    default_value_for_property(component_type, property),
                )
            })
            .collect()
    };

    for (name, value) in field_values {
        component.insert(name, value);
    }

    toml::Value::Table(component)
}

fn sync_live_scene_if_needed(
    project_path: &str,
    root: &Path,
    scene_path: &Path,
    doc: &toml::Value,
    runtime_state: &tauri::State<'_, ShadowRuntimeState>,
    mutation: LiveSceneMutation,
) {
    let Ok(mut sessions) = runtime_state.inner().sessions.lock() else {
        return;
    };
    let Some(session) = sessions.get_mut(project_path) else {
        return;
    };
    if !session.host.is_live() {
        return;
    }

    let normalized_scene_path = scene_path.to_string_lossy().replace('\\', "/");
    let direct_sync_result =
        if session.last_scene_path.as_deref() == Some(normalized_scene_path.as_str()) {
            try_apply_live_scene_mutation(session, root, doc, &mutation)
        } else {
            Ok(false)
        };

    match direct_sync_result {
        Ok(true) => {
            session.last_error = None;
            session.last_scene_path = Some(normalized_scene_path);
        }
        Ok(false) | Err(_) => {
            reload_runtime_scene(session, scene_path);
        }
    }
}

fn generate_compile_commands_for_project(
    root: &Path,
    config: &toml::Value,
) -> Result<String, String> {
    let build = config.get("build");
    let compiler = build
        .and_then(|value| value.get("compiler"))
        .and_then(|value| value.as_str())
        .unwrap_or("clang++")
        .to_string();
    let resolved_compiler =
        resolve_shadow_compiler(&compiler).unwrap_or(ShadowCompilerResolution {
            executable: PathBuf::from(&compiler),
            display_name: compiler.clone(),
            style: ShadowCompilerStyle::GnuLike,
            setup_script: None,
        });
    let standard = build
        .and_then(|value| value.get("standard"))
        .and_then(|value| value.as_str())
        .unwrap_or("c++23")
        .to_string();

    let include_dirs: Vec<String> = build
        .and_then(|value| value.get("include_dirs"))
        .and_then(|value| value.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|value| value.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let define_values: Vec<String> = build
        .and_then(|value| value.get("defines"))
        .and_then(|value| value.as_array())
        .map(|entries| {
            entries
                .iter()
                .filter_map(|value| value.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut cpp_files = Vec::new();
    collect_cpp_sources(&root.join("src"), &mut cpp_files);
    collect_cpp_sources(&root.join("game"), &mut cpp_files);
    cpp_files.sort();
    cpp_files.dedup();

    let mut auto_includes = vec![root.join("src").to_string_lossy().to_string()];
    if root.join("game").exists() {
        auto_includes.push(root.join("game").to_string_lossy().to_string());
    }
    if let Some(sdk) = find_sdk_include_path() {
        auto_includes.push(sdk);
    }

    let mut entries = Vec::new();
    for file in &cpp_files {
        let mut flags = Vec::new();
        match resolved_compiler.style {
            ShadowCompilerStyle::GnuLike => {
                flags.push(resolved_compiler.executable.to_string_lossy().to_string());
                flags.push(format!("-std={}", standard));
                flags.push("-fPIC".to_string());
                flags.push("-g".to_string());
                flags.push("-DSHADOW_EDITOR".to_string());
                for include in &auto_includes {
                    flags.push(format!("-I{}", include));
                }
                for include in &include_dirs {
                    flags.push(format!("-I{}", root.join(include).display()));
                }
                for define in &define_values {
                    flags.push(format!("-D{}", define));
                }
                flags.push("-c".to_string());
                flags.push(file.to_string_lossy().to_string());
            }
            ShadowCompilerStyle::Msvc => {
                flags.push(resolved_compiler.executable.to_string_lossy().to_string());
                flags.push(msvc_standard_flag(&standard).to_string());
                flags.push("/nologo".to_string());
                flags.push("/utf-8".to_string());
                flags.push("/EHsc".to_string());
                flags.push("/Zi".to_string());
                flags.push("/DSHADOW_EDITOR".to_string());
                for include in &auto_includes {
                    flags.push(format!("/I{}", include));
                }
                for include in &include_dirs {
                    flags.push(format!("/I{}", root.join(include).display()));
                }
                for define in &define_values {
                    flags.push(format!("/D{}", define));
                }
                flags.push("/c".to_string());
                flags.push(file.to_string_lossy().to_string());
            }
        }

        entries.push(serde_json::json!({
            "directory": root.to_string_lossy(),
            "arguments": flags,
            "file": file.to_string_lossy(),
        }));
    }

    let json = serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())?;
    std::fs::write(root.join("compile_commands.json"), &json)
        .map_err(|e| format!("Failed to write compile_commands.json: {}", e))?;

    Ok(format!(
        "Generated compile_commands.json ({} entries)",
        entries.len()
    ))
}

fn build_reflection_generated_cpp(components: &[serde_json::Value]) -> Result<String, String> {
    let mut out = String::from(
        "// @generated by ShadowIDE header tool. Do not edit by hand.\n\
         #include \"shadow/shadow_reflect.h\"\n\n\
         namespace shadow_generated {\n\
         struct PropertyInfo {\n\
             const char* name;\n\
             const char* type;\n\
             const char* meta;\n\
         };\n\n\
         struct ComponentInfo {\n\
             const char* name;\n\
             unsigned int property_count;\n\
             const PropertyInfo* properties;\n\
         };\n\n",
    );

    let mut component_entries = Vec::new();
    for (index, component) in components.iter().enumerate() {
        let component_name = component
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or("UnknownComponent");
        let properties = component
            .get("properties")
            .and_then(|value| value.as_array())
            .cloned()
            .unwrap_or_default();
        let property_array_name = format!("kComponent{}Properties", index);
        out.push_str(&format!(
            "static const PropertyInfo {}[] = {{\n",
            property_array_name
        ));
        for property in &properties {
            let property_name = serde_json::to_string(
                property
                    .get("name")
                    .and_then(|value| value.as_str())
                    .unwrap_or("field"),
            )
            .map_err(|e| e.to_string())?;
            let property_type = serde_json::to_string(
                property
                    .get("ty")
                    .and_then(|value| value.as_str())
                    .unwrap_or("unknown"),
            )
            .map_err(|e| e.to_string())?;
            let property_meta = property
                .get("meta")
                .or_else(|| property.get("metadata"))
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let property_meta = serde_json::to_string(&property_meta).map_err(|e| e.to_string())?;
            out.push_str(&format!(
                "    {{ {}, {}, {} }},\n",
                property_name, property_type, property_meta
            ));
        }
        out.push_str("};\n\n");
        component_entries.push(format!(
            "    {{ {}, {}u, {} }},\n",
            serde_json::to_string(component_name).map_err(|e| e.to_string())?,
            properties.len(),
            property_array_name
        ));
    }

    out.push_str("static const ComponentInfo kComponents[] = {\n");
    for entry in component_entries {
        out.push_str(&entry);
    }
    out.push_str(
        "};\n\n\
         static constexpr unsigned int kComponentCount = static_cast<unsigned int>(sizeof(kComponents) / sizeof(kComponents[0]));\n\
         } // namespace shadow_generated\n",
    );
    Ok(out)
}

fn load_reflection_schema(root: &Path) -> Result<BTreeMap<String, HashSet<String>>, String> {
    let reflect_path = root.join(".shadoweditor/shadow_reflect.json");
    if !reflect_path.exists() {
        return Ok(BTreeMap::new());
    }
    let json = std::fs::read_to_string(&reflect_path)
        .map_err(|e| format!("Failed to read {}: {}", reflect_path.display(), e))?;
    let value = serde_json::from_str::<serde_json::Value>(&json)
        .map_err(|e| format!("Failed to parse {}: {}", reflect_path.display(), e))?;
    let mut schema = BTreeMap::new();
    if let Some(components) = value.get("components").and_then(|items| items.as_array()) {
        for component in components {
            let Some(name) = component.get("name").and_then(|item| item.as_str()) else {
                continue;
            };
            let properties = component
                .get("properties")
                .and_then(|items| items.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|property| property.get("name").and_then(|item| item.as_str()))
                        .map(String::from)
                        .collect::<HashSet<_>>()
                })
                .unwrap_or_default();
            schema.insert(name.to_string(), properties);
        }
    }
    Ok(schema)
}

fn run_header_tool_for_project(root: &Path) -> Result<ShadowReflectResult, String> {
    let src_dir = root.join("src");
    let game_dir = root.join("game");
    if !src_dir.exists() && !game_dir.exists() {
        return Err(format!("No src/ or game/ directory in {}", root.display()));
    }

    let mut headers = Vec::new();
    collect_cpp_headers(&src_dir, &mut headers);
    collect_cpp_headers(&game_dir, &mut headers);
    if let Some(sdk) = find_sdk_include_path() {
        collect_cpp_headers(Path::new(&sdk), &mut headers);
    }
    headers.sort();
    headers.dedup();

    if headers.is_empty() {
        return Err("No .h or .hpp files found in src/ or game/".into());
    }

    let mut all_components = Vec::new();
    for path in &headers {
        if let Ok(source) = std::fs::read_to_string(path) {
            all_components.extend(parse_shadow_components(&source));
        }
    }

    let doc = serde_json::json!({ "components": all_components });
    let json_str = serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?;

    let out_dir = root.join(".shadoweditor");
    let _ = std::fs::create_dir_all(&out_dir);
    std::fs::write(out_dir.join("shadow_reflect.json"), &json_str)
        .map_err(|e| format!("Failed to write reflect JSON: {}", e))?;
    let generated_cpp = build_reflection_generated_cpp(&all_components)?;
    let generated_cpp_path = out_dir.join("shadow_reflect_generated.cpp");
    std::fs::write(&generated_cpp_path, generated_cpp)
        .map_err(|e| format!("Failed to write generated reflection C++: {}", e))?;

    Ok(ShadowReflectResult {
        component_count: all_components.len(),
        headers_scanned: headers.len(),
        json: json_str,
        generated_cpp_path: normalize_project_relative_path(root, generated_cpp_path),
    })
}

// ===== Inspector Suggestions =====

#[derive(serde::Serialize, Debug)]
pub struct ShadowSuggestion {
    pub entity_id: String,
    pub entity: String,
    pub message: String,
    pub kind: String, // "warning" | "info" | "tip"
    pub action_label: Option<String>,
    pub action_component_type: Option<String>,
}

/// Rule-based inspector suggestions from reflection + scene data (§7.5).
/// No AI required — purely structural analysis.
#[tauri::command]
pub async fn shadow_inspector_suggestions(
    project_path: String,
) -> Result<Vec<ShadowSuggestion>, String> {
    let root = Path::new(&project_path);
    let mut suggestions: Vec<ShadowSuggestion> = Vec::new();

    // Load reflection metadata
    let reflect_path = root.join(".shadoweditor/shadow_reflect.json");
    let known_components: std::collections::HashSet<String> = if reflect_path.exists() {
        if let Ok(json_str) = std::fs::read_to_string(&reflect_path) {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json_str) {
                val["components"]
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|c| c["name"].as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Default::default()
            }
        } else {
            Default::default()
        }
    } else {
        Default::default()
    };

    // Load and analyze all scene files
    let config_path = root.join(".shadow_project.toml");
    let Ok(config_str) = std::fs::read_to_string(&config_path) else {
        return Ok(suggestions);
    };
    let Ok(config) = toml::from_str::<toml::Value>(&config_str) else {
        return Ok(suggestions);
    };

    let scene_files: Vec<String> = config
        .get("scenes")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_else(|| {
            // fallback: scan scenes/ dir
            let mut found = Vec::new();
            let scenes_dir = root.join("scenes");
            if let Ok(entries) = std::fs::read_dir(&scenes_dir) {
                for e in entries.flatten() {
                    if e.path().extension().and_then(|x| x.to_str()) == Some("shadow") {
                        found.push(e.path().to_string_lossy().to_string());
                    }
                }
            }
            found
        });

    for scene_rel in &scene_files {
        let scene_path = if std::path::Path::new(scene_rel).is_absolute() {
            std::path::PathBuf::from(scene_rel)
        } else {
            root.join(scene_rel)
        };
        let Ok(content) = std::fs::read_to_string(&scene_path) else {
            continue;
        };
        let Ok(value) = toml::from_str::<toml::Value>(&content) else {
            continue;
        };
        let Some(entities) = value.get("entity").and_then(|e| e.as_array()) else {
            continue;
        };

        for entity_val in entities {
            let entity_name = entity_val
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("(unnamed)");
            let entity_id = entity_val
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or(entity_name)
                .to_string();
            let comps: Vec<String> = entity_val
                .get("component")
                .and_then(|c| c.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|c| {
                            c.get("type")
                                .and_then(|t| t.as_str())
                                .map(|s| s.to_string())
                        })
                        .collect()
                })
                .unwrap_or_default();

            let has_mesh = comps
                .iter()
                .any(|c| c == "MeshRenderer" || c == "SpriteRenderer");
            let has_transform = comps.iter().any(|c| c == "Transform");
            let has_physics = comps.iter().any(|c| {
                c.contains("RigidBody") || c.contains("Collider") || c.contains("Physics")
            });
            let has_health = comps.iter().any(|c| c == "Health" || c.contains("Health"));

            if has_mesh && !has_transform {
                suggestions.push(ShadowSuggestion {
                    entity_id: entity_id.clone(),
                    entity: entity_name.to_string(),
                    message:
                        "Has MeshRenderer but no Transform component — entity won't be positioned"
                            .to_string(),
                    kind: "warning".to_string(),
                    action_label: Some("Add Transform".to_string()),
                    action_component_type: Some("Transform".to_string()),
                });
            }
            if has_mesh && !has_physics {
                suggestions.push(ShadowSuggestion {
                    entity_id: entity_id.clone(),
                    entity: entity_name.to_string(),
                    message: "Has a mesh but no physics — add RigidBody or Collider if it should interact physically".to_string(),
                    kind: "tip".to_string(),
                    action_label: Some("Add RigidBody".to_string()),
                    action_component_type: Some("RigidBody".to_string()),
                });
            }
            if has_health {
                // Check for uninitialized Health.current == 0
                if let Some(comp_arr) = entity_val.get("component").and_then(|c| c.as_array()) {
                    for comp in comp_arr {
                        let ty = comp.get("type").and_then(|t| t.as_str()).unwrap_or("");
                        if ty.contains("Health") {
                            let current = comp
                                .get("current")
                                .and_then(|v| v.as_float())
                                .unwrap_or(0.0);
                            if current == 0.0 {
                                suggestions.push(ShadowSuggestion {
                                    entity_id: entity_id.clone(),
                                    entity: entity_name.to_string(),
                                    message: format!("{}.current is 0 — likely uninitialized", ty),
                                    kind: "warning".to_string(),
                                    action_label: None,
                                    action_component_type: None,
                                });
                            }
                        }
                    }
                }
            }
            // Player heuristic: entity named "Player" without a PlayerController
            let name_lower = entity_name.to_lowercase();
            if (name_lower.contains("player") || name_lower.contains("character"))
                && !comps
                    .iter()
                    .any(|c| c.contains("Controller") || c.contains("Movement"))
            {
                suggestions.push(ShadowSuggestion {
                    entity_id: entity_id.clone(),
                    entity: entity_name.to_string(),
                    message: "Entity looks like a player but has no Controller/Movement component"
                        .to_string(),
                    kind: "tip".to_string(),
                    action_label: Some("Add PlayerController".to_string()),
                    action_component_type: Some("PlayerController".to_string()),
                });
            }
            // Projectile heuristic
            if (name_lower.contains("bullet")
                || name_lower.contains("projectile")
                || name_lower.contains("arrow"))
                && !comps
                    .iter()
                    .any(|c| c.contains("Lifetime") || c.contains("Destroy"))
            {
                suggestions.push(ShadowSuggestion {
                    entity_id: entity_id.clone(),
                    entity: entity_name.to_string(),
                    message: "Looks like a projectile — consider adding a Lifetime component so it despawns".to_string(),
                    kind: "tip".to_string(),
                    action_label: Some("Add Lifetime".to_string()),
                    action_component_type: Some("Lifetime".to_string()),
                });
            }
            // Suggestion: known reflected components not yet used in any entity
            if !known_components.is_empty() {
                let unused: Vec<&String> = known_components
                    .iter()
                    .filter(|c| !comps.contains(c))
                    .filter(|c| !c.contains("Base") && !c.contains("Internal"))
                    .collect();
                if !unused.is_empty() && unused.len() <= 3 {
                    suggestions.push(ShadowSuggestion {
                        entity_id: entity_id.clone(),
                        entity: entity_name.to_string(),
                        message: format!(
                            "Unused components available: {}",
                            unused
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                        kind: "info".to_string(),
                        action_label: None,
                        action_component_type: None,
                    });
                }
            }
        }
    }

    // Deduplicate
    let mut seen = std::collections::HashSet::new();
    suggestions.retain(|s| seen.insert(format!("{}:{}", s.entity, s.message)));

    Ok(suggestions)
}

// ===== AI Chat History =====

/// Load AI chat history for a ShadowEditor project.
/// History is stored as newline-delimited JSON in .shadoweditor/ai_history.jsonl
#[tauri::command]
pub async fn shadow_ai_history_load(
    project_path: String,
) -> Result<Vec<serde_json::Value>, String> {
    let path = Path::new(&project_path).join(".shadoweditor/ai_history.jsonl");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let entries: Vec<serde_json::Value> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    Ok(entries)
}

/// Append a single message entry to the project's AI chat history.
#[tauri::command]
pub async fn shadow_ai_history_append(
    project_path: String,
    entry: serde_json::Value,
) -> Result<(), String> {
    let dir = Path::new(&project_path).join(".shadoweditor");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("ai_history.jsonl");
    let line = serde_json::to_string(&entry).map_err(|e| e.to_string())? + "\n";
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    f.write_all(line.as_bytes()).map_err(|e| e.to_string())?;
    Ok(())
}

/// Clear the AI chat history for a project.
#[tauri::command]
pub async fn shadow_ai_history_clear(project_path: String) -> Result<(), String> {
    let path = Path::new(&project_path).join(".shadoweditor/ai_history.jsonl");
    if path.exists() {
        std::fs::remove_file(&path).map_err(|e| e.to_string())?;
    }
    Ok(())
}
