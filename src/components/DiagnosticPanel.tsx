import React, { useState, useMemo, useCallback } from "react";
import type { DiagnosticCounts, DiagnosticItem } from "./Editor";
import { useKeyboardNav } from "../hooks/useKeyboardNav";

interface DiagnosticPanelProps {
  diagnosticCounts: DiagnosticCounts;
  diagnosticItems: DiagnosticItem[];
  onClose: () => void;
  onFileOpen: (path: string, name: string) => void;
  projectRoot?: string;
}

type SeverityFilter = "error" | "warning" | "info" | "hint";

const SEVERITY_ICON: Record<string, string> = {
  error: "❌",
  warning: "⚠️",
  info: "ℹ️",
  hint: "💡",
};

const SEVERITY_ORDER: Record<string, number> = {
  error: 0,
  warning: 1,
  info: 2,
  hint: 3,
};

export function DiagnosticPanel({ diagnosticItems, onClose, onFileOpen, projectRoot }: DiagnosticPanelProps) {
  const [activeFilters, setActiveFilters] = useState<Set<SeverityFilter>>(
    new Set(["error", "warning", "info", "hint"])
  );
  const [collapsedFiles, setCollapsedFiles] = useState<Set<string>>(new Set());

  const toggleFilter = (sev: SeverityFilter) => {
    setActiveFilters((prev) => {
      const next = new Set(prev);
      if (next.has(sev)) {
        next.delete(sev);
      } else {
        next.add(sev);
      }
      return next;
    });
  };

  const toggleCollapse = (file: string) => {
    setCollapsedFiles((prev) => {
      const next = new Set(prev);
      if (next.has(file)) {
        next.delete(file);
      } else {
        next.add(file);
      }
      return next;
    });
  };

  // Filter and sort diagnostics
  const filtered = useMemo(() => {
    return [...diagnosticItems]
      .filter((item) => activeFilters.has(item.severity as SeverityFilter))
      .sort((a, b) => {
        const diff = (SEVERITY_ORDER[a.severity] ?? 3) - (SEVERITY_ORDER[b.severity] ?? 3);
        if (diff !== 0) return diff;
        return a.file.localeCompare(b.file);
      });
  }, [diagnosticItems, activeFilters]);

  // Group by file
  const byFile = useMemo(() => {
    const map = new Map<string, DiagnosticItem[]>();
    for (const item of filtered) {
      const key = item.file;
      const arr = map.get(key) ?? [];
      arr.push(item);
      map.set(key, arr);
    }
    return map;
  }, [filtered]);

  const relPath = (filePath: string) => {
    if (projectRoot && filePath.startsWith(projectRoot)) {
      return filePath.slice(projectRoot.length).replace(/^\//, "");
    }
    return filePath.split("/").slice(-2).join("/");
  };

  const filterBtnStyle = (active: boolean, sev: SeverityFilter): React.CSSProperties => {
    const colorMap: Record<SeverityFilter, string> = {
      error: "#ef4444",
      warning: "#eab308",
      info: "#818cf8",
      hint: "#a3e635",
    };
    return {
      fontSize: 11,
      padding: "2px 8px",
      borderRadius: 4,
      border: `1px solid ${active ? colorMap[sev] : "var(--border-color, #313244)"}`,
      background: active ? `${colorMap[sev]}22` : "transparent",
      color: active ? colorMap[sev] : "var(--text-muted, #6c7086)",
      cursor: "pointer",
    };
  };

  const errorCount = diagnosticItems.filter((d) => d.severity === "error").length;
  const warnCount = diagnosticItems.filter((d) => d.severity === "warning").length;

  const handleSelectDiagnostic = useCallback((item: DiagnosticItem) => {
    const fileName = item.file.split("/").pop() || item.file;
    onFileOpen(item.file, fileName);
    window.dispatchEvent(new CustomEvent("editor-go-to", {
      detail: { file: item.file, line: item.line, column: item.column },
    }));
  }, [onFileOpen]);

  const { focusedIndex, setFocusedIndex, getItemProps, containerProps } = useKeyboardNav(
    filtered,
    handleSelectDiagnostic,
  );

  return (
    <div className="error-panel">
      <div className="error-panel-header">
        <span className="error-panel-title">PROBLEMS</span>
        <span className="error-panel-count" style={{ fontSize: 11 }}>
          {errorCount} errors, {warnCount} warnings
        </span>
        <button className="error-panel-close" onClick={onClose}>
          <svg width="12" height="12" viewBox="0 0 12 12"><line x1="3" y1="3" x2="9" y2="9" stroke="currentColor" strokeWidth="1.2"/><line x1="9" y1="3" x2="3" y2="9" stroke="currentColor" strokeWidth="1.2"/></svg>
        </button>
      </div>

      {/* Filter bar */}
      <div style={{ display: "flex", gap: 4, padding: "4px 8px", borderBottom: "1px solid var(--border-color, #313244)" }}>
        {(["error", "warning", "info", "hint"] as SeverityFilter[]).map((sev) => (
          <button
            key={sev}
            aria-pressed={activeFilters.has(sev)}
            aria-label={`Filter by ${sev}`}
            style={filterBtnStyle(activeFilters.has(sev), sev)}
            onClick={() => toggleFilter(sev)}
          >
            {SEVERITY_ICON[sev]} {sev}
          </button>
        ))}
      </div>

      <div {...containerProps} className="error-panel-list">
        {byFile.size === 0 && (
          <div className="error-panel-empty">No problems detected</div>
        )}

        {(() => {
          let flatIdx = 0;
          return Array.from(byFile.entries()).map(([file, items]) => {
          const isCollapsed = collapsedFiles.has(file);
          const errorCount = items.filter((i) => i.severity === "error").length;
          const warnCount = items.filter((i) => i.severity === "warning").length;
          return (
            <div key={file} role="listitem">
              {/* File header */}
              <div
                aria-expanded={!isCollapsed}
                onClick={() => toggleCollapse(file)}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 6,
                  padding: "3px 8px",
                  background: "var(--bg-surface, #181825)",
                  cursor: "pointer",
                  userSelect: "none",
                  borderBottom: "1px solid var(--border-color, #313244)",
                  fontSize: 11,
                  color: "var(--text-secondary, #bac2de)",
                }}
              >
                <span style={{ fontSize: 9, color: "var(--text-muted)" }}>{isCollapsed ? "▶" : "▼"}</span>
                <span style={{ fontWeight: 600, flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                  {relPath(file)}
                </span>
                {errorCount > 0 && (
                  <span style={{ background: "#ef444433", color: "#ef4444", borderRadius: 4, padding: "0 4px", fontSize: 10 }}>
                    {errorCount}
                  </span>
                )}
                {warnCount > 0 && (
                  <span style={{ background: "#eab30833", color: "#eab308", borderRadius: 4, padding: "0 4px", fontSize: 10 }}>
                    {warnCount}
                  </span>
                )}
              </div>

              {!isCollapsed && items.map((item, i) => {
                const currentIdx = flatIdx++;
                const itemProps = getItemProps(currentIdx);
                return (
                <div
                  key={`${item.file}-${item.line}-${i}`}
                  role="option"
                  aria-selected={focusedIndex === currentIdx}
                  className={`error-panel-item error-panel-${item.severity}`}
                  onClick={() => {
                    setFocusedIndex(currentIdx);
                    handleSelectDiagnostic(item);
                  }}
                  onMouseEnter={() => setFocusedIndex(currentIdx)}
                  tabIndex={itemProps.tabIndex}
                  onKeyDown={itemProps.onKeyDown}
                  style={{ paddingLeft: 20, outline: focusedIndex === currentIdx ? "1px solid var(--theme-accent, #89b4fa)" : "none" }}
                >
                  <span className="error-panel-icon" style={{ fontSize: 13 }}>
                    {SEVERITY_ICON[item.severity] ?? "ℹ️"}
                  </span>
                  <div className="error-panel-content">
                    <div className="error-panel-message">{item.message}</div>
                    <div className="error-panel-location">
                      Ln {item.line}, Col {item.column}
                    </div>
                  </div>
                </div>
                );
              })}
            </div>
          );
        });
        })()}
      </div>
    </div>
  );
}
