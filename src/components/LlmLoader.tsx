import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, emit } from "@tauri-apps/api/event";
import LlmEnginePanel from "./LlmEnginePanel";
import LlmServerStatus from "./LlmServerStatus";
import { LlmModelList, LlmHuggingFace } from "./LlmModelCard";

interface LlmLoaderProps {
  visible: boolean;
  rootPath: string;
}

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

interface LocalModel {
  name: string;
  path: string;
  model_type: string;
  size_bytes: number;
}

interface ServerStatus {
  running: boolean;
  port: number;
  model: string;
  binary: string;
  backend: string;
  error?: string;
}

interface EngineInfo {
  installed: boolean;
  binary_path: string;
  version: string;
  backend: string;
}

interface HfResult {
  id: string;
  downloads: number | null;
  likes: number | null;
}

interface HfRepoFile {
  filename: string;
  size: number | null;
}

export default function LlmLoader({ visible, rootPath }: LlmLoaderProps) {
  const [hwInfo, setHwInfo] = useState<HardwareInfo | null>(null);
  const [localModels, setLocalModels] = useState<LocalModel[]>([]);
  const [scanning, setScanning] = useState(false);
  const [selectedModel, setSelectedModel] = useState<string>(() => {
    return localStorage.getItem("llm-loader-model") || "";
  });

  useEffect(() => {
    localStorage.setItem("llm-loader-model", selectedModel);
  }, [selectedModel]);

  const [config, setConfig] = useState<LlmConfig>(() => {
    try {
      const saved = localStorage.getItem("llm-loader-config");
      if (saved) return JSON.parse(saved);
    } catch { /* ignore */ }
    return {
      n_gpu_layers: 0, use_all_vram: false, n_ctx: 4096, n_threads: 4, n_batch: 512,
      temperature: 0.7, top_p: 0.95, top_k: 40, min_p: 0.05,
      repeat_penalty: 1.1, mmap: true, flash_attention: false,
      seed: -1, keep_alive: true,
      rope_freq_base: null, rope_freq_scale: null,
      cache_type_k: null, cache_type_v: null,
      context_overflow: "truncate_middle", stop_strings: [],
      main_gpu: -1,
    };
  });
  
  const [port, setPort] = useState(() => {
    try { return Number(localStorage.getItem("llm-loader-port")) || 8080; } catch { return 8080; }
  });

  const [networkInfo, setNetworkInfo] = useState<{
    local_ip: string; local_url: string;
    tailscale_ip?: string; tailscale_url?: string;
    public_ip?: string; public_url?: string;
  } | null>(null);
  
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [showSampling, setShowSampling] = useState(false);
  const [stopInput, setStopInput] = useState("");

  const [serverStatus, setServerStatus] = useState<ServerStatus | null>(null);
  const [launching, setLaunching] = useState(false);
  const [serverMsg, setServerMsg] = useState("");

  const [hfSearch, setHfSearch] = useState("");
  const [hfResults, setHfResults] = useState<HfResult[]>([]);
  const [hfSearching, setHfSearching] = useState(false);
  const [hfFilePicker, setHfFilePicker] = useState<{ repoId: string; files: HfRepoFile[]; loading: boolean } | null>(null);
  const [hfDownloading, setHfDownloading] = useState(false);
  const [hfProgress, setHfProgress] = useState<{ filename: string; percent: number } | null>(null);

  // Engine state
  const [engineInfo, setEngineInfo] = useState<EngineInfo | null>(null);
  const [preferredBackend, setPreferredBackend] = useState<string>(() => {
    return localStorage.getItem("llm-loader-backend") || "auto";
  });
  const [autoRecommendedBackend, setAutoRecommendedBackend] = useState<string | null>(null);
  const [installedBackends, setInstalledBackends] = useState<string[]>([]);
  const [installing, setInstalling] = useState(false);
  const [installProgress, setInstallProgress] = useState<any>(null);

  // Save config on change
  useEffect(() => {
    localStorage.setItem("llm-loader-config", JSON.stringify(config));
    window.dispatchEvent(new CustomEvent("llm-config-updated"));
  }, [config]);

  useEffect(() => {
    localStorage.setItem("llm-loader-port", String(port));
  }, [port]);

  useEffect(() => {
    localStorage.setItem("llm-loader-backend", preferredBackend);
  }, [preferredBackend]);

  const statusTimer = useRef<ReturnType<typeof setInterval> | null>(null);
  const autoGpuDone = useRef(false);

  useEffect(() => {
    invoke<HardwareInfo>("detect_hardware").then((hw) => {
      setHwInfo(hw);
      // Auto-select best GPU (prefer discrete, most VRAM)
      if (hw.gpus.length > 0 && !autoGpuDone.current) {
        autoGpuDone.current = true;
        const best = [...hw.gpus].sort((a, b) => {
          if (a.gpu_type === "dGPU" && b.gpu_type !== "dGPU") return -1;
          if (b.gpu_type === "dGPU" && a.gpu_type !== "dGPU") return 1;
          return (b.vram_gb || 0) - (a.vram_gb || 0);
        })[0];
        setConfig((c) => c.main_gpu < 0 ? { ...c, main_gpu: best.index } : c);
      }
    }).catch(() => {});

    invoke<string>("detect_recommended_backend")
      .then((backend) => setAutoRecommendedBackend(backend))
      .catch(() => setAutoRecommendedBackend(null));
    
    refreshEngineInfo();
    checkServerStatus();
    scanModels();
  }, [rootPath]);

  const refreshEngineInfo = async () => {
    try {
      const info = await invoke<EngineInfo>("check_engine", { backend: preferredBackend === "auto" ? null : preferredBackend });
      setEngineInfo(info);
      const installed = await invoke<string[]>("list_installed_engines");
      setInstalledBackends(installed);
    } catch (e) { console.error("Refresh engine info failed:", e); }
  };

  useEffect(() => {
    if (visible) refreshEngineInfo();
  }, [preferredBackend, visible]);

  useEffect(() => {
    if (!visible) return;
    statusTimer.current = setInterval(checkServerStatus, 5000);
    return () => { if (statusTimer.current) clearInterval(statusTimer.current); };
  }, [visible]);

  useEffect(() => {
    const unlistenPromise = listen<{ filename: string; percent: number }>("hf-download-progress", (e) => {
      setHfProgress(e.payload);
    });
    return () => { unlistenPromise.then(fn => fn()); };
  }, []);

  useEffect(() => {
    const unlistenPromise = listen<string>("hf-download-complete", () => {
      setHfDownloading(false);
      setHfProgress(null);
      scanModels();
    });
    return () => { unlistenPromise.then(fn => fn()); };
  }, []);

  useEffect(() => {
    const unlistenPromise = listen<any>("engine-install-progress", (e) => {
      setInstallProgress(e.payload);
    });
    return () => { unlistenPromise.then(fn => fn()); };
  }, []);

  const installSelectedBackend = async () => {
    const backend = preferredBackend === "auto" ? (hwInfo ? await invoke<string>("detect_recommended_backend") : "cpu") : preferredBackend;
    setInstalling(true);
    setServerMsg(`Installing ${backend} engine...`);
    try {
      await invoke("install_engine", { backend });
      // Small delay to allow filesystem to sync
      await new Promise(r => setTimeout(r, 800));
      await refreshEngineInfo();
      setServerMsg(`Successfully installed ${backend} engine.`);
    } catch (e) {
      setServerMsg(`Installation failed: ${e}`);
    }
    setInstalling(false);
    setInstallProgress(null);
  };

  const checkServerStatus = async () => {
    try {
      const status = await invoke<ServerStatus>("get_llm_server_status");
      setServerStatus(status);
      // Always fetch network info so users can see their IPs
      const p = status.running ? status.port : port;
      invoke<any>("get_llm_network_info", { port: p }).then(setNetworkInfo).catch((e) => console.error("network info err:", e));
    } catch { setServerStatus(null); }
  };

  const scanModels = async () => {
    if (!rootPath) return;
    setScanning(true);
    try {
      const models = await invoke<LocalModel[]>("scan_local_models", { basePath: rootPath });
      setLocalModels(models ?? []);
      if ((models ?? []).length > 0 && !selectedModel) setSelectedModel(models[0].path);
    } catch {
      try {
        const home = await invoke<string>("get_home_dir");
        const models = await invoke<LocalModel[]>("scan_local_models", { basePath: home });
        setLocalModels(models ?? []);
        if ((models ?? []).length > 0 && !selectedModel) setSelectedModel(models[0].path);
      } catch { setLocalModels([]); }
    }
    setScanning(false);
  };

  const browseModel = async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        multiple: false, directory: false,
        filters: [
          { name: "GGUF Models", extensions: ["gguf"] },
          { name: "All Model Files", extensions: ["gguf", "bin", "safetensors"] },
        ],
      });
      if (selected && typeof selected === "string") {
        const fileName = selected.split("/").pop() || "model.gguf";
        const size = await invoke<{ size: number; is_binary: boolean }>("get_file_info", { path: selected }).catch(() => ({ size: 0, is_binary: true }));
        setLocalModels((prev) => [
          ...prev,
          { name: fileName.replace(/\.(gguf|bin|safetensors)$/i, ""), path: selected, model_type: fileName.endsWith(".gguf") ? "gguf" : "model", size_bytes: size.size },
        ]);
        setSelectedModel(selected);
      }
    } catch (e) { console.error("Browse model failed:", e); }
  };

  const deleteModel = async (modelPath: string) => {
    try {
      await invoke<string>("delete_local_model", { modelPath });
      setLocalModels((prev) => prev.filter((m) => m.path !== modelPath));
      if (selectedModel === modelPath) setSelectedModel("");
    } catch (e) { console.error("Delete failed:", e); }
  };

  const autoConfigureLlm = useCallback(async () => {
    if (!hwInfo || !selectedModel) return;
    const model = localModels.find((m) => m.path === selectedModel);
    const sizeBytes = model?.size_bytes || 0;
    try {
      const cfg = await invoke<LlmConfig>("auto_configure_llm", {
        hardware: hwInfo, modelSizeBytes: sizeBytes, modelName: model?.name || null,
      });
      setConfig(cfg);
    } catch (e) { console.error("Auto-configure failed:", e); }
  }, [hwInfo, selectedModel, localModels]);

  const getSelectedGpuVendor = (): string | undefined => {
    if (!hwInfo || config.main_gpu < 0) return undefined;
    const gpu = hwInfo.gpus.find((g) => g.index === config.main_gpu);
    return gpu?.vendor;
  };

  const launchServer = async () => {
    if (!selectedModel) { setServerMsg("Select a model first"); return; }
    setLaunching(true);
    setServerMsg("");
    try {
      const gpuVendor = getSelectedGpuVendor();
      const msg = await invoke<string>("launch_llm_server", { 
        modelPath: selectedModel, 
        config, 
        port, 
        gpuVendor: gpuVendor || null,
        backend: preferredBackend === "auto" ? null : preferredBackend
      });
      setServerMsg(msg);
      // Give the process time to initialize, then verify it's still running
      await new Promise((r) => setTimeout(r, 1000));
      const status = await invoke<ServerStatus>("get_llm_server_status");
      setServerStatus(status);
      if (status.running) {
        // Use 0.0.0.0 accessible URL so port forwarding works
        const netInfo = await invoke<any>("get_llm_network_info", { port: status.port }).catch(() => null);
        setNetworkInfo(netInfo);
        const serverUrl = netInfo?.local_url || `http://localhost:${port}/v1`;
        await emit("llm-server-started", { port, url: serverUrl });
      } else {
        setServerMsg(msg + " — but process exited immediately. Check if CUDA libs are available.");
      }
    } catch (e) { setServerMsg(String(e)); }
    setLaunching(false);
  };

  const stopServer = async () => {
    try {
      const msg = await invoke<string>("stop_llm_server");
      setServerMsg(msg);
      setServerStatus(null);
    } catch (e) { setServerMsg(String(e)); }
  };

  const unloadModel = async () => {
    try {
      const msg = await invoke<string>("unload_llm_model");
      setServerMsg(msg);
      setServerStatus(null);
    } catch (e) { setServerMsg(String(e)); }
  };

  const searchHuggingFace = async () => {
    if (!hfSearch.trim()) return;
    setHfSearching(true);
    try {
      const results = await invoke<HfResult[]>("search_hf_models", { query: hfSearch.trim() });
      setHfResults(results);
    } catch (e) { console.error("HF search failed:", e); }
    setHfSearching(false);
  };

  const showFilePicker = async (repoId: string) => {
    setHfFilePicker({ repoId, files: [], loading: true });
    try {
      const files = await invoke<HfRepoFile[]>("list_hf_repo_files", { repoId });
      setHfFilePicker({ repoId, files: files ?? [], loading: false });
    } catch { setHfFilePicker(null); }
  };

  const downloadFile = async (repoId: string, filename: string) => {
    setHfFilePicker(null);
    setHfDownloading(true);
    setHfProgress({ filename, percent: 0 });
    try {
      const saveDir = rootPath ? `${rootPath}/models/gguf` : "models/gguf";
      await invoke("download_hf_model", { repoId, filename, saveDir });
    } catch (e) { console.error("Download failed:", e); }
    setHfDownloading(false);
    setHfProgress(null);
  };


  const addStopString = () => {
    const s = stopInput.trim();
    if (s && !config.stop_strings.includes(s)) {
      setConfig((c) => ({ ...c, stop_strings: [...c.stop_strings, s] }));
    }
    setStopInput("");
  };

  // VRAM estimation
  const selectedGpu = hwInfo?.gpus.find((g) => g.index === config.main_gpu) || hwInfo?.gpus[0];
  const selectedModelInfo = localModels.find((m) => m.path === selectedModel);
  const modelSizeGb = selectedModelInfo ? selectedModelInfo.size_bytes / (1024 * 1024 * 1024) : 0;
  const estLayers = modelSizeGb < 2 ? 24 : modelSizeGb < 6 ? 32 : modelSizeGb < 15 ? 40 : modelSizeGb < 40 ? 60 : 80;
  const gpuFrac = config.n_gpu_layers === 0 ? 0 : Math.min(config.n_gpu_layers / estLayers, 1);
  const modelVram = modelSizeGb * gpuFrac;
  const kvOverhead = (config.n_ctx / 4096) * 0.2;
  const estVram = modelVram + kvOverhead;
  const gpuVramTotal = selectedGpu?.vram_gb || 0;
  const vramPct = gpuVramTotal > 0 ? Math.min((estVram / gpuVramTotal) * 100, 100) : 0;
  const vramOver = estVram > gpuVramTotal && gpuVramTotal > 0;

  if (!visible) return null;

  return (
    <div className="llm-loader">
      <div className="llm-header">
        <span className="llm-title">LLM LOADER</span>
      </div>

      {/* Hardware Info & Engine Panel */}
      {hwInfo && (
        <LlmEnginePanel
          hwInfo={hwInfo}
          config={config}
          setConfig={setConfig}
          preferredBackend={preferredBackend}
          setPreferredBackend={setPreferredBackend}
          autoRecommendedBackend={autoRecommendedBackend}
          installedBackends={installedBackends}
          installing={installing}
          installProgress={installProgress}
          onInstallBackend={installSelectedBackend}
          onRefreshEngineInfo={refreshEngineInfo}
          selectedGpu={selectedGpu}
          selectedModelInfo={selectedModelInfo}
          modelVram={modelVram}
          kvOverhead={kvOverhead}
          estVram={estVram}
          gpuVramTotal={gpuVramTotal}
          vramPct={vramPct}
          vramOver={vramOver}
        />
      )}

      {/* Server Status */}
      <LlmServerStatus
        serverStatus={serverStatus}
        engineInfo={engineInfo}
        networkInfo={networkInfo}
        launching={launching}
        selectedModel={selectedModel}
        port={port}
        setPort={setPort}
        serverMsg={serverMsg}
        onLaunch={launchServer}
        onStop={stopServer}
        onUnload={unloadModel}
      />

      {/* Local Models */}
      <LlmModelList
        localModels={localModels}
        selectedModel={selectedModel}
        scanning={scanning}
        onSelectModel={setSelectedModel}
        onBrowseModel={browseModel}
        onScanModels={scanModels}
        onDeleteModel={deleteModel}
      />

      {/* Settings */}
      <div className="llm-section">
        <div className="llm-section-header">
          <span className="llm-section-title">SETTINGS</span>
          <button className="llm-btn-sm" onClick={autoConfigureLlm} title="Auto-detect optimal settings">Auto</button>
        </div>
        <div className="llm-config-grid">
          <label>Temperature
            <div className="llm-range-row">
              <input type="range" min={0} max={200} value={config.temperature * 100} onChange={(e) => setConfig((c) => ({ ...c, temperature: +e.target.value / 100 }))} />
              <span>{config.temperature.toFixed(2)}</span>
            </div>
          </label>
          <label>Context Length <input type="number" className="llm-input" value={config.n_ctx} min={512} max={262144} step={512} onChange={(e) => setConfig((c) => ({ ...c, n_ctx: +e.target.value }))} /></label>
          <label>Context Overflow
            <select className="llm-select" value={config.context_overflow} onChange={(e) => setConfig((c) => ({ ...c, context_overflow: e.target.value }))}>
              <option value="truncate_middle">Truncate Middle</option>
              <option value="truncate_start">Truncate Start</option>
              <option value="stop">Stop</option>
            </select>
          </label>
          <label>CPU Threads <input type="number" className="llm-input" value={config.n_threads} min={1} max={64} onChange={(e) => setConfig((c) => ({ ...c, n_threads: +e.target.value }))} /></label>
          <label>Stop Strings
            <div className="llm-stop-row">
              <input className="llm-input" value={stopInput} onChange={(e) => setStopInput(e.target.value)} onKeyDown={(e) => e.key === "Enter" && addStopString()} placeholder="Enter string..." />
            </div>
            {config.stop_strings.length > 0 && (
              <div className="llm-stop-tags">
                {config.stop_strings.map((s, i) => (
                  <span key={i} className="llm-stop-tag">
                    {s}
                    <button onClick={() => setConfig((c) => ({ ...c, stop_strings: c.stop_strings.filter((_, j) => j !== i) }))}>&times;</button>
                  </span>
                ))}
              </div>
            )}
          </label>
        </div>
      </div>

      {/* Sampling */}
      <div className="llm-section">
        <div className="llm-section-header llm-collapsible" onClick={() => setShowSampling((v) => !v)}>
          <span className="llm-section-title">SAMPLING</span>
          <span className="llm-chevron">{showSampling ? "\u25BC" : "\u25B6"}</span>
        </div>
        {showSampling && (
          <div className="llm-config-grid">
            <label>Top K <input type="number" className="llm-input" value={config.top_k} min={0} max={200} onChange={(e) => setConfig((c) => ({ ...c, top_k: +e.target.value }))} /></label>
            <label>Top P
              <div className="llm-range-row">
                <input type="range" min={0} max={100} value={config.top_p * 100} onChange={(e) => setConfig((c) => ({ ...c, top_p: +e.target.value / 100 }))} />
                <span>{config.top_p.toFixed(2)}</span>
              </div>
            </label>
            <label>Min P
              <div className="llm-range-row">
                <input type="range" min={0} max={100} value={config.min_p * 100} onChange={(e) => setConfig((c) => ({ ...c, min_p: +e.target.value / 100 }))} />
                <span>{config.min_p.toFixed(2)}</span>
              </div>
            </label>
            <label>Repeat Penalty <input type="number" className="llm-input" value={config.repeat_penalty} min={1} max={2} step={0.05} onChange={(e) => setConfig((c) => ({ ...c, repeat_penalty: +e.target.value }))} /></label>
            <label>Seed <input type="number" className="llm-input" value={config.seed} min={-1} onChange={(e) => setConfig((c) => ({ ...c, seed: +e.target.value }))} />
              <span className="llm-hint-inline">{config.seed === -1 ? "Random" : ""}</span>
            </label>
          </div>
        )}
      </div>

      {/* Advanced / Context & Offload */}
      <div className="llm-section">
        <div className="llm-section-header llm-collapsible" onClick={() => setShowAdvanced((v) => !v)}>
          <span className="llm-section-title">ADVANCED</span>
          <span className="llm-chevron">{showAdvanced ? "\u25BC" : "\u25B6"}</span>
        </div>
        {showAdvanced && (
          <div className="llm-config-grid">
            <label>GPU Offload 
              <div style={{ display: "flex", gap: "8px", alignItems: "center" }}>
                <input type="number" className="llm-input" value={config.n_gpu_layers} min={0} max={200} disabled={config.use_all_vram} onChange={(e) => setConfig((c) => ({ ...c, n_gpu_layers: +e.target.value }))} style={{ width: "60px" }} />
                <label style={{ display: "flex", gap: "4px", margin: 0, fontWeight: "normal" }}>
                  <input type="checkbox" checked={config.use_all_vram} onChange={(e) => setConfig((c) => ({ ...c, use_all_vram: e.target.checked }))} /> Max
                </label>
              </div>
            </label>
            <label>Batch Size <input type="number" className="llm-input" value={config.n_batch} min={64} max={4096} step={64} onChange={(e) => setConfig((c) => ({ ...c, n_batch: +e.target.value }))} /></label>
            <label className="llm-toggle-row">
              <input type="checkbox" checked={config.mmap} onChange={(e) => setConfig((c) => ({ ...c, mmap: e.target.checked }))} /> Try mmap()
            </label>
            <label className="llm-toggle-row">
              <input type="checkbox" checked={config.flash_attention} onChange={(e) => setConfig((c) => ({ ...c, flash_attention: e.target.checked }))} /> Flash Attention
            </label>
            <label className="llm-toggle-row">
              <input type="checkbox" checked={config.keep_alive} onChange={(e) => setConfig((c) => ({ ...c, keep_alive: e.target.checked }))} /> Keep Model in Memory
            </label>
            <label>RoPE Freq Base
              <input type="number" className="llm-input" value={config.rope_freq_base ?? ""} placeholder="Auto" onChange={(e) => setConfig((c) => ({ ...c, rope_freq_base: e.target.value ? +e.target.value : null }))} />
            </label>
            <label>RoPE Freq Scale
              <input type="number" className="llm-input" value={config.rope_freq_scale ?? ""} placeholder="Auto" step={0.01} onChange={(e) => setConfig((c) => ({ ...c, rope_freq_scale: e.target.value ? +e.target.value : null }))} />
            </label>
            <label className="llm-toggle-row">
              <input type="checkbox" checked={config.cache_type_k !== null} onChange={(e) => setConfig((c) => ({ ...c, cache_type_k: e.target.checked ? "q8_0" : null }))} /> KV Cache Quantization (K)
            </label>
            {config.cache_type_k !== null && (
              <label>
                <select className="llm-select" value={config.cache_type_k} onChange={(e) => setConfig((c) => ({ ...c, cache_type_k: e.target.value }))}>
                  <option value="f16">f16</option>
                  <option value="q8_0">q8_0</option>
                  <option value="q4_0">q4_0</option>
                </select>
              </label>
            )}
            <label className="llm-toggle-row">
              <input type="checkbox" checked={config.cache_type_v !== null} onChange={(e) => setConfig((c) => ({ ...c, cache_type_v: e.target.checked ? "q8_0" : null }))} /> KV Cache Quantization (V)
            </label>
            {config.cache_type_v !== null && (
              <label>
                <select className="llm-select" value={config.cache_type_v} onChange={(e) => setConfig((c) => ({ ...c, cache_type_v: e.target.value }))}>
                  <option value="f16">f16</option>
                  <option value="q8_0">q8_0</option>
                  <option value="q4_0">q4_0</option>
                </select>
              </label>
            )}
          </div>
        )}
      </div>

      {/* HuggingFace Download */}
      <LlmHuggingFace
        hfSearch={hfSearch}
        setHfSearch={setHfSearch}
        hfResults={hfResults}
        hfSearching={hfSearching}
        hfDownloading={hfDownloading}
        hfProgress={hfProgress}
        hfFilePicker={hfFilePicker}
        onSearch={searchHuggingFace}
        onShowFilePicker={showFilePicker}
        onDownloadFile={downloadFile}
        onCloseFilePicker={() => setHfFilePicker(null)}
      />
    </div>
  );
}
