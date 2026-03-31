use serde::Serialize;
use std::fs;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

// ===== Structs =====

#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct LocalModel {
    pub name: String,
    pub path: String,
    pub model_type: String,
    pub size_bytes: u64,
    #[serde(default)]
    pub architecture: Option<String>,
    #[serde(default)]
    pub quantization: Option<String>,
    #[serde(default)]
    pub parameter_count: Option<u64>,
    #[serde(default)]
    pub context_length: Option<u32>,
}

// ===== GGUF Metadata Parsing =====

#[derive(Debug, Default)]
struct GgufMeta {
    architecture: Option<String>,
    #[allow(dead_code)]
    name: Option<String>,
    context_length: Option<u32>,
    quantization: Option<String>,
}

const GGUF_MAGIC: u32 = 0x46475547; // "GGUF" in LE

/// Value type IDs in GGUF metadata
const GGUF_TYPE_U8: u32 = 0;
const GGUF_TYPE_I8: u32 = 1;
const GGUF_TYPE_U16: u32 = 2;
const GGUF_TYPE_I16: u32 = 3;
const GGUF_TYPE_U32: u32 = 4;
const GGUF_TYPE_I32: u32 = 5;
const GGUF_TYPE_F32: u32 = 6;
const GGUF_TYPE_BOOL: u32 = 7;
const GGUF_TYPE_STRING: u32 = 8;
const GGUF_TYPE_ARRAY: u32 = 9;
const GGUF_TYPE_U64: u32 = 10;
const GGUF_TYPE_I64: u32 = 11;
const GGUF_TYPE_F64: u32 = 12;

fn read_u32_le<R: Read>(r: &mut R) -> std::io::Result<u32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_u64_le<R: Read>(r: &mut R) -> std::io::Result<u64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

/// Read a GGUF string: u64 length + UTF-8 bytes (v3), or u32 length (v2).
fn read_gguf_string<R: Read>(r: &mut R, version: u32) -> std::io::Result<String> {
    let len = if version >= 3 {
        read_u64_le(r)? as usize
    } else {
        read_u32_le(r)? as usize
    };
    if len > 1_000_000 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "string too long",
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Skip a GGUF value of the given type, advancing the reader past it.
fn skip_gguf_value<R: Read + Seek>(
    r: &mut R,
    value_type: u32,
    version: u32,
) -> std::io::Result<()> {
    match value_type {
        GGUF_TYPE_U8 | GGUF_TYPE_I8 | GGUF_TYPE_BOOL => {
            r.seek(SeekFrom::Current(1))?;
        }
        GGUF_TYPE_U16 | GGUF_TYPE_I16 => {
            r.seek(SeekFrom::Current(2))?;
        }
        GGUF_TYPE_U32 | GGUF_TYPE_I32 | GGUF_TYPE_F32 => {
            r.seek(SeekFrom::Current(4))?;
        }
        GGUF_TYPE_U64 | GGUF_TYPE_I64 | GGUF_TYPE_F64 => {
            r.seek(SeekFrom::Current(8))?;
        }
        GGUF_TYPE_STRING => {
            read_gguf_string(r, version)?;
        }
        GGUF_TYPE_ARRAY => {
            let elem_type = read_u32_le(r)?;
            let count = if version >= 3 {
                read_u64_le(r)?
            } else {
                read_u32_le(r)? as u64
            };
            for _ in 0..count {
                skip_gguf_value(r, elem_type, version)?;
            }
        }
        _ => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "unknown GGUF value type",
            ));
        }
    }
    Ok(())
}

/// Parse GGUF file headers to extract metadata.
/// Returns None if the file is not a valid GGUF or cannot be read.
fn parse_gguf_metadata(path: &Path) -> Option<GgufMeta> {
    let file = fs::File::open(path).ok()?;
    let mut reader = BufReader::new(file);

    // Read magic
    let magic = read_u32_le(&mut reader).ok()?;
    if magic != GGUF_MAGIC {
        return None;
    }

    // Read version
    let version = read_u32_le(&mut reader).ok()?;
    if version < 2 || version > 3 {
        return None;
    }

    // Read tensor count (skip it)
    if version >= 3 {
        read_u64_le(&mut reader).ok()?;
    } else {
        read_u32_le(&mut reader).ok()?;
    }

    // Read metadata KV count
    let kv_count = if version >= 3 {
        read_u64_le(&mut reader).ok()?
    } else {
        read_u32_le(&mut reader).ok()? as u64
    };

    // Safety limit
    if kv_count > 100_000 {
        return None;
    }

    let mut meta = GgufMeta::default();

    for _ in 0..kv_count {
        let key = match read_gguf_string(&mut reader, version) {
            Ok(k) => k,
            Err(_) => return Some(meta), // Return what we have so far
        };

        let value_type = match read_u32_le(&mut reader) {
            Ok(t) => t,
            Err(_) => return Some(meta),
        };

        // Check if this is a key we care about
        if key == "general.architecture" && value_type == GGUF_TYPE_STRING {
            if let Ok(val) = read_gguf_string(&mut reader, version) {
                meta.architecture = Some(val);
            } else {
                return Some(meta);
            }
        } else if key == "general.name" && value_type == GGUF_TYPE_STRING {
            if let Ok(val) = read_gguf_string(&mut reader, version) {
                meta.name = Some(val);
            } else {
                return Some(meta);
            }
        } else if key.ends_with(".context_length") && value_type == GGUF_TYPE_U32 {
            if let Ok(val) = read_u32_le(&mut reader) {
                meta.context_length = Some(val);
            } else {
                return Some(meta);
            }
        } else {
            // Skip this value
            if skip_gguf_value(&mut reader, value_type, version).is_err() {
                return Some(meta);
            }
        }
    }

    Some(meta)
}

/// Extract quantization type from a filename.
/// e.g., "llama-3.1-8b-Q4_K_M.gguf" -> Some("Q4_K_M")
fn quant_from_filename(filename: &str) -> Option<String> {
    // Common quantization patterns in GGUF filenames
    let patterns = [
        "IQ1_S", "IQ1_M", "IQ2_XXS", "IQ2_XS", "IQ2_S", "IQ2_M", "IQ3_XXS", "IQ3_XS", "IQ3_S",
        "IQ3_M", "IQ4_XS", "IQ4_NL", "Q2_K_S", "Q2_K", "Q3_K_S", "Q3_K_M", "Q3_K_L", "Q3_K",
        "Q4_K_S", "Q4_K_M", "Q4_K_L", "Q4_K", "Q4_0", "Q4_1", "Q5_K_S", "Q5_K_M", "Q5_K_L", "Q5_K",
        "Q5_0", "Q5_1", "Q6_K", "Q8_0", "Q8_1", "F16", "F32", "BF16",
    ];

    let upper = filename.to_uppercase();
    // Check longer patterns first (they're already sorted longest-first within each group)
    for pat in &patterns {
        if upper.contains(pat) {
            return Some(pat.to_string());
        }
    }
    None
}

/// Estimate parameter count from file size and quantization type.
fn estimate_parameter_count(size_bytes: u64, quantization: Option<&str>) -> Option<u64> {
    let multiplier = match quantization {
        Some(q) => {
            let q_upper = q.to_uppercase();
            if q_upper.starts_with("Q2") {
                3.2
            }
            // ~0.3125 bytes/param
            else if q_upper.starts_with("IQ1") {
                5.0
            } else if q_upper.starts_with("IQ2") {
                3.2
            } else if q_upper.starts_with("IQ3") || q_upper.starts_with("Q3") {
                2.3
            } else if q_upper.starts_with("IQ4") || q_upper == "Q4_0" || q_upper == "Q4_1" {
                2.1
            } else if q_upper.starts_with("Q4_K") {
                2.0
            }
            // ~0.5 bytes/param
            else if q_upper.starts_with("Q5_K") {
                1.6
            }
            // ~0.625 bytes/param
            else if q_upper.starts_with("Q5") {
                1.7
            } else if q_upper.starts_with("Q6") {
                1.33
            }
            // ~0.75 bytes/param
            else if q_upper.starts_with("Q8") {
                1.0
            }
            // ~1.0 bytes/param
            else if q_upper == "F16" || q_upper == "BF16" {
                0.5
            }
            // 2.0 bytes/param
            else if q_upper == "F32" {
                0.25
            }
            // 4.0 bytes/param
            else {
                return None;
            }
        }
        None => return None,
    };

    Some((size_bytes as f64 * multiplier) as u64)
}

/// Enrich a LocalModel with GGUF metadata parsed from the file.
fn enrich_with_gguf_metadata(model: &mut LocalModel) {
    let path = Path::new(&model.path);
    let meta = parse_gguf_metadata(path);

    // Get quantization from filename as fallback
    let filename = path
        .file_name()
        .map(|f| f.to_string_lossy().to_string())
        .unwrap_or_default();
    let filename_quant = quant_from_filename(&filename);

    if let Some(meta) = meta {
        model.architecture = meta.architecture;
        model.context_length = meta.context_length;
        // Prefer GGUF metadata quantization, fall back to filename
        model.quantization = meta.quantization.or(filename_quant);
    } else {
        model.quantization = filename_quant;
    }

    model.parameter_count =
        estimate_parameter_count(model.size_bytes, model.quantization.as_deref());
}

// ===== Scanning =====

fn scan_gguf_models(gguf_dir: &Path) -> Vec<LocalModel> {
    let mut models = Vec::new();

    let entries = match fs::read_dir(gguf_dir) {
        Ok(e) => e,
        Err(_) => return models,
    };

    for entry in entries.flatten() {
        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_lowercase())
            .unwrap_or_default();

        if ext != "gguf" {
            continue;
        }

        let size_bytes = match fs::metadata(&path) {
            Ok(m) => m.len(),
            Err(_) => continue,
        };

        let name = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let mut model = LocalModel {
            name,
            path: path.to_string_lossy().to_string(),
            model_type: "gguf".to_string(),
            size_bytes,
            architecture: None,
            quantization: None,
            parameter_count: None,
            context_length: None,
        };
        enrich_with_gguf_metadata(&mut model);
        models.push(model);
    }

    models
}

fn scan_mlx_models(mlx_dir: &Path) -> Vec<LocalModel> {
    let mut models = Vec::new();

    let entries = match fs::read_dir(mlx_dir) {
        Ok(e) => e,
        Err(_) => return models,
    };

    for entry in entries.flatten() {
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        // MLX models have a config.json in the model directory
        let config_path = path.join("config.json");
        if !config_path.exists() || !config_path.is_file() {
            continue;
        }

        // Sum up all file sizes in the directory for total model size
        let size_bytes = dir_total_size(&path);

        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        models.push(LocalModel {
            name,
            path: path.to_string_lossy().to_string(),
            model_type: "mlx".to_string(),
            size_bytes,
            architecture: None,
            quantization: None,
            parameter_count: None,
            context_length: None,
        });
    }

    models
}

fn dir_total_size(dir: &Path) -> u64 {
    let mut total: u64 = 0;

    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };

    for entry in entries.flatten() {
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };

        if meta.is_file() {
            total += meta.len();
        } else if meta.is_dir() {
            total += dir_total_size(&entry.path());
        }
    }

    total
}

/// Recursively scan a directory for GGUF files (up to 2 levels deep).
fn scan_gguf_recursive(dir: &Path, depth: u8) -> Vec<LocalModel> {
    let mut models = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return models,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() {
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            if ext == "gguf" {
                let size_bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let name = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                let mut model = LocalModel {
                    name,
                    path: path.to_string_lossy().to_string(),
                    model_type: "gguf".to_string(),
                    size_bytes,
                    architecture: None,
                    quantization: None,
                    parameter_count: None,
                    context_length: None,
                };
                enrich_with_gguf_metadata(&mut model);
                models.push(model);
            }
        } else if path.is_dir() && depth < 3 {
            models.extend(scan_gguf_recursive(&path, depth + 1));
        }
    }
    models
}

/// Scan Ollama manifests to discover installed models.
/// Ollama stores manifests at ~/.ollama/models/manifests/registry.ollama.ai/library/<model>/<tag>
/// Each manifest references blobs in ~/.ollama/models/blobs/
fn scan_ollama_manifests(manifests_dir: &Path, home: &str) -> Vec<LocalModel> {
    let mut models = Vec::new();
    let blobs_dir = Path::new(home).join(".ollama/models/blobs");

    // Walk: manifests/registry.ollama.ai/library/<model_name>/<tag>
    let registry_dir = manifests_dir.join("registry.ollama.ai").join("library");
    let entries = match fs::read_dir(&registry_dir) {
        Ok(e) => e,
        Err(_) => return models,
    };

    for model_entry in entries.flatten() {
        if !model_entry.path().is_dir() {
            continue;
        }
        let model_name = model_entry.file_name().to_string_lossy().to_string();

        let tags = match fs::read_dir(model_entry.path()) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for tag_entry in tags.flatten() {
            if !tag_entry.path().is_file() {
                continue;
            }
            let tag = tag_entry.file_name().to_string_lossy().to_string();
            let display_name = format!("{}:{}", model_name, tag);

            // Parse manifest JSON to find the model layer blob
            let manifest_content = match fs::read_to_string(tag_entry.path()) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let manifest: serde_json::Value = match serde_json::from_str(&manifest_content) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Find the model layer (mediaType contains "model")
            let layers = match manifest["layers"].as_array() {
                Some(l) => l,
                None => continue,
            };
            let model_layer = layers.iter().find(|l| {
                l["mediaType"]
                    .as_str()
                    .map_or(false, |t| t.contains("model"))
            });

            if let Some(layer) = model_layer {
                let digest = match layer["digest"].as_str() {
                    Some(d) => d,
                    None => continue,
                };
                // Ollama blob path: sha256-<hash> (digest format: sha256:<hash>)
                let blob_name = digest.replace(':', "-");
                let blob_path = blobs_dir.join(&blob_name);

                let size_bytes = layer["size"]
                    .as_u64()
                    .unwrap_or_else(|| fs::metadata(&blob_path).map(|m| m.len()).unwrap_or(0));

                let mut model = LocalModel {
                    name: display_name,
                    path: blob_path.to_string_lossy().to_string(),
                    model_type: "ollama".to_string(),
                    size_bytes,
                    architecture: None,
                    quantization: None,
                    parameter_count: None,
                    context_length: None,
                };
                // Ollama blobs are GGUF files, try to parse metadata
                enrich_with_gguf_metadata(&mut model);
                models.push(model);
            }
        }
    }

    models
}

// ===== Tauri Command =====

#[tauri::command]
pub fn scan_local_models(base_path: String) -> Result<Vec<LocalModel>, String> {
    let mut models = Vec::new();
    let mut seen_paths = std::collections::HashSet::new();

    // If explicit base_path is provided but doesn't exist, return error
    let base = Path::new(&base_path);
    if !base_path.is_empty() && !base.exists() {
        return Err(format!("Base path does not exist: {}", base_path));
    }

    // Scan project-local models/ directory if base_path is valid
    if base.exists() && base.is_dir() {
        let gguf_dir = base.join("models").join("gguf");
        if gguf_dir.is_dir() {
            models.extend(scan_gguf_models(&gguf_dir));
        }
        let mlx_dir = base.join("models").join("mlx");
        if mlx_dir.is_dir() {
            models.extend(scan_mlx_models(&mlx_dir));
        }
    }

    // Scan well-known model directories
    let home = dirs_next::home_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    if !home.is_empty() {
        #[cfg(windows)]
        let mut scan_dirs = vec![
            // LM Studio models
            format!("{}/.lmstudio/models", home),
            format!("{}/.cache/lm-studio/models", home),
            // Shadow IDE models
            format!("{}/.local/share/shadow-ide/models", home),
            // Ollama models (blob storage)
            format!("{}/.ollama/models/blobs", home),
            // Common user model directories
            format!("{}/models", home),
            format!("{}/Models", home),
        ];
        #[cfg(not(windows))]
        let scan_dirs = vec![
            // LM Studio models
            format!("{}/.lmstudio/models", home),
            format!("{}/.cache/lm-studio/models", home),
            // Shadow IDE models
            format!("{}/.local/share/shadow-ide/models", home),
            // Ollama models (blob storage)
            format!("{}/.ollama/models/blobs", home),
            // Common user model directories
            format!("{}/models", home),
            format!("{}/Models", home),
        ];
        #[cfg(windows)]
        {
            // Windows-specific model locations
            let appdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
            if !appdata.is_empty() {
                scan_dirs.push(format!("{}/lm-studio/models", appdata));
                scan_dirs.push(format!("{}/shadow-ide/models", appdata));
            }
            let userprofile = std::env::var("USERPROFILE").unwrap_or_default();
            if !userprofile.is_empty() {
                scan_dirs.push(format!("{}/models", userprofile));
                scan_dirs.push(format!("{}/Models", userprofile));
            }
        }

        for dir_path in &scan_dirs {
            let dir = Path::new(dir_path);
            if dir.is_dir() {
                models.extend(scan_gguf_recursive(dir, 0));
                models.extend(scan_mlx_models(dir));
            }
        }

        // Scan Ollama manifests to discover installed models with friendly names
        let ollama_manifests_path = format!("{}/.ollama/models/manifests", home);
        let ollama_manifests = Path::new(&ollama_manifests_path);
        if ollama_manifests.is_dir() {
            models.extend(scan_ollama_manifests(ollama_manifests, &home));
        }
    }

    // Deduplicate by path
    models.retain(|m| seen_paths.insert(m.path.clone()));

    // Sort by size (largest first) for easier selection
    models.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));

    Ok(models)
}

// ===== Tests =====

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs as stdfs;

    /// Build a minimal valid GGUF v3 file with the given metadata key-value pairs.
    /// Each entry is (key, value_type, raw_value_bytes).
    fn build_gguf_v3(kvs: &[(&str, u32, &[u8])]) -> Vec<u8> {
        let mut buf: Vec<u8> = Vec::new();

        // Magic: "GGUF" = 0x46475547 LE
        buf.extend_from_slice(&GGUF_MAGIC.to_le_bytes());
        // Version: 3
        buf.extend_from_slice(&3u32.to_le_bytes());
        // Tensor count: 0
        buf.extend_from_slice(&0u64.to_le_bytes());
        // Metadata KV count
        buf.extend_from_slice(&(kvs.len() as u64).to_le_bytes());

        for (key, vtype, value_bytes) in kvs {
            // Key: u64 len + bytes
            buf.extend_from_slice(&(key.len() as u64).to_le_bytes());
            buf.extend_from_slice(key.as_bytes());
            // Value type
            buf.extend_from_slice(&vtype.to_le_bytes());
            // Value bytes (caller must format correctly)
            buf.extend_from_slice(value_bytes);
        }

        buf
    }

    /// Build a GGUF v3 string value (u64 len + bytes) for use in KV pairs.
    fn gguf_string_value(s: &str) -> Vec<u8> {
        let mut v = Vec::new();
        v.extend_from_slice(&(s.len() as u64).to_le_bytes());
        v.extend_from_slice(s.as_bytes());
        v
    }

    #[test]
    fn test_parse_gguf_metadata_valid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test-model-Q4_K_M.gguf");

        let arch_val = gguf_string_value("llama");
        let ctx_val = 4096u32.to_le_bytes();

        let data = build_gguf_v3(&[
            ("general.architecture", GGUF_TYPE_STRING, &arch_val),
            ("llama.context_length", GGUF_TYPE_U32, &ctx_val),
        ]);

        stdfs::write(&path, &data).unwrap();

        let meta = parse_gguf_metadata(&path);
        assert!(meta.is_some());
        let meta = meta.unwrap();
        assert_eq!(meta.architecture.as_deref(), Some("llama"));
        assert_eq!(meta.context_length, Some(4096));
    }

    #[test]
    fn test_parse_gguf_metadata_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-a-model.bin");
        stdfs::write(&path, b"this is not a GGUF file at all").unwrap();

        let meta = parse_gguf_metadata(&path);
        assert!(meta.is_none());
    }

    #[test]
    fn test_parse_gguf_metadata_wrong_magic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad-magic.gguf");
        let mut data = vec![0u8; 64];
        // Write wrong magic
        data[0..4].copy_from_slice(&0xDEADBEEFu32.to_le_bytes());
        stdfs::write(&path, &data).unwrap();

        assert!(parse_gguf_metadata(&path).is_none());
    }

    #[test]
    fn test_quant_from_filename() {
        assert_eq!(
            quant_from_filename("llama-3.1-8b-Q4_K_M.gguf"),
            Some("Q4_K_M".to_string())
        );
        assert_eq!(
            quant_from_filename("mistral-7b-instruct-Q6_K.gguf"),
            Some("Q6_K".to_string())
        );
        assert_eq!(
            quant_from_filename("phi-3-mini-F16.gguf"),
            Some("F16".to_string())
        );
        assert_eq!(
            quant_from_filename("model-Q8_0.gguf"),
            Some("Q8_0".to_string())
        );
        assert_eq!(
            quant_from_filename("model-Q5_K_S.gguf"),
            Some("Q5_K_S".to_string())
        );
        assert_eq!(quant_from_filename("some-random-model.gguf"), None);
        assert_eq!(quant_from_filename("readme.txt"), None);
    }

    #[test]
    fn test_estimate_parameter_count() {
        // Q4_K_M: ~0.5 bytes/param -> multiplier 2.0
        let params = estimate_parameter_count(5_000_000_000, Some("Q4_K_M"));
        assert!(params.is_some());
        assert_eq!(params.unwrap(), 10_000_000_000);

        // Q8_0: ~1.0 bytes/param -> multiplier 1.0
        let params = estimate_parameter_count(7_000_000_000, Some("Q8_0"));
        assert_eq!(params.unwrap(), 7_000_000_000);

        // F16: ~2.0 bytes/param -> multiplier 0.5
        let params = estimate_parameter_count(14_000_000_000, Some("F16"));
        assert_eq!(params.unwrap(), 7_000_000_000);

        // Unknown quant
        assert!(estimate_parameter_count(1000, Some("UNKNOWN")).is_none());

        // No quant
        assert!(estimate_parameter_count(1000, None).is_none());
    }

    #[test]
    fn test_parse_gguf_with_name_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("model.gguf");

        let arch_val = gguf_string_value("qwen2");
        let name_val = gguf_string_value("Qwen2-7B-Instruct");
        let ctx_val = 131072u32.to_le_bytes();

        let data = build_gguf_v3(&[
            ("general.architecture", GGUF_TYPE_STRING, &arch_val),
            ("general.name", GGUF_TYPE_STRING, &name_val),
            ("qwen2.context_length", GGUF_TYPE_U32, &ctx_val),
        ]);

        stdfs::write(&path, &data).unwrap();

        let meta = parse_gguf_metadata(&path).unwrap();
        assert_eq!(meta.architecture.as_deref(), Some("qwen2"));
        assert_eq!(meta.name.as_deref(), Some("Qwen2-7B-Instruct"));
        assert_eq!(meta.context_length, Some(131072));
    }

    #[test]
    fn test_enrich_with_gguf_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("llama-8b-Q4_K_M.gguf");

        let arch_val = gguf_string_value("llama");
        let ctx_val = 8192u32.to_le_bytes();

        let data = build_gguf_v3(&[
            ("general.architecture", GGUF_TYPE_STRING, &arch_val),
            ("llama.context_length", GGUF_TYPE_U32, &ctx_val),
        ]);

        stdfs::write(&path, &data).unwrap();
        let file_size = stdfs::metadata(&path).unwrap().len();

        let mut model = LocalModel {
            name: "llama-8b-Q4_K_M".to_string(),
            path: path.to_string_lossy().to_string(),
            model_type: "gguf".to_string(),
            size_bytes: file_size,
            architecture: None,
            quantization: None,
            parameter_count: None,
            context_length: None,
        };

        enrich_with_gguf_metadata(&mut model);

        assert_eq!(model.architecture.as_deref(), Some("llama"));
        assert_eq!(model.context_length, Some(8192));
        assert_eq!(model.quantization.as_deref(), Some("Q4_K_M"));
        // parameter_count should be estimated
        assert!(model.parameter_count.is_some());
    }

    #[test]
    fn test_scan_local_models_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let result = scan_local_models(dir.path().to_str().unwrap().to_string());
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_scan_local_models_gguf() {
        let dir = tempfile::tempdir().unwrap();
        let gguf_dir = dir.path().join("models").join("gguf");
        stdfs::create_dir_all(&gguf_dir).unwrap();
        stdfs::write(gguf_dir.join("llama-7b.gguf"), "fake model data").unwrap();
        stdfs::write(gguf_dir.join("mistral.gguf"), "more data").unwrap();
        stdfs::write(gguf_dir.join("readme.txt"), "not a model").unwrap();

        let result = scan_local_models(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.iter().all(|m| m.model_type == "gguf"));
        assert!(result.iter().any(|m| m.name == "llama-7b"));
        assert!(result.iter().any(|m| m.name == "mistral"));
    }

    #[test]
    fn test_scan_local_models_mlx() {
        let dir = tempfile::tempdir().unwrap();
        let mlx_dir = dir.path().join("models").join("mlx");
        let model_dir = mlx_dir.join("phi-3-mini");
        stdfs::create_dir_all(&model_dir).unwrap();
        stdfs::write(model_dir.join("config.json"), r#"{"model_type":"phi3"}"#).unwrap();
        stdfs::write(model_dir.join("weights.safetensors"), "fake weights").unwrap();

        // Directory without config.json should be skipped
        let no_config_dir = mlx_dir.join("incomplete-model");
        stdfs::create_dir_all(&no_config_dir).unwrap();
        stdfs::write(no_config_dir.join("weights.safetensors"), "data").unwrap();

        let result = scan_local_models(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "phi-3-mini");
        assert_eq!(result[0].model_type, "mlx");
        assert!(result[0].size_bytes > 0);
    }

    #[test]
    fn test_scan_local_models_mixed() {
        let dir = tempfile::tempdir().unwrap();

        // GGUF
        let gguf_dir = dir.path().join("models").join("gguf");
        stdfs::create_dir_all(&gguf_dir).unwrap();
        stdfs::write(gguf_dir.join("codellama.gguf"), "data").unwrap();

        // MLX
        let mlx_dir = dir.path().join("models").join("mlx");
        let model_dir = mlx_dir.join("qwen-7b");
        stdfs::create_dir_all(&model_dir).unwrap();
        stdfs::write(model_dir.join("config.json"), "{}").unwrap();

        let result = scan_local_models(dir.path().to_str().unwrap().to_string()).unwrap();
        assert_eq!(result.len(), 2);
        // Sorted alphabetically
        assert_eq!(result[0].name, "codellama");
        assert_eq!(result[1].name, "qwen-7b");
    }

    #[test]
    fn test_scan_local_models_invalid_path() {
        let result = scan_local_models("/nonexistent/path".to_string());
        assert!(result.is_err());
    }

    #[test]
    fn test_dir_total_size() {
        let dir = tempfile::tempdir().unwrap();
        stdfs::write(dir.path().join("a.txt"), "hello").unwrap(); // 5 bytes
        stdfs::write(dir.path().join("b.txt"), "world!").unwrap(); // 6 bytes

        let sub = dir.path().join("sub");
        stdfs::create_dir(&sub).unwrap();
        stdfs::write(sub.join("c.txt"), "hi").unwrap(); // 2 bytes

        assert_eq!(dir_total_size(dir.path()), 13);
    }

    #[test]
    fn test_scan_gguf_models_empty() {
        let dir = tempfile::tempdir().unwrap();
        let models = scan_gguf_models(dir.path());
        assert!(models.is_empty());
    }

    #[test]
    fn test_scan_mlx_models_empty() {
        let dir = tempfile::tempdir().unwrap();
        let models = scan_mlx_models(dir.path());
        assert!(models.is_empty());
    }
}
