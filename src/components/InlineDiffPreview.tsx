import React, { useState, useMemo } from "react";
import { DiffEditor as MonacoDiffEditor } from "@monaco-editor/react";

interface InlineDiffPreviewProps {
  originalContent: string;
  proposedContent: string;
  filePath: string;
  onAccept: () => void;
  onReject: () => void;
  onAcceptHunk: (hunkIndex: number) => void;
}

interface DiffHunk {
  index: number;
  startLine: number;
  originalLines: string[];
  proposedLines: string[];
}

/** Parse unified-diff-style @@ markers or compute simple line-by-line hunks. */
function parseHunks(original: string, proposed: string): DiffHunk[] {
  const origLines = original.split("\n");
  const propLines = proposed.split("\n");

  // Simple hunk detection: find contiguous blocks of differing lines
  const maxLen = Math.max(origLines.length, propLines.length);
  const hunks: DiffHunk[] = [];
  let i = 0;

  while (i < maxLen) {
    const origLine = origLines[i] ?? "";
    const propLine = propLines[i] ?? "";
    if (origLine !== propLine) {
      // Collect the contiguous differing block
      const hunkStart = i;
      const origHunkLines: string[] = [];
      const propHunkLines: string[] = [];
      while (
        i < maxLen &&
        (origLines[i] !== propLines[i] ||
          (origLines[i] === undefined && propLines[i] !== undefined) ||
          (propLines[i] === undefined && origLines[i] !== undefined))
      ) {
        origHunkLines.push(origLines[i] ?? "");
        propHunkLines.push(propLines[i] ?? "");
        i++;
      }
      hunks.push({
        index: hunks.length,
        startLine: hunkStart + 1,
        originalLines: origHunkLines,
        proposedLines: propHunkLines,
      });
    } else {
      i++;
    }
  }

  return hunks;
}

export default function InlineDiffPreview({
  originalContent,
  proposedContent,
  filePath,
  onAccept,
  onReject,
  onAcceptHunk,
}: InlineDiffPreviewProps) {
  const [currentHunkIndex, setCurrentHunkIndex] = useState(0);

  const hunks = useMemo(
    () => parseHunks(originalContent, proposedContent),
    [originalContent, proposedContent]
  );

  const totalHunks = hunks.length;
  const fileName = filePath.split("/").pop() ?? filePath;

  const goPrevHunk = () => setCurrentHunkIndex((prev) => Math.max(0, prev - 1));
  const goNextHunk = () => setCurrentHunkIndex((prev) => Math.min(totalHunks - 1, prev + 1));

  const btnStyle = (color: string): React.CSSProperties => ({
    background: color,
    color: "#fff",
    border: "none",
    borderRadius: 4,
    padding: "4px 12px",
    cursor: "pointer",
    fontSize: 12,
    fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
  });

  const ghostBtnStyle: React.CSSProperties = {
    background: "transparent",
    color: "#89b4fa",
    border: "1px solid #313244",
    borderRadius: 4,
    padding: "4px 10px",
    cursor: "pointer",
    fontSize: 12,
    fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
  };

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        background: "#1e1e2e",
        color: "#cdd6f4",
        fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
      }}
    >
      {/* Toolbar */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "6px 12px",
          borderBottom: "1px solid #313244",
          flexShrink: 0,
          flexWrap: "wrap",
        }}
      >
        <span style={{ fontSize: 12, opacity: 0.8, marginRight: 4 }}>
          Diff: <strong>{fileName}</strong>
        </span>
        <span style={{ fontSize: 11, color: "#6c7086" }}>
          {totalHunks === 0 ? "No changes" : `${totalHunks} hunk${totalHunks !== 1 ? "s" : ""}`}
        </span>

        <div style={{ flex: 1 }} />

        {/* Hunk navigation */}
        {totalHunks > 0 && (
          <>
            <button style={ghostBtnStyle} onClick={goPrevHunk} disabled={currentHunkIndex === 0}
              title="Previous hunk">‹</button>
            <span style={{ fontSize: 11 }}>
              {currentHunkIndex + 1} / {totalHunks}
            </span>
            <button style={ghostBtnStyle} onClick={goNextHunk} disabled={currentHunkIndex >= totalHunks - 1}
              title="Next hunk">›</button>
            <button
              style={btnStyle("#238636")}
              onClick={() => onAcceptHunk(currentHunkIndex)}
              title={`Accept hunk ${currentHunkIndex + 1}`}
            >
              Accept hunk {currentHunkIndex + 1}/{totalHunks}
            </button>
          </>
        )}

        <button style={btnStyle("#238636")} onClick={onAccept}>Accept All</button>
        <button style={btnStyle("#da3633")} onClick={onReject}>Reject All</button>
      </div>

      {/* Hunk summary */}
      {totalHunks > 0 && (
        <div
          style={{
            padding: "4px 12px",
            borderBottom: "1px solid #313244",
            fontSize: 11,
            color: "#7f849c",
            flexShrink: 0,
          }}
        >
          Hunk {currentHunkIndex + 1} @ line {hunks[currentHunkIndex]?.startLine ?? "?"}
          {" "}·{" "}
          <span style={{ color: "#f85149" }}>-{hunks[currentHunkIndex]?.originalLines.length ?? 0}</span>
          {" / "}
          <span style={{ color: "#3fb950" }}>+{hunks[currentHunkIndex]?.proposedLines.length ?? 0}</span>
        </div>
      )}

      {/* Monaco diff editor */}
      <div style={{ flex: 1, overflow: "hidden" }}>
        <MonacoDiffEditor
          height="100%"
          original={originalContent}
          modified={proposedContent}
          theme="vs-dark"
          options={{
            renderSideBySide: true,
            automaticLayout: true,
            readOnly: true,
            fontSize: 13,
            fontFamily: "'JetBrains Mono', 'Fira Code', 'Cascadia Code', monospace",
            minimap: { enabled: false },
            scrollBeyondLastLine: false,
          }}
        />
      </div>
    </div>
  );
}
