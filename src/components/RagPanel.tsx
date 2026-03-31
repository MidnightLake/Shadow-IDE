import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

interface DocFileInfo {
  name: string;
  path: string;
  size: number;
  extension: string;
  subfolder: string;
}

interface RagStats {
  files_indexed: number;
  total_chunks: number;
  last_index_time: string;
}

interface DocumentsFolderInfo {
  path: string;
  created: boolean;
  files: DocFileInfo[];
  subfolders: string[];
  total_size: number;
}

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

const EXT_ICONS: Record<string, string> = {
  pdf: "PDF", md: "MD", txt: "TXT", json: "JSON", yaml: "YML",
  yml: "YML", xml: "XML", csv: "CSV", html: "HTML", log: "LOG",
  toml: "TOML", rst: "RST", org: "ORG",
};

export default function RagPanel({ visible, rootPath }: { visible: boolean; rootPath: string }) {
  const [stats, setStats] = useState<RagStats | null>(null);
  const [docFiles, setDocFiles] = useState<DocFileInfo[]>([]);
  const [docsInfo, setDocsInfo] = useState<DocumentsFolderInfo | null>(null);
  const [indexing, setIndexing] = useState(false);
  const [expandedFolders, setExpandedFolders] = useState<Set<string>>(new Set([""]));
  const [statusMsg, setStatusMsg] = useState("");

  const refreshStats = useCallback(async () => {
    try {
      const s = await invoke<RagStats>("rag_get_stats");
      setStats(s);
    } catch { /* ignore */ }
  }, []);

  const refreshDocs = useCallback(async () => {
    if (!rootPath) return;
    try {
      const info = await invoke<DocumentsFolderInfo>("ensure_documents_folder", { rootPath });
      setDocsInfo(info);
      setDocFiles(info.files);
    } catch { /* ignore */ }
  }, [rootPath]);

  useEffect(() => {
    if (visible && rootPath) {
      refreshStats();
      refreshDocs();
    }
  }, [visible, rootPath, refreshStats, refreshDocs]);

  const buildIndex = useCallback(async () => {
    if (!rootPath || indexing) return;
    setIndexing(true);
    setStatusMsg("Indexing project...");
    try {
      await invoke("rag_build_index", { rootPath });
      setStatusMsg("Indexing Documents...");
      await invoke("rag_index_documents", { docPath: `${rootPath}/Documents` });
      setStatusMsg("Done!");
      await refreshStats();
    } catch (e) {
      setStatusMsg(`Error: ${e}`);
    }
    setIndexing(false);
    setTimeout(() => setStatusMsg(""), 3000);
  }, [rootPath, indexing, refreshStats]);

  const indexDocumentsOnly = useCallback(async () => {
    if (!rootPath || indexing) return;
    setIndexing(true);
    setStatusMsg("Indexing Documents folder...");
    try {
      await invoke("rag_index_documents", { docPath: `${rootPath}/Documents` });
      setStatusMsg("Done!");
      await refreshStats();
    } catch (e) {
      setStatusMsg(`Error: ${e}`);
    }
    setIndexing(false);
    setTimeout(() => setStatusMsg(""), 3000);
  }, [rootPath, indexing, refreshStats]);

  if (!visible) return null;

  // Group files by subfolder
  const folderGroups: Record<string, DocFileInfo[]> = {};
  for (const f of docFiles) {
    const key = f.subfolder || "";
    if (!folderGroups[key]) folderGroups[key] = [];
    folderGroups[key].push(f);
  }
  const folderKeys = Object.keys(folderGroups).sort();

  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column", fontSize: 12 }}>
      {/* Header */}
      <div style={{
        padding: "8px 12px", borderBottom: "1px solid var(--border)",
        display: "flex", justifyContent: "space-between", alignItems: "center",
      }}>
        <span style={{ fontWeight: 600, fontSize: 11, textTransform: "uppercase", letterSpacing: "0.5px" }}>
          RAG Documents
        </span>
        <button
          onClick={refreshDocs}
          style={{ background: "none", border: "none", color: "var(--text-secondary)", cursor: "pointer", fontSize: 11 }}
          title="Refresh"
        >
          &#8635;
        </button>
      </div>

      {/* Stats */}
      {stats && (
        <div style={{
          padding: "6px 12px", background: "var(--bg-secondary)", borderBottom: "1px solid var(--border)",
          display: "flex", gap: 12, fontSize: 10, color: "var(--text-secondary)",
        }}>
          <span>{stats.files_indexed} files</span>
          <span>{stats.total_chunks} chunks</span>
          {stats.last_index_time && <span>{stats.last_index_time}</span>}
        </div>
      )}

      {/* Actions */}
      <div style={{ padding: "8px 12px", display: "flex", gap: 6, borderBottom: "1px solid var(--border)" }}>
        <button
          onClick={buildIndex}
          disabled={indexing || !rootPath}
          style={{
            flex: 1, padding: "6px 8px", fontSize: 11, fontWeight: 600,
            background: indexing ? "var(--bg-secondary)" : "var(--accent)", color: "#fff",
            border: "none", borderRadius: 4, cursor: indexing ? "default" : "pointer",
            opacity: indexing ? 0.6 : 1,
          }}
        >
          {indexing ? "Indexing..." : "Build Full Index"}
        </button>
        <button
          onClick={indexDocumentsOnly}
          disabled={indexing || !rootPath}
          style={{
            flex: 1, padding: "6px 8px", fontSize: 11, fontWeight: 600,
            background: "var(--bg-secondary)", color: "var(--text-primary)",
            border: "1px solid var(--border)", borderRadius: 4,
            cursor: indexing ? "default" : "pointer", opacity: indexing ? 0.6 : 1,
          }}
        >
          Index Docs Only
        </button>
      </div>

      {statusMsg && (
        <div style={{ padding: "4px 12px", fontSize: 10, color: "var(--accent)" }}>{statusMsg}</div>
      )}

      {/* Documents folder info */}
      {docsInfo && (
        <div style={{
          padding: "6px 12px", fontSize: 10, color: "var(--text-secondary)",
          borderBottom: "1px solid var(--border)",
        }}>
          <div style={{ display: "flex", justifyContent: "space-between" }}>
            <span>Documents/</span>
            <span>{docFiles.length} files ({formatSize(docsInfo.total_size)})</span>
          </div>
          {docsInfo.created && (
            <div style={{ color: "var(--accent)", marginTop: 2 }}>Created Documents folder</div>
          )}
        </div>
      )}

      {/* File tree */}
      <div style={{ flex: 1, overflow: "auto", padding: "4px 0" }}>
        {docFiles.length === 0 ? (
          <div style={{ padding: "20px 12px", textAlign: "center", color: "var(--text-secondary)", fontSize: 11 }}>
            <div style={{ marginBottom: 8 }}>No documents found</div>
            <div>Add .txt, .md, .pdf, .json, .yaml files to:</div>
            <div style={{ fontFamily: "monospace", marginTop: 4 }}>{rootPath}/Documents/</div>
          </div>
        ) : (
          folderKeys.map(folder => {
            const files = folderGroups[folder];
            const isRoot = folder === "";
            const isExpanded = expandedFolders.has(folder);

            return (
              <div key={folder}>
                {!isRoot && (
                  <div
                    style={{
                      padding: "3px 12px", cursor: "pointer", display: "flex", alignItems: "center", gap: 4,
                      color: "var(--text-secondary)", fontSize: 11, fontWeight: 600,
                    }}
                    onClick={() => setExpandedFolders(prev => {
                      const next = new Set(prev);
                      if (next.has(folder)) next.delete(folder); else next.add(folder);
                      return next;
                    })}
                  >
                    <span style={{ fontSize: 8 }}>{isExpanded ? "\u25BC" : "\u25B6"}</span>
                    <span>{folder}/</span>
                    <span style={{ fontWeight: 400, marginLeft: "auto" }}>{files.length}</span>
                  </div>
                )}
                {(isRoot || isExpanded) && files.map(f => (
                  <div
                    key={f.path}
                    style={{
                      padding: "2px 12px 2px " + (isRoot ? "12px" : "24px"),
                      display: "flex", alignItems: "center", gap: 6,
                      fontSize: 11, color: "var(--text-primary)",
                    }}
                    title={f.path}
                  >
                    <span style={{
                      fontSize: 8, fontWeight: 700, padding: "1px 3px",
                      borderRadius: 2, background: "var(--bg-secondary)",
                      color: "var(--text-secondary)", minWidth: 24, textAlign: "center",
                    }}>
                      {EXT_ICONS[f.extension] || f.extension.toUpperCase()}
                    </span>
                    <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                      {f.name}
                    </span>
                    <span style={{ fontSize: 10, color: "var(--text-secondary)", flexShrink: 0 }}>
                      {formatSize(f.size)}
                    </span>
                  </div>
                ))}
              </div>
            );
          })
        )}
      </div>

      {/* Tips */}
      <div style={{
        padding: "8px 12px", borderTop: "1px solid var(--border)",
        fontSize: 10, color: "var(--text-secondary)", lineHeight: 1.5,
      }}>
        Supported: txt, md, pdf, json, yaml, xml, csv, html, rst, org, log, toml
      </div>
    </div>
  );
}
