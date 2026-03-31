import { useState, useEffect, useCallback, useRef, useMemo, type ReactNode, memo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import { useKeyboardNav } from "../hooks/useKeyboardNav";

interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
  is_symlink: boolean;
  size: number;
  extension: string | null;
}

interface FileExplorerProps {
  onFileOpen: (path: string, name: string) => void;
  rootPath: string;
  onRootPathChange: (path: string) => void;
}

interface FlatTreeNode {
  entry: FileEntry;
  depth: number;
  expanded: boolean;
  hasChildren: boolean;
}

const ROW_HEIGHT = 28;

interface FileRowProps {
  node: FlatTreeNode;
  index: number;
  startIndex: number;
  onClick: (entry: FileEntry) => void;
  dragSourcePath: string | null;
  dropTargetPath: string | null;
  loading: boolean;
  onDragStart: (e: React.DragEvent, path: string) => void;
  onDragOver: (e: React.DragEvent, entry: FileEntry) => void;
  onDragLeave: () => void;
  onDrop: (e: React.DragEvent, targetDir: FileEntry) => void;
  onDragEnd: () => void;
  onContextMenu: (e: React.MouseEvent, entry: FileEntry) => void;
  onTouchStart: (e: React.TouchEvent, entry: FileEntry) => void;
  onTouchEnd: () => void;
  renaming: { path: string; name: string } | null;
  onFinishRename: (newName: string) => void;
}

const FileRow = memo(({
  node,
  index,
  startIndex,
  onClick,
  dragSourcePath,
  dropTargetPath,
  loading,
  onDragStart,
  onDragOver,
  onDragLeave,
  onDrop,
  onDragEnd,
  onContextMenu,
  onTouchStart,
  onTouchEnd,
  renaming,
  onFinishRename,
}: FileRowProps) => {
  const isRenaming = renaming?.path === node.entry.path;
  const ext = node.entry.extension?.toLowerCase();
  const isTscn = ext === "tscn" || ext === "scn";

  // Build tooltip: for .tscn show scene info, for others use path
  const buildTooltip = (): string => {
    if (isTscn) {
      return `${node.entry.name} — Godot Scene`;
    }
    return node.entry.path;
  };

  return (
    <div
      className={`tree-node-label${dragSourcePath === node.entry.path ? " dragging" : ""}${dropTargetPath === node.entry.path ? " drop-target" : ""}`}
      style={{
        position: "absolute",
        top: `${(startIndex + index) * ROW_HEIGHT}px`,
        left: 0,
        right: 0,
        height: `${ROW_HEIGHT}px`,
        paddingLeft: `${node.depth * 16 + 8}px`,
      }}
      onClick={() => { if (!isRenaming) onClick(node.entry); }}
      title={buildTooltip()}
      draggable={!isRenaming}
      onDragStart={(e) => onDragStart(e, node.entry.path)}
      onDragOver={(e) => onDragOver(e, node.entry)}
      onDragLeave={onDragLeave}
      onDrop={(e) => onDrop(e, node.entry)}
      onDragEnd={onDragEnd}
      onContextMenu={(e) => onContextMenu(e, node.entry)}
      onTouchStart={(e) => onTouchStart(e, node.entry)}
      onTouchEnd={onTouchEnd}
      onTouchMove={onTouchEnd}
    >
      <span className="tree-icon">
        {getFileIcon(node.entry, node.expanded)}
      </span>
      {isRenaming ? (
        <input
          className="tree-rename-input"
          defaultValue={renaming.name}
          autoFocus
          onBlur={(e) => onFinishRename(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") onFinishRename((e.target as HTMLInputElement).value);
            if (e.key === "Escape") onFinishRename(renaming.name);
          }}
          onClick={(e) => e.stopPropagation()}
          style={{
            flex: 1, background: "var(--bg-primary)", border: "1px solid var(--accent)",
            borderRadius: 3, color: "var(--text-primary)", padding: "1px 4px",
            fontSize: 12, outline: "none", minWidth: 0,
          }}
        />
      ) : (
        <span className="tree-name">{node.entry.name}</span>
      )}
      {loading && (
        <span className="tree-loading">...</span>
      )}
    </div>
  );
});

function getFileIcon(entry: FileEntry, expanded: boolean): ReactNode {
  const s = { width: 16, height: 16, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: 1.8, strokeLinecap: "round" as const, strokeLinejoin: "round" as const };

  if (entry.is_dir) {
    return expanded ? (
      <svg {...s} style={{ color: "#818cf8" }}><path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" fill="rgba(129,140,248,0.15)" /><line x1="2" y1="10" x2="22" y2="10" /></svg>
    ) : (
      <svg {...s} style={{ color: "#818cf8" }}><path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" /></svg>
    );
  }

  const ext = entry.extension?.toLowerCase();
  switch (ext) {
    case "rs":
      return <svg {...s} style={{ color: "#f97316" }}><path d="M12 2L2 7l10 5 10-5-10-5z" /><path d="M2 17l10 5 10-5" /><path d="M2 12l10 5 10-5" /></svg>;
    case "ts":
    case "tsx":
      return <svg {...s} style={{ color: "#3b82f6" }}><rect x="3" y="3" width="18" height="18" rx="2" fill="rgba(59,130,246,0.12)" /><text x="12" y="16" textAnchor="middle" fill="#3b82f6" stroke="none" fontSize="10" fontWeight="bold" fontFamily="monospace">TS</text></svg>;
    case "js":
    case "jsx":
      return <svg {...s} style={{ color: "#eab308" }}><rect x="3" y="3" width="18" height="18" rx="2" fill="rgba(234,179,8,0.12)" /><text x="12" y="16" textAnchor="middle" fill="#eab308" stroke="none" fontSize="10" fontWeight="bold" fontFamily="monospace">JS</text></svg>;
    case "json":
      return <svg {...s} style={{ color: "#a3a3a3" }}><path d="M8 3H7a2 2 0 00-2 2v5a2 2 0 01-2 2 2 2 0 012 2v5a2 2 0 002 2h1" /><path d="M16 3h1a2 2 0 012 2v5a2 2 0 002 2 2 2 0 00-2 2v5a2 2 0 01-2 2h-1" /></svg>;
    case "md":
    case "mdx":
      return <svg {...s} style={{ color: "#94a3b8" }}><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" /><polyline points="14 2 14 8 20 8" /><line x1="8" y1="13" x2="16" y2="13" /><line x1="8" y1="17" x2="13" y2="17" /></svg>;
    case "css":
    case "scss":
    case "less":
      return <svg {...s} style={{ color: "#a78bfa" }}><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" fill="rgba(167,139,250,0.08)" /><text x="12" y="17" textAnchor="middle" fill="#a78bfa" stroke="none" fontSize="8" fontWeight="bold" fontFamily="monospace">CSS</text></svg>;
    case "html":
    case "htm":
      return <svg {...s} style={{ color: "#f97316" }}><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" fill="rgba(249,115,22,0.08)" /><text x="12" y="17" textAnchor="middle" fill="#f97316" stroke="none" fontSize="7" fontWeight="bold" fontFamily="monospace">HTML</text></svg>;
    case "py":
      return <svg {...s} style={{ color: "#22c55e" }}><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" fill="rgba(34,197,94,0.08)" /><text x="12" y="17" textAnchor="middle" fill="#22c55e" stroke="none" fontSize="8" fontWeight="bold" fontFamily="monospace">PY</text></svg>;
    case "go":
      return <svg {...s} style={{ color: "#06b6d4" }}><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" fill="rgba(6,182,212,0.08)" /><text x="12" y="17" textAnchor="middle" fill="#06b6d4" stroke="none" fontSize="8" fontWeight="bold" fontFamily="monospace">GO</text></svg>;
    case "c":
    case "cpp":
    case "h":
    case "hpp":
      return <svg {...s} style={{ color: "#60a5fa" }}><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" fill="rgba(96,165,250,0.08)" /><text x="12" y="17" textAnchor="middle" fill="#60a5fa" stroke="none" fontSize="9" fontWeight="bold" fontFamily="monospace">C</text></svg>;
    case "java":
    case "kt":
      return <svg {...s} style={{ color: "#ef4444" }}><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" fill="rgba(239,68,68,0.08)" /><polyline points="14 2 14 8 20 8" /></svg>;
    case "toml":
    case "yaml":
    case "yml":
    case "ini":
    case "env":
      return <svg {...s} style={{ color: "#a3a3a3" }}><circle cx="12" cy="12" r="3" /><path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 01-2.83 2.83l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z" /></svg>;
    case "sh":
    case "bash":
    case "zsh":
    case "fish":
      return <svg {...s} style={{ color: "#22c55e" }}><polyline points="4 17 10 11 4 5" /><line x1="12" y1="19" x2="20" y2="19" /></svg>;
    case "lock":
      return <svg {...s} style={{ color: "#475569" }}><rect x="3" y="11" width="18" height="11" rx="2" ry="2" /><path d="M7 11V7a5 5 0 0110 0v4" /></svg>;
    case "svg":
      return <svg {...s} style={{ color: "#f59e0b" }}><rect x="3" y="3" width="18" height="18" rx="2" /><circle cx="8.5" cy="8.5" r="1.5" fill="#f59e0b" /><polyline points="21 15 16 10 5 21" /></svg>;
    case "png":
    case "jpg":
    case "jpeg":
    case "gif":
    case "webp":
    case "ico":
      return <svg {...s} style={{ color: "#a78bfa" }}><rect x="3" y="3" width="18" height="18" rx="2" /><circle cx="8.5" cy="8.5" r="1.5" fill="#a78bfa" /><polyline points="21 15 16 10 5 21" /></svg>;
    case "wasm":
      return <svg {...s} style={{ color: "#8b5cf6" }}><path d="M21 16V8a2 2 0 00-1-1.73l-7-4a2 2 0 00-2 0l-7 4A2 2 0 003 8v8a2 2 0 001 1.73l7 4a2 2 0 002 0l7-4A2 2 0 0021 16z" /></svg>;
    case "swift":
      return <svg {...s} style={{ color: "#f97316" }}><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" fill="rgba(249,115,22,0.08)" /><polyline points="14 2 14 8 20 8" /></svg>;
    case "rb":
      return <svg {...s} style={{ color: "#ef4444" }}><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" fill="rgba(239,68,68,0.08)" /><polyline points="14 2 14 8 20 8" /></svg>;
    case "php":
      return <svg {...s} style={{ color: "#8b5cf6" }}><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" fill="rgba(139,92,246,0.08)" /><polyline points="14 2 14 8 20 8" /></svg>;
    case "vue":
      return <svg {...s} style={{ color: "#22c55e" }}><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" fill="rgba(34,197,94,0.08)" /><polyline points="14 2 14 8 20 8" /></svg>;
    case "dart":
      return <svg {...s} style={{ color: "#06b6d4" }}><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" fill="rgba(6,182,212,0.08)" /><polyline points="14 2 14 8 20 8" /></svg>;
    case "lua":
      return <svg {...s} style={{ color: "#3b82f6" }}><circle cx="12" cy="12" r="10" /><path d="M12 16v-4" /><path d="M12 8h.01" /></svg>;
    case "zig":
      return <svg {...s} style={{ color: "#f59e0b" }}><path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" /></svg>;
    case "sql":
      return <svg {...s} style={{ color: "#a3a3a3" }}><ellipse cx="12" cy="5" rx="9" ry="3" /><path d="M21 12c0 1.66-4 3-9 3s-9-1.34-9-3" /><path d="M3 5v14c0 1.66 4 3 9 3s9-1.34 9-3V5" /></svg>;
    case "gitignore":
    case "dockerignore":
      return <svg {...s} style={{ color: "#475569" }}><circle cx="12" cy="12" r="10" /><line x1="4.93" y1="4.93" x2="19.07" y2="19.07" /></svg>;
    case "tscn":
    case "scn":
      return <span title="Godot Scene" style={{ fontSize: 14, lineHeight: 1 }}>🎬</span>;
    case "glsl":
    case "frag":
    case "vert":
    case "gdshader":
      return <span title="Shader" style={{ fontSize: 14, lineHeight: 1 }}>🔷</span>;
    default:
      return <svg {...s} style={{ color: "#64748b" }}><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" /><polyline points="14 2 14 8 20 8" /></svg>;
  }
}

function FileExplorer({
  onFileOpen,
  rootPath,
  onRootPathChange,
}: FileExplorerProps) {
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [expandedPaths, setExpandedPaths] = useState<Set<string>>(new Set());
  const [childrenMap, setChildrenMap] = useState<Map<string, FileEntry[]>>(
    new Map()
  );
  const [loadingPaths, setLoadingPaths] = useState<Set<string>>(new Set());
  const [scrollTop, setScrollTop] = useState(0);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [showHidden, setShowHidden] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [searchResults, setSearchResults] = useState<FileEntry[]>([]);
  const [searchMode, setSearchMode] = useState(false);
  const [dragSourcePath, setDragSourcePath] = useState<string | null>(null);
  const [dropTargetPath, setDropTargetPath] = useState<string | null>(null);
  const [ctxMenu, setCtxMenu] = useState<{ x: number; y: number; entry: FileEntry } | null>(null);
  const [renaming, setRenaming] = useState<{ path: string; name: string } | null>(null);
  const [inlineCreate, setInlineCreate] = useState<{ dir: string; isFolder: boolean } | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<FileEntry | null>(null);
  const [trashedFile, setTrashedFile] = useState<{path: string, content: string, isDir: boolean} | null>(null);
  const undoTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const longPressTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const searchDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const dirCacheTimestamps = useRef<Map<string, number>>(new Map());
  const DIR_CACHE_TTL = 30000; // 30 seconds

  const loadDirectory = useCallback(async (path: string) => {
    try {
      setError(null);
      const result = await invoke<FileEntry[]>("read_directory", { path, showHidden });
      setEntries(result ?? []);
      setExpandedPaths(new Set());
      setChildrenMap(new Map());
    } catch (err) {
      setError(String(err));
      setEntries([]);
    }
  }, [showHidden]);

  useEffect(() => {
    if (rootPath) {
      loadDirectory(rootPath);
    }
  }, [rootPath, loadDirectory]);

  // Start file watcher and listen for workspace changes
  useEffect(() => {
    if (!rootPath) return;

    // Start watching the workspace
    invoke("watch_workspace", { rootPath }).catch(() => {});

    let debounceTimer: ReturnType<typeof setTimeout> | null = null;
    const pendingDirs = new Set<string>();

    const unlisten = listen<{ kind: string; paths: string[]; dir: string }>(
      "workspace-fs-changed",
      (e) => {
        const { kind, dir } = e.payload;
        // Collect changed directories, debounce to avoid flooding
        pendingDirs.add(dir);
        if (kind === "create" || kind === "remove") {
          // Also refresh parent of parent for tree structure changes
          const parent = dir.split("/").slice(0, -1).join("/");
          if (parent && parent !== dir) pendingDirs.add(parent);
        }

        if (debounceTimer) clearTimeout(debounceTimer);
        debounceTimer = setTimeout(() => {
          // Refresh changed directories
          for (const changedDir of pendingDirs) {
            // If the root itself changed, reload root entries
            if (changedDir === rootPath) {
              invoke<FileEntry[]>("read_directory", { path: rootPath, showHidden })
                .then((result) => {
                  if (result) setEntries(result);
                })
                .catch(() => {});
            }
            // Refresh expanded subdirectories
            if (expandedPaths.has(changedDir)) {
              invoke<FileEntry[]>("read_directory", { path: changedDir, showHidden })
                .then((result) => {
                  if (result) {
                    setChildrenMap((prev) => new Map(prev).set(changedDir, result));
                    dirCacheTimestamps.current.set(changedDir, Date.now());
                  }
                })
                .catch(() => {});
            }
          }
          pendingDirs.clear();
        }, 300); // 300ms debounce
      }
    );

    return () => {
      unlisten.then((u) => u());
      invoke("unwatch_workspace").catch(() => {});
      if (debounceTimer) clearTimeout(debounceTimer);
    };
  }, [rootPath, showHidden]);

  // Search files by name (debounced)
  useEffect(() => {
    if (!searchMode || !searchQuery.trim() || !rootPath) {
      setSearchResults([]);
      return;
    }
    if (searchDebounceRef.current) clearTimeout(searchDebounceRef.current);
    searchDebounceRef.current = setTimeout(async () => {
      try {
        const results = await invoke<FileEntry[]>("search_files_by_name", {
          root: rootPath,
          query: searchQuery,
        });
        setSearchResults(results);
      } catch {
        setSearchResults([]);
      }
    }, 300);
    return () => {
      if (searchDebounceRef.current) clearTimeout(searchDebounceRef.current);
    };
  }, [searchQuery, searchMode, rootPath]);

  const toggleNode = useCallback(
    async (entry: FileEntry) => {
      if (!entry.is_dir) {
        onFileOpen(entry.path, entry.name);
        return;
      }

      if (expandedPaths.has(entry.path)) {
        setExpandedPaths((prev) => {
          const next = new Set(prev);
          next.delete(entry.path);
          return next;
        });
      } else {
        // Use cached children if fresh, otherwise refetch
        const cachedAt = dirCacheTimestamps.current.get(entry.path) ?? 0;
        const isFresh = childrenMap.has(entry.path) && (Date.now() - cachedAt) < DIR_CACHE_TTL;
        if (!isFresh) {
          setLoadingPaths((prev) => new Set(prev).add(entry.path));
          try {
            const children = await invoke<FileEntry[]>("read_directory", {
              path: entry.path,
              showHidden,
            });
            setChildrenMap((prev) => new Map(prev).set(entry.path, children ?? []));
            dirCacheTimestamps.current.set(entry.path, Date.now());
          } catch (err) {
            console.error("Failed to read directory:", err);
          }
          setLoadingPaths((prev) => {
            const next = new Set(prev);
            next.delete(entry.path);
            return next;
          });
        }
        setExpandedPaths((prev) => new Set(prev).add(entry.path));
      }
    },
    [expandedPaths, childrenMap, onFileOpen, showHidden]
  );

  // Flatten the tree into a virtual list
  const flatNodes = useMemo(() => {
    const result: FlatTreeNode[] = [];
    const buildFlat = (items: FileEntry[], depth: number) => {
      for (const item of items) {
        const expanded = expandedPaths.has(item.path);
        result.push({
          entry: item,
          depth,
          expanded,
          hasChildren: item.is_dir,
        });
        if (expanded && childrenMap.has(item.path)) {
          buildFlat(childrenMap.get(item.path)!, depth + 1);
        }
      }
    };
    buildFlat(entries, 0);
    return result;
  }, [entries, expandedPaths, childrenMap]);

  // Keyboard navigation for the flat file list
  const navSelectRef = useRef<(node: FlatTreeNode) => void>(() => {});
  navSelectRef.current = (node: FlatTreeNode) => toggleNode(node.entry);
  const { containerProps: navContainerProps } = useKeyboardNav(
    flatNodes,
    (node) => navSelectRef.current(node),
  );

  // Virtual scrolling calculations
  const containerHeight = scrollRef.current?.clientHeight ?? 400;
  const totalHeight = flatNodes.length * ROW_HEIGHT;
  const startIndex = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - 2);
  const endIndex = Math.min(
    flatNodes.length,
    Math.ceil((scrollTop + containerHeight) / ROW_HEIGHT) + 2
  );
  const visibleNodes = flatNodes.slice(startIndex, endIndex);

  const navigateUp = () => {
    const parts = rootPath.split("/").filter(Boolean);
    if (parts.length > 1) {
      parts.pop();
      const newPath = "/" + parts.join("/");
      onRootPathChange(newPath);
    }
  };

  const openFolder = async () => {
    try {
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Open Folder",
      });
      if (selected && typeof selected === "string") {
        onRootPathChange(selected);
      }
    } catch (err) {
      console.error("Failed to open folder dialog:", err);
    }
  };

  const createNewFile = () => {
    setInlineCreate({ dir: rootPath, isFolder: false });
  };

  const createNewFolder = () => {
    setInlineCreate({ dir: rootPath, isFolder: true });
  };

  const submitInlineCreate = async (name: string) => {
    if (!inlineCreate || !name.trim()) { setInlineCreate(null); return; }
    const path = `${inlineCreate.dir}/${name.trim()}`.replace(/\/\//g, "/");
    try {
      if (inlineCreate.isFolder) {
        await invoke("create_directory", { path });
      } else {
        await invoke("write_file_content", { path, content: "" });
      }
      if (inlineCreate.dir === rootPath) {
        loadDirectory(rootPath);
      } else {
        const children = await invoke<FileEntry[]>("read_directory", { path: inlineCreate.dir, showHidden });
        setChildrenMap((prev) => new Map(prev).set(inlineCreate.dir, children ?? []));
        dirCacheTimestamps.current.set(inlineCreate.dir, Date.now());
        setExpandedPaths((prev) => new Set(prev).add(inlineCreate.dir));
      }
    } catch (err) {
      console.error(`Failed to create: ${err}`);
    }
    setInlineCreate(null);
  };

  const handleDragStart = (e: React.DragEvent, path: string) => {
    setDragSourcePath(path);
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", path);
  };

  const handleDragOver = (e: React.DragEvent, entry: FileEntry) => {
    if (!dragSourcePath) return;
    if (!entry.is_dir) return;
    if (entry.path === dragSourcePath) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    setDropTargetPath(entry.path);
  };

  const handleDragLeave = () => {
    setDropTargetPath(null);
  };

  const handleDrop = async (e: React.DragEvent, targetDir: FileEntry) => {
    e.preventDefault();
    setDropTargetPath(null);
    if (!dragSourcePath || !targetDir.is_dir) return;
    const sourceName = dragSourcePath.split("/").pop() || "";
    const newPath = `${targetDir.path}/${sourceName}`;
    if (newPath === dragSourcePath) return;
    try {
      await invoke("rename_entry", { oldPath: dragSourcePath, newPath });
      loadDirectory(rootPath);
    } catch (err) {
      console.error("Failed to move file:", err);
    }
    setDragSourcePath(null);
  };

  const handleDragEnd = () => {
    setDragSourcePath(null);
    setDropTargetPath(null);
  };

  const handleContextMenu = (e: React.MouseEvent | React.TouchEvent, entry: FileEntry) => {
    e.preventDefault();
    e.stopPropagation();
    const x = "clientX" in e ? e.clientX : (e as React.TouchEvent).touches?.[0]?.clientX ?? 0;
    const y = "clientY" in e ? e.clientY : (e as React.TouchEvent).touches?.[0]?.clientY ?? 0;
    setCtxMenu({ x, y, entry });
  };

  const handleTouchStart = (e: React.TouchEvent, entry: FileEntry) => {
    const touch = e.touches[0];
    longPressTimer.current = setTimeout(() => {
      setCtxMenu({ x: touch.clientX, y: touch.clientY, entry });
    }, 500);
  };

  const handleTouchEnd = () => {
    if (longPressTimer.current) {
      clearTimeout(longPressTimer.current);
      longPressTimer.current = null;
    }
  };

  const deleteEntry = async (entry: FileEntry) => {
    setCtxMenu(null);
    setConfirmDelete(entry);
  };

  const executeDelete = async () => {
    if (!confirmDelete) return;
    // Save content for undo (files only, not directories)
    let savedContent = "";
    const isDir = confirmDelete.is_dir;
    if (!isDir) {
      try {
        savedContent = await invoke<string>("read_file_content", { path: confirmDelete.path });
      } catch {
        // If we can't read the content, undo won't be available
        savedContent = "";
      }
    }
    try {
      await invoke("delete_entry", { path: confirmDelete.path });
      const parentPath = confirmDelete.path.substring(0, confirmDelete.path.lastIndexOf("/")) || rootPath;
      if (parentPath === rootPath) {
        loadDirectory(rootPath);
      } else {
        const children = await invoke<FileEntry[]>("read_directory", { path: parentPath, showHidden });
        setChildrenMap((prev) => new Map(prev).set(parentPath, children ?? []));
        dirCacheTimestamps.current.set(parentPath, Date.now());
      }
      // Set trashed file for undo
      if (undoTimerRef.current) clearTimeout(undoTimerRef.current);
      setTrashedFile({ path: confirmDelete.path, content: savedContent, isDir });
      undoTimerRef.current = setTimeout(() => {
        setTrashedFile(null);
        undoTimerRef.current = null;
      }, 5000);
    } catch (err) {
      console.error(`Failed to delete: ${err}`);
    }
    setConfirmDelete(null);
  };

  const handleUndo = async () => {
    if (!trashedFile) return;
    if (undoTimerRef.current) {
      clearTimeout(undoTimerRef.current);
      undoTimerRef.current = null;
    }
    try {
      if (trashedFile.isDir) {
        await invoke("create_directory", { path: trashedFile.path });
      } else {
        await invoke("write_file_content", { path: trashedFile.path, content: trashedFile.content });
      }
      const parentPath = trashedFile.path.substring(0, trashedFile.path.lastIndexOf("/")) || rootPath;
      if (parentPath === rootPath) {
        loadDirectory(rootPath);
      } else {
        const children = await invoke<FileEntry[]>("read_directory", { path: parentPath, showHidden });
        setChildrenMap((prev) => new Map(prev).set(parentPath, children ?? []));
        dirCacheTimestamps.current.set(parentPath, Date.now());
      }
    } catch (err) {
      console.error(`Failed to undo delete: ${err}`);
    }
    setTrashedFile(null);
  };

  const startRename = (entry: FileEntry) => {
    setRenaming({ path: entry.path, name: entry.name });
    setCtxMenu(null);
  };

  const finishRename = async (newName: string) => {
    if (!renaming || !newName.trim() || newName === renaming.name) {
      setRenaming(null);
      return;
    }
    const parentPath = renaming.path.substring(0, renaming.path.lastIndexOf("/"));
    const newPath = `${parentPath}/${newName}`.replace(/\/\//g, "/");
    try {
      await invoke("rename_entry", { oldPath: renaming.path, newPath });
      if (parentPath === rootPath || parentPath === "") {
        loadDirectory(rootPath);
      } else {
        const children = await invoke<FileEntry[]>("read_directory", { path: parentPath, showHidden });
        setChildrenMap((prev) => new Map(prev).set(parentPath, children ?? []));
        dirCacheTimestamps.current.set(parentPath, Date.now());
      }
    } catch (err) {
      alert(`Failed to rename: ${err}`);
    }
    setRenaming(null);
  };

  const createInDir = (dirPath: string, isFolder: boolean) => {
    setCtxMenu(null);
    setInlineCreate({ dir: dirPath, isFolder });
  };

  // Close context menu on click outside
  useEffect(() => {
    if (!ctxMenu) return;
    const close = () => setCtxMenu(null);
    window.addEventListener("click", close);
    window.addEventListener("scroll", close, true);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("scroll", close, true);
    };
  }, [ctxMenu]);

  // Cleanup undo timer on unmount
  useEffect(() => {
    return () => {
      if (undoTimerRef.current) clearTimeout(undoTimerRef.current);
    };
  }, []);

  return (
    <div className="file-explorer">
      <div className="explorer-header">
        <span className="explorer-title">EXPLORER</span>
        <button
          className="explorer-btn"
          onClick={createNewFile}
          title="New File"
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" style={{ color: "#22c55e" }}>
            <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" />
            <polyline points="14 2 14 8 20 8" />
            <line x1="12" y1="18" x2="12" y2="12" />
            <line x1="9" y1="15" x2="15" y2="15" />
          </svg>
        </button>
        <button
          className="explorer-btn"
          onClick={createNewFolder}
          title="New Folder"
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" style={{ color: "#818cf8" }}>
            <path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" />
            <line x1="12" y1="11" x2="12" y2="17" />
            <line x1="9" y1="14" x2="15" y2="14" />
          </svg>
        </button>
        <button
          className="explorer-btn"
          onClick={openFolder}
          title="Open Folder"
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" style={{ color: "#818cf8" }}>
            <path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" />
            <line x1="12" y1="11" x2="12" y2="17" /><line x1="9" y1="14" x2="15" y2="14" />
          </svg>
        </button>
        <button
          className="explorer-btn"
          onClick={() => setSearchMode(prev => !prev)}
          title="Search files"
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="11" cy="11" r="8" /><path d="M21 21l-4.35-4.35" />
          </svg>
        </button>
        <button
          className="explorer-btn"
          onClick={() => {
            setShowHidden(prev => !prev);
          }}
          title={showHidden ? "Hide dotfiles" : "Show dotfiles"}
          style={{ opacity: showHidden ? 1 : 0.5 }}
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
            <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" /><circle cx="12" cy="12" r="3" />
          </svg>
        </button>
        <button className="explorer-btn" onClick={navigateUp} title="Go up">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
            <polyline points="18 15 12 9 6 15" />
          </svg>
        </button>
        <button
          className="explorer-btn"
          onClick={() => loadDirectory(rootPath)}
          title="Refresh"
        >
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
            <polyline points="23 4 23 10 17 10" /><path d="M20.49 15a9 9 0 11-2.12-9.36L23 10" />
          </svg>
        </button>
      </div>
      {searchMode && (
        <div className="explorer-search">
          <input
            className="explorer-search-input"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder="Search files by name..."
            autoFocus
          />
        </div>
      )}
      {searchMode && searchQuery.trim() && searchResults.length > 0 && (
        <div className="explorer-search-results">
          {searchResults.map(entry => (
            <div
              key={entry.path}
              className="tree-node-label"
              style={{ paddingLeft: "8px", height: `${ROW_HEIGHT}px` }}
              onClick={() => onFileOpen(entry.path, entry.name)}
              title={entry.path}
            >
              <span className="tree-icon">
                {getFileIcon(entry, false)}
              </span>
              <span className="tree-name">{entry.name}</span>
            </div>
          ))}
        </div>
      )}
      <div className="explorer-path" title={rootPath}>
        {rootPath.split("/").pop() || "/"}
      </div>
      {/* Inline create input */}
      {inlineCreate && (
        <div style={{ display: "flex", alignItems: "center", gap: 4, padding: "4px 8px", background: "var(--bg-primary)", borderBottom: "1px solid var(--border-color)" }}>
          <span style={{ fontSize: 11, color: "var(--text-muted)", flexShrink: 0 }}>
            {inlineCreate.isFolder ? "New folder:" : "New file:"}
          </span>
          <input
            autoFocus
            placeholder="name..."
            style={{
              flex: 1, background: "var(--bg-secondary)", border: "1px solid var(--accent)",
              borderRadius: 3, color: "var(--text-primary)", padding: "3px 6px", fontSize: 12,
              outline: "none", minWidth: 0,
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") submitInlineCreate((e.target as HTMLInputElement).value);
              if (e.key === "Escape") setInlineCreate(null);
            }}
            onBlur={(e) => submitInlineCreate(e.target.value)}
          />
        </div>
      )}
      {/* Confirm delete dialog */}
      {confirmDelete && (
        <div style={{
          display: "flex", alignItems: "center", gap: 6, padding: "6px 8px",
          background: "#7f1d1d", borderBottom: "1px solid #ef4444", fontSize: 12, color: "#fca5a5",
        }}>
          <span style={{ flex: 1 }}>Delete "{confirmDelete.name}"?</span>
          <button onClick={executeDelete} style={{
            padding: "3px 10px", background: "#ef4444", border: "none", borderRadius: 4,
            color: "#fff", fontSize: 11, fontWeight: 600, cursor: "pointer",
          }}>Delete</button>
          <button onClick={() => setConfirmDelete(null)} style={{
            padding: "3px 10px", background: "transparent", border: "1px solid #6b7280", borderRadius: 4,
            color: "#d1d5db", fontSize: 11, cursor: "pointer",
          }}>Cancel</button>
        </div>
      )}
      {/* Undo delete toast */}
      {trashedFile && (
        <div style={{
          display: "flex", alignItems: "center", gap: 8, padding: "6px 8px",
          background: "#1e3a5f", borderBottom: "1px solid #3b82f6", fontSize: 12, color: "#93c5fd",
        }}>
          <span style={{ flex: 1 }}>
            Deleted &ldquo;{trashedFile.path.split("/").pop()}&rdquo;
          </span>
          <button onClick={handleUndo} style={{
            padding: "3px 10px", background: "#3b82f6", border: "none", borderRadius: 4,
            color: "#fff", fontSize: 11, fontWeight: 600, cursor: "pointer",
          }}>Undo</button>
          <button onClick={() => { setTrashedFile(null); if (undoTimerRef.current) { clearTimeout(undoTimerRef.current); undoTimerRef.current = null; } }} style={{
            padding: "3px 10px", background: "transparent", border: "1px solid #6b7280", borderRadius: 4,
            color: "#d1d5db", fontSize: 11, cursor: "pointer",
          }}>Dismiss</button>
        </div>
      )}
      <div
        className="explorer-tree"
        ref={scrollRef}
        tabIndex={0}
        role={navContainerProps.role}
        onScroll={(e) => {
          const top = e.currentTarget.scrollTop;
          requestAnimationFrame(() => setScrollTop(top));
        }}
        onKeyDown={(e) => navContainerProps.onKeyDown(e as Parameters<typeof navContainerProps.onKeyDown>[0])}
      >
        {error && <div className="explorer-error">{error}</div>}
        <div style={{ height: `${totalHeight}px`, position: "relative" }}>
          {visibleNodes.map((node, i) => (
            <FileRow
              key={node.entry.path}
              node={node}
              index={i}
              startIndex={startIndex}
              onClick={toggleNode}
              dragSourcePath={dragSourcePath}
              dropTargetPath={dropTargetPath}
              loading={loadingPaths.has(node.entry.path)}
              onDragStart={handleDragStart}
              onDragOver={handleDragOver}
              onDragLeave={handleDragLeave}
              onDrop={handleDrop}
              onDragEnd={handleDragEnd}
              onContextMenu={handleContextMenu}
              onTouchStart={handleTouchStart}
              onTouchEnd={handleTouchEnd}
              renaming={renaming}
              onFinishRename={finishRename}
            />
          ))}
        </div>
      </div>

      {/* Context menu */}
      {ctxMenu && (
        <div
          style={{
            position: "fixed", top: ctxMenu.y, left: ctxMenu.x,
            background: "var(--bg-secondary)", border: "1px solid var(--border-color)",
            borderRadius: 6, boxShadow: "0 4px 16px rgba(0,0,0,0.5)", zIndex: 9999,
            padding: "4px 0", minWidth: 160, fontSize: 12,
          }}
          onClick={(e) => e.stopPropagation()}
        >
          <div
            className="ctx-menu-item"
            style={{ padding: "6px 12px", cursor: "pointer", color: "var(--text-primary)" }}
            onMouseEnter={(e) => (e.currentTarget.style.background = "var(--bg-hover)")}
            onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
            onClick={() => startRename(ctxMenu.entry)}
          >
            Rename
          </div>
          <div
            style={{ padding: "6px 12px", cursor: "pointer", color: "#ef4444" }}
            onMouseEnter={(e) => (e.currentTarget.style.background = "var(--bg-hover)")}
            onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
            onClick={() => deleteEntry(ctxMenu.entry)}
          >
            Delete
          </div>
          {ctxMenu.entry.is_dir && (
            <>
              <div style={{ height: 1, background: "var(--border-color)", margin: "4px 0" }} />
              <div
                style={{ padding: "6px 12px", cursor: "pointer", color: "var(--text-primary)" }}
                onMouseEnter={(e) => (e.currentTarget.style.background = "var(--bg-hover)")}
                onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
                onClick={() => createInDir(ctxMenu.entry.path, false)}
              >
                New File Here
              </div>
              <div
                style={{ padding: "6px 12px", cursor: "pointer", color: "var(--text-primary)" }}
                onMouseEnter={(e) => (e.currentTarget.style.background = "var(--bg-hover)")}
                onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
                onClick={() => createInDir(ctxMenu.entry.path, true)}
              >
                New Folder Here
              </div>
            </>
          )}
          {(() => {
            const ext = ctxMenu.entry.extension?.toLowerCase();
            const isGlsl = ext === "glsl" || ext === "frag" || ext === "vert" || ext === "gdshader";
            if (!isGlsl) return null;
            return (
              <>
                <div style={{ height: 1, background: "var(--border-color)", margin: "4px 0" }} />
                <div
                  style={{ padding: "6px 12px", cursor: "pointer", color: "#89b4fa" }}
                  onMouseEnter={(e) => (e.currentTarget.style.background = "var(--bg-hover)")}
                  onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
                  onClick={() => {
                    setCtxMenu(null);
                    window.dispatchEvent(new CustomEvent("open-glsl-preview", {
                      detail: { path: ctxMenu.entry.path },
                    }));
                  }}
                >
                  🔷 Open in GLSL Preview
                </div>
              </>
            );
          })()}
          {(() => {
            const ext = ctxMenu.entry.extension?.toLowerCase();
            const isMesh = ext === "obj" || ext === "gltf" || ext === "fbx";
            if (!isMesh) return null;
            return (
              <>
                <div style={{ height: 1, background: "var(--border-color)", margin: "4px 0" }} />
                <div
                  style={{ padding: "6px 12px", cursor: "pointer", color: "#00e5ff" }}
                  onMouseEnter={(e) => (e.currentTarget.style.background = "var(--bg-hover)")}
                  onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
                  onClick={() => {
                    setCtxMenu(null);
                    onFileOpen(ctxMenu.entry.path, ctxMenu.entry.name);
                  }}
                >
                  🗺️ Preview Mesh
                </div>
              </>
            );
          })()}
        </div>
      )}
    </div>
  );
}
export default memo(FileExplorer);
