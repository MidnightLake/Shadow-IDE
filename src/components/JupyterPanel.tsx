import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import Editor from "@monaco-editor/react";

interface NotebookOutput {
  output_type: string; // 'stream', 'display_data', 'execute_result', 'error'
  text?: string[];
  data?: Record<string, string[]>;
  traceback?: string[];
}

interface NotebookCell {
  id: string;
  cell_type: "code" | "markdown" | "raw";
  source: string[];
  outputs: NotebookOutput[];
  execution_count: number | null;
}

interface NotebookKernelspec {
  display_name?: string;
  language?: string;
  name?: string;
}

interface NotebookMetadata {
  kernelspec?: NotebookKernelspec;
  language_info?: { name?: string };
}

interface Notebook {
  nbformat: number;
  nbformat_minor: number;
  metadata: NotebookMetadata;
  cells: NotebookCell[];
}

interface JupyterPanelProps {
  filePath: string;
}

function renderMarkdownSimple(md: string): string {
  let html = md
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
  html = html.replace(/^#{6}\s+(.+)$/gm, "<h6>$1</h6>");
  html = html.replace(/^#{5}\s+(.+)$/gm, "<h5>$1</h5>");
  html = html.replace(/^#{4}\s+(.+)$/gm, "<h4>$1</h4>");
  html = html.replace(/^#{3}\s+(.+)$/gm, "<h3>$1</h3>");
  html = html.replace(/^#{2}\s+(.+)$/gm, "<h2>$1</h2>");
  html = html.replace(/^#{1}\s+(.+)$/gm, "<h1>$1</h1>");
  html = html.replace(/\*\*(.+?)\*\*/g, "<strong>$1</strong>");
  html = html.replace(/\*(.+?)\*/g, "<em>$1</em>");
  html = html.replace(/`([^`]+)`/g, "<code>$1</code>");
  html = html.replace(/^(?!<[a-z]|$)(.*\S.*)$/gm, "<p>$1</p>");
  return html;
}

function generateId(): string {
  return Math.random().toString(36).slice(2, 10);
}

export default function JupyterPanel({ filePath }: JupyterPanelProps) {
  const [notebook, setNotebook] = useState<Notebook | null>(null);
  const [editingMarkdownId, setEditingMarkdownId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [runningIds, setRunningIds] = useState<Set<string>>(new Set());
  const cellSourceRef = useRef<Map<string, string>>(new Map());

  useEffect(() => {
    invoke<string>("read_file_content", { path: filePath })
      .then((raw) => {
        try {
          const nb = JSON.parse(raw) as Notebook;
          // Normalize cell IDs
          nb.cells = nb.cells.map((c, i) => ({ ...c, id: c.id ?? generateId() + i }));
          setNotebook(nb);
          // Initialize source refs
          nb.cells.forEach((c) => {
            cellSourceRef.current.set(c.id, c.source.join(""));
          });
        } catch {
          setError("Failed to parse notebook JSON.");
        }
      })
      .catch((e) => setError(String(e)));
  }, [filePath]);

  const kernelLanguage = notebook?.metadata?.kernelspec?.language
    ?? notebook?.metadata?.language_info?.name
    ?? "python";

  const saveNotebook = useCallback(async (nb: Notebook) => {
    setSaving(true);
    try {
      await invoke("write_file_content", {
        path: filePath,
        content: JSON.stringify(nb, null, 1),
      });
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }, [filePath]);

  const updateCellSource = useCallback((id: string, source: string) => {
    cellSourceRef.current.set(id, source);
    setNotebook((prev) => {
      if (!prev) return prev;
      return {
        ...prev,
        cells: prev.cells.map((c) =>
          c.id === id ? { ...c, source: [source] } : c
        ),
      };
    });
  }, []);

  const runCell = useCallback(async (cell: NotebookCell) => {
    if (cell.cell_type !== "code") return;
    const source = cellSourceRef.current.get(cell.id) ?? cell.source.join("");
    setRunningIds((prev) => new Set(prev).add(cell.id));
    try {
      const result = await invoke<string>("shell_exec", {
        command: `python3 -c ${JSON.stringify(source)}`,
      });
      const output: NotebookOutput = {
        output_type: "stream",
        text: [String(result ?? "")],
      };
      setNotebook((prev) => {
        if (!prev) return prev;
        const nb = {
          ...prev,
          cells: prev.cells.map((c) =>
            c.id === cell.id
              ? { ...c, outputs: [output], execution_count: (c.execution_count ?? 0) + 1 }
              : c
          ),
        };
        saveNotebook(nb);
        return nb;
      });
    } catch (e) {
      const errOutput: NotebookOutput = {
        output_type: "error",
        traceback: [String(e)],
      };
      setNotebook((prev) => {
        if (!prev) return prev;
        return {
          ...prev,
          cells: prev.cells.map((c) =>
            c.id === cell.id ? { ...c, outputs: [errOutput] } : c
          ),
        };
      });
    } finally {
      setRunningIds((prev) => {
        const next = new Set(prev);
        next.delete(cell.id);
        return next;
      });
    }
  }, [saveNotebook]);

  const runAll = useCallback(async () => {
    if (!notebook) return;
    for (const cell of notebook.cells) {
      if (cell.cell_type === "code") {
        await runCell(cell);
      }
    }
  }, [notebook, runCell]);

  const addCell = useCallback(() => {
    setNotebook((prev) => {
      if (!prev) return prev;
      const newCell: NotebookCell = {
        id: generateId(),
        cell_type: "code",
        source: [""],
        outputs: [],
        execution_count: null,
      };
      const nb = { ...prev, cells: [...prev.cells, newCell] };
      saveNotebook(nb);
      return nb;
    });
  }, [saveNotebook]);

  const moveCell = useCallback((id: string, dir: "up" | "down") => {
    setNotebook((prev) => {
      if (!prev) return prev;
      const cells = [...prev.cells];
      const idx = cells.findIndex((c) => c.id === id);
      if (dir === "up" && idx > 0) {
        [cells[idx - 1], cells[idx]] = [cells[idx], cells[idx - 1]];
      } else if (dir === "down" && idx < cells.length - 1) {
        [cells[idx], cells[idx + 1]] = [cells[idx + 1], cells[idx]];
      } else {
        return prev;
      }
      const nb = { ...prev, cells };
      saveNotebook(nb);
      return nb;
    });
  }, [saveNotebook]);

  const handleSave = useCallback(() => {
    if (notebook) saveNotebook(notebook);
  }, [notebook, saveNotebook]);

  if (error) {
    return (
      <div style={{ padding: 16, color: "#f38ba8", fontFamily: "monospace" }}>
        Error: {error}
      </div>
    );
  }

  if (!notebook) {
    return (
      <div style={{ padding: 16, color: "#6c7086", fontFamily: "monospace" }}>
        Loading notebook...
      </div>
    );
  }

  return (
    <div style={{ height: "100%", overflowY: "auto", background: "#1e1e2e", color: "#cdd6f4", fontFamily: "monospace", fontSize: 13 }}>
      {/* Toolbar */}
      <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "8px 12px", borderBottom: "1px solid #313244", position: "sticky", top: 0, background: "#1e1e2e", zIndex: 10 }}>
        <span style={{ fontWeight: "bold", color: "#89b4fa", fontSize: 12 }}>
          {filePath.split("/").pop()}
        </span>
        <button
          onClick={runAll}
          style={{ background: "#a6e3a1", color: "#1e1e2e", border: "none", borderRadius: 4, padding: "3px 10px", fontSize: 11, cursor: "pointer", fontWeight: 600 }}
        >
          ▶ Run All
        </button>
        <button
          onClick={addCell}
          style={{ background: "#313244", color: "#cdd6f4", border: "1px solid #45475a", borderRadius: 4, padding: "3px 10px", fontSize: 11, cursor: "pointer" }}
        >
          + Add Cell
        </button>
        <button
          onClick={handleSave}
          disabled={saving}
          style={{ background: "#313244", color: saving ? "#6c7086" : "#f9e2af", border: "1px solid #45475a", borderRadius: 4, padding: "3px 10px", fontSize: 11, cursor: "pointer" }}
        >
          {saving ? "Saving..." : "Save"}
        </button>
        <span style={{ marginLeft: "auto", color: "#6c7086", fontSize: 11 }}>
          {kernelLanguage} · {notebook.cells.length} cells
        </span>
      </div>

      {/* Cells */}
      <div style={{ padding: "8px 12px" }}>
        {notebook.cells.map((cell, idx) => (
          <CellView
            key={cell.id}
            cell={cell}
            index={idx}
            language={kernelLanguage}
            isRunning={runningIds.has(cell.id)}
            isEditingMarkdown={editingMarkdownId === cell.id}
            onRun={() => runCell(cell)}
            onSourceChange={(src) => updateCellSource(cell.id, src)}
            onMoveUp={() => moveCell(cell.id, "up")}
            onMoveDown={() => moveCell(cell.id, "down")}
            onMarkdownEdit={() => setEditingMarkdownId(cell.id)}
            onMarkdownBlur={() => setEditingMarkdownId(null)}
            isFirst={idx === 0}
            isLast={idx === notebook.cells.length - 1}
          />
        ))}
      </div>
    </div>
  );
}

interface CellViewProps {
  cell: NotebookCell;
  index: number;
  language: string;
  isRunning: boolean;
  isEditingMarkdown: boolean;
  onRun: () => void;
  onSourceChange: (src: string) => void;
  onMoveUp: () => void;
  onMoveDown: () => void;
  onMarkdownEdit: () => void;
  onMarkdownBlur: () => void;
  isFirst: boolean;
  isLast: boolean;
}

function CellView({
  cell, index, language, isRunning,
  isEditingMarkdown,
  onRun, onSourceChange, onMoveUp, onMoveDown,
  onMarkdownEdit, onMarkdownBlur,
  isFirst, isLast,
}: CellViewProps) {
  const source = cell.source.join("");
  const lineCount = source.split("\n").length;
  const editorHeight = Math.max(60, lineCount * 19 + 16);

  return (
    <div style={{ marginBottom: 12, border: "1px solid #313244", borderRadius: 6, overflow: "hidden" }}>
      {/* Cell header */}
      <div style={{ display: "flex", alignItems: "center", gap: 4, padding: "4px 8px", background: "#181825", borderBottom: "1px solid #313244" }}>
        <span style={{ color: "#6c7086", fontSize: 10, minWidth: 24 }}>
          {cell.cell_type === "code" ? `[${cell.execution_count ?? " "}]` : cell.cell_type.toUpperCase()}
        </span>
        {cell.cell_type === "code" && (
          <button
            onClick={onRun}
            disabled={isRunning}
            title="Run Cell"
            style={{ background: "transparent", border: "none", color: isRunning ? "#6c7086" : "#a6e3a1", cursor: "pointer", fontSize: 12, padding: "0 4px" }}
          >
            {isRunning ? "⏳" : "▶"}
          </button>
        )}
        <span style={{ flex: 1 }} />
        <span style={{ fontSize: 10, color: "#45475a" }}>{index + 1}</span>
        <button onClick={onMoveUp} disabled={isFirst} title="Move Up" style={{ background: "transparent", border: "none", color: isFirst ? "#45475a" : "#a6adc8", cursor: "pointer", fontSize: 11 }}>▲</button>
        <button onClick={onMoveDown} disabled={isLast} title="Move Down" style={{ background: "transparent", border: "none", color: isLast ? "#45475a" : "#a6adc8", cursor: "pointer", fontSize: 11 }}>▼</button>
      </div>

      {/* Cell body */}
      {cell.cell_type === "code" && (
        <div style={{ background: "#181825" }}>
          <Editor
            height={`${editorHeight}px`}
            language={language}
            value={source}
            theme="vs-dark"
            onChange={(v) => onSourceChange(v ?? "")}
            options={{
              minimap: { enabled: false },
              scrollBeyondLastLine: false,
              lineNumbers: "on",
              fontSize: 13,
              wordWrap: "on",
              folding: false,
              glyphMargin: false,
              lineDecorationsWidth: 4,
              renderLineHighlight: "none",
              overviewRulerLanes: 0,
              hideCursorInOverviewRuler: true,
              scrollbar: { vertical: "hidden", horizontal: "hidden" },
            }}
          />
        </div>
      )}

      {cell.cell_type === "markdown" && (
        <div
          style={{ background: "#1e1e2e", padding: "8px 12px", cursor: "pointer", minHeight: 40 }}
          onDoubleClick={onMarkdownEdit}
          title="Double-click to edit"
        >
          {isEditingMarkdown ? (
            <textarea
              autoFocus
              defaultValue={source}
              onBlur={(e) => { onSourceChange(e.target.value); onMarkdownBlur(); }}
              style={{
                width: "100%", minHeight: 80, background: "#181825", color: "#cdd6f4",
                border: "1px solid #45475a", borderRadius: 4, padding: 8, fontFamily: "monospace",
                fontSize: 13, resize: "vertical", outline: "none",
              }}
            />
          ) : (
            <div
              style={{ color: "#cdd6f4", lineHeight: 1.6 }}
              dangerouslySetInnerHTML={{ __html: renderMarkdownSimple(source) }}
            />
          )}
        </div>
      )}

      {cell.cell_type === "raw" && (
        <div style={{ background: "#181825", padding: "8px 12px", color: "#a6adc8", fontFamily: "monospace", fontSize: 12, whiteSpace: "pre-wrap" }}>
          {source}
        </div>
      )}

      {/* Outputs */}
      {cell.outputs.length > 0 && (
        <div style={{ borderTop: "1px solid #313244" }}>
          {cell.outputs.map((out, i) => (
            <OutputView key={i} output={out} />
          ))}
        </div>
      )}
    </div>
  );
}

function OutputView({ output }: { output: NotebookOutput }) {
  if (output.output_type === "error") {
    return (
      <div style={{ background: "#3b0a0a", padding: "8px 12px", color: "#f38ba8", fontFamily: "monospace", fontSize: 12, whiteSpace: "pre-wrap" }}>
        {output.traceback?.join("\n") ?? "Error"}
      </div>
    );
  }

  if (output.output_type === "display_data" || output.output_type === "execute_result") {
    const pngData = output.data?.["image/png"];
    if (pngData) {
      const src = `data:image/png;base64,${Array.isArray(pngData) ? pngData.join("") : pngData}`;
      return (
        <div style={{ padding: "8px 12px", background: "#181825" }}>
          <img src={src} alt="output" style={{ maxWidth: "100%", borderRadius: 4 }} />
        </div>
      );
    }
    const textData = output.data?.["text/plain"];
    if (textData) {
      return (
        <div style={{ padding: "8px 12px", background: "#181825", color: "#a6e3a1", fontFamily: "monospace", fontSize: 12, whiteSpace: "pre-wrap" }}>
          {Array.isArray(textData) ? textData.join("") : textData}
        </div>
      );
    }
  }

  if (output.output_type === "stream") {
    return (
      <div style={{ padding: "8px 12px", background: "#181825", color: "#a6e3a1", fontFamily: "monospace", fontSize: 12, whiteSpace: "pre-wrap" }}>
        {output.text?.join("") ?? ""}
      </div>
    );
  }

  return null;
}
