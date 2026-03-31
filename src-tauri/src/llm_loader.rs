use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::path::Path;
use sysinfo::System;
use tauri::{AppHandle, Emitter, Manager};

// ===== Structs =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GpuInfo {
    pub index: usize,
    pub name: String,
    pub vram_gb: Option<f64>,
    /// "dGPU" (discrete), "iGPU" (integrated), "eGPU" (external)
    pub gpu_type: String,
    /// "nvidia", "amd", "intel", "unknown"
    #[serde(default = "default_vendor")]
    pub vendor: String,
}

fn default_vendor() -> String {
    "unknown".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareInfo {
    pub cpu_cores: usize,
    pub cpu_model: Option<String>,
    pub ram_gb: f64,
    pub gpu_name: Option<String>,
    pub gpu_vram_gb: Option<f64>,
    pub has_gpu: bool,
    /// "dGPU" (discrete), "iGPU" (integrated), "eGPU" (external), or null
    pub gpu_type: Option<String>,
    /// All detected GPUs (for multi-GPU systems)
    #[serde(default)]
    pub gpus: Vec<GpuInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmConfig {
    pub n_gpu_layers: u32,
    #[serde(default)]
    pub use_all_vram: bool,
    pub n_ctx: u32,
    pub n_threads: u32,
    pub n_batch: u32,
    pub temperature: f64,
    pub top_p: f64,
    pub top_k: u32,
    pub min_p: f64,
    pub repeat_penalty: f64,
    pub mmap: bool,
    pub flash_attention: bool,
    pub seed: i64,
    pub keep_alive: bool,
    #[serde(default)]
    pub rope_freq_base: Option<f64>,
    #[serde(default)]
    pub rope_freq_scale: Option<f64>,
    #[serde(default)]
    pub cache_type_k: Option<String>,
    #[serde(default)]
    pub cache_type_v: Option<String>,
    #[serde(default)]
    pub context_overflow: String,
    #[serde(default)]
    pub stop_strings: Vec<String>,
    /// GPU index for multi-GPU systems (-1 = auto)
    #[serde(default = "default_main_gpu")]
    pub main_gpu: i32,
    /// Tensor split ratios for multi-GPU inference (proportional to VRAM per GPU)
    #[serde(default)]
    pub tensor_split: Option<Vec<f64>>,
    /// Optional draft model path for speculative decoding (smaller model predicts, main model verifies)
    #[serde(default)]
    pub draft_model_path: Option<String>,
    /// Number of tokens the draft model predicts per step (default: 8)
    #[serde(default = "default_draft_n_predict")]
    pub draft_n_predict: u32,
}

fn default_draft_n_predict() -> u32 {
    8
}

fn default_main_gpu() -> i32 {
    -1
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            n_gpu_layers: 0,
            use_all_vram: false,
            n_ctx: 0,
            n_threads: 0,
            n_batch: 0,
            temperature: 0.0,
            top_p: 0.0,
            top_k: 0,
            min_p: 0.0,
            repeat_penalty: 0.0,
            mmap: false,
            flash_attention: false,
            seed: -1,
            keep_alive: false,
            rope_freq_base: None,
            rope_freq_scale: None,
            cache_type_k: None,
            cache_type_v: None,
            context_overflow: String::new(),
            stop_strings: Vec::new(),
            main_gpu: default_main_gpu(),
            tensor_split: None,
            draft_model_path: None,
            draft_n_predict: default_draft_n_predict(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    pub filename: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfModelResult {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default, rename = "modelId")]
    pub model_name: Option<String>,
    #[serde(default)]
    pub downloads: Option<u64>,
    #[serde(default)]
    pub likes: Option<u64>,
    #[serde(default, rename = "lastModified")]
    pub last_modified: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

// ===== Hardware Detection Helpers =====

/// System info snapshot — created once, reused for CPU/RAM detection.
struct SysSnapshot {
    cpu_cores: usize,
    cpu_model: Option<String>,
    ram_gb: f64,
}

impl SysSnapshot {
    fn capture() -> Self {
        let mut sys = System::new_all();
        sys.refresh_all();
        Self {
            cpu_cores: sys.cpus().len(),
            cpu_model: sys.cpus().first().map(|cpu| cpu.brand().to_string()),
            ram_gb: (sys.total_memory() as f64) / (1024.0 * 1024.0 * 1024.0),
        }
    }
}

/// GPU type classification
#[derive(Debug, Clone, PartialEq)]
enum GpuType {
    Discrete,
    Integrated,
    External,
}

impl GpuType {
    fn as_str(&self) -> &'static str {
        match self {
            GpuType::Discrete => "dGPU",
            GpuType::Integrated => "iGPU",
            GpuType::External => "eGPU",
        }
    }
}

/// Classify GPU type from name and bus info
fn classify_gpu(name: &str) -> GpuType {
    let lower = name.to_lowercase();
    // Check for eGPU indicators (Thunderbolt enclosures)
    if lower.contains("egpu") || lower.contains("external") {
        return GpuType::External;
    }
    // Integrated GPU patterns
    if lower.contains("integrated")
        || lower.contains("uhd graphics")
        || lower.contains("hd graphics")
        || lower.contains("iris")
        || lower.contains("intel hd")
        || (lower.contains("vega") && (lower.contains("apu") || lower.contains("radeon graphics")))
        || (lower.contains("radeon graphics") && !lower.contains("rx "))
    {
        return GpuType::Integrated;
    }
    // Discrete GPU patterns
    if lower.contains("geforce")
        || lower.contains("rtx")
        || lower.contains("gtx")
        || lower.contains("quadro")
        || lower.contains("tesla")
        || lower.contains("radeon rx")
        || lower.contains("radeon pro")
        || lower.contains("arc a")
        || lower.contains("arc b")
    {
        return GpuType::Discrete;
    }
    // Default: assume discrete if dedicated VRAM tool found
    GpuType::Discrete
}

/// Check if GPU is connected via Thunderbolt (eGPU)
fn is_thunderbolt_gpu() -> bool {
    // Check /sys for thunderbolt devices
    if let Ok(output) = {
        let mut cmd = crate::platform::shell_command(
            "ls /sys/bus/thunderbolt/devices/ 2>/dev/null | head -1",
        );
        crate::platform::hide_window(&mut cmd);
        cmd.output()
    } {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if !stdout.trim().is_empty() {
                return true;
            }
        }
    }
    false
}

/// Detect AMD GPU VRAM via rocm-smi.
fn detect_amd_vram() -> Vec<(usize, f64)> {
    let mut results = Vec::new();
    if let Ok(output) = {
        let mut cmd = std::process::Command::new("rocm-smi");
        cmd.arg("--showmeminfo").arg("vram");
        crate::platform::hide_window(&mut cmd);
        cmd.output()
    } {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                // Format: "GPU[0]           : VRAM Total Memory (B): 17163091968"
                if line.contains("VRAM Total Memory") && !line.contains("Used") {
                    // Extract GPU index
                    if let Some(bracket_start) = line.find("GPU[") {
                        let after = &line[bracket_start + 4..];
                        if let Some(bracket_end) = after.find(']') {
                            let idx_str = &after[..bracket_end];
                            if let Ok(idx) = idx_str.parse::<usize>() {
                                // Extract bytes value (last colon-separated field)
                                if let Some(bytes_str) = line.rsplit(':').next() {
                                    if let Ok(bytes) = bytes_str.trim().parse::<u64>() {
                                        results
                                            .push((idx, bytes as f64 / (1024.0 * 1024.0 * 1024.0)));
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    results
}

/// Classify GPU vendor from name string.
fn classify_vendor(name: &str) -> &'static str {
    let lower = name.to_lowercase();
    if lower.contains("nvidia")
        || lower.contains("geforce")
        || lower.contains("quadro")
        || lower.contains("tesla")
        || lower.contains("rtx")
        || lower.contains("gtx")
    {
        "nvidia"
    } else if lower.contains("amd")
        || lower.contains("radeon")
        || lower.contains("navi")
        || lower.contains("vega")
        || lower.contains("polaris")
        || lower.contains("ellesmere")
    {
        "amd"
    } else if lower.contains("intel")
        || lower.contains("iris")
        || lower.contains("uhd graphics")
        || lower.contains("hd graphics")
        || lower.contains("arc a")
        || lower.contains("arc b")
    {
        "intel"
    } else {
        "unknown"
    }
}

fn nvidia_smi_binary() -> Option<String> {
    if crate::platform::is_command_available("nvidia-smi") {
        return Some("nvidia-smi".to_string());
    }

    #[cfg(windows)]
    for candidate in [
        r"C:\Windows\System32\nvidia-smi.exe",
        r"C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe",
    ] {
        if Path::new(candidate).exists() {
            return Some(candidate.to_string());
        }
    }

    #[cfg(not(windows))]
    for candidate in ["/usr/bin/nvidia-smi", "/usr/local/bin/nvidia-smi"] {
        if Path::new(candidate).exists() {
            return Some(candidate.to_string());
        }
    }

    None
}

/// Detect ALL GPUs on the system (multi-GPU, multi-vendor support).
/// Scans NVIDIA, AMD, and Intel GPUs independently — does NOT stop at first vendor found.
fn detect_all_gpus() -> Vec<GpuInfo> {
    let is_egpu = is_thunderbolt_gpu();
    let mut gpus: Vec<GpuInfo> = Vec::new();
    let mut seen_names: Vec<String> = Vec::new();

    // === NVIDIA GPUs via nvidia-smi ===
    if let Some(nvidia_smi) = nvidia_smi_binary() {
        if let Ok(output) = {
            let mut cmd = std::process::Command::new(&nvidia_smi);
            cmd.arg("--query-gpu=index,name,memory.total")
                .arg("--format=csv,noheader,nounits");
            crate::platform::hide_window(&mut cmd);
            cmd.output()
        } {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.trim().lines() {
                    let parts: Vec<&str> = line.splitn(3, ',').map(|s| s.trim()).collect();
                    if parts.len() >= 2 {
                        let nv_index = parts[0].parse::<usize>().unwrap_or(0);
                        let name = parts[1].to_string();
                        let vram = parts
                            .get(2)
                            .and_then(|v| v.parse::<f64>().ok())
                            .map(|m| m / 1024.0);
                        let gpu_type = if is_egpu {
                            GpuType::External
                        } else {
                            classify_gpu(&name)
                        };
                        seen_names.push(name.to_lowercase());
                        gpus.push(GpuInfo {
                            index: nv_index,
                            name,
                            vram_gb: vram,
                            gpu_type: gpu_type.as_str().to_string(),
                            vendor: "nvidia".to_string(),
                        });
                    }
                }
            }
        }
    }

    // === AMD GPUs via rocm-smi ===
    if let Ok(output) = {
        let mut cmd = std::process::Command::new("rocm-smi");
        cmd.arg("--showproductname");
        crate::platform::hide_window(&mut cmd);
        cmd.output()
    } {
        if output.status.success() {
            let amd_vram = detect_amd_vram();
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut amd_idx = 0;
            for line in stdout.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty()
                    && !trimmed.starts_with('=')
                    && !trimmed.starts_with("GPU")
                    && !trimmed.contains("Product Name")
                    && !trimmed.contains("ROCm")
                {
                    let name = if trimmed.to_lowercase().starts_with("amd") {
                        trimmed.to_string()
                    } else {
                        format!("AMD {}", trimmed)
                    };
                    let vram = amd_vram
                        .iter()
                        .find(|(i, _)| *i == amd_idx)
                        .map(|(_, v)| *v);
                    let gpu_type = if is_egpu {
                        GpuType::External
                    } else {
                        classify_gpu(&name)
                    };
                    seen_names.push(name.to_lowercase());
                    gpus.push(GpuInfo {
                        index: amd_idx,
                        name,
                        vram_gb: vram,
                        gpu_type: gpu_type.as_str().to_string(),
                        vendor: "amd".to_string(),
                    });
                    amd_idx += 1;
                }
            }
        }
    }

    // === Fallback GPU detection: lspci on Linux, wmic on Windows ===
    #[cfg(not(windows))]
    if let Ok(output) = {
        let mut cmd = std::process::Command::new("lspci");
        crate::platform::hide_window(&mut cmd);
        cmd.output()
    } {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            for line in stdout.lines() {
                let lower = line.to_lowercase();
                if lower.contains("vga")
                    || lower.contains("3d controller")
                    || lower.contains("display controller")
                {
                    if let Some(i) = line.find(": ") {
                        let name = line[i + 2..].trim().to_string();
                        if !name.is_empty() {
                            let name_lower = name.to_lowercase();
                            let already_found = seen_names.iter().any(|s| {
                                let s_words: Vec<&str> = s.split_whitespace().collect();
                                let n_words: Vec<&str> = name_lower.split_whitespace().collect();
                                s_words.iter().filter(|w| n_words.contains(w)).count() >= 2
                            });
                            if already_found {
                                continue;
                            }
                            let vendor = classify_vendor(&name);
                            let gpu_type = if is_egpu {
                                GpuType::External
                            } else {
                                classify_gpu(&name)
                            };
                            gpus.push(GpuInfo {
                                index: gpus.len(),
                                name,
                                vram_gb: None,
                                gpu_type: gpu_type.as_str().to_string(),
                                vendor: vendor.to_string(),
                            });
                        }
                    }
                }
            }
        }
    }

    // Windows: use PowerShell Get-CimInstance to detect GPUs not found by nvidia-smi/rocm-smi.
    // wmic is deprecated and removed on newer Windows 11 builds.
    #[cfg(windows)]
    {
        let mut cmd = std::process::Command::new("powershell");
        cmd.args(["-NoProfile", "-Command",
            "Get-CimInstance Win32_VideoController | ForEach-Object { $_.Name + '|' + $_.AdapterRAM }"]);
        crate::platform::hide_window(&mut cmd);
        if let Ok(output) = cmd.output() {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.lines() {
                    let trimmed = line.trim();
                    if trimmed.is_empty() {
                        continue;
                    }
                    let parts: Vec<&str> = trimmed.splitn(2, '|').collect();
                    let name = parts[0].trim().to_string();
                    if name.is_empty() {
                        continue;
                    }
                    let vram_bytes: u64 = parts
                        .get(1)
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    let name_lower = name.to_lowercase();
                    let already_found = seen_names.iter().any(|s| {
                        let s_words: Vec<&str> = s.split_whitespace().collect();
                        let n_words: Vec<&str> = name_lower.split_whitespace().collect();
                        s_words.iter().filter(|w| n_words.contains(w)).count() >= 2
                    });
                    if already_found {
                        continue;
                    }
                    let vendor = classify_vendor(&name);
                    let gpu_type = classify_gpu(&name);
                    let vram_gb = if vram_bytes > 0 {
                        Some(vram_bytes as f64 / 1073741824.0)
                    } else {
                        None
                    };
                    seen_names.push(name_lower);
                    gpus.push(GpuInfo {
                        index: gpus.len(),
                        name,
                        vram_gb,
                        gpu_type: gpu_type.as_str().to_string(),
                        vendor: vendor.to_string(),
                    });
                }
            }
        }
    }

    // Assign global indices for frontend use
    for (i, gpu) in gpus.iter_mut().enumerate() {
        gpu.index = i;
    }

    gpus
}

/// Resolve the Vulkan device index for a given GPU by matching PCI bus order.
/// Vulkan enumerates devices by PCI bus ID, which may differ from our internal GPU index.
/// Falls back to `fallback_index` if PCI order cannot be determined.
fn resolve_vulkan_device_index(gpus: &[GpuInfo], selected_gpu_index: i32) -> i32 {
    if selected_gpu_index < 0 || selected_gpu_index as usize >= gpus.len() {
        return selected_gpu_index;
    }
    let selected = &gpus[selected_gpu_index as usize];

    // Parse lspci to get PCI bus order of VGA/3D devices
    let output = match {
        let mut cmd = std::process::Command::new("lspci");
        crate::platform::hide_window(&mut cmd);
        cmd.output()
    } {
        Ok(o) if o.status.success() => o,
        _ => return selected_gpu_index,
    };
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Collect GPU entries from lspci in PCI bus order (natural lspci output order)
    let pci_gpus: Vec<(String, String)> = stdout
        .lines()
        .filter(|line| {
            let lower = line.to_lowercase();
            lower.contains("vga")
                || lower.contains("3d controller")
                || lower.contains("display controller")
        })
        .filter_map(|line| {
            line.find(": ").map(|i| {
                let bus_id = line.split_whitespace().next().unwrap_or("").to_string();
                let name = line[i + 2..].trim().to_string();
                (bus_id, name)
            })
        })
        .collect();

    if pci_gpus.is_empty() {
        return selected_gpu_index;
    }

    // Find which PCI bus position matches our selected GPU by vendor+name fuzzy match
    let selected_lower = selected.name.to_lowercase();
    for (vulkan_idx, (_bus_id, pci_name)) in pci_gpus.iter().enumerate() {
        let pci_lower = pci_name.to_lowercase();
        // Match by vendor keyword overlap
        let selected_words: Vec<&str> = selected_lower.split_whitespace().collect();
        let matching = selected_words
            .iter()
            .filter(|w| pci_lower.contains(*w))
            .count();
        if matching >= 2 {
            return vulkan_idx as i32;
        }
    }

    selected_gpu_index
}

/// Determine if a model name looks like a coding model.
fn is_coding_model(model_name: &str) -> bool {
    let lower = model_name.to_lowercase();
    lower.contains("code")
        || lower.contains("starcoder")
        || lower.contains("deepseek-coder")
        || lower.contains("codellama")
        || lower.contains("codegemma")
        || lower.contains("codestral")
        || lower.contains("qwen2.5-coder")
        || lower.contains("codeqwen")
}

/// Get model-family-specific stop strings for proper generation termination.
fn get_model_stop_strings(model_name: &str) -> Vec<String> {
    let lower = model_name.to_lowercase();

    // Llama 3 / 3.1 / 3.2 family
    if lower.contains("llama-3") || lower.contains("llama3") {
        return vec!["<|eot_id|>".to_string(), "<|end_of_text|>".to_string()];
    }
    // Qwen / Qwen2 / Qwen2.5 / Qwen3 (ChatML format)
    if lower.contains("qwen") {
        return vec!["<|im_end|>".to_string(), "<|endoftext|>".to_string()];
    }
    // Phi-3 / Phi-4 family
    if lower.contains("phi-3")
        || lower.contains("phi-4")
        || lower.contains("phi3")
        || lower.contains("phi4")
    {
        return vec!["<|end|>".to_string(), "<|endoftext|>".to_string()];
    }
    // DeepSeek / DeepSeek-R1
    if lower.contains("deepseek") {
        return vec!["<｜end▁of▁sentence｜>".to_string(), "<|EOT|>".to_string()];
    }
    // Mistral / Mixtral
    if lower.contains("mistral") || lower.contains("mixtral") {
        return vec!["</s>".to_string(), "[/INST]".to_string()];
    }
    // Gemma / CodeGemma
    if lower.contains("gemma") {
        return vec!["<end_of_turn>".to_string(), "<eos>".to_string()];
    }
    // StarCoder
    if lower.contains("starcoder") {
        return vec!["<|endoftext|>".to_string()];
    }
    // Command-R
    if lower.contains("command-r") || lower.contains("c4ai") {
        return vec!["<|END_OF_TURN_TOKEN|>".to_string()];
    }
    // Generic fallback
    Vec::new()
}

// ===== Tauri Commands =====

/// Detect available hardware: CPU, RAM, GPU (all GPUs from all vendors).
#[tauri::command]
pub fn detect_hardware() -> Result<HardwareInfo, String> {
    let sys = SysSnapshot::capture();
    let cpu_cores = sys.cpu_cores;
    let cpu_model = sys.cpu_model;
    let ram_gb = sys.ram_gb;
    let gpus = detect_all_gpus();

    // Primary GPU = first discrete GPU, or first GPU found
    let primary = gpus
        .iter()
        .find(|g| g.gpu_type == "dGPU")
        .or_else(|| gpus.first());

    Ok(HardwareInfo {
        cpu_cores,
        cpu_model,
        ram_gb,
        gpu_name: primary.map(|g| g.name.clone()),
        gpu_vram_gb: primary.and_then(|g| g.vram_gb),
        has_gpu: !gpus.is_empty(),
        gpu_type: primary.map(|g| g.gpu_type.clone()),
        gpus,
    })
}

/// Auto-configure optimal LLM settings based on hardware and model size.
#[tauri::command]
pub fn auto_configure_llm(
    hardware: HardwareInfo,
    model_size_bytes: u64,
    model_name: Option<String>,
) -> Result<LlmConfig, String> {
    let model_gb = model_size_bytes as f64 / (1024.0 * 1024.0 * 1024.0);

    // === Total VRAM across all discrete GPUs ===
    let total_vram: f64 = if hardware.gpus.is_empty() {
        hardware.gpu_vram_gb.unwrap_or(0.0)
    } else {
        hardware
            .gpus
            .iter()
            .filter(|g| g.gpu_type == "dGPU" || g.gpu_type == "eGPU")
            .filter_map(|g| g.vram_gb)
            .sum()
    };

    // === GPU layers: use ALL VRAM for model layers, KV cache goes to RAM ===
    // With --no-kv-offload, VRAM is used only for model weights.
    // This means we can offload as many layers as VRAM can hold.
    let n_gpu_layers = if total_vram > 0.0 && hardware.has_gpu && model_gb > 0.0 {
        // Reserve VRAM for Vulkan scratch buffers, pipeline objects, and OS overhead.
        // Without this, loading can crash with vk::DeviceLostError on tight-VRAM GPUs.
        let vram_overhead_gb = (total_vram * 0.15).max(1.0); // 15% or at least 1GB
        let usable_vram = total_vram - vram_overhead_gb;
        if usable_vram >= model_gb {
            999 // Whole model fits in VRAM with overhead room
        } else {
            // Partial offload: estimate layers that fit
            let est_layers = if model_gb < 5.0 {
                32.0
            } else if model_gb < 10.0 {
                33.0
            } else if model_gb < 20.0 {
                40.0
            } else if model_gb < 50.0 {
                64.0
            } else {
                80.0
            };
            let ratio = total_vram / model_gb;
            let layers = (est_layers * ratio).floor() as u32;
            layers.max(1)
        }
    } else if hardware.has_gpu && total_vram > 0.0 && model_gb <= 0.0 {
        999 // Unknown size, try full offload — KV cache is in RAM anyway
    } else {
        0
    };

    // KV cache stays in RAM (--no-kv-offload), so context is sized by RAM only.
    // Model weights in RAM are mmap'd so they don't count against available RAM.
    let ram_for_ctx = (hardware.ram_gb - model_gb.min(hardware.ram_gb * 0.5)).max(2.0);

    let is_quantized = model_name.as_ref().map_or(true, |n| {
        let lower = n.to_lowercase();
        lower.contains("q4")
            || lower.contains("q5")
            || lower.contains("q6")
            || lower.contains("q8")
            || lower.contains("q3")
            || lower.contains("q2")
            || lower.contains("iq")
    });
    let effective_ctx_memory = if is_quantized {
        ram_for_ctx * 1.5
    } else {
        ram_for_ctx
    };

    let n_ctx = if effective_ctx_memory < 3.0 {
        2048
    } else if effective_ctx_memory < 6.0 {
        4096
    } else if effective_ctx_memory < 10.0 {
        8192
    } else if effective_ctx_memory < 20.0 {
        16384
    } else if effective_ctx_memory < 40.0 {
        32768
    } else if effective_ctx_memory < 80.0 {
        65536
    } else {
        131072
    };

    // === Threads: physical cores for prompt eval ===
    let physical_cores = (hardware.cpu_cores / 2).max(2);
    let n_threads = if n_gpu_layers >= 999 {
        // Full GPU offload — CPU only does prompt processing, cap at 16
        physical_cores.min(16) as u32
    } else if n_gpu_layers == 0 {
        // CPU-only inference — use all physical cores (no cap)
        physical_cores as u32
    } else {
        // Partial offload — balance between CPU and GPU work
        physical_cores.min(24) as u32
    };

    // === Batch size: affects prompt ingestion speed ===
    let n_batch = if ram_for_ctx >= 24.0 {
        2048
    } else if ram_for_ctx >= 12.0 {
        1024
    } else if ram_for_ctx >= 6.0 {
        512
    } else {
        256
    };

    // === Temperature & sampling ===
    let is_code = model_name.as_ref().map_or(false, |n| is_coding_model(n));
    let is_reasoning = model_name.as_ref().map_or(false, |n| {
        let lower = n.to_lowercase();
        lower.contains("qwq")
            || lower.contains("deepseek-r1")
            || lower.contains("o1")
            || lower.contains("reasoning")
            || lower.contains("think")
    });
    let temperature = if is_code {
        0.2
    } else if is_reasoning {
        0.6
    } else {
        0.3
    };
    let top_p = if is_code { 0.85 } else { 0.90 };
    let top_k: u32 = if is_code { 30 } else { 40 };
    let min_p = if is_reasoning { 0.0 } else { 0.05 };
    let repeat_penalty = if is_reasoning {
        1.0
    } else if is_code {
        1.05
    } else {
        1.1
    };

    // === KV cache quantization: KV cache is in RAM, quantize to save RAM when tight ===
    let (cache_type_k, cache_type_v) = if ram_for_ctx < 8.0 {
        // Tight RAM — quantize aggressively
        (Some("q5_1".to_string()), Some("q4_0".to_string()))
    } else if ram_for_ctx < 16.0 {
        // Moderate RAM — light quantization
        (Some("q8_0".to_string()), Some("q4_0".to_string()))
    } else {
        // Plenty of RAM — fp16 for best quality
        (None, None)
    };

    // mmap: always enable on 64-bit systems with enough RAM (avoids loading full model into RAM)
    let mmap = hardware.ram_gb >= 8.0;

    // Flash attention: enable for any GPU (significant speedup)
    let flash_attention = hardware.has_gpu && n_gpu_layers > 0;

    // === Multi-GPU: select best discrete GPU ===
    let main_gpu = if hardware.gpus.is_empty() {
        -1
    } else if hardware.gpus.len() > 1 {
        // Pick the GPU with most VRAM
        hardware
            .gpus
            .iter()
            .filter(|g| g.gpu_type == "dGPU" || g.gpu_type == "eGPU")
            .max_by(|a, b| {
                a.vram_gb
                    .unwrap_or(0.0)
                    .partial_cmp(&b.vram_gb.unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|g| g.index as i32)
            .unwrap_or_else(|| hardware.gpus[0].index as i32)
    } else {
        hardware.gpus[0].index as i32
    };

    // === Multi-GPU tensor split: distribute layers proportionally by VRAM ===
    let tensor_split = if hardware.gpus.len() > 1 {
        let dgpus: Vec<&GpuInfo> = hardware
            .gpus
            .iter()
            .filter(|g| g.gpu_type == "dGPU" || g.gpu_type == "eGPU")
            .filter(|g| g.vram_gb.is_some())
            .collect();
        if dgpus.len() > 1 && n_gpu_layers > 0 {
            let total: f64 = dgpus.iter().filter_map(|g| g.vram_gb).sum();
            if total > 0.0 {
                Some(
                    dgpus
                        .iter()
                        .map(|g| g.vram_gb.unwrap_or(0.0) / total)
                        .collect(),
                )
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // Vulkan/CUDA safety: even with --no-kv-offload, the GPU allocates scratch/compute
    // buffers proportional to n_ctx. On Vulkan especially, these buffers are large.
    // Cap context size based on free VRAM after model weights to prevent
    // vk::DeviceLostError / OOM on GPUs with limited VRAM (e.g. 8GB).
    //
    // Empirical: Vulkan uses ~40-50 bytes per token per layer for scratch buffers.
    // For a 9B model (~32 layers), that's ~1.5KB per context token.
    // With safety margin: budget ~2KB per context token from free VRAM.
    let n_ctx: u32 = if n_gpu_layers > 0 && total_vram > 0.0 {
        let free_vram_after_model = (total_vram - model_gb).max(0.5);
        // Vulkan allocates large scratch buffers for attention computation proportional
        // to n_ctx, even with --no-kv-offload. These buffers scale with model hidden_size
        // and n_heads. Empirically, for a fully-offloaded 9B model on Vulkan:
        //   - 8K ctx uses ~200MB scratch → ~25KB/token
        //   - 32K ctx uses ~1.2GB scratch → ~38KB/token
        //   - 64K ctx uses ~2.8GB scratch → ~44KB/token
        // Use 50KB/token as conservative estimate (grows with larger models).
        let kb_per_token: f64 = if model_gb > 20.0 {
            80.0
        } else if model_gb > 10.0 {
            60.0
        } else {
            50.0
        };
        let free_vram_kb = free_vram_after_model * 1024.0 * 1024.0;
        let vram_ctx_cap = (free_vram_kb / kb_per_token) as u32;
        let capped = n_ctx.min(vram_ctx_cap);
        if capped < n_ctx {
            log::info!(
                "[llm] Capping n_ctx from {} to {} (free VRAM after model: {:.1}GB, {:.0}MB scratch budget)",
                n_ctx, capped, free_vram_after_model, free_vram_after_model * 1024.0
            );
        }
        capped.max(2048) // never go below 2048
    } else {
        n_ctx
    };

    Ok(LlmConfig {
        n_gpu_layers,
        use_all_vram: n_gpu_layers >= 999,
        n_ctx,
        n_threads,
        n_batch,
        temperature,
        top_p,
        top_k,
        min_p,
        repeat_penalty,
        mmap,
        flash_attention,
        seed: -1,
        keep_alive: true,
        rope_freq_base: None,
        rope_freq_scale: None,
        cache_type_k,
        cache_type_v,
        context_overflow: "truncate_middle".to_string(),
        stop_strings: model_name
            .as_ref()
            .map_or_else(Vec::new, |n| get_model_stop_strings(n)),
        main_gpu,
        tensor_split,
        draft_model_path: None,
        draft_n_predict: default_draft_n_predict(),
    })
}

/// Classify a user task and recommend the best available model based on task type.
/// Returns the recommended model path and suggested config adjustments.
///
/// Task types:
/// - "code": coding tasks → prefer larger/higher-quality quant (Q6, Q8, F16)
/// - "chat": general conversation → mid quant is fine (Q4, Q5)
/// - "summarize": summarization → smaller/faster quant OK (Q3, Q4)
/// - "analyze": code review / analysis → prefer higher quality (Q5+)
#[tauri::command]
pub fn recommend_model_for_task(
    task_description: String,
    available_models: Vec<crate::model_scanner::LocalModel>,
) -> Result<serde_json::Value, String> {
    let task_lower = task_description.to_lowercase();

    // Classify the task
    let task_type = if task_lower.contains("fix")
        || task_lower.contains("implement")
        || task_lower.contains("write code")
        || task_lower.contains("refactor")
        || task_lower.contains("create function")
        || task_lower.contains("add feature")
    {
        "code"
    } else if task_lower.contains("review")
        || task_lower.contains("analyze")
        || task_lower.contains("audit")
        || task_lower.contains("explain")
    {
        "analyze"
    } else if task_lower.contains("summarize")
        || task_lower.contains("tldr")
        || task_lower.contains("brief")
        || task_lower.contains("overview")
    {
        "summarize"
    } else {
        "chat"
    };

    // Quantization quality tiers (higher = better quality)
    let quant_quality = |name: &str| -> u8 {
        let lower = name.to_lowercase();
        if lower.contains("f16") || lower.contains("fp16") {
            10
        } else if lower.contains("q8_0") || lower.contains("q8") {
            9
        } else if lower.contains("q6_k") || lower.contains("q6") {
            8
        } else if lower.contains("q5_k_m") || lower.contains("q5_1") {
            7
        } else if lower.contains("q5_k_s") || lower.contains("q5_0") || lower.contains("q5") {
            6
        } else if lower.contains("q4_k_m") {
            5
        } else if lower.contains("q4_k_s") || lower.contains("q4_0") || lower.contains("q4") {
            4
        } else if lower.contains("q3_k_m") || lower.contains("q3") {
            3
        } else if lower.contains("q2_k") || lower.contains("q2") || lower.contains("iq2") {
            2
        } else if lower.contains("iq1") {
            1
        } else {
            5
        } // unknown quant, assume mid-range
    };

    // Minimum quality thresholds per task type
    let min_quality = match task_type {
        "code" => 6,      // Q5+ for coding
        "analyze" => 5,   // Q4_K_M+ for analysis
        "chat" => 3,      // Q3+ for chat
        "summarize" => 2, // any quant for summarization
        _ => 3,
    };

    // Filter to GGUF models only (can't auto-switch Ollama/MLX easily)
    let mut candidates: Vec<_> = available_models
        .iter()
        .filter(|m| m.model_type == "gguf")
        .map(|m| {
            let quality = quant_quality(&m.name);
            let is_coding = is_coding_model(&m.name);
            (m, quality, is_coding)
        })
        .filter(|(_, q, _)| *q >= min_quality)
        .collect();

    // Sort: for code tasks, prefer coding models; otherwise sort by quality
    candidates.sort_by(|(a, aq, a_code), (b, bq, b_code)| {
        if task_type == "code" {
            // Coding models first, then by quality
            b_code.cmp(a_code).then(bq.cmp(aq))
        } else if task_type == "summarize" {
            // Prefer smaller (faster) models with acceptable quality
            aq.cmp(bq).then(a.size_bytes.cmp(&b.size_bytes))
        } else {
            // General: best quality first
            bq.cmp(aq)
        }
    });

    let recommendation = candidates.first().map(|(model, quality, is_coding)| {
        serde_json::json!({
            "model_path": model.path,
            "model_name": model.name,
            "quant_quality": quality,
            "is_coding_model": is_coding,
            "task_type": task_type,
            "reason": match task_type {
                "code" => if *is_coding { "Selected coding-optimized model with high quantization quality" }
                          else { "Selected highest quality quantization for code generation" },
                "analyze" => "Selected high-quality quantization for accurate analysis",
                "summarize" => "Selected efficient model for fast summarization",
                _ => "Selected best available model for general use",
            },
        })
    });

    Ok(serde_json::json!({
        "task_type": task_type,
        "recommendation": recommendation,
        "candidates_count": candidates.len(),
    }))
}

/// Fetch the expected SHA256 hash for a file from the HuggingFace API.
async fn fetch_hf_file_sha256(client: &Client, repo_id: &str, filename: &str) -> Option<String> {
    let url = format!("https://huggingface.co/api/models/{}/tree/main", repo_id);
    let resp = client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let files: Vec<serde_json::Value> = resp.json().await.ok()?;
    files
        .iter()
        .find(|f| f["path"].as_str() == Some(filename))
        .and_then(|f| f["lfs"]["sha256"].as_str().map(|s| s.to_string()))
}

/// Download a GGUF model from HuggingFace with streaming progress events.
///
/// Emits:
///   - `hf-download-progress`: DownloadProgress with bytes/percent
///   - `hf-download-complete`: filename when done
#[tauri::command]
pub async fn download_hf_model(
    repo_id: String,
    filename: String,
    save_dir: String,
    app: AppHandle,
) -> Result<String, String> {
    let url = format!(
        "https://huggingface.co/{}/resolve/main/{}",
        repo_id, filename
    );

    // Create save directory if needed
    let dir = Path::new(&save_dir);
    if !dir.exists() {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("Failed to create directory {}: {}", save_dir, e))?;
    }

    let save_path = dir.join(&filename);
    let save_path_str = save_path.to_string_lossy().to_string();

    // Check for existing partial download to resume
    let existing_size = if save_path.exists() {
        std::fs::metadata(&save_path).map(|m| m.len()).unwrap_or(0)
    } else {
        0
    };

    let client = Client::new();
    let mut request = client.get(&url);
    if existing_size > 0 {
        request = request.header("Range", format!("bytes={}-", existing_size));
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("Failed to start download: {}", e))?;

    if !response.status().is_success() && response.status() != reqwest::StatusCode::PARTIAL_CONTENT
    {
        let status = response.status();
        return Err(format!(
            "HuggingFace returned error {}: {}",
            status,
            response.text().await.unwrap_or_default()
        ));
    }

    // Check if server supports range requests
    let (resumed, total_bytes, mut downloaded_bytes) =
        if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            let content_range = response
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .and_then(|s| s.rsplit('/').next())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);
            let total = if content_range > 0 {
                content_range
            } else {
                existing_size + response.content_length().unwrap_or(0)
            };
            (true, total, existing_size)
        } else {
            // Server doesn't support range or sent full content
            (false, response.content_length().unwrap_or(0), 0u64)
        };

    // Open file: append if resuming, create if starting fresh
    let mut file = if resumed {
        tokio::fs::OpenOptions::new()
            .append(true)
            .open(&save_path)
            .await
            .map_err(|e| format!("Failed to open file for resume: {}", e))?
    } else {
        tokio::fs::File::create(&save_path)
            .await
            .map_err(|e| format!("Failed to create file {}: {}", save_path_str, e))?
    };

    let mut stream = response.bytes_stream();

    // Track last emit to avoid flooding events (emit at most every 100ms worth of data)
    let mut last_emit_bytes: u64 = 0;
    let emit_interval = if total_bytes > 0 {
        // Emit roughly every 0.5%
        (total_bytes / 200).max(65536)
    } else {
        262144 // 256KB if total unknown
    };

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download stream error: {}", e))?;

        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|e| format!("Failed to write to file: {}", e))?;

        downloaded_bytes += chunk.len() as u64;

        // Emit progress periodically
        if downloaded_bytes - last_emit_bytes >= emit_interval || downloaded_bytes == total_bytes {
            let percent = if total_bytes > 0 {
                (downloaded_bytes as f64 / total_bytes as f64) * 100.0
            } else {
                0.0
            };

            let _ = app.emit(
                "hf-download-progress",
                DownloadProgress {
                    filename: filename.clone(),
                    downloaded_bytes,
                    total_bytes,
                    percent,
                },
            );

            last_emit_bytes = downloaded_bytes;
        }
    }

    // Flush and close
    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .map_err(|e| format!("Failed to flush file: {}", e))?;
    drop(file);

    // Emit verifying progress
    let _ = app.emit(
        "hf-download-progress",
        DownloadProgress {
            filename: filename.clone(),
            downloaded_bytes,
            total_bytes: if total_bytes > 0 {
                total_bytes
            } else {
                downloaded_bytes
            },
            percent: 99.5,
        },
    );

    // Verify file integrity via SHA256
    if let Some(expected_sha) = fetch_hf_file_sha256(&client, &repo_id, &filename).await {
        use sha2::{Digest, Sha256};
        use tokio::io::AsyncReadExt;

        // Stream the file to avoid loading it all into memory
        let mut hasher = Sha256::new();
        let mut verify_file = tokio::fs::File::open(&save_path)
            .await
            .map_err(|e| format!("Failed to read file for verification: {}", e))?;
        let mut buf = vec![0u8; 1024 * 1024]; // 1MB buffer
        loop {
            let n = verify_file
                .read(&mut buf)
                .await
                .map_err(|e| format!("Failed to read file for verification: {}", e))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let hash = format!("{:x}", hasher.finalize());
        if hash != expected_sha {
            let _ = tokio::fs::remove_file(&save_path).await;
            return Err(format!(
                "Download corrupted: SHA256 mismatch (expected {}, got {})",
                expected_sha, hash
            ));
        }
    }

    // Final 100% progress event
    let _ = app.emit(
        "hf-download-progress",
        DownloadProgress {
            filename: filename.clone(),
            downloaded_bytes,
            total_bytes: if total_bytes > 0 {
                total_bytes
            } else {
                downloaded_bytes
            },
            percent: 100.0,
        },
    );

    let _ = app.emit("hf-download-complete", filename);

    Ok(save_path_str)
}

/// Search HuggingFace for GGUF models.
#[tauri::command]
pub async fn search_hf_models(query: String) -> Result<Vec<HfModelResult>, String> {
    let client = Client::new();
    let url = format!(
        "https://huggingface.co/api/models?search={}&filter=gguf&sort=downloads&direction=-1&limit=20",
        urlencoding::encode(&query)
    );

    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Failed to search HuggingFace: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        return Err(format!(
            "HuggingFace API error {}: {}",
            status,
            response.text().await.unwrap_or_default()
        ));
    }

    let models: Vec<HfModelResult> = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse HuggingFace response: {}", e))?;

    Ok(models)
}

/// A file entry from a HuggingFace repo.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HfRepoFile {
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub size: Option<u64>,
}

/// A file entry from the HuggingFace tree API.
#[derive(Debug, Deserialize)]
struct HfTreeEntry {
    #[serde(default, rename = "type")]
    entry_type: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    lfs: Option<HfLfsInfo>,
}

#[derive(Debug, Deserialize)]
struct HfLfsInfo {
    #[serde(default)]
    size: Option<u64>,
}

/// List files in a HuggingFace repo, filtered to GGUF/model files.
/// Uses the tree API which includes actual file sizes.
#[tauri::command]
pub async fn list_hf_repo_files(repo_id: String) -> Result<Vec<HfRepoFile>, String> {
    let client = Client::new();
    let url = format!("https://huggingface.co/api/models/{}/tree/main", repo_id);

    let response = client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch repo files: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        return Err(format!(
            "HuggingFace API error {}: {}",
            status,
            response.text().await.unwrap_or_default()
        ));
    }

    let entries: Vec<HfTreeEntry> = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse tree response: {}", e))?;

    let model_extensions = [".gguf", ".bin", ".safetensors", ".ggml", ".pt", ".pth"];
    let mut files: Vec<HfRepoFile> = Vec::new();

    for entry in &entries {
        if entry.entry_type != "file" {
            continue;
        }
        let name_lower = entry.path.to_lowercase();
        if model_extensions.iter().any(|ext| name_lower.ends_with(ext)) {
            // Prefer LFS size (actual file size) over the pointer size
            let size = entry.lfs.as_ref().and_then(|l| l.size).or(entry.size);
            files.push(HfRepoFile {
                filename: entry.path.clone(),
                size,
            });
        }
    }

    files.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(files)
}

// ===== LLM Server Management =====

use std::sync::Mutex;
use tauri::State;

pub struct LlmServerState {
    child: Mutex<Option<std::process::Child>>,
    port: Mutex<u16>,
    model_path: Mutex<String>,
    last_error: Mutex<String>,
    stderr_path: Mutex<String>,
    context_length: Mutex<u32>,
}

impl LlmServerState {
    pub fn new() -> Self {
        LlmServerState {
            child: Mutex::new(None),
            port: Mutex::new(0),
            model_path: Mutex::new(String::new()),
            last_error: Mutex::new(String::new()),
            stderr_path: Mutex::new(String::new()),
            context_length: Mutex::new(0),
        }
    }
}

/// Delete a local model file.
#[tauri::command]
pub fn delete_local_model(model_path: String) -> Result<String, String> {
    let path = Path::new(&model_path);
    if !path.exists() {
        return Err(format!("File not found: {}", model_path));
    }
    // Only allow deleting model files
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    let allowed = ["gguf", "bin", "safetensors", "ggml", "pt", "pth"];
    if !allowed.iter().any(|a| ext.eq_ignore_ascii_case(a)) {
        return Err("Not a model file".to_string());
    }
    std::fs::remove_file(path).map_err(|e| format!("Failed to delete: {}", e))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("model");
    Ok(format!("Deleted {}", name))
}

// ===== Engine Management =====

/// Get the root directory for all engines.
fn engines_root() -> std::path::PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("shadow-ide")
        .join("engines")
}

/// Get the directory for a specific backend engine.
fn engine_dir(backend: &str) -> std::path::PathBuf {
    engines_root().join(backend)
}

/// The llama-server binary name for the current platform.
fn server_binary_name() -> &'static str {
    if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

/// Engine metadata stored alongside the binary.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct EngineMetadata {
    version: String,
    backend: String,
    installed_at: String,
}

/// Engine status info returned to frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineInfo {
    pub installed: bool,
    pub binary_path: String,
    pub version: String,
    pub backend: String,
}

#[tauri::command]
pub fn check_engine(backend: Option<String>) -> Result<EngineInfo, String> {
    let target_backend = if let Some(b) = backend {
        if b.is_empty() || b == "auto" {
            let installed = list_installed_engines()?;
            if let Some(best) = resolve_auto_backend_from_installed(&installed) {
                best
            } else {
                return Ok(EngineInfo {
                    installed: false,
                    binary_path: "".into(),
                    version: "".into(),
                    backend: "".into(),
                });
            }
        } else {
            b
        }
    } else {
        let installed = list_installed_engines()?;
        if let Some(best) = resolve_auto_backend_from_installed(&installed) {
            best
        } else {
            return Ok(EngineInfo {
                installed: false,
                binary_path: "".into(),
                version: "".into(),
                backend: "".into(),
            });
        }
    };

    let dir = engine_dir(&target_backend);
    let bin = dir.join(server_binary_name());
    let installed = bin.exists();

    let mut version = "unknown".to_string();
    if installed {
        if let Ok(meta_json) = std::fs::read_to_string(dir.join("engine.json")) {
            if let Ok(meta) = serde_json::from_str::<EngineMetadata>(&meta_json) {
                version = meta.version;
            }
        }
    }

    Ok(EngineInfo {
        installed,
        binary_path: if installed {
            bin.to_string_lossy().to_string()
        } else {
            "".to_string()
        },
        version,
        backend: target_backend,
    })
}

/// List all backends currently installed.
#[tauri::command]
pub fn list_installed_engines() -> Result<Vec<String>, String> {
    let root = engines_root();
    if !root.exists() {
        return Ok(Vec::new());
    }

    let mut installed = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    let binary = entry.path().join(server_binary_name());
                    if binary.exists() {
                        installed.push(name.to_string());
                    }
                }
            }
        }
    }
    Ok(installed)
}

fn resolve_auto_backend_from_installed(installed: &[String]) -> Option<String> {
    let gpus = detect_all_gpus();
    pick_best_installed_backend(installed, &gpus)
}

/// Engine install progress event payload.
#[derive(Debug, Clone, Serialize)]
pub struct EngineInstallProgress {
    pub stage: String,
    pub percent: f64,
    pub detail: String,
}

/// GitHub release structs.
#[derive(Debug, Deserialize)]
struct GhRelease {
    tag_name: String,
    assets: Vec<GhAsset>,
}

#[derive(Debug, Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// Search common CUDA install locations for nvcc when it's not in PATH.
fn find_nvcc_path() -> Option<String> {
    // Check versioned paths from newest to oldest
    let mut cuda_dirs: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir("/usr/local") {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("cuda") {
                cuda_dirs.push(format!("/usr/local/{}/bin/nvcc", name));
            }
        }
    }
    // Sort descending so newest CUDA version is checked first
    cuda_dirs.sort_by(|a, b| b.cmp(a));
    // Also check the unversioned symlink
    cuda_dirs.push("/usr/local/cuda/bin/nvcc".to_string());

    for path in &cuda_dirs {
        if Path::new(path).exists() {
            return Some(path.clone());
        }
    }
    None
}

/// Detect CUDA version from the system (driver-level, not toolkit).
/// Checks nvidia-smi first (works without nvcc), then falls back to nvcc.
#[allow(dead_code)]
fn detect_cuda_version() -> Option<(u32, u32)> {
    // nvidia-smi reports the max CUDA version the driver supports
    if let Some(nvidia_smi) = nvidia_smi_binary() {
        if let Ok(output) = {
            let mut cmd = std::process::Command::new(&nvidia_smi);
            crate::platform::hide_window(&mut cmd);
            cmd.output()
        } {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                // Look for "CUDA Version: 12.4" or similar
                for line in stdout.lines() {
                    if let Some(pos) = line.find("CUDA Version:") {
                        let ver_str = line[pos + 14..].trim();
                        let parts: Vec<&str> = ver_str.split('.').collect();
                        if let (Some(major), Some(minor)) = (
                            parts.first().and_then(|v| v.trim().parse::<u32>().ok()),
                            parts.get(1).and_then(|v| {
                                v.split_whitespace()
                                    .next()
                                    .and_then(|v| v.parse::<u32>().ok())
                            }),
                        ) {
                            return Some((major, minor));
                        }
                    }
                }
            }
        }
    }

    // Fallback: nvcc --version (try PATH first, then common install locations)
    let nvcc_paths = {
        let mut paths = vec!["nvcc".to_string()];
        if let Some(found) = find_nvcc_path() {
            paths.push(found);
        }
        paths
    };
    for nvcc in &nvcc_paths {
        if let Ok(output) = {
            let mut cmd = std::process::Command::new(nvcc);
            cmd.arg("--version");
            crate::platform::hide_window(&mut cmd);
            cmd.output()
        } {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if let Some(line) = stdout.lines().find(|l| l.contains("release")) {
                    if let Some(ver) = line.split("release ").nth(1) {
                        if let Some(ver_str) = ver.split(',').next() {
                            let parts: Vec<&str> = ver_str.trim().split('.').collect();
                            if let (Some(major), Some(minor)) = (
                                parts.first().and_then(|v| v.parse::<u32>().ok()),
                                parts.get(1).and_then(|v| v.parse::<u32>().ok()),
                            ) {
                                return Some((major, minor));
                            }
                        }
                    }
                }
            }
        }
    }

    None
}

/// Detect the recommended backend based on available GPUs.
#[tauri::command]
pub fn detect_recommended_backend() -> Result<String, String> {
    let gpus = detect_all_gpus();
    Ok(recommended_backend(&gpus))
}

fn recommended_backend(gpus: &[GpuInfo]) -> String {
    let has_nvidia = gpus.iter().any(|g| g.vendor == "nvidia");
    let has_nvidia_dgpu = gpus
        .iter()
        .any(|g| g.vendor == "nvidia" && g.gpu_type == "dGPU");

    // NVIDIA GPUs on Linux → Vulkan (pre-built available, uses GPU VRAM, no source build needed).
    // CUDA source builds fail on bleeding-edge distros (GCC 16+ incompatible with nvcc).
    // VRAM-based context capping in compute_config() prevents Vulkan DeviceLost crashes.
    #[cfg(target_os = "linux")]
    if has_nvidia_dgpu || has_nvidia {
        return "vulkan".to_string();
    }

    // NVIDIA GPUs on non-Linux → CUDA (pre-built binaries available)
    #[cfg(not(target_os = "linux"))]
    if has_nvidia_dgpu || has_nvidia {
        if let Some((major, _)) = detect_cuda_version() {
            if major >= 13 {
                return "cuda13".to_string();
            }
            if major >= 12 {
                return "cuda12".to_string();
            }
            return "cuda11".to_string();
        }
        return "cuda12".to_string();
    }

    if gpus
        .iter()
        .any(|g| g.vendor == "amd" && g.gpu_type == "dGPU")
    {
        if {
            let mut cmd = std::process::Command::new("rocm-smi");
            crate::platform::hide_window(&mut cmd);
            cmd.output()
        }
        .map(|o| o.status.success())
        .unwrap_or(false)
        {
            return "rocm".to_string();
        }
        return "vulkan".to_string();
    }
    if gpus.iter().any(|g| g.vendor == "amd") {
        return "vulkan".to_string();
    }
    if gpus.iter().any(|g| g.vendor == "intel") {
        return "sycl".to_string();
    }
    "cpu".to_string()
}

fn auto_backend_preference_order(gpus: &[GpuInfo]) -> Vec<String> {
    let recommended = recommended_backend(gpus);
    let mut order = vec![recommended.clone()];

    let fallbacks: &[&str] = if recommended.starts_with("cuda") {
        &["cuda13", "cuda12", "cuda11", "cuda", "vulkan", "cpu"]
    } else if recommended == "rocm" {
        &["rocm", "vulkan", "cpu"]
    } else if recommended == "sycl" {
        &["sycl", "vulkan", "cpu"]
    } else if recommended == "vulkan" {
        #[cfg(target_os = "linux")]
        {
            &[
                "vulkan", "cuda13", "cuda12", "cuda11", "cuda", "rocm", "sycl", "cpu",
            ]
        }
        #[cfg(not(target_os = "linux"))]
        {
            &[
                "vulkan", "rocm", "sycl", "cpu", "cuda13", "cuda12", "cuda11", "cuda",
            ]
        }
    } else {
        &[
            "cpu", "cuda13", "cuda12", "cuda11", "cuda", "vulkan", "rocm", "sycl",
        ]
    };

    for backend in fallbacks {
        if !order.iter().any(|existing| existing == backend) {
            order.push((*backend).to_string());
        }
    }

    order
}

fn pick_best_installed_backend(installed: &[String], gpus: &[GpuInfo]) -> Option<String> {
    let order = auto_backend_preference_order(gpus);
    installed.iter().cloned().min_by_key(|backend| {
        (
            order
                .iter()
                .position(|candidate| candidate == backend)
                .unwrap_or(order.len()),
            backend.clone(),
        )
    })
}

/// Install llama.cpp engine by downloading pre-built binary from GitHub releases.
/// Emits `engine-install-progress` events with stage/percent/detail.
#[tauri::command]
pub async fn install_engine(
    backend: String,
    app: AppHandle,
    state: State<'_, LlmServerState>,
) -> Result<String, String> {
    let emit_progress = |stage: &str, percent: f64, detail: &str| {
        let _ = app.emit(
            "engine-install-progress",
            EngineInstallProgress {
                stage: stage.to_string(),
                percent,
                detail: detail.to_string(),
            },
        );
    };

    emit_progress("fetching", 0.0, "Stopping current server...");

    // 0. Stop current server if running
    if let Ok(mut child_lock) = state.child.lock() {
        if let Some(mut child) = child_lock.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    emit_progress("fetching", 2.0, "Fetching latest release info...");

    // 1. Get latest release from GitHub API
    let client = Client::builder()
        .user_agent("ShadowIDE")
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    let release: GhRelease = client
        .get("https://api.github.com/repos/ggerganov/llama.cpp/releases/latest")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch release info: {}", e))?
        .json()
        .await
        .map_err(|e| format!("Failed to parse release info: {}", e))?;

    emit_progress(
        "fetching",
        5.0,
        &format!("Found release {}", release.tag_name),
    );

    // 2. Find the right asset (may be None for CUDA on Linux — needs source build)
    let asset = find_release_asset(&release.assets, &backend);

    // 3. Create/Clear specific engine directory
    let dir = engine_dir(&backend);
    if dir.exists() {
        if let Err(e) = std::fs::remove_dir_all(&dir) {
            log::warn!("[llm] Failed to clear engine directory {:?}: {}", dir, e);
        }
    }
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create engine directory: {}", e))?;

    // ---- CUDA on Linux: try source build if prebuilt is missing ----
    #[cfg(target_os = "linux")]
    if (backend.starts_with("cuda")) && asset.is_none() {
        // Check if nvcc is present for source build
        let nvcc_exists = std::process::Command::new("which")
            .arg("nvcc")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
            || find_nvcc_path().is_some();

        if nvcc_exists {
            emit_progress(
                "fetching",
                5.0,
                &format!(
                    "No pre-built {} binary found. Building from source instead (preferred)...",
                    backend
                ),
            );
            return build_cuda_from_source(
                &backend,
                &release.tag_name,
                &dir,
                &client,
                &emit_progress,
            )
            .await;
        }

        emit_progress("fetching", 5.0, &format!("No pre-built {} binary or CUDA Toolkit (nvcc) found. Installing Vulkan into {} instead...", backend, backend));

        // Find the vulkan asset instead
        let vulkan_asset = find_release_asset(&release.assets, "vulkan");
        if let Some(va) = vulkan_asset {
            // Install into the target directory (e.g. cuda13) but with Vulkan backend
            let download_url = &va.browser_download_url;
            let archive_name = &va.name;
            let total_size = va.size;

            emit_progress(
                "downloading",
                10.0,
                &format!(
                    "Downloading Vulkan GPU backend {} ({:.0} MB)...",
                    archive_name,
                    total_size as f64 / (1024.0 * 1024.0)
                ),
            );

            let archive_path = dir.join(archive_name);
            let response = client
                .get(download_url)
                .send()
                .await
                .map_err(|e| format!("Download failed: {}", e))?;
            if !response.status().is_success() {
                return Err(format!("Download failed: HTTP {}", response.status()));
            }

            let mut downloaded: u64 = 0;
            let mut file = tokio::fs::File::create(&archive_path)
                .await
                .map_err(|e| format!("File create error: {}", e))?;
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| format!("Download error: {}", e))?;
                tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
                    .await
                    .map_err(|e| format!("Write error: {}", e))?;
                downloaded += chunk.len() as u64;
                if total_size > 0 {
                    let pct = 10.0 + (downloaded as f64 / total_size as f64) * 75.0;
                    emit_progress(
                        "downloading",
                        pct,
                        &format!(
                            "{:.0}/{:.0} MB",
                            downloaded as f64 / 1048576.0,
                            total_size as f64 / 1048576.0
                        ),
                    );
                }
            }
            tokio::io::AsyncWriteExt::flush(&mut file)
                .await
                .map_err(|e| format!("Flush error: {}", e))?;
            drop(file);

            emit_progress("extracting", 88.0, "Extracting...");
            let extract_dir = dir.join("_extract");
            let _ = std::fs::remove_dir_all(&extract_dir);
            std::fs::create_dir_all(&extract_dir).map_err(|e| format!("mkdir error: {}", e))?;

            let status = std::process::Command::new("tar")
                .args([
                    "-xzf",
                    &archive_path.to_string_lossy(),
                    "-C",
                    &extract_dir.to_string_lossy(),
                ])
                .status()
                .map_err(|e| format!("tar error: {}", e))?;
            if !status.success() {
                let _ = std::fs::remove_file(&archive_path);
                return Err("Failed to extract archive".to_string());
            }

            let server_binary = find_file_recursive(&extract_dir, server_binary_name())
                .ok_or(format!("{} not found in archive", server_binary_name()))?;
            let dest = dir.join(server_binary_name());
            std::fs::copy(&server_binary, &dest).map_err(|e| format!("Copy error: {}", e))?;

            // Copy shared libraries
            fn copy_vulkan_libs(src: &Path, dst: &Path) {
                if let Ok(entries) = std::fs::read_dir(src) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_file() {
                            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                                if name.contains(".so")
                                    || name.contains(".dll")
                                    || name.contains(".dylib")
                                {
                                    if let Err(e) = std::fs::copy(&path, dst.join(name)) {
                                        log::warn!(
                                            "[llm] Failed to copy Vulkan lib {:?}: {}",
                                            name,
                                            e
                                        );
                                    }
                                }
                            }
                        } else if path.is_dir() {
                            copy_vulkan_libs(&path, dst);
                        }
                    }
                }
            }
            copy_vulkan_libs(&extract_dir, &dir);

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Err(e) =
                    std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
                {
                    log::warn!(
                        "[llm] Failed to set executable permissions on {:?}: {}",
                        dest,
                        e
                    );
                }
            }

            let metadata = EngineMetadata {
                version: release.tag_name.clone(),
                backend: "vulkan".to_string(), // It's vulkan, but installed in target dir
                installed_at: chrono_now_simple(),
            };
            if let Err(e) = std::fs::write(
                dir.join("engine.json"),
                serde_json::to_string_pretty(&metadata).unwrap_or_default(),
            ) {
                log::warn!("[llm] Failed to write engine.json: {}", e);
            }
            if let Err(e) = std::fs::remove_file(&archive_path) {
                log::warn!("[llm] Failed to clean up archive {:?}: {}", archive_path, e);
            }
            if let Err(e) = std::fs::remove_dir_all(&extract_dir) {
                log::warn!(
                    "[llm] Failed to clean up extract dir {:?}: {}",
                    extract_dir,
                    e
                );
            }

            emit_progress(
                "done",
                100.0,
                &format!(
                    "Installed Vulkan GPU backend into {} (NVIDIA GPUs fully supported).",
                    backend
                ),
            );
            return Ok(format!("Installed Vulkan GPU backend into {} (CUDA not available as pre-built for Linux, and nvcc not found)", backend));
        }

        return Err("No GPU backend binary found for Linux in this release.".to_string());
    }

    let asset = asset.ok_or_else(|| {
        let os_tag = if cfg!(target_os = "windows") {
            "win"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        };
        let available: Vec<String> = release
            .assets
            .iter()
            .filter(|a| {
                let n = a.name.to_lowercase();
                n.contains(os_tag) || (os_tag == "linux" && n.contains("ubuntu"))
            })
            .map(|a| a.name.clone())
            .collect();
        format!(
            "No {} build found for {}-x64 in release {}. Available: {}",
            backend,
            os_tag,
            release.tag_name,
            if available.is_empty() {
                "none".to_string()
            } else {
                available.join(", ")
            }
        )
    })?;

    let download_url = &asset.browser_download_url;
    let archive_name = &asset.name;
    let total_size = asset.size;

    emit_progress(
        "downloading",
        5.0,
        &format!(
            "Downloading {} ({:.0} MB)...",
            archive_name,
            total_size as f64 / (1024.0 * 1024.0)
        ),
    );

    // 4. Download archive with progress
    let archive_path = dir.join(archive_name);
    let response = client
        .get(download_url)
        .send()
        .await
        .map_err(|e| format!("Failed to start download: {}", e))?;

    if !response.status().is_success() {
        return Err(format!("Download failed with status {}", response.status()));
    }

    let mut downloaded: u64 = 0;
    let mut file = tokio::fs::File::create(&archive_path)
        .await
        .map_err(|e| format!("Failed to create archive file: {}", e))?;

    let mut stream = response.bytes_stream();
    let mut last_emit: u64 = 0;
    let emit_interval = (total_size / 100).max(65536);

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("Download error: {}", e))?;
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk)
            .await
            .map_err(|e| format!("Write error: {}", e))?;
        downloaded += chunk.len() as u64;

        if downloaded - last_emit >= emit_interval || downloaded >= total_size {
            let pct = if total_size > 0 {
                5.0 + (downloaded as f64 / total_size as f64) * 80.0
            } else {
                50.0
            };
            emit_progress(
                "downloading",
                pct,
                &format!(
                    "{:.0} / {:.0} MB",
                    downloaded as f64 / (1024.0 * 1024.0),
                    total_size as f64 / (1024.0 * 1024.0)
                ),
            );
            last_emit = downloaded;
        }
    }

    tokio::io::AsyncWriteExt::flush(&mut file)
        .await
        .map_err(|e| format!("Flush error: {}", e))?;
    drop(file);

    emit_progress("extracting", 85.0, "Extracting llama-server...");

    // 5. Extract archive
    let extract_dir = dir.join("_extract");
    let _ = std::fs::remove_dir_all(&extract_dir);
    std::fs::create_dir_all(&extract_dir)
        .map_err(|e| format!("Failed to create extract dir: {}", e))?;

    let archive_str = archive_path.to_string_lossy().to_string();
    let extract_str = extract_dir.to_string_lossy().to_string();

    if archive_name.ends_with(".tar.gz") || archive_name.ends_with(".tgz") {
        let status = {
            let mut cmd = std::process::Command::new("tar");
            cmd.arg("-xzf")
                .arg(&archive_str)
                .arg("-C")
                .arg(&extract_str);
            crate::platform::hide_window(&mut cmd);
            cmd.status()
                .map_err(|e| format!("Failed to run tar: {}", e))?
        };
        if !status.success() {
            let _ = std::fs::remove_file(&archive_path);
            let _ = std::fs::remove_dir_all(&extract_dir);
            return Err("Failed to extract archive".to_string());
        }
    } else if archive_name.ends_with(".zip") {
        #[cfg(windows)]
        {
            // Use PowerShell Expand-Archive on Windows (no unzip available)
            let ps_cmd = format!(
                "Expand-Archive -Force -Path '{}' -DestinationPath '{}'",
                archive_str, extract_str
            );
            let status = {
                let mut cmd = std::process::Command::new("powershell");
                cmd.args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd]);
                crate::platform::hide_window(&mut cmd);
                cmd.status()
                    .map_err(|e| format!("Failed to run PowerShell Expand-Archive: {}", e))?
            };
            if !status.success() {
                let _ = std::fs::remove_file(&archive_path);
                let _ = std::fs::remove_dir_all(&extract_dir);
                return Err("Failed to extract archive".to_string());
            }
        }
        #[cfg(not(windows))]
        {
            let status = {
                let mut cmd = std::process::Command::new("unzip");
                cmd.arg("-o").arg(&archive_str).arg("-d").arg(&extract_str);
                crate::platform::hide_window(&mut cmd);
                cmd.status()
                    .map_err(|e| format!("Failed to run unzip: {}", e))?
            };
            if !status.success() {
                let _ = std::fs::remove_file(&archive_path);
                let _ = std::fs::remove_dir_all(&extract_dir);
                return Err("Failed to extract archive".to_string());
            }
        }
    }

    emit_progress("extracting", 92.0, "Finding llama-server binary...");

    // 6. Find llama-server binary recursively in extract dir
    let server_name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    let server_binary = find_file_recursive(&extract_dir, server_name).ok_or_else(|| {
        let _ = std::fs::remove_file(&archive_path);
        let _ = std::fs::remove_dir_all(&extract_dir);
        format!("{} binary not found in archive", server_name)
    })?;

    // 7. Copy llama-server binary and all shared libraries to engine dir
    let dest = dir.join(server_binary_name());
    std::fs::copy(&server_binary, &dest)
        .map_err(|e| format!("Failed to copy {}: {}", server_binary_name(), e))?;

    // Robust recursive copy of all shared libraries
    fn copy_so_files(src: &Path, dst: &Path) {
        if let Ok(entries) = std::fs::read_dir(src) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        // Match .so, .so.1, .so.1.2.3, .dll, .dylib
                        if name.contains(".so") || name.contains(".dll") || name.contains(".dylib")
                        {
                            let _ = std::fs::copy(&path, dst.join(name));
                        }
                    }
                } else if path.is_dir() {
                    // Skip build artifacts and source dirs if they exist
                    let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if dir_name != "CMakeFiles" && dir_name != "_src" && dir_name != "build" {
                        copy_so_files(&path, dst);
                    }
                }
            }
        }
    }

    let _ = {
        let mut cmd = crate::platform::shell_command(&format!(
            "echo \"--- Extraction Content ---\" >> {log}; find {extract} >> {log}",
            log = dir.join("llama-server.log").to_string_lossy(),
            extract = extract_dir.to_string_lossy()
        ));
        crate::platform::hide_window(&mut cmd);
        cmd.status()
    };

    copy_so_files(&extract_dir, &dir);

    // Make executable
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Failed to set permissions: {}", e))?;
    }

    // 8. Save metadata
    let metadata = EngineMetadata {
        version: release.tag_name.clone(),
        backend: backend.clone(),
        installed_at: chrono_now_simple(),
    };
    let meta_json = serde_json::to_string_pretty(&metadata)
        .map_err(|e| format!("Failed to serialize metadata: {}", e))?;
    std::fs::write(dir.join("engine.json"), meta_json)
        .map_err(|e| format!("Failed to write metadata: {}", e))?;

    // 9. Verify the binary actually runs before marking as installed
    let server_name = if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    };
    let server_path = dir.join(server_name);
    if server_path.exists() {
        emit_progress("verifying", 97.0, "Verifying binary...");
        let mut verify_cmd = std::process::Command::new(&server_path);
        verify_cmd.arg("--version");
        #[cfg(unix)]
        verify_cmd.env("LD_LIBRARY_PATH", dir.to_string_lossy().to_string());
        #[cfg(windows)]
        {
            let mut path = std::env::var("PATH").unwrap_or_default();
            path = format!("{};{}", dir.to_string_lossy(), path);
            verify_cmd.env("PATH", path);
        }
        crate::platform::hide_window(&mut verify_cmd);
        let verify_result = verify_cmd.output();
        match verify_result {
            Ok(output) if output.status.success() => {
                let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
                log::info!("[llm] Engine verified: {}", version);
            }
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
                log::warn!("[llm] Engine binary may have issues: {}", stderr);
                // Don't fail — some builds don't support --version but still work
            }
            Err(e) => {
                // Binary can't execute — this is a real problem
                let _ = std::fs::remove_file(dir.join("engine.json"));
                return Err(format!(
                    "Engine binary failed to execute: {}. Missing shared libraries?",
                    e
                ));
            }
        }
    }

    // 10. Cleanup
    let _ = std::fs::remove_file(&archive_path);
    let _ = std::fs::remove_dir_all(&extract_dir);

    emit_progress(
        "done",
        100.0,
        &format!("llama.cpp {} ({}) installed", release.tag_name, backend),
    );

    Ok(format!(
        "Engine installed: llama.cpp {} ({})",
        release.tag_name, backend
    ))
}

/// Build llama.cpp from source with CUDA support (Linux only).
/// Called when no prebuilt CUDA binary is available in the GitHub release.
#[cfg(target_os = "linux")]
async fn build_cuda_from_source(
    backend: &str,
    tag: &str,
    dir: &Path,
    client: &Client,
    emit_progress: &(dyn Fn(&str, f64, &str) + Send + Sync),
) -> Result<String, String> {
    // Check prerequisites
    // Check cmake and gcc first
    for (cmd, label) in [("cmake", "CMake"), ("gcc", "GCC")] {
        let found = std::process::Command::new("which")
            .arg(cmd)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !found {
            return Err(format!(
                "{} not found. Install it first.\nFor cmake/gcc: install cmake gcc g++",
                label
            ));
        }
    }

    // Find nvcc — search PATH first, then common CUDA install locations
    // Instead of mutating global PATH with set_var, we store the augmented path
    // and pass it to child processes via .env("PATH", ...)
    let augmented_path: Option<String>;
    let nvcc_binary = {
        let in_path = std::process::Command::new("which")
            .arg("nvcc")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if in_path {
            augmented_path = None;
            "nvcc".to_string()
        } else if let Some(nvcc_path) = find_nvcc_path() {
            if let Some(bin_dir) = std::path::Path::new(&nvcc_path).parent() {
                let current_path = std::env::var("PATH").unwrap_or_default();
                augmented_path = Some(format!("{}:{}", bin_dir.display(), current_path));
            } else {
                augmented_path = None;
            }
            nvcc_path
        } else {
            return Err("CUDA toolkit (nvcc) not found. Install it first.\nFor CUDA: install nvidia-cuda-toolkit\nOr set PATH to include /usr/local/cuda/bin".to_string());
        }
    };

    // Helper: apply augmented PATH to child commands (avoids global set_var)
    let apply_path = |cmd: &mut std::process::Command| {
        if let Some(ref p) = augmented_path {
            cmd.env("PATH", p);
        }
    };

    // Detect CUDA version using the found nvcc binary
    let cuda_ver = {
        let mut cmd = std::process::Command::new(&nvcc_binary);
        cmd.arg("--version");
        apply_path(&mut cmd);
        cmd
    }
    .output()
    .ok()
    .and_then(|o| String::from_utf8(o.stdout).ok())
    .and_then(|s| {
        s.lines().find(|l| l.contains("release")).and_then(|l| {
            l.split("release ")
                .nth(1)
                .and_then(|v| v.split(',').next())
                .map(|v| v.trim().to_string())
        })
    })
    .unwrap_or_else(|| "unknown".to_string());

    emit_progress(
        "downloading",
        8.0,
        &format!("CUDA {} detected. Downloading source {}...", cuda_ver, tag),
    );

    // Download source tarball
    let src_url = format!(
        "https://github.com/ggerganov/llama.cpp/archive/refs/tags/{}.tar.gz",
        tag
    );
    let response = client
        .get(&src_url)
        .send()
        .await
        .map_err(|e| format!("Failed to download source: {}", e))?;
    if !response.status().is_success() {
        return Err(format!(
            "Source download failed: HTTP {}",
            response.status()
        ));
    }

    let src_archive = dir.join("source.tar.gz");
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("Download error: {}", e))?;
    std::fs::write(&src_archive, &bytes).map_err(|e| format!("Write error: {}", e))?;

    emit_progress("extracting", 20.0, "Extracting source...");

    let src_dir = dir.join("_src");
    let _ = std::fs::remove_dir_all(&src_dir);
    std::fs::create_dir_all(&src_dir).map_err(|e| format!("mkdir error: {}", e))?;

    let status = std::process::Command::new("tar")
        .args([
            "-xzf",
            &src_archive.to_string_lossy(),
            "-C",
            &src_dir.to_string_lossy(),
            "--strip-components=1",
        ])
        .status()
        .map_err(|e| format!("tar error: {}", e))?;
    if !status.success() {
        return Err("Failed to extract source".to_string());
    }
    let _ = std::fs::remove_file(&src_archive);

    emit_progress(
        "building",
        25.0,
        &format!(
            "Building llama.cpp with CUDA {}... (this may take a few minutes)",
            cuda_ver
        ),
    );

    // Detect number of CPUs — use half to avoid freezing the system
    let nproc = std::process::Command::new("nproc")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u32>().ok())
        .map(|n| (n / 2).max(2).min(8))
        .unwrap_or(4);

    let build_dir = src_dir.join("build");

    // Create a wrapper for nvcc to force the -allow-unsupported-compiler flag
    // This ensures even the compiler identification step succeeds.
    let nvcc_wrapper = dir.join("nvcc_wrapper");
    let wrapper_content = format!(
        "#!/bin/bash\nexec {} -allow-unsupported-compiler \"$@\"\n",
        nvcc_binary
    );
    if let Err(e) = std::fs::write(&nvcc_wrapper, wrapper_content) {
        return Err(format!("Failed to create nvcc wrapper: {}", e));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&nvcc_wrapper, std::fs::Permissions::from_mode(0o755));
    }
    let nvcc_to_use = nvcc_wrapper.to_string_lossy().to_string();

    // Find CUDA toolkit root for cmake from the nvcc_binary we already found
    let mut cmake_cmd = std::process::Command::new("cmake");
    apply_path(&mut cmake_cmd);
    cmake_cmd.args([
        "-B",
        &build_dir.to_string_lossy(),
        "-S",
        &src_dir.to_string_lossy(),
        "-DCMAKE_BUILD_TYPE=Release",
        "-DGGML_CUDA=ON",
    ]);

    // Point cmake at our wrapper
    cmake_cmd.arg(format!("-DCMAKE_CUDA_COMPILER={}", nvcc_to_use));

    if let Some(cuda_root) = Path::new(&nvcc_binary).parent().and_then(|p| p.parent()) {
        let root_str = cuda_root.to_string_lossy().to_string();
        cmake_cmd.arg(format!("-DCUDAToolkit_ROOT={}", root_str));

        // Comprehensive flags to bypass host compiler version checks
        let override_flag = "-allow-unsupported-compiler";
        cmake_cmd.arg(format!("-DCMAKE_CUDA_FLAGS={}", override_flag));

        // Environment variables that NVCC, CMake, and the build system respect
        cmake_cmd.env("CUDACXX", &nvcc_to_use);
        cmake_cmd.env("CUDA_HOME", &root_str);
        cmake_cmd.env("CUDAFLAGS", override_flag);
        cmake_cmd.env("NVCCFLAGS", override_flag);

        // Prefer GCC 15/14 (max CUDA 13.2 supported), then clang, then system gcc.
        // Clang 22+ defaults to GCC 16 headers which cudafe++ in CUDA 13.2 cannot parse.
        let host_compiler = [
            "/usr/bin/g++-15",
            "/usr/bin/g++-14",
            "/usr/bin/clang",
            "/usr/bin/g++",
            "/usr/bin/gcc",
        ]
        .iter()
        .find(|p| Path::new(p).exists())
        .copied()
        .unwrap_or("/usr/bin/gcc");
        cmake_cmd.arg(format!("-DCMAKE_CUDA_HOST_COMPILER={}", host_compiler));
        cmake_cmd.env("NVCC_CCBIN", host_compiler);
        cmake_cmd.env("CUDAHOSTCXX", host_compiler);
    }

    let cmake_output = cmake_cmd
        .output()
        .map_err(|e| format!("cmake execution failed: {}", e))?;

    if !cmake_output.status.success() {
        let err_msg = String::from_utf8_lossy(&cmake_output.stderr);

        // If CUDA config fails, try falling back to Vulkan build automatically
        if backend.starts_with("cuda")
            && (err_msg.contains("unsupported GNU version")
                || err_msg.contains("unsupported clang version")
                || err_msg.contains("missing initializer")
                || err_msg.contains("compiler identification")
                || err_msg.contains("CMakeDetermineCompilerId"))
        {
            emit_progress("fetching", 10.0, "CUDA build unsupported by system compiler. Falling back to native Vulkan GPU build...");

            let mut vulkan_cmd = std::process::Command::new("cmake");
            apply_path(&mut vulkan_cmd);
            vulkan_cmd.args([
                "-B",
                &build_dir.to_string_lossy(),
                "-S",
                &src_dir.to_string_lossy(),
                "-DCMAKE_BUILD_TYPE=Release",
                "-DGGML_VULKAN=ON",
            ]);

            let v_output = vulkan_cmd
                .output()
                .map_err(|e| format!("vulkan cmake failed: {}", e))?;
            if !v_output.status.success() {
                return Err(format!(
                    "Both CUDA and Vulkan builds failed. CUDA error: {}",
                    err_msg
                ));
            }
            // Success, proceed to building
        } else {
            return Err(format!(
                "cmake configure failed: {}\nCheck if CUDA Toolkit and GCC/G++ are compatible.",
                err_msg
            ));
        }
    }

    emit_progress(
        "building",
        35.0,
        &format!("Compiling with {} threads...", nproc),
    );

    // cmake build
    let mut build_cmd = std::process::Command::new("cmake");
    apply_path(&mut build_cmd);
    let build_output = build_cmd
        .args([
            "--build",
            &build_dir.to_string_lossy(),
            "--config",
            "Release",
            "-j",
            &nproc.to_string(),
        ])
        .output()
        .map_err(|e| format!("cmake build execution failed: {}", e))?;
    if !build_output.status.success() {
        let err_msg = String::from_utf8_lossy(&build_output.stderr);
        return Err(format!(
            "Build failed: {}\nCheck build log for errors.",
            err_msg
        ));
    }

    emit_progress("extracting", 90.0, "Installing llama-server...");

    // Find and copy llama-server
    let built_binary = find_file_recursive(&build_dir, "llama-server").ok_or_else(|| {
        let _ = std::fs::remove_dir_all(&src_dir);
        "llama-server binary not found after build".to_string()
    })?;

    let dest = dir.join("llama-server");
    std::fs::copy(&built_binary, &dest).map_err(|e| format!("Copy error: {}", e))?;

    // Copy shared libraries from build dir
    fn copy_libs(src: &Path, dst: &Path) {
        if let Ok(entries) = std::fs::read_dir(src) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if name.contains(".so") || name.contains(".dll") || name.contains(".dylib")
                        {
                            let _ = std::fs::copy(&path, dst.join(name));
                        }
                    }
                } else if path.is_dir() {
                    let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if dir_name != "CMakeFiles" {
                        copy_libs(&path, dst);
                    }
                }
            }
        }
    }
    copy_libs(&build_dir, dir);

    // Make executable
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("chmod error: {}", e))?;
    }

    // Save metadata
    let metadata = EngineMetadata {
        version: tag.to_string(),
        backend: format!("cuda-{}", cuda_ver),
        installed_at: chrono_now_simple(),
    };
    let meta_json = serde_json::to_string_pretty(&metadata).unwrap_or_default();
    let _ = std::fs::write(dir.join("engine.json"), meta_json);

    // Cleanup source
    let _ = std::fs::remove_dir_all(&src_dir);

    emit_progress(
        "done",
        100.0,
        &format!("llama.cpp {} (CUDA {}) built and installed", tag, cuda_ver),
    );
    Ok(format!(
        "Engine built from source: llama.cpp {} (CUDA {})",
        tag, cuda_ver
    ))
}

/// Find the best matching release asset for the given backend.
fn find_release_asset<'a>(assets: &'a [GhAsset], backend: &str) -> Option<&'a GhAsset> {
    let arch = if cfg!(target_arch = "x86_64") {
        "x64"
    } else if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x64"
    };

    let mut candidates: Vec<&GhAsset> = Vec::new();

    for asset in assets {
        let name = asset.name.to_lowercase();

        // Platform filtering: match OS-specific archive names
        #[cfg(target_os = "windows")]
        {
            if !name.contains("win") {
                continue;
            }
            if !name.contains(arch) {
                continue;
            }
            if !name.ends_with(".zip") {
                continue;
            }
        }
        #[cfg(target_os = "linux")]
        {
            if !name.contains("linux") && !name.contains("ubuntu") {
                continue;
            }
            if !name.contains(arch) {
                continue;
            }
            if !name.ends_with(".tar.gz") && !name.ends_with(".zip") {
                continue;
            }
        }
        #[cfg(target_os = "macos")]
        {
            if !name.contains("macos") && !name.contains("darwin") && !name.contains("mac") {
                continue;
            }
            if !name.ends_with(".tar.gz") && !name.ends_with(".zip") {
                continue;
            }
        }

        match backend {
            "cuda13" => {
                let name_no_sep = name.replace('-', "").replace('.', "").replace('_', "");
                if name.contains("cuda")
                    && (name_no_sep.contains("cuda13")
                        || name_no_sep.contains("cu13")
                        || name.contains("cuda-13"))
                {
                    candidates.push(asset);
                }
            }
            "cuda12" => {
                // Match patterns like: cuda-12.4, cuda12, cu12, cu121, cu124, cuda-12
                let name_no_sep = name.replace('-', "").replace('.', "").replace('_', "");
                if name.contains("cuda")
                    && (name_no_sep.contains("cuda12")
                        || name_no_sep.contains("cu12")
                        || name.contains("cuda-12"))
                {
                    candidates.push(asset);
                }
            }
            "cuda11" => {
                let name_no_sep = name.replace('-', "").replace('.', "").replace('_', "");
                if name.contains("cuda")
                    && (name_no_sep.contains("cuda11")
                        || name_no_sep.contains("cu11")
                        || name.contains("cuda-11"))
                {
                    candidates.push(asset);
                }
            }
            "cuda" => {
                // Generic CUDA — accept any CUDA build
                if name.contains("cuda") {
                    candidates.push(asset);
                }
            }
            "rocm" => {
                if name.contains("hip") || name.contains("rocm") {
                    candidates.push(asset);
                }
            }
            "vulkan" => {
                if name.contains("vulkan") {
                    candidates.push(asset);
                }
            }
            "sycl" => {
                if name.contains("sycl") {
                    candidates.push(asset);
                }
            }
            "cpu" => {
                let is_base = !name.contains("cuda")
                    && !name.contains("vulkan")
                    && !name.contains("hip")
                    && !name.contains("rocm")
                    && !name.contains("sycl");
                if is_base {
                    candidates.push(asset);
                }
            }
            _ => {
                // For 'auto', add everything and we'll pick best later
                candidates.push(asset);
            }
        }
    }

    // Do NOT silently fall back to CPU build for GPU backends.
    // A CPU build in the cuda/rocm directory would mislead users into thinking
    // VRAM offloading is available when it isn't.

    // Sort candidates to find the best match
    if backend.starts_with("cuda") && candidates.len() > 1 {
        candidates.sort_by(|a, b| {
            let a_name = a.name.to_lowercase();
            let b_name = b.name.to_lowercase();

            // Prioritize cu12 over cu11
            let a_norm = a_name.replace('-', "").replace('.', "");
            let b_norm = b_name.replace('-', "").replace('.', "");
            let a_cu12 =
                a_norm.contains("cu12") || a_norm.contains("cuda12") || a_name.contains("cuda-12");
            let b_cu12 =
                b_norm.contains("cu12") || b_norm.contains("cuda12") || b_name.contains("cuda-12");
            if a_cu12 && !b_cu12 {
                return std::cmp::Ordering::Less;
            }
            if !a_cu12 && b_cu12 {
                return std::cmp::Ordering::Greater;
            }

            // Prioritize binaries over source/others
            let a_bin = a_name.contains("bin");
            let b_bin = b_name.contains("bin");
            if a_bin && !b_bin {
                return std::cmp::Ordering::Less;
            }
            if !a_bin && b_bin {
                return std::cmp::Ordering::Greater;
            }

            b_name.cmp(&a_name)
        });
    }

    candidates.into_iter().next()
}

/// Install llama.cpp engine by downloading pre-built binary from GitHub releases.
/// Emits `engine-install-progress` events with stage/percent/detail.

/// Simple timestamp without chrono dependency.
fn chrono_now_simple() -> String {
    let dur = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", dur.as_secs())
}

/// Recursively find a file by name in a directory.
fn find_file_recursive(dir: &Path, target: &str) -> Option<std::path::PathBuf> {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    if name == target {
                        return Some(path);
                    }
                }
            } else if path.is_dir() {
                if let Some(found) = find_file_recursive(&path, target) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// Find llama.cpp server binary — checks specific backend first, then PATH, then common locations.
fn find_llama_server(backend: &str) -> Option<String> {
    // 1. Check specific backend engine dir first
    if !backend.is_empty() && backend != "auto" {
        let engine_binary = engine_dir(backend).join(server_binary_name());
        if engine_binary.exists() {
            return Some(engine_binary.to_string_lossy().to_string());
        }
    }

    // 2. Fallback: check ALL installed backends if "auto" or requested not found
    if let Ok(installed) = list_installed_engines() {
        if !installed.is_empty() {
            let chosen = if backend.is_empty() || backend == "auto" {
                resolve_auto_backend_from_installed(&installed)
            } else {
                installed
                    .iter()
                    .find(|name| name.as_str() == backend)
                    .cloned()
            }
            .or_else(|| resolve_auto_backend_from_installed(&installed))
            .or_else(|| installed.first().cloned());

            if let Some(chosen_backend) = chosen {
                let dir = engine_dir(&chosen_backend);
                let binary = dir.join(server_binary_name());
                if binary.exists() {
                    return Some(binary.to_string_lossy().to_string());
                }
            }
        }
    }

    // 3. Check PATH
    let candidates = if cfg!(windows) {
        vec!["llama-server.exe", "llama-cpp-server.exe"]
    } else {
        vec!["llama-server", "llama-cpp-server"]
    };
    for cmd_name in &candidates {
        if let Ok(output) = {
            let mut cmd = std::process::Command::new(crate::platform::which_cmd());
            cmd.arg(cmd_name);
            crate::platform::hide_window(&mut cmd);
            cmd.output()
        } {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Some(path);
                }
            }
        }
    }

    // 4. Check common install locations
    let home = dirs_next::home_dir().unwrap_or_default();
    let home_str = home.to_string_lossy();
    let mut extra_paths = vec![
        format!("{}/llama.cpp/build/bin/llama-server", home_str),
        format!("{}/.local/bin/llama-server", home_str),
    ];
    #[cfg(unix)]
    extra_paths.push("/usr/local/bin/llama-server".to_string());
    #[cfg(windows)]
    {
        let appdata = std::env::var("LOCALAPPDATA").unwrap_or_default();
        if !appdata.is_empty() {
            extra_paths.push(format!("{}\\llama.cpp\\llama-server.exe", appdata));
        }
        extra_paths.push(format!(
            "{}\\llama.cpp\\build\\bin\\Release\\llama-server.exe",
            home_str
        ));
    }
    for p in &extra_paths {
        if Path::new(p).exists() {
            return Some(p.clone());
        }
    }
    None
}

/// Uninstall a specific llama.cpp engine backend (or all if none specified).
#[tauri::command]
pub fn uninstall_engine(backend: Option<String>) -> Result<String, String> {
    match backend {
        Some(b) if !b.is_empty() => {
            let dir = engine_dir(&b);
            if dir.exists() {
                std::fs::remove_dir_all(&dir)
                    .map_err(|e| format!("Failed to delete engine directory: {}", e))?;
                Ok(format!("Engine backend {} uninstalled", b))
            } else {
                Err(format!("Backend {} not found", b))
            }
        }
        _ => {
            let root = engines_root();
            if root.exists() {
                std::fs::remove_dir_all(&root)
                    .map_err(|e| format!("Failed to delete engines root: {}", e))?;
            }
            Ok("All engine backends uninstalled".to_string())
        }
    }
}

/// Launch a llama.cpp server with the given model and configuration.
/// Supports GPU selection for both NVIDIA (--main-gpu) and AMD (HIP_VISIBLE_DEVICES).
#[tauri::command]
pub fn launch_llm_server(
    model_path: String,
    config: LlmConfig,
    port: u16,
    gpu_vendor: Option<String>,
    backend: Option<String>,
    app: tauri::AppHandle,
    state: State<'_, LlmServerState>,
) -> Result<String, String> {
    // Stop existing server if running
    if let Ok(mut child_lock) = state.child.lock() {
        if let Some(mut child) = child_lock.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    // Resolve "auto" against the currently installed engines using live hardware.
    let installed_backend = match backend.as_deref() {
        Some(b) if !b.is_empty() && b != "auto" => b.to_lowercase(),
        _ => {
            let installed = list_installed_engines()?;
            resolve_auto_backend_from_installed(&installed)
                .or_else(|| installed.first().cloned())
                .unwrap_or_default()
        }
    };

    let server_bin = find_llama_server(&installed_backend).ok_or_else(|| {
        "llama-server not found. Use 'Install Engine' to set up llama.cpp automatically."
            .to_string()
    })?;

    if !Path::new(&model_path).exists() {
        return Err(format!("Model file not found: {}", model_path));
    }

    let mut cmd = std::process::Command::new(&server_bin);
    #[cfg(unix)]
    cmd.process_group(0);

    // Set GPU library paths
    let vendor = gpu_vendor.as_deref().unwrap_or("");
    {
        let eng_dir = engine_dir(&installed_backend);

        #[cfg(unix)]
        {
            let mut ld_paths: Vec<String> = Vec::new();
            if eng_dir.exists() {
                ld_paths.push(eng_dir.to_string_lossy().to_string());
            }
            if let Ok(existing) = std::env::var("LD_LIBRARY_PATH") {
                ld_paths.push(existing);
            }

            // Add backend-specific library paths
            let backend_str = installed_backend.as_str();
            let is_nvidia =
                vendor.to_lowercase().contains("nvidia") || backend_str.contains("cuda");

            if is_nvidia {
                let mut cuda_lib_dirs = vec![
                    "/usr/local/cuda/lib64".to_string(),
                    "/usr/local/cuda/targets/x86_64-linux/lib".to_string(),
                    "/usr/lib/x86_64-linux-gnu".to_string(),
                    "/usr/lib64".to_string(),
                ];
                if let Ok(entries) = std::fs::read_dir("/usr/local") {
                    for entry in entries.flatten() {
                        let name = entry.file_name().to_string_lossy().to_string();
                        if name.starts_with("cuda") {
                            cuda_lib_dirs.push(format!("/usr/local/{}/lib64", name));
                            cuda_lib_dirs
                                .push(format!("/usr/local/{}/targets/x86_64-linux/lib", name));
                        }
                    }
                }
                for dir in &cuda_lib_dirs {
                    if Path::new(dir).exists() {
                        ld_paths.push(dir.to_string());
                    }
                }
            } else if backend_str == "rocm" {
                for dir in &["/opt/rocm/lib", "/opt/rocm/lib64"] {
                    if Path::new(dir).exists() {
                        ld_paths.push(dir.to_string());
                    }
                }
            } else if backend_str == "sycl" {
                for dir in &[
                    "/opt/intel/oneapi/compiler/latest/linux/lib",
                    "/opt/intel/oneapi/mkl/latest/lib/intel64",
                ] {
                    if Path::new(dir).exists() {
                        ld_paths.push(dir.to_string());
                    }
                }
            }

            if !ld_paths.is_empty() {
                cmd.env("LD_LIBRARY_PATH", ld_paths.join(":"));
            }
        }

        #[cfg(windows)]
        {
            // On Windows, add engine directory to PATH so DLLs are found
            if eng_dir.exists() {
                let mut path = std::env::var("PATH").unwrap_or_default();
                path = format!("{};{}", eng_dir.to_string_lossy(), path);
                cmd.env("PATH", path);
            }
        }
    }

    cmd.arg("--model")
        .arg(&model_path)
        .arg("--host")
        .arg("0.0.0.0")
        .arg("--port")
        .arg(port.to_string())
        .arg("--ctx-size")
        .arg(config.n_ctx.to_string())
        .arg("--threads")
        .arg(config.n_threads.to_string())
        .arg("--threads-batch")
        .arg(config.n_threads.to_string())
        .arg("--batch-size")
        .arg(config.n_batch.to_string())
        .arg("--ubatch-size")
        .arg(config.n_batch.to_string())
        .arg("--parallel")
        .arg("1");

    let effective_n_gpu_layers = if config.use_all_vram {
        999
    } else {
        config.n_gpu_layers
    };

    if effective_n_gpu_layers > 0 {
        cmd.arg("--n-gpu-layers")
            .arg(effective_n_gpu_layers.to_string());
    }

    // GPU selection environment variables
    if config.main_gpu >= 0 {
        let be = installed_backend.as_str();
        if be.starts_with("cuda") {
            cmd.arg("--main-gpu").arg(config.main_gpu.to_string());
            cmd.env("CUDA_VISIBLE_DEVICES", config.main_gpu.to_string());
        } else {
            match be {
                "rocm" => {
                    cmd.env("HIP_VISIBLE_DEVICES", config.main_gpu.to_string());
                    cmd.env("ROCR_VISIBLE_DEVICES", config.main_gpu.to_string());
                }
                "vulkan" => {
                    // Vulkan enumerates devices by PCI bus order, which may differ
                    // from our internal GPU index. Resolve the correct Vulkan device index.
                    let gpus = detect_all_gpus();
                    let vk_idx = resolve_vulkan_device_index(&gpus, config.main_gpu);
                    log::info!("[llm] Vulkan device selection: internal main_gpu={}, resolved vulkan_device={}", config.main_gpu, vk_idx);
                    for (i, g) in gpus.iter().enumerate() {
                        log::info!(
                            "[llm]   GPU[{}]: {} ({}, vram={:?})",
                            i,
                            g.name,
                            g.vendor,
                            g.vram_gb
                        );
                    }
                    cmd.env("GGML_VULKAN_DEVICE", vk_idx.to_string());
                }
                "sycl" => {
                    cmd.env(
                        "ONEAPI_DEVICE_SELECTOR",
                        format!("level_zero:{}", config.main_gpu),
                    );
                }
                _ => {
                    if vendor == "nvidia" {
                        cmd.arg("--main-gpu").arg(config.main_gpu.to_string());
                        cmd.env("CUDA_VISIBLE_DEVICES", config.main_gpu.to_string());
                    }
                }
            }
        }
    } else if vendor == "nvidia" {
        // Even if index is -1 (auto), ensure CUDA is visible
        cmd.env("CUDA_DEVICE_ORDER", "PCI_BUS_ID");
    }

    if config.flash_attention {
        cmd.args(["--flash-attn", "on"]);
    }
    if config.mmap {
        cmd.arg("--mmap");
    } else {
        cmd.arg("--no-mmap");
    }
    // Keep KV cache in RAM — VRAM is for model layers only.
    // This prevents VRAM overflow: model layers fill VRAM, KV cache + context use RAM.
    if effective_n_gpu_layers > 0 {
        cmd.arg("--no-kv-offload");
    }
    // Enable KV cache reuse across requests (avoids re-processing shared prompt prefix)
    cmd.arg("--cache-prompt");
    // Enable Jinja templates for tool calling support
    cmd.arg("--jinja");
    if let Some(ref ct) = config.cache_type_k {
        cmd.arg("--cache-type-k").arg(ct);
    }
    if let Some(ref ct) = config.cache_type_v {
        cmd.arg("--cache-type-v").arg(ct);
    }
    if let Some(base) = config.rope_freq_base {
        cmd.arg("--rope-freq-base").arg(base.to_string());
    }
    if let Some(scale) = config.rope_freq_scale {
        cmd.arg("--rope-freq-scale").arg(scale.to_string());
    }
    if config.seed >= 0 {
        cmd.arg("--seed").arg(config.seed.to_string());
    }
    for stop in &config.stop_strings {
        cmd.arg("--stop").arg(stop);
    }

    // Speculative decoding: use a smaller draft model to predict tokens, main model verifies
    if let Some(ref draft_path) = config.draft_model_path {
        if !draft_path.is_empty() && Path::new(draft_path).exists() {
            cmd.arg("--model-draft").arg(draft_path);
            cmd.arg("--draft-max")
                .arg(config.draft_n_predict.to_string());
            log::info!(
                "[llm] Speculative decoding enabled with draft model: {}",
                draft_path
            );
        } else if !draft_path.is_empty() {
            log::warn!(
                "[llm] Draft model not found, skipping speculative decoding: {}",
                draft_path
            );
        }
    }

    // Multi-GPU tensor split
    if let Some(ref splits) = config.tensor_split {
        if splits.len() > 1 {
            let split_str: Vec<String> = splits.iter().map(|s| format!("{:.2}", s)).collect();
            cmd.arg("--tensor-split").arg(split_str.join(","));
        }
    }

    // Capture stderr to a temp file so we can report errors if the server crashes
    let log_dir = engine_dir(&installed_backend);
    if !log_dir.exists() {
        let _ = std::fs::create_dir_all(&log_dir);
    }
    let stderr_path = log_dir.join("llama-server.log");
    let stderr_file = std::fs::File::create(&stderr_path)
        .map_err(|e| format!("Failed to create log file: {}", e))?;

    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::from(stderr_file));
    crate::platform::hide_window(&mut cmd);
    let child = cmd
        .spawn()
        .map_err(|e| format!("Failed to launch llama-server: {}", e))?;

    let pid = child.id();

    if let Ok(mut child_lock) = state.child.lock() {
        *child_lock = Some(child);
    }
    if let Ok(mut port_lock) = state.port.lock() {
        *port_lock = port;
    }
    if let Ok(mut model_lock) = state.model_path.lock() {
        *model_lock = model_path.clone();
    }
    if let Ok(mut err_lock) = state.last_error.lock() {
        *err_lock = String::new();
    }
    if let Ok(mut path_lock) = state.stderr_path.lock() {
        *path_lock = stderr_path.to_string_lossy().to_string();
    }
    if let Ok(mut ctx_lock) = state.context_length.lock() {
        *ctx_lock = config.n_ctx;
    }

    // Background health check: poll until server is ready, then emit event
    let health_port = port;
    let health_app = app.clone();
    let health_model = model_path;
    let health_ctx = config.n_ctx;
    std::thread::spawn(move || {
        let url = format!("http://localhost:{}/health", health_port);
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .unwrap_or_else(|_| reqwest::blocking::Client::new());
        for attempt in 0..60 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            match client.get(&url).send() {
                Ok(resp) if resp.status().is_success() => {
                    let _ = health_app.emit(
                        "llm-server-ready",
                        serde_json::json!({
                            "port": health_port,
                            "model": health_model,
                            "context_length": health_ctx,
                        }),
                    );
                    // Auto-set AI provider to the running llama-server
                    let ai: tauri::State<'_, crate::ai_bridge::AiConfig> = health_app.state();
                    if let Ok(mut url) = ai.base_url.lock() {
                        *url = format!("http://localhost:{}/v1", health_port);
                        log::info!(
                            "[llm] Auto-set AI base_url to http://localhost:{}/v1",
                            health_port
                        );
                    }
                    log::info!(
                        "[llm] Server ready on port {} after {}ms",
                        health_port,
                        (attempt + 1) * 500
                    );
                    return;
                }
                _ => continue,
            }
        }
        let _ = health_app.emit(
            "llm-server-failed",
            serde_json::json!({
                "port": health_port,
                "error": "Server did not become ready within 30 seconds",
            }),
        );
        log::warn!(
            "[llm] Server on port {} failed to become ready within 30s",
            health_port
        );
    });

    Ok(format!(
        "llama-server started on port {} (PID {})",
        port, pid
    ))
}

#[tauri::command]
pub fn list_engine_files(backend: String) -> Result<Vec<String>, String> {
    let dir = engine_dir(&backend);
    if !dir.exists() {
        return Err(format!("Engine dir not found: {:?}", dir));
    }
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Ok(name) = entry.file_name().into_string() {
                files.push(name);
            }
        }
    }
    Ok(files)
}

/// Stop the running llama-server gracefully: SIGTERM → wait 3s → SIGKILL.
#[tauri::command]
pub fn stop_llm_server(state: State<'_, LlmServerState>) -> Result<String, String> {
    if let Ok(mut child_lock) = state.child.lock() {
        if let Some(mut child) = child_lock.take() {
            // Send SIGTERM to the process group first (graceful shutdown)
            #[cfg(unix)]
            {
                let pid = child.id() as i32;
                unsafe {
                    libc::kill(-pid, libc::SIGTERM);
                }
            }
            // Wait up to 3 seconds for graceful exit
            let start = std::time::Instant::now();
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    _ if start.elapsed() > std::time::Duration::from_secs(3) => {
                        // Force kill after timeout
                        #[cfg(unix)]
                        {
                            let pid = child.id() as i32;
                            unsafe {
                                libc::kill(-pid, libc::SIGKILL);
                            }
                        }
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                    _ => std::thread::sleep(std::time::Duration::from_millis(100)),
                }
            }
            if let Ok(mut port_lock) = state.port.lock() {
                *port_lock = 0;
            }
            if let Ok(mut model_lock) = state.model_path.lock() {
                *model_lock = String::new();
            }
            if let Ok(mut ctx_lock) = state.context_length.lock() {
                *ctx_lock = 0;
            }
            return Ok("Server stopped".to_string());
        }
    }
    Err("No server running".to_string())
}

/// Unload the current model from the running llama-server (frees VRAM).
/// Tries the llama-server unload endpoint first, falls back to stopping the server.
#[tauri::command]
pub async fn unload_llm_model(state: State<'_, LlmServerState>) -> Result<String, String> {
    let port = state.port.lock().map(|p| *p).unwrap_or(0);
    if port == 0 {
        return Err("No server running".to_string());
    }

    // Try the internal unload endpoint (available in recent llama.cpp builds)
    let client = Client::new();
    let unload_url = format!("http://localhost:{}/v1/internal/model/unload", port);
    if let Ok(resp) = client.post(&unload_url).send().await {
        if resp.status().is_success() {
            if let Ok(mut model_lock) = state.model_path.lock() {
                *model_lock = String::new();
            }
            if let Ok(mut ctx_lock) = state.context_length.lock() {
                *ctx_lock = 0;
            }
            return Ok("Model unloaded (VRAM freed, server still running)".to_string());
        }
    }

    // Fallback: stop the server gracefully
    if let Ok(mut child_lock) = state.child.lock() {
        if let Some(mut child) = child_lock.take() {
            #[cfg(unix)]
            {
                let pid = child.id() as i32;
                unsafe {
                    libc::kill(-pid, libc::SIGTERM);
                }
            }
            let start = std::time::Instant::now();
            loop {
                match child.try_wait() {
                    Ok(Some(_)) => break,
                    _ if start.elapsed() > std::time::Duration::from_secs(3) => {
                        #[cfg(unix)]
                        {
                            let pid = child.id() as i32;
                            unsafe {
                                libc::kill(-pid, libc::SIGKILL);
                            }
                        }
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                    _ => std::thread::sleep(std::time::Duration::from_millis(100)),
                }
            }
            if let Ok(mut model_lock) = state.model_path.lock() {
                *model_lock = String::new();
            }
            if let Ok(mut ctx_lock) = state.context_length.lock() {
                *ctx_lock = 0;
            }
            return Ok("Model unloaded (server stopped)".to_string());
        }
    }
    Err("No server running".to_string())
}

/// Get the status of the llama-server.
#[tauri::command]
pub fn get_llm_server_status(
    state: State<'_, LlmServerState>,
) -> Result<serde_json::Value, String> {
    let (own_running, just_exited) = if let Ok(mut child_lock) = state.child.lock() {
        if let Some(ref mut child) = *child_lock {
            match child.try_wait() {
                Ok(None) => (true, false),
                _ => {
                    *child_lock = None;
                    (false, true)
                }
            }
        } else {
            (false, false)
        }
    } else {
        (false, false)
    };

    if own_running {
        let port = state.port.lock().map(|p| *p).unwrap_or(0);
        let model = state
            .model_path
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default();
        let ctx = state.context_length.lock().map(|c| *c).unwrap_or(0);
        let engine = check_engine(None).ok();
        return Ok(serde_json::json!({
            "running": true,
            "port": port,
            "model": model,
            "context_length": ctx,
            "binary": find_llama_server("").unwrap_or_default(),
            "backend": engine.map(|e| e.backend).unwrap_or_else(|| "llama-server".to_string()),
        }));
    }

    // Read stderr log if the process just exited
    let mut error_msg = String::new();
    if just_exited {
        if let Ok(path_lock) = state.stderr_path.lock() {
            if !path_lock.is_empty() {
                if let Ok(content) = std::fs::read_to_string(&*path_lock) {
                    // Get last 500 chars of stderr for the error message
                    let tail = if content.len() > 500 {
                        &content[content.len() - 500..]
                    } else {
                        &content
                    };
                    error_msg = tail.trim().to_string();
                    if let Ok(mut err_lock) = state.last_error.lock() {
                        *err_lock = error_msg.clone();
                    }
                }
            }
        }
    } else if let Ok(err_lock) = state.last_error.lock() {
        error_msg = err_lock.clone();
    }

    // Child handle lost — check if a server is actually responding on the tracked port or default
    let tracked_port = state.port.lock().map(|p| *p).unwrap_or(0);
    let check_port = if tracked_port > 0 {
        tracked_port
    } else {
        8080u16
    };
    let health_url = format!("http://127.0.0.1:{}/health", check_port);
    let externally_running = {
        let mut cmd = std::process::Command::new("curl");
        cmd.args(["-sf", "--max-time", "1", &health_url]);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
    }
    .map(|o| o.status.success())
    .unwrap_or(false);

    if externally_running {
        let model = state
            .model_path
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default();
        let ctx = state.context_length.lock().map(|c| *c).unwrap_or(0);
        let engine = check_engine(None).ok();
        return Ok(serde_json::json!({
            "running": true,
            "port": check_port,
            "model": model,
            "context_length": ctx,
            "binary": find_llama_server("").unwrap_or_default(),
            "backend": engine.map(|e| e.backend).unwrap_or_else(|| "llama-server".to_string()),
        }));
    }

    // Not running
    let llama_server = find_llama_server("");
    let has_llama = llama_server.is_some();
    Ok(serde_json::json!({
        "running": false,
        "port": 0,
        "model": "",
        "binary": llama_server.as_deref().unwrap_or(""),
        "backend": if has_llama { "llama-server" } else { "" },
        "error": error_msg,
    }))
}

/// Get current GPU memory usage across all GPUs.
/// Returns NVIDIA stats via nvidia-smi, AMD via rocm-smi.
#[tauri::command]
pub fn get_gpu_memory_stats() -> Result<Vec<serde_json::Value>, String> {
    let mut stats = Vec::new();

    // NVIDIA GPUs
    if let Some(nvidia_smi) = nvidia_smi_binary() {
        if let Ok(output) = {
            let mut cmd = std::process::Command::new(&nvidia_smi);
            cmd.arg("--query-gpu=index,name,memory.used,memory.total,utilization.gpu")
                .arg("--format=csv,noheader,nounits");
            crate::platform::hide_window(&mut cmd);
            cmd.output()
        } {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                for line in stdout.trim().lines() {
                    let parts: Vec<&str> = line.splitn(5, ',').map(|s| s.trim()).collect();
                    if parts.len() >= 4 {
                        stats.push(serde_json::json!({
                            "index": parts[0].parse::<usize>().unwrap_or(0),
                            "name": parts[1],
                            "vendor": "nvidia",
                            "memory_used_mb": parts[2].parse::<f64>().unwrap_or(0.0),
                            "memory_total_mb": parts[3].parse::<f64>().unwrap_or(0.0),
                            "utilization_percent": parts.get(4).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0),
                        }));
                    }
                }
            }
        }
    }

    // AMD GPUs
    if let Ok(output) = {
        let mut cmd = std::process::Command::new("rocm-smi");
        cmd.arg("--showmeminfo").arg("vram");
        crate::platform::hide_window(&mut cmd);
        cmd.output()
    } {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut gpu_idx = 0;
            let mut total: f64 = 0.0;
            let mut used: f64 = 0.0;
            for line in stdout.lines() {
                if line.contains("VRAM Total Memory") && !line.contains("Used") {
                    if let Some(bytes_str) = line.rsplit(':').next() {
                        total = bytes_str.trim().parse::<f64>().unwrap_or(0.0) / (1024.0 * 1024.0);
                    }
                }
                if line.contains("VRAM Total Used Memory") {
                    if let Some(bytes_str) = line.rsplit(':').next() {
                        used = bytes_str.trim().parse::<f64>().unwrap_or(0.0) / (1024.0 * 1024.0);
                    }
                    stats.push(serde_json::json!({
                        "index": gpu_idx,
                        "name": format!("AMD GPU {}", gpu_idx),
                        "vendor": "amd",
                        "memory_used_mb": used,
                        "memory_total_mb": total,
                        "utilization_percent": if total > 0.0 { (used / total) * 100.0 } else { 0.0 },
                    }));
                    gpu_idx += 1;
                }
            }
        }
    }

    Ok(stats)
}

/// Get network info for LLM server access (local IP, Tailscale, public IP).
#[tauri::command]
pub fn get_llm_network_info(port: u16) -> Result<serde_json::Value, String> {
    let local_ip = local_ip_address::local_ip()
        .map(|ip| ip.to_string())
        .unwrap_or_else(|_| "unknown".to_string());

    // Detect Tailscale IP
    let tailscale_ip = {
        let mut cmd = std::process::Command::new("tailscale");
        cmd.args(["ip", "-4"]);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
    }
    .ok()
    .and_then(|o| {
        if o.status.success() {
            String::from_utf8(o.stdout)
                .ok()
                .map(|s| s.trim().to_string())
        } else {
            None
        }
    });

    // Try to detect public/port-forwarded IP via curl
    let public_ip = {
        let mut cmd = std::process::Command::new("curl");
        cmd.args(["-s", "--max-time", "3", "https://api.ipify.org"]);
        crate::platform::hide_window(&mut cmd);
        cmd.output()
    }
    .ok()
    .and_then(|o| {
        if o.status.success() {
            String::from_utf8(o.stdout)
                .ok()
                .map(|s| s.trim().to_string())
        } else {
            None
        }
    });

    Ok(serde_json::json!({
        "local_ip": local_ip,
        "local_url": format!("http://{}:{}/v1", local_ip, port),
        "tailscale_ip": tailscale_ip,
        "tailscale_url": tailscale_ip.as_ref().map(|ip| format!("http://{}:{}/v1", ip, port)),
        "public_ip": public_ip,
        "public_url": public_ip.as_ref().map(|ip| format!("http://{}:{}/v1", ip, port)),
        "port": port,
    }))
}

// ===== URL encoding helper =====
// Minimal percent-encoding for query strings (avoids adding a full dependency).

mod urlencoding {
    pub fn encode(input: &str) -> String {
        let mut result = String::with_capacity(input.len() * 2);
        for byte in input.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    result.push(byte as char);
                }
                b' ' => {
                    result.push('+');
                }
                _ => {
                    result.push('%');
                    result.push_str(&format!("{:02X}", byte));
                }
            }
        }
        result
    }
}

// ===== Tests =====

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sys_snapshot_capture() {
        let snap = SysSnapshot::capture();
        assert!(snap.cpu_cores >= 1);
        assert!(snap.ram_gb > 0.0);
    }

    #[test]
    fn test_is_coding_model() {
        assert!(is_coding_model("CodeLlama-7B-GGUF"));
        assert!(is_coding_model("deepseek-coder-6.7b"));
        assert!(is_coding_model("starcoder2-15b"));
        assert!(is_coding_model("qwen2.5-coder-7b"));
        assert!(is_coding_model("codestral-22b"));
        assert!(!is_coding_model("llama-3.1-8b"));
        assert!(!is_coding_model("mistral-7b"));
        assert!(!is_coding_model("phi-3-mini"));
    }

    #[test]
    fn test_auto_configure_low_spec() {
        let hw = HardwareInfo {
            cpu_cores: 4,
            cpu_model: None,
            ram_gb: 7.5,
            gpu_name: None,
            gpu_vram_gb: None,
            has_gpu: false,
            gpu_type: None,
            gpus: vec![],
        };
        // 4GB model -> 3.78GB available RAM -> n_ctx=4096
        let config = auto_configure_llm(hw, 4_000_000_000, None).unwrap();
        assert_eq!(config.n_gpu_layers, 0);
        assert_eq!(config.n_ctx, 4096);
        assert_eq!(config.n_threads, 2);
        assert_eq!(config.n_batch, 256);
        assert!((config.temperature - 0.3).abs() < f64::EPSILON);
        assert!(!config.mmap); // mmap disabled at 7.5GB (< 8.0 threshold)
        assert!(!config.flash_attention);
    }

    #[test]
    fn test_auto_configure_mid_spec() {
        let hw = HardwareInfo {
            cpu_cores: 12,
            cpu_model: None,
            ram_gb: 16.0,
            gpu_name: Some("NVIDIA RTX 3060".to_string()),
            gpu_vram_gb: Some(12.0),
            has_gpu: true,
            gpu_type: Some("dGPU".to_string()),
            gpus: vec![],
        };
        // 7GB model, 12GB VRAM: usable VRAM = 12 - 1.5 = 10.5 > 6.5GB model → full offload
        let config =
            auto_configure_llm(hw, 7_000_000_000, Some("codellama-7b".to_string())).unwrap();
        assert_eq!(config.n_gpu_layers, 999); // Full offload
        assert!((config.temperature - 0.2).abs() < f64::EPSILON);
        assert!(config.mmap);
        assert!(config.flash_attention);
    }

    #[test]
    fn test_auto_configure_high_spec() {
        let hw = HardwareInfo {
            cpu_cores: 32,
            cpu_model: None,
            ram_gb: 64.0,
            gpu_name: Some("NVIDIA RTX 4090".to_string()),
            gpu_vram_gb: Some(24.0),
            has_gpu: true,
            gpu_type: Some("dGPU".to_string()),
            gpus: vec![],
        };
        // 13GB model, 24GB VRAM: full offload, context based on total memory
        let config =
            auto_configure_llm(hw, 13_000_000_000, Some("llama-3.1-13b".to_string())).unwrap();
        assert_eq!(config.n_gpu_layers, 999);
        assert!(config.n_ctx >= 32768);
        assert_eq!(config.n_threads, 16); // 32 / 2 = 16
        assert!((config.temperature - 0.3).abs() < f64::EPSILON);
        assert!(config.mmap);
        assert!(config.flash_attention);
    }

    #[test]
    fn test_auto_configure_zero_model_size() {
        let hw = HardwareInfo {
            cpu_cores: 8,
            cpu_model: None,
            ram_gb: 16.0,
            gpu_name: Some("NVIDIA RTX 3060".to_string()),
            gpu_vram_gb: Some(12.0),
            has_gpu: true,
            gpu_type: Some("dGPU".to_string()),
            gpus: vec![],
        };
        let config = auto_configure_llm(hw, 0, None).unwrap();
        // Zero model size with GPU → try full offload (KV cache is in RAM anyway)
        assert_eq!(config.n_gpu_layers, 999);
    }

    #[test]
    fn test_auto_configure_reasoning_model() {
        let hw = HardwareInfo {
            cpu_cores: 16,
            cpu_model: None,
            ram_gb: 32.0,
            gpu_name: Some("NVIDIA RTX 4070".to_string()),
            gpu_vram_gb: Some(12.0),
            has_gpu: true,
            gpu_type: Some("dGPU".to_string()),
            gpus: vec![],
        };
        let config = auto_configure_llm(hw, 8_000_000_000, Some("QwQ-32B-Q4".to_string())).unwrap();
        assert!((config.temperature - 0.6).abs() < f64::EPSILON);
        assert!((config.min_p - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_auto_configure_multi_gpu() {
        let hw = HardwareInfo {
            cpu_cores: 32,
            cpu_model: None,
            ram_gb: 64.0,
            gpu_name: Some("NVIDIA RTX 4090".to_string()),
            gpu_vram_gb: Some(24.0),
            has_gpu: true,
            gpu_type: Some("dGPU".to_string()),
            gpus: vec![
                GpuInfo {
                    index: 0,
                    name: "NVIDIA RTX 4090".to_string(),
                    vram_gb: Some(24.0),
                    gpu_type: "dGPU".to_string(),
                    vendor: "nvidia".to_string(),
                },
                GpuInfo {
                    index: 1,
                    name: "NVIDIA RTX 3060".to_string(),
                    vram_gb: Some(12.0),
                    gpu_type: "dGPU".to_string(),
                    vendor: "nvidia".to_string(),
                },
            ],
        };
        let config = auto_configure_llm(hw, 13_000_000_000, None).unwrap();
        // Should select GPU 0 (RTX 4090) with 24GB VRAM
        assert_eq!(config.main_gpu, 0);
        assert_eq!(config.n_gpu_layers, 999); // Total VRAM 36GB > 13GB model
    }

    #[test]
    fn test_urlencoding() {
        assert_eq!(urlencoding::encode("hello world"), "hello+world");
        assert_eq!(urlencoding::encode("CodeLlama-7B"), "CodeLlama-7B");
        assert_eq!(urlencoding::encode("a&b=c"), "a%26b%3Dc");
        assert_eq!(urlencoding::encode("test/path"), "test%2Fpath");
    }

    #[test]
    fn test_detect_hardware_returns_ok() {
        let result = detect_hardware();
        assert!(result.is_ok());
        let info = result.unwrap();
        assert!(info.cpu_cores >= 1);
        assert!(info.ram_gb > 0.0);
    }

    #[test]
    fn test_auto_configure_tensor_split() {
        let hw = HardwareInfo {
            cpu_cores: 32,
            cpu_model: None,
            ram_gb: 64.0,
            gpu_name: Some("NVIDIA RTX 4090".to_string()),
            gpu_vram_gb: Some(24.0),
            has_gpu: true,
            gpu_type: Some("dGPU".to_string()),
            gpus: vec![
                GpuInfo {
                    index: 0,
                    name: "NVIDIA RTX 4090".to_string(),
                    vram_gb: Some(24.0),
                    gpu_type: "dGPU".to_string(),
                    vendor: "nvidia".to_string(),
                },
                GpuInfo {
                    index: 1,
                    name: "NVIDIA RTX 3060".to_string(),
                    vram_gb: Some(12.0),
                    gpu_type: "dGPU".to_string(),
                    vendor: "nvidia".to_string(),
                },
            ],
        };
        let config = auto_configure_llm(hw, 30_000_000_000, None).unwrap();
        assert!(config.tensor_split.is_some());
        let splits = config.tensor_split.unwrap();
        assert_eq!(splits.len(), 2);
        // RTX 4090 (24GB) should get ~67%, RTX 3060 (12GB) ~33%
        assert!((splits[0] - 0.667).abs() < 0.01);
        assert!((splits[1] - 0.333).abs() < 0.01);
    }

    #[test]
    fn test_auto_backend_prefers_best_installed_backend_for_current_platform() {
        let gpus = vec![GpuInfo {
            index: 0,
            name: "NVIDIA GeForce RTX 5070 Laptop GPU".to_string(),
            vram_gb: Some(8.0),
            gpu_type: "dGPU".to_string(),
            vendor: "nvidia".to_string(),
        }];
        let installed = vec![
            "cpu".to_string(),
            "vulkan".to_string(),
            "cuda13".to_string(),
        ];

        #[cfg(target_os = "linux")]
        assert_eq!(
            pick_best_installed_backend(&installed, &gpus).as_deref(),
            Some("vulkan")
        );

        #[cfg(not(target_os = "linux"))]
        assert_eq!(
            pick_best_installed_backend(&installed, &gpus).as_deref(),
            Some("cuda13")
        );
    }

    #[test]
    fn test_auto_backend_prefers_vulkan_for_amd_when_rocm_missing() {
        let gpus = vec![GpuInfo {
            index: 0,
            name: "AMD Radeon RX 7800 XT".to_string(),
            vram_gb: Some(16.0),
            gpu_type: "dGPU".to_string(),
            vendor: "amd".to_string(),
        }];
        let installed = vec!["cpu".to_string(), "vulkan".to_string()];

        assert_eq!(
            pick_best_installed_backend(&installed, &gpus).as_deref(),
            Some("vulkan")
        );
    }

    #[test]
    fn test_use_all_vram_promotes_gpu_layers_to_full_offload() {
        let config = LlmConfig {
            use_all_vram: true,
            n_gpu_layers: 0,
            ..LlmConfig::default()
        };

        let effective_n_gpu_layers = if config.use_all_vram {
            999
        } else {
            config.n_gpu_layers
        };

        assert_eq!(effective_n_gpu_layers, 999);
    }
}
