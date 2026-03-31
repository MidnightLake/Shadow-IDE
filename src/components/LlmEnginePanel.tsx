import { invoke } from "@tauri-apps/api/core";

interface GpuInfo {
  index: number;
  name: string;
  vram_gb: number | null;
  gpu_type: string;
  vendor: string;
}

interface HardwareInfo {
  cpu_cores: number;
  cpu_model: string | null;
  ram_gb: number;
  gpu_name: string | null;
  gpu_vram_gb: number | null;
  has_gpu: boolean;
  gpu_type: string | null;
  gpus: GpuInfo[];
}

interface LlmConfig {
  n_gpu_layers: number;
  use_all_vram: boolean;
  n_ctx: number;
  n_threads: number;
  n_batch: number;
  temperature: number;
  top_p: number;
  top_k: number;
  min_p: number;
  repeat_penalty: number;
  mmap: boolean;
  flash_attention: boolean;
  seed: number;
  keep_alive: boolean;
  rope_freq_base: number | null;
  rope_freq_scale: number | null;
  cache_type_k: string | null;
  cache_type_v: string | null;
  context_overflow: string;
  stop_strings: string[];
  main_gpu: number;
}

interface LlmEnginePanelProps {
  hwInfo: HardwareInfo;
  config: LlmConfig;
  setConfig: React.Dispatch<React.SetStateAction<LlmConfig>>;
  preferredBackend: string;
  setPreferredBackend: (v: string) => void;
  autoRecommendedBackend: string | null;
  installedBackends: string[];
  installing: boolean;
  installProgress: { percent?: number } | null;
  onInstallBackend: () => void;
  onRefreshEngineInfo: () => void;
  // VRAM estimation props
  selectedGpu: GpuInfo | undefined;
  selectedModelInfo: { size_bytes: number } | undefined;
  modelVram: number;
  kvOverhead: number;
  estVram: number;
  gpuVramTotal: number;
  vramPct: number;
  vramOver: boolean;
}

export default function LlmEnginePanel({
  hwInfo,
  config,
  setConfig,
  preferredBackend,
  setPreferredBackend,
  autoRecommendedBackend,
  installedBackends,
  installing,
  installProgress,
  onInstallBackend,
  onRefreshEngineInfo,
  selectedGpu,
  selectedModelInfo,
  modelVram,
  kvOverhead,
  estVram,
  gpuVramTotal,
  vramPct,
  vramOver,
}: LlmEnginePanelProps) {
  return (
    <div className="llm-section">
      <div className="llm-hw-chips">
        <span className="llm-chip" title={hwInfo.cpu_model || "CPU"}>
          {hwInfo.cpu_model ? hwInfo.cpu_model.split(" ").slice(-3).join(" ") : "CPU"} ({hwInfo.cpu_cores} cores)
        </span>
        <span className="llm-chip">{hwInfo.ram_gb.toFixed(1)} GB RAM</span>
        {hwInfo.gpus.length > 0 ? (
          hwInfo.gpus.map((gpu) => (
            <span key={gpu.index} className={`llm-chip llm-chip-gpu ${config.main_gpu === gpu.index ? "selected" : ""}`}
              title={`${gpu.vendor.toUpperCase()} | ${gpu.gpu_type} | ${gpu.vram_gb ? gpu.vram_gb.toFixed(1) + " GB VRAM" : "VRAM unknown"}`}
            >
              <span className="llm-gpu-type">{gpu.gpu_type}</span>
              <span className={`llm-vendor-badge llm-vendor-${gpu.vendor}`}>{gpu.vendor}</span>
              {gpu.name.split(" ").slice(-3).join(" ")}
              {gpu.vram_gb ? ` ${gpu.vram_gb.toFixed(0)}GB` : ""}
            </span>
          ))
        ) : hwInfo.has_gpu && (
          <span className="llm-chip llm-chip-gpu">
            {hwInfo.gpu_type && <span className="llm-gpu-type">{hwInfo.gpu_type}</span>}
            {hwInfo.gpu_name?.split(" ").slice(-3).join(" ") || "GPU"}
            {hwInfo.gpu_vram_gb ? ` ${hwInfo.gpu_vram_gb.toFixed(0)}GB` : ""}
          </span>
        )}
      </div>

      {/* GPU Selector Dropdown */}
      {hwInfo.gpus.length > 0 && (
        <div className="llm-gpu-selector">
          <label>GPU for Inference</label>
          <select
            className="llm-select"
            value={config.main_gpu}
            onChange={(e) => setConfig((c) => ({ ...c, main_gpu: Number(e.target.value) }))}
          >
            <option value={-1}>Auto (system default)</option>
            {hwInfo.gpus.map((gpu) => (
              <option key={gpu.index} value={gpu.index}>
                GPU {gpu.index}: {gpu.name}{gpu.vram_gb ? ` (${gpu.vram_gb.toFixed(0)}GB)` : ""} [{gpu.vendor.toUpperCase()}]
              </option>
            ))}
          </select>
        </div>
      )}

      {/* Engine Backend Selector */}
      <div className="llm-gpu-selector" style={{ marginTop: hwInfo.gpus.length > 0 ? "0px" : "8px" }}>
        <label style={{ marginTop: "12px", display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <span>Engine Backend</span>
          <span style={{ fontSize: "9px", opacity: 0.6 }}>{installedBackends.length} installed</span>
        </label>
        <div className="llm-backend-selector" style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: "3px", marginTop: "4px" }}>
          {["auto", "cuda13", "cuda12", "cuda11", "rocm", "vulkan", "sycl", "cpu"].map((b) => {
            const isInstalled = installedBackends.includes(b);
            const label = b === "sycl" ? "intel" : b === "cuda13" ? "CUDA 13" : b === "cuda12" ? "CUDA 12" : b === "cuda11" ? "CUDA 11" : b;
            return (
              <button
                key={b}
                className={`llm-btn-sm ${preferredBackend === b ? "active" : ""} ${isInstalled ? "installed" : ""}`}
                onClick={() => setPreferredBackend(b)}
                style={{
                  textTransform: "uppercase",
                  fontSize: "9px",
                  position: "relative",
                  borderColor: isInstalled && preferredBackend !== b ? "rgba(34, 197, 94, 0.4)" : undefined
                }}
                title={isInstalled ? `${label} is installed` : `${label} not installed`}
              >
                {label}
                {isInstalled && b !== "auto" && (
                  <span style={{
                    position: "absolute",
                    top: "-2px",
                    right: "-2px",
                    width: "6px",
                    height: "6px",
                    background: "#22c55e",
                    borderRadius: "50%",
                    border: "1px solid var(--bg-secondary)"
                  }} />
                )}
              </button>
            );
          })}
        </div>

        {(() => {
          const gpu = hwInfo.gpus.find((g) => g.index === config.main_gpu) || hwInfo.gpus[0];
          const targetBackend = preferredBackend === "auto"
            ? (autoRecommendedBackend || (gpu?.vendor === "nvidia" ? "cuda12" : gpu?.vendor === "amd" ? "rocm" : gpu?.vendor === "intel" ? "sycl" : "cpu"))
            : preferredBackend;
          const isTargetInstalled = installedBackends.includes(targetBackend);

          return (
            <div style={{ marginTop: "8px" }}>
              <div className={`llm-gpu-hint ${!isTargetInstalled ? "llm-gpu-warn" : ""}`}>
                {isTargetInstalled
                  ? `Ready: ${targetBackend.toUpperCase()} engine is installed.`
                  : `Notice: ${targetBackend.toUpperCase()} engine needs to be downloaded.`
                }
              </div>

              {!isTargetInstalled && (
                <button
                  className="llm-btn llm-btn-primary"
                  style={{ width: "100%", marginTop: "8px", fontSize: "11px", height: "28px" }}
                  onClick={onInstallBackend}
                  disabled={installing}
                >
                  {installing ? `Installing... ${installProgress?.percent?.toFixed(0) || 0}%` : `Download ${targetBackend.toUpperCase()} Engine`}
                </button>
              )}

              {isTargetInstalled && preferredBackend !== "auto" && (
                <button
                  className="llm-btn-sm"
                  style={{ width: "100%", marginTop: "8px", opacity: 0.6, fontSize: "9px" }}
                  onClick={async () => {
                    if (confirm(`Uninstall ${preferredBackend.toUpperCase()}?`)) {
                      await invoke("uninstall_engine", { backend: preferredBackend });
                      onRefreshEngineInfo();
                    }
                  }}
                >
                  Uninstall {preferredBackend.toUpperCase()}
                </button>
              )}
            </div>
          );
        })()}
      </div>

      {/* VRAM Usage Bar */}
      {selectedGpu && gpuVramTotal > 0 && selectedModelInfo && (
        <div className="llm-vram-section">
          <div className="llm-vram-header">
            <span>VRAM USAGE</span>
            <span className={vramOver ? "llm-vram-over" : ""}>{estVram.toFixed(1)} / {gpuVramTotal.toFixed(1)} GB</span>
          </div>
          <div className="llm-vram-bar">
            <div className={`llm-vram-fill ${vramOver ? "over" : ""}`} style={{ width: `${vramPct}%` }} />
          </div>
          <div className="llm-vram-detail">
            <span>Model: {modelVram.toFixed(1)} GB</span>
            <span>KV cache: ~{kvOverhead.toFixed(1)} GB</span>
          </div>
          <div className="llm-vram-layers-row">
            <label>
              GPU Layers
              <div className="llm-range-row">
                <input type="range" min={0} max={200} value={config.use_all_vram ? 200 : config.n_gpu_layers} disabled={config.use_all_vram}
                  onChange={(e) => setConfig((c) => ({ ...c, n_gpu_layers: +e.target.value }))} />
                <span>{config.use_all_vram ? "MAX" : config.n_gpu_layers}</span>
              </div>
            </label>
            <button className={`llm-btn-sm ${config.use_all_vram ? "active" : ""}`} title="Offload all layers to GPU (use all VRAM)"
              onClick={() => setConfig((c) => ({ ...c, use_all_vram: !c.use_all_vram }))}>
              {config.use_all_vram ? "Using Max" : "Max GPU"}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
