import { useState, useEffect, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import type { CursorInfo, DiagnosticCounts } from "./Editor";
import type { PlanengineShellSummary } from "../planengine/summary";

interface AiCostUpdate {
  session_cost: number;
  daily_cost: number;
  total_tokens: number;
}

interface LspStatusChangedEvent {
  language: string;
  status: "loading" | "ready" | "error";
}

const LSP_STATUS_DOT_COLOR: Record<"loading" | "ready" | "error", string> = {
  loading: "#f9c74f",
  ready: "#3fb950",
  error: "#f85149",
};

interface StatusBarProps {
  diagnosticCounts: DiagnosticCounts;
  cursorInfo: CursorInfo;
  currentLanguage: string;
  activeFile: boolean;
  aiCompletionEnabled: boolean;
  currentModel?: string;
  contextTokens?: number;
  maxContextTokens?: number;
  planSummary?: PlanengineShellSummary;
  onOpenPlanengine?: () => void;
  onToggleErrorPanel: () => void;
  onHide: () => void;
}

export function StatusBar({
  diagnosticCounts, cursorInfo, currentLanguage,
  activeFile, aiCompletionEnabled, currentModel, contextTokens, maxContextTokens, planSummary,
  onOpenPlanengine,
  onToggleErrorPanel, onHide,
}: StatusBarProps) {
  const [sessionCost, setSessionCost] = useState<number | null>(null);
  const [lspStatus, setLspStatus] = useState<Record<string, "loading" | "ready" | "error">>({});
  const [startupMs, setStartupMs] = useState<number | null>(null);
  const [startupMarks, setStartupMarks] = useState<Array<{ name: string; timestamp_ms: number }>>([]);
  const [showStartupPopup, setShowStartupPopup] = useState(false);
  const startupRecordedRef = useRef(false);

  useEffect(() => {
    const unsub = listen<AiCostUpdate>("ai-cost-update", (e) => {
      setSessionCost(e.payload.session_cost);
    });
    return () => { unsub.then((fn) => fn()); };
  }, []);

  useEffect(() => {
    const unsub = listen<LspStatusChangedEvent>("lsp-status-changed", (e) => {
      const { language, status } = e.payload;
      setLspStatus((prev) => ({ ...prev, [language]: status }));
    });
    return () => { unsub.then((fn) => fn()); };
  }, []);

  useEffect(() => {
    if (startupRecordedRef.current) return;
    startupRecordedRef.current = true;
    const nowMs = performance.now();
    invoke("record_startup_mark", { markName: "ui-interactive", timestampMs: nowMs }).catch(() => {});
    invoke<Array<{ name: string; timestamp_ms: number }>>("get_startup_metrics")
      .then((marks) => {
        setStartupMarks(marks);
        if (marks.length >= 2) {
          const first = marks[0].timestamp_ms;
          const last = marks[marks.length - 1].timestamp_ms;
          setStartupMs(Math.round(last - first));
        }
      })
      .catch(() => {});
  }, []);

  const planSummaryLabel = planSummary?.totals
    ? `Plan ${planSummary.totals.done}/${planSummary.totals.total}`
    : "Plan";
  const planSummaryTitle = planSummary
    ? `${planSummaryLabel}${planSummary.criticalGaps.length > 0 ? ` • ${planSummary.criticalGaps.length} blocking gap${planSummary.criticalGaps.length === 1 ? "" : "s"}` : " • no blocking gaps listed"}${planSummary.nextSteps[0] ? ` • Next: ${planSummary.nextSteps[0]}` : ""}`
    : "Open PlanEngine roadmap";

  return (
    <div className="status-bar" role="status" aria-label="Status bar">
      <div className="status-diagnostics" role="button" aria-label="Toggle diagnostics panel" onClick={onToggleErrorPanel} style={{ cursor: "pointer" }}>
        <span className={`status-diag status-diag-error`} title={`${diagnosticCounts.errors} error${diagnosticCounts.errors !== 1 ? "s" : ""} — click to view`} style={diagnosticCounts.errors > 0 ? undefined : { opacity: 0.5 }}>
          <svg width="12" height="12" viewBox="0 0 16 16" fill="currentColor"><circle cx="8" cy="8" r="7"/><path d="M5.5 5.5l5 5M10.5 5.5l-5 5" stroke="var(--bg-primary)" strokeWidth="1.5"/></svg>
          {diagnosticCounts.errors}
        </span>
        <span className={`status-diag status-diag-warn`} title={`${diagnosticCounts.warnings} warning${diagnosticCounts.warnings !== 1 ? "s" : ""} — click to view`} style={diagnosticCounts.warnings > 0 ? undefined : { opacity: 0.5 }}>
          <svg width="12" height="12" viewBox="0 0 16 16" fill="currentColor"><path d="M7.56 1.44a.5.5 0 01.88 0l6.5 12A.5.5 0 0114.5 14h-13a.5.5 0 01-.44-.74l6.5-12z"/><path d="M8 6v3" stroke="var(--bg-primary)" strokeWidth="1.5" strokeLinecap="round"/><circle cx="8" cy="11" r=".75" fill="var(--bg-primary)"/></svg>
          {diagnosticCounts.warnings}
        </span>
        {diagnosticCounts.infos > 0 && (
          <span className="status-diag status-diag-info" title={`${diagnosticCounts.infos} info — click to view`}>
            <svg width="12" height="12" viewBox="0 0 16 16" fill="currentColor"><circle cx="8" cy="8" r="7"/><path d="M8 7v4" stroke="var(--bg-primary)" strokeWidth="1.5" strokeLinecap="round"/><circle cx="8" cy="5" r=".75" fill="var(--bg-primary)"/></svg>
            {diagnosticCounts.infos}
          </span>
        )}
      </div>

      <div className="status-spacer" />

      {activeFile && (
        <span className="status-cursor">
          Ln {cursorInfo.line}, Col {cursorInfo.column}
          {cursorInfo.selected > 0 && <span className="status-selected">({cursorInfo.selected} sel)</span>}
        </span>
      )}

      <div className="status-spacer" />

      {currentLanguage && <span className="status-language">{currentLanguage}</span>}
      {aiCompletionEnabled && <span className="status-ai-badge">AI</span>}
      {onOpenPlanengine && (
        <>
          <button
            onClick={onOpenPlanengine}
            title={planSummaryTitle}
            style={{
              background: "transparent",
              border: "1px solid rgba(233, 170, 95, 0.35)",
              color: "#e9aa5f",
              borderRadius: 5,
              fontSize: "10px",
              fontWeight: 700,
              letterSpacing: "0.08em",
              textTransform: "uppercase",
              padding: "1px 6px",
              cursor: "pointer",
            }}
          >
            Plan
          </button>
          {planSummary && (
            <button
              onClick={onOpenPlanengine}
              title={planSummaryTitle}
              style={{
                background: "transparent",
                border: "1px solid rgba(142, 184, 212, 0.25)",
                color: "#8eb5c4",
                borderRadius: 5,
                fontSize: "10px",
                fontWeight: 700,
                padding: "1px 6px",
                cursor: "pointer",
              }}
            >
              {planSummaryLabel}
              {planSummary.criticalGaps.length > 0 ? ` • ${planSummary.criticalGaps.length} gaps` : ""}
            </button>
          )}
        </>
      )}

      {/* LSP status indicators */}
      {Object.entries(lspStatus).map(([lang, status]) => (
        <span
          key={lang}
          title={`LSP ${lang}: ${status}`}
          style={{ display: "flex", alignItems: "center", gap: "3px", fontSize: "11px", opacity: 0.9 }}
        >
          <span
            style={{
              display: "inline-block",
              width: "7px",
              height: "7px",
              borderRadius: "50%",
              background: LSP_STATUS_DOT_COLOR[status],
              flexShrink: 0,
              animation: status === "loading" ? "lsp-pulse 1s ease-in-out infinite" : undefined,
            }}
          />
          <span style={{ textTransform: "capitalize" }}>{lang}</span>
        </span>
      ))}

      {/* Model indicator */}
      {currentModel && (
        <span className="status-language" title={`Current model: ${currentModel}`} style={{ opacity: 0.7, fontSize: "10px" }}>
          {currentModel.length > 20 ? currentModel.slice(0, 20) + "…" : currentModel}
        </span>
      )}

      {/* Context token bar */}
      {contextTokens != null && maxContextTokens != null && maxContextTokens > 0 && (
        <span title={`Context: ${contextTokens}/${maxContextTokens} tokens`} style={{ display: "flex", alignItems: "center", gap: "3px", opacity: 0.7 }}>
          <span style={{ fontSize: "10px" }}>{Math.round((contextTokens / maxContextTokens) * 100)}%</span>
          <span style={{ width: "32px", height: "4px", background: "var(--bg-primary)", borderRadius: "2px", overflow: "hidden", display: "inline-block" }}>
            <span style={{ display: "block", height: "100%", width: `${Math.min(100, Math.round((contextTokens / maxContextTokens) * 100))}%`, background: contextTokens / maxContextTokens > 0.85 ? "#f38ba8" : "#89b4fa", borderRadius: "2px" }} />
          </span>
        </span>
      )}

      {/* Cost indicator */}
      {sessionCost != null && sessionCost > 0 && (
        <span title="Session cost" style={{ opacity: 0.6, fontSize: "10px", fontFamily: "monospace" }}>
          ${sessionCost.toFixed(4)}
        </span>
      )}

      {/* Startup time indicator */}
      {startupMs != null && (
        <span style={{ position: "relative" }}>
          <button
            onClick={() => setShowStartupPopup((v) => !v)}
            style={{
              background: "transparent",
              border: "none",
              color: "#6c7086",
              fontSize: "10px",
              fontFamily: "monospace",
              cursor: "pointer",
              padding: "0 2px",
              opacity: 0.7,
            }}
            title="Startup metrics"
          >
            ⚡ {startupMs}ms
          </button>
          {showStartupPopup && (
            <div
              style={{
                position: "absolute",
                bottom: "100%",
                right: 0,
                background: "#1e1e2e",
                border: "1px solid #313244",
                borderRadius: 6,
                padding: "8px 10px",
                minWidth: 200,
                zIndex: 1000,
                boxShadow: "0 4px 20px rgba(0,0,0,0.4)",
                fontSize: 11,
                marginBottom: 4,
              }}
            >
              <div style={{ fontWeight: 700, color: "#89b4fa", marginBottom: 6 }}>Startup Marks</div>
              {startupMarks.map((mark, i) => {
                const delta = i > 0 ? mark.timestamp_ms - startupMarks[i - 1].timestamp_ms : 0;
                return (
                  <div key={mark.name} style={{ display: "flex", justifyContent: "space-between", gap: 12, marginBottom: 2 }}>
                    <span style={{ color: "#cdd6f4" }}>{mark.name}</span>
                    <span style={{ color: "#6c7086" }}>
                      {i > 0 ? `+${delta.toFixed(0)}ms` : "start"}
                    </span>
                  </div>
                );
              })}
              <div style={{ marginTop: 4, borderTop: "1px solid #313244", paddingTop: 4, display: "flex", justifyContent: "space-between" }}>
                <span style={{ color: "#a6e3a1" }}>Total</span>
                <span style={{ color: "#a6e3a1" }}>{startupMs}ms</span>
              </div>
            </div>
          )}
        </span>
      )}

      <button className="status-toggle-btn" title="Hide status bar" aria-label="Hide status bar" onClick={onHide}>
        <svg width="10" height="10" viewBox="0 0 12 12"><line x1="3" y1="3" x2="9" y2="9" stroke="currentColor" strokeWidth="1.2"/><line x1="9" y1="3" x2="3" y2="9" stroke="currentColor" strokeWidth="1.2"/></svg>
      </button>
    </div>
  );
}
