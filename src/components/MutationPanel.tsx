import { useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

interface SurvivingMutant {
  file: string;
  line: number;
  operator: string;
  snippet: string;
  testName?: string;
}

interface MutationSummary {
  caught: number;
  survived: number;
  timeout: number;
  score: number;
  mutants: SurvivingMutant[];
}

type RunState = "idle" | "running" | "done" | "error";

interface MutationPanelProps {
  rootPath: string;
  visible?: boolean;
}

function ScoreBadge({ score }: { score: number }) {
  const emoji = score >= 80 ? "🟢" : score >= 60 ? "🟡" : "🔴";
  return (
    <span
      style={{
        fontSize: 11,
        fontWeight: 700,
        padding: "2px 8px",
        borderRadius: 10,
        background: score >= 80 ? "#22543d" : score >= 60 ? "#7b5e1a" : "#6e1a1a",
        color: score >= 80 ? "#68d391" : score >= 60 ? "#f6e05e" : "#fc8181",
      }}
    >
      {emoji} {score.toFixed(0)}%
    </span>
  );
}

function ProgressBar({ score }: { score: number }) {
  const color = score >= 80 ? "#68d391" : score >= 60 ? "#f6e05e" : "#fc8181";
  return (
    <div
      style={{
        height: 8,
        background: "rgba(255,255,255,0.1)",
        borderRadius: 4,
        overflow: "hidden",
        margin: "6px 0",
      }}
    >
      <div
        style={{
          height: "100%",
          width: `${Math.min(100, score)}%`,
          background: color,
          borderRadius: 4,
          transition: "width 0.5s ease",
        }}
      />
    </div>
  );
}

export default function MutationPanel({ rootPath, visible = true }: MutationPanelProps) {
  const [runState, setRunState] = useState<RunState>("idle");
  const [summary, setSummary] = useState<MutationSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [killingMutantIdx, setKillingMutantIdx] = useState<number | null>(null);

  const runMutations = useCallback(async () => {
    if (!rootPath) return;
    setRunState("running");
    setError(null);
    setSummary(null);
    try {
      const result = await invoke<MutationSummary>("run_mutation_tests", {
        projectDir: rootPath,
        targetFile: null,
      });
      setSummary(result);
      setRunState("done");
    } catch (err) {
      setError(String(err));
      setRunState("error");
    }
  }, [rootPath]);

  const openMutantFile = (mutant: SurvivingMutant) => {
    window.dispatchEvent(
      new CustomEvent("editor-open-file", {
        detail: { file: mutant.file, line: mutant.line },
      }),
    );
  };

  const killMutantWithAi = useCallback(
    async (mutant: SurvivingMutant, idx: number) => {
      setKillingMutantIdx(idx);
      try {
        const prompt = `Write a test that kills this surviving mutant:\nFile: ${mutant.file}\nLine: ${mutant.line}\nOperator: ${mutant.operator}\nCode: ${mutant.snippet}\n\nProvide only the test function code.`;
        await invoke("ai_chat_complete", {
          messages: [{ role: "user", content: prompt }],
          model: null,
          maxTokens: 512,
        });
      } catch { /* ignore */ }
      setKillingMutantIdx(null);
    },
    [],
  );

  if (!visible) return null;

  return (
    <div
      style={{
        height: "100%",
        display: "flex",
        flexDirection: "column",
        background: "var(--bg-primary)",
        color: "var(--text-primary)",
        fontFamily: "monospace",
        fontSize: 12,
        overflow: "hidden",
      }}
    >
      {/* Header */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 8,
          padding: "8px 10px",
          borderBottom: "1px solid var(--border-color)",
          flexShrink: 0,
        }}
      >
        <span style={{ fontWeight: 700, color: "var(--accent-hover)", fontSize: 13 }}>
          🧬 Mutation Testing
        </span>
        <div style={{ flex: 1 }} />
        {summary && <ScoreBadge score={summary.score} />}
        <button
          onClick={runMutations}
          disabled={runState === "running"}
          style={{
            fontSize: 11,
            padding: "3px 10px",
            borderRadius: 4,
            border: "1px solid #89b4fa",
            background: runState === "running" ? "var(--bg-hover)" : "transparent",
            color: runState === "running" ? "var(--text-muted)" : "var(--accent-hover)",
            cursor: runState === "running" ? "not-allowed" : "pointer",
            fontWeight: 600,
          }}
        >
          {runState === "running" ? "Running..." : "Run"}
        </button>
      </div>

      <div style={{ flex: 1, overflowY: "auto", padding: 10 }}>
        {runState === "idle" && (
          <div style={{ color: "var(--text-muted)", padding: 8, textAlign: "center" }}>
            Click "Run" to start mutation testing.
          </div>
        )}

        {runState === "running" && (
          <div style={{ color: "#fab387", padding: 8, textAlign: "center" }}>
            Running mutation tests... this may take a while.
          </div>
        )}

        {runState === "error" && error && (
          <div style={{ color: "#f38ba8", padding: 8 }}>Error: {error}</div>
        )}

        {runState === "done" && summary && (
          <>
            {/* Summary */}
            <div
              style={{
                background: "var(--bg-hover)",
                borderRadius: 6,
                padding: "8px 10px",
                marginBottom: 10,
              }}
            >
              <div
                style={{
                  display: "flex",
                  gap: 16,
                  flexWrap: "wrap",
                  marginBottom: 4,
                }}
              >
                <span>
                  <span style={{ color: "#a6e3a1" }}>Caught: </span>
                  <strong>{summary.caught}</strong>
                </span>
                <span>
                  <span style={{ color: "#f38ba8" }}>Survived: </span>
                  <strong>{summary.survived}</strong>
                </span>
                <span>
                  <span style={{ color: "#fab387" }}>Timeout: </span>
                  <strong>{summary.timeout}</strong>
                </span>
                <span>
                  <span style={{ color: "var(--accent-hover)" }}>Score: </span>
                  <strong>{summary.score.toFixed(1)}%</strong>
                </span>
              </div>
              <ProgressBar score={summary.score} />
            </div>

            {/* Surviving mutants */}
            {summary.survived > 0 && (
              <>
                <div
                  style={{
                    fontSize: 11,
                    fontWeight: 700,
                    color: "#f38ba8",
                    marginBottom: 6,
                    textTransform: "uppercase",
                    letterSpacing: "0.06em",
                  }}
                >
                  Surviving Mutants ({summary.survived})
                </div>
                <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                  {summary.mutants.map((mutant, idx) => (
                    <div
                      key={idx}
                      style={{
                        background: "var(--bg-secondary)",
                        border: "1px solid var(--border-color)",
                        borderRadius: 4,
                        padding: "6px 8px",
                        cursor: "pointer",
                      }}
                      onClick={() => openMutantFile(mutant)}
                    >
                      <div
                        style={{
                          display: "flex",
                          alignItems: "center",
                          gap: 6,
                          marginBottom: 3,
                        }}
                      >
                        <span style={{ color: "#fab387", fontSize: 10 }}>
                          {mutant.file.split("/").pop()}:{mutant.line}
                        </span>
                        <span
                          style={{
                            fontSize: 9,
                            background: "var(--bg-hover)",
                            color: "var(--text-primary)",
                            borderRadius: 3,
                            padding: "1px 5px",
                          }}
                        >
                          {mutant.operator}
                        </span>
                        <div style={{ flex: 1 }} />
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            void killMutantWithAi(mutant, idx);
                          }}
                          disabled={killingMutantIdx === idx}
                          style={{
                            fontSize: 9,
                            padding: "1px 6px",
                            borderRadius: 3,
                            border: "1px solid #cba6f7",
                            background: "transparent",
                            color: killingMutantIdx === idx ? "var(--text-muted)" : "var(--accent)",
                            cursor:
                              killingMutantIdx === idx ? "not-allowed" : "pointer",
                            whiteSpace: "nowrap",
                          }}
                        >
                          {killingMutantIdx === idx ? "..." : "Kill Mutant (AI)"}
                        </button>
                      </div>
                      <code
                        style={{
                          fontSize: 10,
                          color: "var(--text-secondary)",
                          display: "block",
                          overflow: "hidden",
                          textOverflow: "ellipsis",
                          whiteSpace: "nowrap",
                        }}
                      >
                        {mutant.snippet}
                      </code>
                    </div>
                  ))}
                </div>
              </>
            )}

            {summary.survived === 0 && (
              <div style={{ color: "#a6e3a1", padding: 8, textAlign: "center" }}>
                All mutants killed! Perfect score.
              </div>
            )}
          </>
        )}
      </div>
    </div>
  );
}
