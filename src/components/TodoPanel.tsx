import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

interface TodoItem {
  file: string;
  line: number;
  marker: string;
  text: string;
  priority: string;
}

interface TodoPanelProps {
  visible: boolean;
  rootPath: string;
  onFileOpen: (path: string, name: string) => void;
}

const MARKER_COLORS: Record<string, string> = {
  BUG: "#ff6b6b",
  FIXME: "#ff6b6b",
  HACK: "#ffd43b",
  TODO: "#5c7cfa",
  XXX: "#ffd43b",
  WARN: "#ff922b",
  NOTE: "#51cf66",
};

export default function TodoPanel({
  visible,
  rootPath,
  onFileOpen,
}: TodoPanelProps) {
  const [items, setItems] = useState<TodoItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [filter, setFilter] = useState<string>("all");
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  const scan = useCallback(async () => {
    if (!rootPath) return;
    setLoading(true);
    try {
      const results = await invoke<TodoItem[]>("scan_todos", {
        path: rootPath,
      });
      setItems(results ?? []);
    } catch (err) {
      console.error("Failed to scan TODOs:", err);
    }
    setLoading(false);
  }, [rootPath]);

  useEffect(() => {
    if (visible && rootPath) {
      scan();
    }
  }, [visible, rootPath, scan]);

  const filtered =
    filter === "all" ? items : items.filter((i) => i.marker === filter);

  // Group by file
  const grouped = new Map<string, TodoItem[]>();
  for (const item of filtered) {
    const list = grouped.get(item.file) || [];
    list.push(item);
    grouped.set(item.file, list);
  }

  const toggleFile = (file: string) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(file)) {
        next.delete(file);
      } else {
        next.add(file);
      }
      return next;
    });
  };

  const getFileName = (path: string) => {
    const parts = path.split("/");
    return parts[parts.length - 1];
  };

  const getRelativePath = (path: string) => {
    if (rootPath && path.startsWith(rootPath)) {
      return path.slice(rootPath.length + 1);
    }
    return path;
  };

  // Count by marker type
  const counts = new Map<string, number>();
  for (const item of items) {
    counts.set(item.marker, (counts.get(item.marker) || 0) + 1);
  }

  if (!visible) return null;

  return (
    <div className="todo-panel">
      <div className="todo-header">
        <span className="todo-title">DIAGNOSTICS</span>
        <div className="todo-controls">
          <span className="todo-count">{items.length}</span>
          <button
            className="todo-btn"
            onClick={scan}
            title="Refresh"
            disabled={loading}
          >
            {loading ? "..." : "\u27F3"}
          </button>
        </div>
      </div>

      <div className="todo-filters">
        <button
          className={`todo-filter-btn ${filter === "all" ? "active" : ""}`}
          onClick={() => setFilter("all")}
        >
          All ({items.length})
        </button>
        {Array.from(counts.entries()).map(([marker, count]) => (
          <button
            key={marker}
            className={`todo-filter-btn ${filter === marker ? "active" : ""}`}
            onClick={() => setFilter(marker)}
            style={{
              borderColor:
                filter === marker ? MARKER_COLORS[marker] : undefined,
            }}
          >
            {marker} ({count})
          </button>
        ))}
      </div>

      <div className="todo-list">
        {filtered.length === 0 && !loading && (
          <div className="todo-empty">
            {items.length === 0
              ? "No markers found. Click refresh to scan."
              : "No matches for this filter."}
          </div>
        )}
        {loading && filtered.length === 0 && (
          <div className="todo-empty">Scanning...</div>
        )}
        {Array.from(grouped.entries()).map(([file, fileItems]) => (
          <div key={file} className="todo-file-group">
            <div className="todo-file-header" onClick={() => toggleFile(file)}>
              <span className="todo-file-arrow">
                {collapsed.has(file) ? "\u25B6" : "\u25BC"}
              </span>
              <span className="todo-file-name" title={file}>
                {getRelativePath(file)}
              </span>
              <span className="todo-file-count">{fileItems.length}</span>
            </div>
            {!collapsed.has(file) &&
              fileItems.map((item, idx) => (
                <div
                  key={`${file}-${idx}`}
                  className="todo-item"
                  onClick={() => onFileOpen(item.file, getFileName(item.file))}
                  title={`${item.file}:${item.line}`}
                >
                  <span
                    className="todo-marker"
                    style={{
                      color: MARKER_COLORS[item.marker] || "#8b949e",
                    }}
                  >
                    {item.marker}
                  </span>
                  <span className="todo-line">:{item.line}</span>
                  <span className="todo-text">{item.text}</span>
                </div>
              ))}
          </div>
        ))}
      </div>
    </div>
  );
}
