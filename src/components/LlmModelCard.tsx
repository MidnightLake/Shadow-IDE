interface LocalModel {
  name: string;
  path: string;
  model_type: string;
  size_bytes: number;
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

export const formatSize = (bytes: number | null): string => {
  if (bytes === null || bytes === 0) return "unknown";
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GB`;
};

// ===== Local Models List =====

interface LlmModelListProps {
  localModels: LocalModel[];
  selectedModel: string;
  scanning: boolean;
  onSelectModel: (path: string) => void;
  onBrowseModel: () => void;
  onScanModels: () => void;
  onDeleteModel: (path: string) => void;
}

export function LlmModelList({
  localModels,
  selectedModel,
  scanning,
  onSelectModel,
  onBrowseModel,
  onScanModels,
  onDeleteModel,
}: LlmModelListProps) {
  return (
    <div className="llm-section">
      <div className="llm-section-header">
        <span className="llm-section-title">LOCAL MODELS</span>
        <div className="llm-btn-group">
          <button className="llm-btn-sm" onClick={onBrowseModel}>Browse</button>
          <button className="llm-btn-sm" onClick={onScanModels} disabled={scanning}>{scanning ? "..." : "Scan"}</button>
        </div>
      </div>
      {localModels.length === 0 && !scanning && (
        <div className="llm-empty">No local models found. Browse or download from HuggingFace.</div>
      )}
      <div className="llm-model-list">
        {localModels.map((m) => (
          <div
            key={m.path}
            className={`llm-model-row ${selectedModel === m.path ? "selected" : ""}`}
            onClick={() => onSelectModel(m.path)}
          >
            <span className={`llm-model-type ${m.model_type}`}>{m.model_type.toUpperCase()}</span>
            <span className="llm-model-name" title={m.path}>{m.name}</span>
            <span className="llm-model-size">{formatSize(m.size_bytes)}</span>
            <button
              className="llm-btn-icon llm-model-delete"
              title="Delete model"
              onClick={(e) => { e.stopPropagation(); onDeleteModel(m.path); }}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6m3 0V4a2 2 0 012-2h4a2 2 0 012 2v2"/></svg>
            </button>
          </div>
        ))}
      </div>
    </div>
  );
}

// ===== HuggingFace Search & Download =====

interface LlmHuggingFaceProps {
  hfSearch: string;
  setHfSearch: (v: string) => void;
  hfResults: HfResult[];
  hfSearching: boolean;
  hfDownloading: boolean;
  hfProgress: { filename: string; percent: number } | null;
  hfFilePicker: { repoId: string; files: HfRepoFile[]; loading: boolean } | null;
  onSearch: () => void;
  onShowFilePicker: (repoId: string) => void;
  onDownloadFile: (repoId: string, filename: string) => void;
  onCloseFilePicker: () => void;
}

export function LlmHuggingFace({
  hfSearch,
  setHfSearch,
  hfResults,
  hfSearching,
  hfDownloading,
  hfProgress,
  hfFilePicker,
  onSearch,
  onShowFilePicker,
  onDownloadFile,
  onCloseFilePicker,
}: LlmHuggingFaceProps) {
  return (
    <div className="llm-section">
      <div className="llm-section-title">HUGGINGFACE</div>
      <div className="llm-hf-search">
        <input className="llm-input" style={{ flex: 1 }} value={hfSearch} onChange={(e) => setHfSearch(e.target.value)} placeholder="Search models (e.g. qwen3.5-9b)..." onKeyDown={(e) => e.key === "Enter" && onSearch()} />
        <button className="llm-btn-sm" onClick={onSearch} disabled={hfSearching}>{hfSearching ? "..." : "Search"}</button>
      </div>

      {hfDownloading && hfProgress && (
        <div className="llm-progress">
          <div className="llm-progress-label">{hfProgress.filename}</div>
          <div className="llm-progress-bar"><div className="llm-progress-fill" style={{ width: `${hfProgress.percent}%` }} /></div>
          <span className="llm-progress-pct">{hfProgress.percent.toFixed(1)}%</span>
        </div>
      )}

      {hfFilePicker && (
        <div className="llm-file-picker">
          <div className="llm-file-picker-header">
            <span>Select file from {hfFilePicker.repoId.split("/").pop()}</span>
            <button className="llm-btn-icon" onClick={onCloseFilePicker}>
              <svg width="10" height="10" viewBox="0 0 12 12"><line x1="3" y1="3" x2="9" y2="9" stroke="currentColor" strokeWidth="1.2"/><line x1="9" y1="3" x2="3" y2="9" stroke="currentColor" strokeWidth="1.2"/></svg>
            </button>
          </div>
          {hfFilePicker.loading && <div className="llm-empty">Loading files...</div>}
          {!hfFilePicker.loading && hfFilePicker.files.map((f) => (
            <div key={f.filename} className="llm-file-row">
              <span className="llm-file-name">{f.filename}</span>
              <span className="llm-file-size">{formatSize(f.size)}</span>
              <button className="llm-btn-sm" onClick={() => onDownloadFile(hfFilePicker.repoId, f.filename)} disabled={hfDownloading}>Download</button>
            </div>
          ))}
        </div>
      )}

      {hfResults.length > 0 && !hfFilePicker && (
        <div className="llm-hf-results">
          {hfResults.map((r) => (
            <div key={r.id} className="llm-hf-result">
              <div className="llm-hf-result-name">{r.id}</div>
              <div className="llm-hf-result-meta">
                <span>{((r.downloads ?? 0) / 1000).toFixed(0)}k dl</span>
                <span>{r.likes ?? 0} likes</span>
              </div>
              <button className="llm-btn-sm" onClick={() => onShowFilePicker(r.id)} disabled={hfDownloading}>Select File</button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
