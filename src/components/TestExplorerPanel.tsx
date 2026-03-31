import { useEffect, useState, useCallback, memo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import InlineDiffPreview from "./InlineDiffPreview";
import { DiffEditor } from "@monaco-editor/react";

type TestStatus = "pending" | "running" | "pass" | "fail";

interface TestCase {
  name: string;
  status: TestStatus;
}

interface DescribeBlock {
  name: string;
  tests: TestCase[];
}

interface TestFile {
  path: string;
  name: string;
  describes: DescribeBlock[];
  topLevelTests: TestCase[];
}

interface TestResultEvent {
  testName: string;
  status: "pass" | "fail";
  file?: string;
}

interface FileEntry {
  name: string;
  path: string;
  is_dir: boolean;
}

interface TestExplorerPanelProps {
  rootPath: string;
  visible?: boolean;
}

interface CoverageData {
  file: string;
  covered: number;
  total: number;
  percent: number;
}

interface GeneratedTestPreview {
  filePath: string;
  functionName: string;
  generatedContent: string;
  existingContent: string;
  targetPath: string;
}

function CoverageBar({ file, coverageMap }: { file: string; coverageMap: Map<string, CoverageData> }) {
  const data = coverageMap.get(file);
  if (!data) return null;
  const color = data.percent < 20 ? "#ef4444" : data.percent < 50 ? "#f97316" : "#22c55e";
  return (
    <div style={{ height: 3, background: "var(--bg-hover, #313244)", borderRadius: 2, margin: "2px 0" }}>
      <div style={{
        height: "100%",
        width: `${Math.min(data.percent, 100)}%`,
        background: color,
        borderRadius: 2,
        transition: "width 0.3s ease",
      }} title={`${data.percent.toFixed(0)}% coverage (${data.covered}/${data.total} lines)`} />
    </div>
  );
}

const TEST_FILE_PATTERNS = [
  /\.test\.(ts|tsx|js|jsx)$/,
  /\.spec\.(ts|tsx|js|jsx)$/,
  /_test\.(ts|tsx|js|jsx|py|rs)$/,
  /^test_.*\.py$/,
];

function isTestFile(name: string): boolean {
  return TEST_FILE_PATTERNS.some((re) => re.test(name));
}

// Simple regex extraction of test cases from file content
function parseTestCases(content: string): { describes: DescribeBlock[]; topLevel: TestCase[] } {
  const describes: DescribeBlock[] = [];
  const topLevel: TestCase[] = [];

  // Match describe blocks
  const describeRe = /(?:describe|suite)\s*\(\s*['"`](.*?)['"`]/g;
  let dMatch;
  while ((dMatch = describeRe.exec(content)) !== null) {
    describes.push({ name: dMatch[1], tests: [] });
  }

  // Match test cases: it(, test(, #[test], fn test_
  const testRe = /(?:it|test)\s*\(\s*['"`](.*?)['"`]|#\[test\].*?fn\s+(\w+)|fn\s+(test_\w+)/g;
  let tMatch;
  while ((tMatch = testRe.exec(content)) !== null) {
    const name = tMatch[1] ?? tMatch[2] ?? tMatch[3] ?? "unknown";
    topLevel.push({ name, status: "pending" });
  }

  return { describes, topLevel };
}

async function findTestFiles(rootPath: string): Promise<TestFile[]> {
  const results: TestFile[] = [];
  const skipDirs = new Set(["node_modules", ".git", "target", "dist", "build"]);

  async function walk(dirPath: string, depth: number): Promise<void> {
    if (depth > 5) return;
    try {
      const entries = await invoke<FileEntry[]>("read_directory", { path: dirPath });
      for (const entry of entries) {
        if (entry.is_dir) {
          if (!skipDirs.has(entry.name)) {
            await walk(entry.path, depth + 1);
          }
        } else if (isTestFile(entry.name)) {
          try {
            const content = await invoke<string>("read_file_content", { path: entry.path });
            const { describes, topLevel } = parseTestCases(content);
            results.push({
              path: entry.path,
              name: entry.name,
              describes,
              topLevelTests: topLevel,
            });
          } catch {
            results.push({ path: entry.path, name: entry.name, describes: [], topLevelTests: [] });
          }
        }
      }
    } catch { /* skip unreadable dirs */ }
  }

  await walk(rootPath, 0);
  return results;
}

const statusIcon = (status: TestStatus) => {
  switch (status) {
    case "pass": return <span style={{ color: "#a6e3a1" }}>✓</span>;
    case "fail": return <span style={{ color: "#f38ba8" }}>✗</span>;
    case "running": return <span style={{ color: "#fab387" }}>◐</span>;
    default: return <span style={{ color: "var(--text-muted)" }}>○</span>;
  }
};

type PanelTab = "tests" | "snapshots";

interface SnapshotResult {
  name: string;
  status: "pass" | "fail" | "updated";
  received?: string;
  expected?: string;
}

const TestExplorerPanel = memo(function TestExplorerPanel({
  rootPath, visible = true,
}: TestExplorerPanelProps) {
  const [activeTab, setActiveTab] = useState<PanelTab>("tests");
  const [testFiles, setTestFiles] = useState<TestFile[]>([]);
  const [loading, setLoading] = useState(false);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [coverageMap, setCoverageMap] = useState<Map<string, CoverageData>>(new Map());
  const [generatingFor, setGeneratingFor] = useState<string | null>(null);
  const [testPreview, setTestPreview] = useState<GeneratedTestPreview | null>(null);
  const [acceptingPreview, setAcceptingPreview] = useState(false);

  // Snapshot tab state
  const [snapshotResults, setSnapshotResults] = useState<SnapshotResult[]>([]);
  const [snapshotRunning, setSnapshotRunning] = useState(false);
  const [snapshotError, setSnapshotError] = useState<string | null>(null);
  const [selectedSnapshot, setSelectedSnapshot] = useState<SnapshotResult | null>(null);
  const [updatingSnapshots, setUpdatingSnapshots] = useState(false);
  const [updatingSnapshotName, setUpdatingSnapshotName] = useState<string | null>(null);

  const loadTests = useCallback(async () => {
    if (!rootPath || !visible) return;
    setLoading(true);
    try {
      const files = await findTestFiles(rootPath);
      setTestFiles(files);
      // Expand all by default
      setExpanded(new Set(files.map((f) => f.path)));
    } catch { /* ignore */ }
    setLoading(false);
  }, [rootPath, visible]);

  useEffect(() => { loadTests(); }, [loadTests]);

  // Listen for coverage events
  useEffect(() => {
    const unsub = listen<CoverageData[]>("test-coverage", (e) => {
      const map = new Map<string, CoverageData>();
      for (const cov of e.payload) {
        map.set(cov.file, cov);
      }
      setCoverageMap(map);
    });
    return () => { unsub.then((fn) => fn()); };
  }, []);

  useEffect(() => {
    const unsub = listen<TestResultEvent>("test-result", (e) => {
      const { testName, status, file } = e.payload;
      setTestFiles((prev) =>
        prev.map((tf) => {
          if (file && !tf.path.includes(file)) return tf;
          return {
            ...tf,
            topLevelTests: tf.topLevelTests.map((t) =>
              t.name === testName ? { ...t, status } : t
            ),
            describes: tf.describes.map((d) => ({
              ...d,
              tests: d.tests.map((t) =>
                t.name === testName ? { ...t, status } : t
              ),
            })),
          };
        })
      );
    });
    return () => { unsub.then((fn) => fn()); };
  }, []);

  const runTest = async (filter: string) => {
    try {
      await invoke("run_tests", { root: rootPath, filter });
    } catch { /* ignore if command not available */ }
  };

  const runSnapshotTests = async () => {
    if (!rootPath) return;
    setSnapshotRunning(true);
    setSnapshotError(null);
    setSnapshotResults([]);
    setSelectedSnapshot(null);
    try {
      const results = await invoke<SnapshotResult[]>("run_snapshot_tests", { projectDir: rootPath });
      setSnapshotResults(results ?? []);
    } catch (err) {
      setSnapshotError(String(err));
    }
    setSnapshotRunning(false);
  };

  const updateAllSnapshots = async () => {
    if (!rootPath) return;
    setUpdatingSnapshots(true);
    try {
      await invoke("update_snapshots", { projectDir: rootPath, testName: null });
      await runSnapshotTests();
    } catch { /* ignore */ }
    setUpdatingSnapshots(false);
  };

  const updateSnapshot = async (name: string) => {
    if (!rootPath) return;
    setUpdatingSnapshotName(name);
    try {
      await invoke("update_snapshots", { projectDir: rootPath, testName: name });
      setSnapshotResults((prev) =>
        prev.map((s) => s.name === name ? { ...s, status: "updated" as const } : s)
      );
    } catch { /* ignore */ }
    setUpdatingSnapshotName(null);
  };

  const generateTests = async (filePath: string, functionName: string) => {
    const ext = filePath.split(".").pop() ?? "ts";
    const langMap: Record<string, string> = {
      ts: "typescript", tsx: "typescript", js: "javascript", jsx: "javascript",
      py: "python", rs: "rust",
    };
    const language = langMap[ext] ?? ext;
    const key = `${filePath}:${functionName}`;
    setGeneratingFor(key);
    try {
      const result = await invoke<string>("ai_generate_tests", { filePath, functionName, language });
      // Compute target path: foo.ts -> foo.test.ts
      const targetPath = filePath.replace(/(\.[^./]+)$/, ".test$1");
      let existingContent = "";
      try {
        existingContent = await invoke<string>("read_file_content", { path: targetPath });
      } catch { /* file doesn't exist yet */ }
      setTestPreview({ filePath, functionName, generatedContent: existingContent + "\n" + result, existingContent, targetPath });
    } catch (err) {
      console.error("Failed to generate tests:", err);
    }
    setGeneratingFor(null);
  };

  const acceptTestPreview = async () => {
    if (!testPreview) return;
    setAcceptingPreview(true);
    try {
      await invoke("write_file_content", { path: testPreview.targetPath, content: testPreview.generatedContent });
      setTestPreview(null);
    } catch (err) {
      console.error("Failed to write test file:", err);
    }
    setAcceptingPreview(false);
  };

  const toggleExpand = (path: string) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };

  if (!visible) return null;

  // Test preview modal overlay
  if (testPreview) {
    return (
      <div style={{ height: "100%", display: "flex", flexDirection: "column", background: "var(--bg-primary)", color: "var(--text-primary)" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "6px 10px", borderBottom: "1px solid var(--border-color)", flexShrink: 0 }}>
          <span style={{ fontWeight: 700, color: "var(--accent-hover)", fontSize: 12 }}>Generated Tests for: {testPreview.functionName}</span>
          <div style={{ flex: 1 }} />
          <button
            onClick={() => void acceptTestPreview()}
            disabled={acceptingPreview}
            style={{ fontSize: 11, padding: "2px 8px", borderRadius: 4, border: "none", background: "#238636", color: "#fff", cursor: "pointer" }}
          >
            {acceptingPreview ? "Writing..." : "Accept"}
          </button>
          <button
            onClick={() => setTestPreview(null)}
            style={{ fontSize: 11, padding: "2px 8px", borderRadius: 4, border: "none", background: "#da3633", color: "#fff", cursor: "pointer" }}
          >
            Discard
          </button>
        </div>
        <div style={{ flex: 1, overflow: "hidden" }}>
          <InlineDiffPreview
            originalContent={testPreview.existingContent}
            proposedContent={testPreview.generatedContent}
            filePath={testPreview.targetPath}
            onAccept={() => void acceptTestPreview()}
            onReject={() => setTestPreview(null)}
            onAcceptHunk={() => { /* not needed here */ }}
          />
        </div>
      </div>
    );
  }

  return (
    <div
      style={{
        background: "var(--bg-primary)",
        color: "var(--text-primary)",
        height: "100%",
        display: "flex",
        flexDirection: "column",
        fontFamily: "monospace",
        fontSize: "12px",
      }}
    >
      {/* Tab headers */}
      <div style={{ display: "flex", borderBottom: "1px solid var(--border-color)", flexShrink: 0 }}>
        {(["tests", "snapshots"] as PanelTab[]).map((tab) => (
          <button
            key={tab}
            onClick={() => setActiveTab(tab)}
            style={{
              background: "transparent",
              border: "none",
              borderBottom: activeTab === tab ? "2px solid #89b4fa" : "2px solid transparent",
              color: activeTab === tab ? "var(--accent-hover)" : "var(--text-muted)",
              padding: "6px 12px",
              cursor: "pointer",
              fontSize: 11,
              fontWeight: activeTab === tab ? 700 : 400,
              fontFamily: "monospace",
              textTransform: "capitalize",
            }}
          >
            {tab === "tests" ? "Tests" : "Snapshots"}
          </button>
        ))}
      </div>

      {/* Snapshots tab */}
      {activeTab === "snapshots" && (
        <div style={{ flex: 1, overflowY: "auto", padding: 8, display: "flex", flexDirection: "column", gap: 6 }}>
          <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
            <button
              onClick={runSnapshotTests}
              disabled={snapshotRunning}
              style={{ background: "var(--bg-hover)", border: "1px solid var(--border-color)", color: "#a6e3a1", borderRadius: 4, padding: "2px 8px", cursor: "pointer", fontSize: 11 }}
            >
              {snapshotRunning ? "Running..." : "Run Snapshot Tests"}
            </button>
            {snapshotResults.length > 0 && (
              <button
                onClick={updateAllSnapshots}
                disabled={updatingSnapshots}
                style={{ background: "var(--bg-hover)", border: "1px solid var(--border-color)", color: "var(--accent-hover)", borderRadius: 4, padding: "2px 8px", cursor: "pointer", fontSize: 11 }}
              >
                {updatingSnapshots ? "Updating..." : "Update Snapshots"}
              </button>
            )}
          </div>
          {snapshotError && (
            <div style={{ color: "#f38ba8", fontSize: 11 }}>Error: {snapshotError}</div>
          )}
          {snapshotResults.length === 0 && !snapshotRunning && !snapshotError && (
            <div style={{ color: "var(--text-muted)", padding: 4 }}>Run snapshot tests to see results.</div>
          )}
          {snapshotResults.map((snap) => (
            <div key={snap.name} style={{ background: "var(--bg-secondary)", border: "1px solid var(--border-color)", borderRadius: 4, padding: "6px 8px" }}>
              <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                {snap.status === "pass" && <span style={{ color: "#a6e3a1" }}>✓</span>}
                {snap.status === "fail" && <span style={{ color: "#f38ba8" }}>✗</span>}
                {snap.status === "updated" && <span style={{ color: "var(--accent-hover)" }}>↻</span>}
                <span style={{ flex: 1, color: "var(--text-primary)", fontSize: 11 }}>{snap.name}</span>
                {snap.status === "fail" && (
                  <>
                    <button
                      onClick={() => setSelectedSnapshot(selectedSnapshot?.name === snap.name ? null : snap)}
                      style={{ background: "transparent", border: "1px solid var(--border-color)", borderRadius: 3, color: "var(--accent-hover)", cursor: "pointer", fontSize: 10, padding: "1px 5px" }}
                    >
                      {selectedSnapshot?.name === snap.name ? "Hide Diff" : "Diff"}
                    </button>
                    <button
                      onClick={() => void updateSnapshot(snap.name)}
                      disabled={updatingSnapshotName === snap.name}
                      style={{ background: "transparent", border: "1px solid #fab387", borderRadius: 3, color: "#fab387", cursor: "pointer", fontSize: 10, padding: "1px 5px" }}
                    >
                      {updatingSnapshotName === snap.name ? "..." : "Update This"}
                    </button>
                  </>
                )}
              </div>
              {selectedSnapshot?.name === snap.name && snap.expected != null && snap.received != null && (
                <div style={{ marginTop: 6, height: 200 }}>
                  <DiffEditor
                    original={snap.expected}
                    modified={snap.received}
                    language="text"
                    theme="vs-dark"
                    options={{ readOnly: true, minimap: { enabled: false }, fontSize: 11 }}
                  />
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {/* Tests tab */}
      {activeTab === "tests" && (
      <div style={{ flex: 1, overflowY: "auto", padding: "8px" }}>
      <div style={{ display: "flex", alignItems: "center", gap: "8px", marginBottom: "8px" }}>
        <span style={{ fontWeight: "bold", color: "var(--accent-hover)" }}>Test Explorer</span>
        <button
          onClick={loadTests}
          disabled={loading}
          style={{
            background: "var(--bg-hover)",
            border: "1px solid var(--border-color)",
            color: "var(--text-primary)",
            borderRadius: "4px",
            padding: "2px 8px",
            cursor: "pointer",
            fontSize: "11px",
          }}
        >
          {loading ? "Scanning..." : "Refresh"}
        </button>
        <button
          onClick={() => runTest("")}
          style={{
            background: "var(--bg-hover)",
            border: "1px solid var(--border-color)",
            color: "#a6e3a1",
            borderRadius: "4px",
            padding: "2px 8px",
            cursor: "pointer",
            fontSize: "11px",
          }}
        >
          ▶ Run All
        </button>
      </div>

      {testFiles.length === 0 && !loading && (
        <div style={{ color: "var(--text-muted)", padding: "8px" }}>No test files found.</div>
      )}

      {testFiles.map((tf) => (
        <div key={tf.path} style={{ marginBottom: "4px" }}>
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "4px",
              padding: "3px 4px",
              background: "var(--bg-hover)",
              borderRadius: "4px",
              cursor: "pointer",
              userSelect: "none",
            }}
            onClick={() => toggleExpand(tf.path)}
          >
            <span style={{ color: "var(--accent)", fontSize: "10px" }}>
              {expanded.has(tf.path) ? "▼" : "▶"}
            </span>
            <span style={{ color: "#fab387" }}>{tf.name}</span>
            <span style={{ color: "var(--text-muted)", fontSize: "10px", marginLeft: "auto" }}>
              {tf.topLevelTests.length + tf.describes.reduce((s, d) => s + d.tests.length, 0)} tests
            </span>
            <button
              onClick={(e) => { e.stopPropagation(); void generateTests(tf.path, tf.name); }}
              disabled={generatingFor !== null}
              style={{
                background: "transparent",
                border: "1px solid var(--border-color)",
                borderRadius: 4,
                color: "var(--accent-hover)",
                cursor: "pointer",
                fontSize: "10px",
                padding: "0 4px",
              }}
              title="Generate tests with AI"
            >
              {generatingFor === `${tf.path}:${tf.name}` ? "..." : "Gen"}
            </button>
            <button
              onClick={(e) => { e.stopPropagation(); runTest(tf.name); }}
              style={{
                background: "transparent",
                border: "none",
                color: "#a6e3a1",
                cursor: "pointer",
                fontSize: "11px",
                padding: "0 4px",
              }}
              title="Run file tests"
            >
              ▶
            </button>
          </div>
          <CoverageBar file={tf.path} coverageMap={coverageMap} />

          {expanded.has(tf.path) && (
            <div style={{ paddingLeft: "16px" }}>
              {tf.describes.map((desc, di) => (
                <div key={di} style={{ marginTop: "2px" }}>
                  <div style={{ color: "var(--accent-hover)", padding: "2px 0" }}>
                    <span style={{ fontSize: "10px" }}>▸ </span>{desc.name}
                  </div>
                  {desc.tests.map((t, ti) => (
                    <div
                      key={ti}
                      style={{
                        display: "flex",
                        alignItems: "center",
                        gap: "4px",
                        paddingLeft: "12px",
                        padding: "1px 0 1px 12px",
                      }}
                    >
                      {statusIcon(t.status)}
                      <span style={{ color: "var(--text-primary)" }}>{t.name}</span>
                      <button
                        onClick={() => runTest(t.name)}
                        style={{
                          background: "transparent",
                          border: "none",
                          color: "#a6e3a1",
                          cursor: "pointer",
                          fontSize: "10px",
                          padding: "0 4px",
                          marginLeft: "auto",
                        }}
                      >
                        ▶
                      </button>
                    </div>
                  ))}
                </div>
              ))}

              {tf.topLevelTests.map((t, ti) => (
                <div
                  key={ti}
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: "4px",
                    padding: "1px 0",
                  }}
                >
                  {statusIcon(t.status)}
                  <span style={{ color: "var(--text-primary)" }}>{t.name}</span>
                  <button
                    onClick={() => runTest(t.name)}
                    style={{
                      background: "transparent",
                      border: "none",
                      color: "#a6e3a1",
                      cursor: "pointer",
                      fontSize: "10px",
                      padding: "0 4px",
                      marginLeft: "auto",
                    }}
                  >
                    ▶
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>
      ))}
      </div>
      )}
    </div>
  );
});

export default TestExplorerPanel;
