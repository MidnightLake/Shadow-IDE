import React, { useState, useEffect, useRef, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

interface DocsPanelProps {
  language: string;
  fileUri: string;
}

interface LspHoverResult {
  contents: string;
}

interface CursorMoveDetail {
  line: number;
  character: number;
  word: string;
}

export default function DocsPanel({ language, fileUri }: DocsPanelProps) {
  const [symbol, setSymbol] = useState<string>("");
  const [hoverResult, setHoverResult] = useState<LspHoverResult | null>(null);
  const [loadingHover, setLoadingHover] = useState(false);
  const [aiEnrichment, setAiEnrichment] = useState<string>("");
  const [loadingAi, setLoadingAi] = useState(false);

  // Export API Docs state
  const [showExportForm, setShowExportForm] = useState(false);
  const [outputDir, setOutputDir] = useState("./docs");
  const [exporting, setExporting] = useState(false);
  const [exportResult, setExportResult] = useState<{ success: boolean; path?: string; error?: string } | null>(null);

  const debounceTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  // Listen for cursor move events (debounced 300ms)
  useEffect(() => {
    const handler = (e: Event) => {
      const detail = (e as CustomEvent<CursorMoveDetail>).detail;
      if (!detail) return;

      if (debounceTimer.current) clearTimeout(debounceTimer.current);
      debounceTimer.current = setTimeout(async () => {
        setSymbol(detail.word);
        setAiEnrichment("");
        if (!detail.word || !fileUri || !language) {
          setHoverResult(null);
          return;
        }
        setLoadingHover(true);
        try {
          const result = await invoke<LspHoverResult | null>("lsp_hover", {
            language,
            fileUri,
            line: detail.line,
            character: detail.character,
          });
          setHoverResult(result);
        } catch {
          setHoverResult(null);
        }
        setLoadingHover(false);
      }, 300);
    };

    window.addEventListener("editor-cursor-move", handler);
    return () => {
      window.removeEventListener("editor-cursor-move", handler);
      if (debounceTimer.current) clearTimeout(debounceTimer.current);
    };
  }, [language, fileUri]);

  const handleExport = useCallback(async () => {
    setExporting(true);
    setExportResult(null);
    try {
      const projectDir = fileUri ? fileUri.replace(/^file:\/\//, "").replace(/\/[^/]+$/, "") : ".";
      const result = await invoke<{ path: string }>("export_api_docs", { projectDir, outputDir });
      setExportResult({ success: true, path: result?.path ?? outputDir });
    } catch (err) {
      setExportResult({ success: false, error: String(err) });
    }
    setExporting(false);
  }, [fileUri, outputDir]);

  const openInBrowser = useCallback(async () => {
    const htmlPath = `${outputDir}/api-docs.html`;
    try {
      await invoke("shell_exec", { cmd: `xdg-open ${htmlPath}` });
    } catch { /* ignore */ }
  }, [outputDir]);

  const enrichWithAi = useCallback(async () => {
    if (!hoverResult?.contents) return;
    setLoadingAi(true);
    setAiEnrichment("");
    try {
      const result = await invoke<string>("ai_action", {
        action: "explain-symbol",
        text: hoverResult.contents,
        language,
      });
      setAiEnrichment(typeof result === "string" ? result : JSON.stringify(result));
    } catch (err) {
      setAiEnrichment(`Error: ${String(err)}`);
    }
    setLoadingAi(false);
  }, [hoverResult, language]);

  const codeStyle: React.CSSProperties = {
    background: "var(--bg-secondary, #181825)",
    borderRadius: 4,
    padding: "6px 10px",
    fontSize: 12,
    fontFamily: "var(--font-mono, monospace)",
    overflowX: "auto",
    whiteSpace: "pre-wrap",
    wordBreak: "break-all",
    color: "var(--text-primary, #cdd6f4)",
    margin: "4px 0",
  };

  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column", fontFamily: "monospace", fontSize: 12, color: "var(--text-primary, #cdd6f4)" }}>
      {/* Header */}
      <div style={{ padding: "8px 10px", borderBottom: "1px solid var(--border-color, #313244)", flexShrink: 0, display: "flex", alignItems: "center", gap: 8 }}>
        <span style={{ fontWeight: 700, color: "var(--accent, #89b4fa)", flex: 1 }}>
          Docs{symbol ? `: ${symbol}` : ""}
        </span>
        {loadingHover && <span style={{ fontSize: 10, color: "var(--text-muted)" }}>Loading...</span>}
        <button
          onClick={() => { setShowExportForm((v) => !v); setExportResult(null); }}
          style={{
            fontSize: 10,
            padding: "2px 7px",
            borderRadius: 4,
            border: "1px solid var(--border-color, #313244)",
            background: showExportForm ? "var(--accent, #89b4fa)" : "transparent",
            color: showExportForm ? "#1e1e2e" : "var(--accent, #89b4fa)",
            cursor: "pointer",
          }}
        >
          Export API Docs
        </button>
      </div>

      {/* Export form */}
      {showExportForm && (
        <div style={{ padding: "8px 10px", borderBottom: "1px solid var(--border-color, #313244)", flexShrink: 0, display: "flex", flexDirection: "column", gap: 6 }}>
          <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <label style={{ fontSize: 11, color: "var(--text-muted)", whiteSpace: "nowrap" }}>Output Dir:</label>
            <input
              type="text"
              value={outputDir}
              onChange={(e) => setOutputDir(e.target.value)}
              style={{
                flex: 1,
                background: "var(--bg-secondary, #181825)",
                border: "1px solid var(--border-color, #313244)",
                borderRadius: 3,
                color: "var(--text-primary, #cdd6f4)",
                padding: "3px 6px",
                fontSize: 11,
                outline: "none",
              }}
            />
            <button
              onClick={() => void handleExport()}
              disabled={exporting}
              style={{
                fontSize: 11,
                padding: "3px 8px",
                borderRadius: 4,
                border: "1px solid var(--accent, #89b4fa)",
                background: "transparent",
                color: exporting ? "var(--text-muted)" : "var(--accent, #89b4fa)",
                cursor: exporting ? "not-allowed" : "pointer",
              }}
            >
              {exporting ? "Generating..." : "Generate"}
            </button>
          </div>
          {exporting && (
            <div style={{ fontSize: 10, color: "var(--text-muted)" }}>Generating API docs...</div>
          )}
          {exportResult && exportResult.success && (
            <div style={{ fontSize: 11, color: "var(--theme-success, #a6e3a1)", display: "flex", alignItems: "center", gap: 6 }}>
              <span>Done: {exportResult.path}</span>
              <button
                onClick={() => void openInBrowser()}
                style={{
                  fontSize: 10,
                  padding: "1px 6px",
                  borderRadius: 3,
                  border: "1px solid var(--theme-accent, #89b4fa)",
                  background: "transparent",
                  color: "var(--theme-accent, #89b4fa)",
                  cursor: "pointer",
                }}
              >
                Open in browser
              </button>
            </div>
          )}
          {exportResult && !exportResult.success && (
            <div style={{ fontSize: 11, color: "var(--theme-error, #f38ba8)" }}>Error: {exportResult.error}</div>
          )}
        </div>
      )}

      <div style={{ flex: 1, overflowY: "auto", padding: 10 }}>
        {!symbol && !loadingHover && (
          <div style={{ color: "var(--text-muted)", padding: 8 }}>
            Move cursor over a symbol to see documentation.
          </div>
        )}

        {symbol && !loadingHover && !hoverResult && (
          <div style={{ color: "var(--text-muted)", padding: 8 }}>No documentation available for &ldquo;{symbol}&rdquo;.</div>
        )}

        {hoverResult && (
          <div>
            {/* LSP hover content */}
            <div style={{ marginBottom: 10 }}>
              <div style={{ fontSize: 11, fontWeight: 700, color: "var(--text-secondary)", marginBottom: 4, textTransform: "uppercase", letterSpacing: "0.06em" }}>
                Documentation
              </div>
              <div style={codeStyle}>{hoverResult.contents}</div>
            </div>

            {/* AI Enrichment section */}
            <div style={{ borderTop: "1px solid var(--border-color, #313244)", paddingTop: 8 }}>
              <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 6 }}>
                <span style={{ fontSize: 11, fontWeight: 700, color: "var(--accent)", textTransform: "uppercase", letterSpacing: "0.06em" }}>AI Enrichment</span>
                {!aiEnrichment && !loadingAi && (
                  <button
                    onClick={() => void enrichWithAi()}
                    style={{
                      fontSize: 11,
                      padding: "2px 8px",
                      borderRadius: 4,
                      border: "1px solid var(--accent, #89b4fa)",
                      background: "transparent",
                      color: "var(--accent, #89b4fa)",
                      cursor: "pointer",
                    }}
                  >
                    Enrich with AI
                  </button>
                )}
                {loadingAi && <span style={{ fontSize: 10, color: "var(--text-muted)" }}>Loading...</span>}
              </div>
              {aiEnrichment && (
                <div style={{ ...codeStyle, color: "var(--text-secondary, #bac2de)", fontSize: 12 }}>
                  {aiEnrichment}
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
