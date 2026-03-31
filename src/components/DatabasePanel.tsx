import React, { useState, useCallback } from "react";
import MonacoEditor from "@monaco-editor/react";
import { invoke } from "@tauri-apps/api/core";

interface Connection {
  id: string;
  name: string;
  type: "postgresql" | "mysql" | "sqlite" | "mongodb" | "redis";
  host: string;
  port: number;
  database: string;
  username?: string;
}

type SortDirection = "asc" | "desc" | null;

interface ColumnSort {
  column: string;
  direction: SortDirection;
}

interface SchemaTable {
  name: string;
  columns: Array<{ name: string; type: string; nullable: boolean }>;
}

// eslint-disable-next-line @typescript-eslint/no-explicit-any -- query results can be any shape
type QueryRow = Record<string, unknown>;

const DEFAULT_NEW_CONN: Omit<Connection, "id"> = {
  name: "New Connection",
  type: "postgresql",
  host: "localhost",
  port: 5432,
  database: "",
  username: "",
};

const PAGE_SIZE = 100;

function generateId(): string {
  return Math.random().toString(36).slice(2) + Date.now().toString(36);
}

export default function DatabasePanel() {
  const [connections, setConnections] = useState<Connection[]>([]);
  const [activeConnId, setActiveConnId] = useState<string | null>(null);
  const [showAddForm, setShowAddForm] = useState(false);
  const [newConn, setNewConn] = useState<Omit<Connection, "id">>(DEFAULT_NEW_CONN);

  const [sqlQuery, setSqlQuery] = useState("SELECT 1;");
  const [queryResults, setQueryResults] = useState<QueryRow[]>([]);
  const [queryError, setQueryError] = useState<string | null>(null);
  const [queryRunning, setQueryRunning] = useState(false);

  const [schema, setSchema] = useState<SchemaTable[]>([]);
  const [schemaLoading, setSchemaLoading] = useState(false);
  const [expandedTables, setExpandedTables] = useState<Set<string>>(new Set());

  const [columnSort, setColumnSort] = useState<ColumnSort | null>(null);
  const [page, setPage] = useState(0);

  const activeConn = connections.find((c) => c.id === activeConnId) ?? null;

  const handleAddConnection = useCallback(() => {
    const conn: Connection = { ...newConn, id: generateId() };
    setConnections((prev) => [...prev, conn]);
    setNewConn(DEFAULT_NEW_CONN);
    setShowAddForm(false);
  }, [newConn]);

  const handleConnect = useCallback(async (conn: Connection) => {
    setActiveConnId(conn.id);
    try {
      await invoke("db_connect", {
        id: conn.id,
        connType: conn.type,
        host: conn.host,
        port: conn.port,
        database: conn.database,
        username: conn.username ?? null,
      });
    } catch {
      // Backend may not implement yet — ignore
    }

    // Load schema
    setSchemaLoading(true);
    try {
      const tables = await invoke<SchemaTable[]>("db_schema", { connId: conn.id });
      setSchema(tables);
    } catch {
      setSchema([]);
    } finally {
      setSchemaLoading(false);
    }
  }, []);

  const handleRunQuery = useCallback(async () => {
    if (!activeConnId) return;
    setQueryRunning(true);
    setQueryError(null);
    setQueryResults([]);
    setPage(0);
    try {
      const rows = await invoke<QueryRow[]>("db_query", {
        connId: activeConnId,
        query: sqlQuery,
      });
      setQueryResults(rows);
    } catch (err) {
      setQueryError(String(err));
    } finally {
      setQueryRunning(false);
    }
  }, [activeConnId, sqlQuery]);

  // Sorted + paginated results
  const displayRows = (() => {
    let rows = [...queryResults];
    if (columnSort && columnSort.direction) {
      const { column, direction } = columnSort;
      rows.sort((a, b) => {
        const av = a[column] ?? "";
        const bv = b[column] ?? "";
        const cmp = String(av).localeCompare(String(bv), undefined, { numeric: true });
        return direction === "asc" ? cmp : -cmp;
      });
    }
    return rows.slice(page * PAGE_SIZE, (page + 1) * PAGE_SIZE);
  })();

  const columns = queryResults.length > 0 ? Object.keys(queryResults[0]) : [];
  const totalPages = Math.max(1, Math.ceil(queryResults.length / PAGE_SIZE));

  const handleSortColumn = (col: string) => {
    setColumnSort((prev) => {
      if (prev?.column !== col) return { column: col, direction: "asc" };
      if (prev.direction === "asc") return { column: col, direction: "desc" };
      return null;
    });
  };

  const panelStyle: React.CSSProperties = {
    display: "flex",
    height: "100%",
    fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
    fontSize: 13,
    color: "var(--text-primary)",
    background: "var(--bg-primary)",
    overflow: "hidden",
  };

  const sidebarStyle: React.CSSProperties = {
    width: 220,
    borderRight: "1px solid #313244",
    display: "flex",
    flexDirection: "column",
    flexShrink: 0,
    overflow: "hidden",
  };

  const mainStyle: React.CSSProperties = {
    flex: 1,
    display: "flex",
    flexDirection: "column",
    overflow: "hidden",
  };

  return (
    <div style={panelStyle}>
      {/* Connection sidebar */}
      <div style={sidebarStyle}>
        <div style={{ padding: "8px 10px", borderBottom: "1px solid var(--border-color)", display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <span style={{ fontWeight: 700, fontSize: 12, color: "var(--accent-hover)" }}>Database</span>
          <button
            onClick={() => setShowAddForm((p) => !p)}
            style={iconBtnStyle}
            title="Add connection"
          >+</button>
        </div>

        {showAddForm && (
          <div style={{ padding: 8, borderBottom: "1px solid var(--border-color)", display: "flex", flexDirection: "column", gap: 4 }}>
            <input style={inputStyle} placeholder="Name" value={newConn.name} onChange={(e) => setNewConn((p) => ({ ...p, name: e.target.value }))} />
            <select style={inputStyle} value={newConn.type} onChange={(e) => setNewConn((p) => ({ ...p, type: e.target.value as Connection["type"] }))}>
              {(["postgresql", "mysql", "sqlite", "mongodb", "redis"] as const).map((t) => (
                <option key={t} value={t}>{t}</option>
              ))}
            </select>
            <input style={inputStyle} placeholder="Host" value={newConn.host} onChange={(e) => setNewConn((p) => ({ ...p, host: e.target.value }))} />
            <input style={inputStyle} placeholder="Port" type="number" value={newConn.port} onChange={(e) => setNewConn((p) => ({ ...p, port: parseInt(e.target.value, 10) || 5432 }))} />
            <input style={inputStyle} placeholder="Database" value={newConn.database} onChange={(e) => setNewConn((p) => ({ ...p, database: e.target.value }))} />
            <input style={inputStyle} placeholder="Username" value={newConn.username ?? ""} onChange={(e) => setNewConn((p) => ({ ...p, username: e.target.value }))} />
            <div style={{ display: "flex", gap: 4 }}>
              <button style={primaryBtnStyle} onClick={handleAddConnection}>Add</button>
              <button style={ghostBtnStyle} onClick={() => setShowAddForm(false)}>Cancel</button>
            </div>
          </div>
        )}

        <div style={{ flex: 1, overflowY: "auto" }} role="list" aria-label="Database connections">
          {connections.length === 0 && (
            <div style={{ padding: "12px 10px", color: "var(--text-muted)", fontSize: 12 }}>No connections yet</div>
          )}
          {connections.map((conn) => (
            <div
              key={conn.id}
              role="listitem"
              style={{
                display: "flex",
                alignItems: "center",
                padding: "6px 10px",
                cursor: "pointer",
                background: activeConnId === conn.id ? "var(--bg-hover)" : "transparent",
                borderBottom: "1px solid #181825",
                gap: 6,
              }}
              onClick={() => handleConnect(conn)}
            >
              <span style={{ fontSize: 10, opacity: 0.6, flexShrink: 0 }}>{conn.type.slice(0, 2).toUpperCase()}</span>
              <span style={{ flex: 1, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>{conn.name}</span>
              <button
                style={{ ...iconBtnStyle, color: "#f38ba8", fontSize: 14 }}
                onClick={(e) => { e.stopPropagation(); setConnections((p) => p.filter((c) => c.id !== conn.id)); if (activeConnId === conn.id) setActiveConnId(null); }}
                title="Remove"
              >×</button>
            </div>
          ))}
        </div>

        {/* Schema browser */}
        {activeConn && (
          <div style={{ borderTop: "1px solid var(--border-color)", flex: 1, overflowY: "auto", maxHeight: "40%" }}>
            <div style={{ padding: "6px 10px", fontSize: 11, color: "var(--text-muted)", textTransform: "uppercase" }}>Schema</div>
            {schemaLoading && <div style={{ padding: "8px 10px", color: "var(--text-muted)", fontSize: 12 }}>Loading…</div>}
            {schema.map((table) => (
              <div key={table.name}>
                <div
                  style={{ padding: "4px 10px", cursor: "pointer", display: "flex", gap: 4, alignItems: "center" }}
                  onClick={() => setExpandedTables((prev) => {
                    const next = new Set(prev);
                    next.has(table.name) ? next.delete(table.name) : next.add(table.name);
                    return next;
                  })}
                >
                  <span style={{ fontSize: 9, opacity: 0.5 }}>{expandedTables.has(table.name) ? "▼" : "▶"}</span>
                  <span style={{ color: "var(--accent-hover)" }}>{table.name}</span>
                </div>
                {expandedTables.has(table.name) && table.columns.map((col) => (
                  <div key={col.name} style={{ padding: "2px 10px 2px 24px", fontSize: 11, display: "flex", gap: 6 }}>
                    <span style={{ color: "var(--text-primary)" }}>{col.name}</span>
                    <span style={{ color: "var(--text-muted)" }}>{col.type}</span>
                    {!col.nullable && <span style={{ color: "#f38ba8", fontSize: 9 }}>NOT NULL</span>}
                  </div>
                ))}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Main: SQL editor + results */}
      <div style={mainStyle}>
        {/* SQL editor */}
        <div style={{ height: 180, borderBottom: "1px solid var(--border-color)", flexShrink: 0, position: "relative" }}>
          <MonacoEditor
            height={180}
            language="sql"
            value={sqlQuery}
            theme="vs-dark"
            onChange={(v) => { if (v !== undefined) setSqlQuery(v); }}
            options={{
              fontSize: 13,
              fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
              minimap: { enabled: false },
              scrollBeyondLastLine: false,
              automaticLayout: true,
              lineNumbers: "off",
            }}
          />
          <button
            style={{ ...primaryBtnStyle, position: "absolute", bottom: 8, right: 8, zIndex: 1 }}
            onClick={handleRunQuery}
            disabled={!activeConnId || queryRunning}
          >
            {queryRunning ? "Running…" : "▶ Run"}
          </button>
        </div>

        {/* Results */}
        <div style={{ flex: 1, overflowY: "auto", overflowX: "auto" }}>
          {queryError && (
            <div style={{ padding: 12, color: "#f38ba8", background: "#2d1a1a", borderBottom: "1px solid var(--border-color)", fontSize: 12 }}>
              Error: {queryError}
            </div>
          )}
          {queryResults.length > 0 && (
            <>
              <div style={{ padding: "4px 10px", borderBottom: "1px solid var(--border-color)", fontSize: 11, color: "var(--text-muted)" }}>
                {queryResults.length} row{queryResults.length !== 1 ? "s" : ""} · page {page + 1}/{totalPages}
              </div>
              <div style={{ overflowX: "auto" }}>
                <table role="grid" aria-label="Query results" style={{ width: "100%", borderCollapse: "collapse", fontSize: 12 }}>
                  <thead>
                    <tr>
                      {columns.map((col) => (
                        <th
                          key={col}
                          role="columnheader"
                          onClick={() => handleSortColumn(col)}
                          style={{
                            padding: "4px 10px",
                            textAlign: "left",
                            background: "var(--bg-primary)",
                            borderBottom: "1px solid var(--border-color)",
                            cursor: "pointer",
                            whiteSpace: "nowrap",
                            color: columnSort?.column === col ? "var(--accent-hover)" : "var(--text-primary)",
                            userSelect: "none",
                          }}
                        >
                          {col}{columnSort?.column === col ? (columnSort.direction === "asc" ? " ↑" : " ↓") : ""}
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {displayRows.map((row, i) => (
                      <tr key={i} style={{ background: i % 2 === 0 ? "var(--bg-primary)" : "var(--bg-primary)" }}>
                        {columns.map((col) => (
                          <td role="gridcell" key={col} style={{ padding: "3px 10px", borderBottom: "1px solid var(--border-color)", maxWidth: 300, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                            {row[col] === null ? <span style={{ opacity: 0.4 }}>NULL</span> : String(row[col])}
                          </td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              {/* Pagination */}
              <div style={{ display: "flex", gap: 4, padding: 8, justifyContent: "center" }}>
                <button style={ghostBtnStyle} disabled={page === 0} onClick={() => setPage((p) => p - 1)}>‹ Prev</button>
                <button style={ghostBtnStyle} disabled={page >= totalPages - 1} onClick={() => setPage((p) => p + 1)}>Next ›</button>
              </div>
            </>
          )}
          {!queryRunning && queryResults.length === 0 && !queryError && (
            <div style={{ padding: 20, color: "var(--text-muted)", fontSize: 12 }}>
              {activeConnId ? "Run a query to see results" : "Connect to a database to get started"}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

const inputStyle: React.CSSProperties = {
  background: "var(--bg-primary)",
  border: "1px solid var(--border-color)",
  borderRadius: 4,
  color: "var(--text-primary)",
  padding: "3px 6px",
  fontSize: 12,
  fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
  outline: "none",
  width: "100%",
};

const primaryBtnStyle: React.CSSProperties = {
  background: "var(--accent-hover)",
  color: "var(--bg-primary)",
  border: "none",
  borderRadius: 4,
  padding: "4px 10px",
  cursor: "pointer",
  fontSize: 12,
  fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
  fontWeight: 600,
};

const ghostBtnStyle: React.CSSProperties = {
  background: "transparent",
  color: "var(--accent-hover)",
  border: "1px solid var(--border-color)",
  borderRadius: 4,
  padding: "3px 8px",
  cursor: "pointer",
  fontSize: 12,
  fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
};

const iconBtnStyle: React.CSSProperties = {
  background: "transparent",
  border: "none",
  color: "var(--text-muted)",
  cursor: "pointer",
  padding: "0 4px",
  fontSize: 16,
  lineHeight: 1,
};
