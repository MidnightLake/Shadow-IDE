import React, { useEffect, useRef, useState, useCallback, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";

interface QuickOpenPanelProps {
  isOpen: boolean;
  onClose: () => void;
  onFileSelect: (path: string) => void;
  projectPath: string;
}

interface FileMatch {
  path: string;
  fileName: string;
  dirName: string;
  score: number;
  /** Character indices in fileName that matched the query */
  matchedIndices: number[];
}

const RECENT_FILES_KEY = "shadowide-recent-files";
const MAX_RECENT = 10;
const MAX_RESULTS = 50;

function loadRecentFiles(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_FILES_KEY);
    if (raw) return JSON.parse(raw) as string[];
  } catch { /* ignore */ }
  return [];
}

function saveRecentFiles(files: string[]): void {
  try {
    localStorage.setItem(RECENT_FILES_KEY, JSON.stringify(files.slice(0, MAX_RECENT)));
  } catch { /* ignore */ }
}

/** Fuzzy match: query characters must appear in order (not necessarily adjacent) in target. */
function fuzzyMatch(query: string, target: string): { score: number; indices: number[] } | null {
  const lq = query.toLowerCase();
  const lt = target.toLowerCase();
  const indices: number[] = [];
  let qi = 0;
  let score = 0;
  let lastMatchIdx = -1;

  for (let ti = 0; ti < lt.length && qi < lq.length; ti++) {
    if (lt[ti] === lq[qi]) {
      indices.push(ti);
      // Consecutive bonus
      if (ti === lastMatchIdx + 1) score += 2;
      // Word boundary bonus
      if (ti === 0 || target[ti - 1] === "/" || target[ti - 1] === "-" || target[ti - 1] === "_" || target[ti - 1] === ".") score += 3;
      score += 1;
      lastMatchIdx = ti;
      qi++;
    }
  }

  if (qi < lq.length) return null; // not all query chars matched
  return { score, indices };
}

function highlightText(text: string, indices: number[]): React.ReactNode {
  const set = new Set(indices);
  return text.split("").map((char, i) =>
    set.has(i) ? (
      <span key={i} style={{ color: "#f9c74f", fontWeight: 700 }}>{char}</span>
    ) : (
      <span key={i}>{char}</span>
    )
  );
}

export default function QuickOpenPanel({
  isOpen,
  onClose,
  onFileSelect,
  projectPath,
}: QuickOpenPanelProps) {
  const [query, setQuery] = useState("");
  const [allFiles, setAllFiles] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const recentFiles = useMemo(() => loadRecentFiles(), []);

  // Load project files when panel opens
  useEffect(() => {
    if (!isOpen) return;
    setQuery("");
    setActiveIndex(0);
    setLoading(true);

    invoke<string[]>("list_project_files", { project_path: projectPath })
      .then((files) => setAllFiles(files))
      .catch(() => setAllFiles([]))
      .finally(() => setLoading(false));

    setTimeout(() => inputRef.current?.focus(), 30);
  }, [isOpen, projectPath]);

  // Filter & rank results
  const results: FileMatch[] = useMemo(() => {
    if (!query.trim()) {
      // Show recent files before all files
      const recentMatches = recentFiles
        .filter((p) => allFiles.includes(p) || recentFiles.includes(p))
        .slice(0, MAX_RECENT)
        .map((p) => {
          const parts = p.split("/");
          return {
            path: p,
            fileName: parts[parts.length - 1] ?? p,
            dirName: parts.slice(0, -1).join("/"),
            score: 1000,
            matchedIndices: [],
          };
        });
      return recentMatches;
    }

    const matches: FileMatch[] = [];
    for (const filePath of allFiles) {
      const parts = filePath.split("/");
      const fileName = parts[parts.length - 1] ?? filePath;
      const dirName = parts.slice(0, -1).join("/");
      const result = fuzzyMatch(query, fileName);
      if (result) {
        matches.push({
          path: filePath,
          fileName,
          dirName,
          score: result.score,
          matchedIndices: result.indices,
        });
      }
    }

    matches.sort((a, b) => b.score - a.score);
    return matches.slice(0, MAX_RESULTS);
  }, [query, allFiles, recentFiles]);

  // Keep activeIndex in bounds
  useEffect(() => {
    setActiveIndex(0);
  }, [query]);

  // Scroll active item into view
  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const item = list.children[activeIndex] as HTMLElement | undefined;
    item?.scrollIntoView({ block: "nearest" });
  }, [activeIndex]);

  const handleSelect = useCallback(
    (path: string) => {
      const recent = loadRecentFiles().filter((p) => p !== path);
      saveRecentFiles([path, ...recent]);
      onFileSelect(path);
      onClose();
    },
    [onFileSelect, onClose]
  );

  // Keyboard navigation
  useEffect(() => {
    if (!isOpen) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") { e.preventDefault(); onClose(); return; }
      if (e.key === "ArrowDown") { e.preventDefault(); setActiveIndex((prev) => Math.min(prev + 1, results.length - 1)); return; }
      if (e.key === "ArrowUp") { e.preventDefault(); setActiveIndex((prev) => Math.max(prev - 1, 0)); return; }
      if (e.key === "Enter") {
        e.preventDefault();
        const item = results[activeIndex];
        if (item) handleSelect(item.path);
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, results, activeIndex, handleSelect, onClose]);

  if (!isOpen) return null;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Quick Open"
      style={{
        position: "fixed",
        inset: 0,
        zIndex: 9000,
        display: "flex",
        alignItems: "flex-start",
        justifyContent: "center",
        paddingTop: "15vh",
        background: "rgba(0,0,0,0.5)",
      }}
      onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div
        style={{
          width: 560,
          maxWidth: "90vw",
          background: "#1e1e2e",
          border: "1px solid #313244",
          borderRadius: 10,
          boxShadow: "0 16px 48px rgba(0,0,0,0.6)",
          overflow: "hidden",
          fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
          color: "#cdd6f4",
        }}
      >
        {/* Search input */}
        <div style={{ padding: "10px 14px", borderBottom: "1px solid #313244", display: "flex", alignItems: "center", gap: 8 }}>
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="#6c7086" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="11" cy="11" r="8"/><path d="M21 21l-4.35-4.35"/>
          </svg>
          <input
            ref={inputRef}
            type="text"
            placeholder="Go to file…"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.stopPropagation()}
            aria-label="Quick open file"
            aria-autocomplete="list"
            style={{
              flex: 1,
              background: "transparent",
              border: "none",
              outline: "none",
              color: "#cdd6f4",
              fontSize: 14,
              fontFamily: "inherit",
            }}
          />
          {loading && (
            <span style={{ fontSize: 11, color: "#6c7086" }}>Loading…</span>
          )}
        </div>

        {/* Results list */}
        <div
          ref={listRef}
          role="listbox"
          aria-label="File search results"
          style={{ maxHeight: "50vh", overflowY: "auto" }}
        >
          {!query.trim() && recentFiles.length > 0 && (
            <div style={{ padding: "6px 14px 2px", fontSize: 10, color: "#6c7086", textTransform: "uppercase", letterSpacing: "0.05em" }}>
              Recent
            </div>
          )}

          {results.length === 0 && !loading && (
            <div style={{ padding: "16px 14px", color: "#6c7086", fontSize: 13 }}>
              {query ? "No matching files" : "No recent files"}
            </div>
          )}

          {results.map((match, idx) => (
            <div
              key={match.path}
              role="option"
              aria-selected={idx === activeIndex}
              onMouseEnter={() => setActiveIndex(idx)}
              onClick={() => handleSelect(match.path)}
              style={{
                display: "flex",
                flexDirection: "column",
                padding: "6px 14px",
                cursor: "pointer",
                background: idx === activeIndex ? "#313244" : "transparent",
                transition: "background 0.08s",
              }}
            >
              <span style={{ fontSize: 13, color: "#cdd6f4" }}>
                {match.matchedIndices.length > 0
                  ? highlightText(match.fileName, match.matchedIndices)
                  : match.fileName}
              </span>
              {match.dirName && (
                <span style={{ fontSize: 11, color: "#6c7086", marginTop: 1 }}>
                  {match.dirName}
                </span>
              )}
            </div>
          ))}
        </div>

        {/* Footer hint */}
        <div style={{
          padding: "6px 14px",
          borderTop: "1px solid #313244",
          fontSize: 10,
          color: "#6c7086",
          display: "flex",
          gap: 12,
        }}>
          <span><kbd style={kbdStyle}>↑↓</kbd> navigate</span>
          <span><kbd style={kbdStyle}>↵</kbd> open</span>
          <span><kbd style={kbdStyle}>Esc</kbd> close</span>
        </div>
      </div>
    </div>
  );
}

const kbdStyle: React.CSSProperties = {
  background: "#313244",
  border: "1px solid #45475a",
  borderRadius: 3,
  padding: "1px 4px",
  fontSize: 10,
  fontFamily: "inherit",
};
