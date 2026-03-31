import React, { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { summarizePlanengineMarkdown, type PlanengineShellSummary } from "../planengine/summary";

interface ShadowTemplate {
  id: string;
  name: string;
  description: string;
}

interface Props {
  onClose: () => void;
  onCreated: (projectPath: string) => void;
}

interface ShadowPlanengineDocs {
  plan_path: string;
  finish_path: string;
  plan_markdown: string;
  finish_markdown: string;
  finish_available: boolean;
}

const TEMPLATE_ORDER = ["3d_platformer", "2d_rpg", "empty_3d", "empty_2d"];

export default function NewProjectWizard({ onClose, onCreated }: Props) {
  const [templates, setTemplates] = useState<ShadowTemplate[]>([]);
  const [selectedTemplate, setSelectedTemplate] = useState<string>("3d_platformer");
  const [projectName, setProjectName] = useState("MyGame");
  const [parentDir, setParentDir] = useState<string>("");
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [planSummary, setPlanSummary] = useState<PlanengineShellSummary | null>(null);

  useEffect(() => {
    invoke<ShadowTemplate[]>("shadow_list_templates")
      .then(setTemplates)
      .catch(() => {
        setTemplates([
          { id: "empty_3d", name: "Empty 3D", description: "Bare C++23 game library with C ABI exports and reflection." },
          { id: "3d_platformer", name: "3D Platformer", description: "Player controller, camera, health component, and a test scene." },
          { id: "empty_2d", name: "Empty 2D", description: "Minimal 2D setup with orthographic camera component." },
          { id: "2d_rpg", name: "2D RPG", description: "Top-down player, NPC dialogue, and a simple 2D RPG starter scene." },
        ]);
      });
  }, []);

  useEffect(() => {
    invoke<ShadowPlanengineDocs>("shadow_load_planengine_docs")
      .then((docs) => {
        setPlanSummary(summarizePlanengineMarkdown(docs.plan_markdown));
      })
      .catch(() => {
        setPlanSummary(null);
      });
  }, []);

  const pickDir = async () => {
    const dir = await open({ directory: true, multiple: false, title: "Choose parent directory" });
    if (typeof dir === "string") setParentDir(dir);
  };

  const handleCreate = async () => {
    if (!parentDir) { setError("Please choose a parent directory."); return; }
    if (!projectName.trim()) { setError("Project name cannot be empty."); return; }
    setError(null);
    setCreating(true);
    try {
      const path = await invoke<string>("shadow_new_project", {
        parentDir,
        name: projectName.trim(),
        template: selectedTemplate,
      });
      onCreated(path);
    } catch (e) {
      setError(String(e));
    } finally {
      setCreating(false);
    }
  };

  const orderedTemplates = [...templates].sort((a, b) => {
    const indexA = TEMPLATE_ORDER.indexOf(a.id);
    const indexB = TEMPLATE_ORDER.indexOf(b.id);
    const safeA = indexA === -1 ? Number.MAX_SAFE_INTEGER : indexA;
    const safeB = indexB === -1 ? Number.MAX_SAFE_INTEGER : indexB;
    return safeA - safeB || a.name.localeCompare(b.name);
  });

  const selectedTemplateInfo = orderedTemplates.find((template) => template.id === selectedTemplate) ?? null;
  const selectedTemplateNote = selectedTemplate === "3d_platformer"
    ? "Best roadmap-aligned starter for the live viewport, gameplay loop, and build/runtime flow."
    : selectedTemplate === "2d_rpg"
      ? "Good plan-aligned starter for 2D authoring, scene work, and gameplay iteration."
      : selectedTemplate === "empty_3d"
        ? "Minimal 3D foundation if you want to build the runtime and viewport flow from scratch."
        : "Minimal 2D foundation for a leaner roadmap start.";

  const TEMPLATE_ICONS: Record<string, React.ReactNode> = {
    empty_3d: (
      <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
        <path d="M12 2L2 7l10 5 10-5-10-5z"/><path d="M2 17l10 5 10-5"/><path d="M2 12l10 5 10-5"/>
      </svg>
    ),
    "3d_platformer": (
      <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
        <circle cx="12" cy="8" r="3"/><path d="M6 20v-2a6 6 0 0 1 12 0v2"/>
        <rect x="2" y="18" width="8" height="3" rx="1"/><rect x="14" y="18" width="8" height="3" rx="1"/>
      </svg>
    ),
    empty_2d: (
      <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
        <rect x="3" y="3" width="18" height="18" rx="2"/><path d="M3 9h18M9 21V9"/>
      </svg>
    ),
    "2d_rpg": (
      <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
        <path d="M12 3v18"/><path d="M5 8l7-5 7 5"/><path d="M8 21v-6h8v6"/><circle cx="7" cy="13" r="1.5"/><circle cx="17" cy="13" r="1.5"/>
      </svg>
    ),
  };

  return (
    <div style={styles.overlay} onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div style={styles.modal}>
        <div style={styles.header}>
          <span style={styles.title}>New ShadowEditor Project</span>
          <button style={styles.closeBtn} onClick={onClose} aria-label="Close">✕</button>
        </div>

        <div style={styles.body}>
          <div style={styles.planIntroCard}>
            <div style={styles.planIntroKicker}>PlanEngine First Launch</div>
            <div style={styles.planIntroTitle}>Start The Way `planengine.md` Describes</div>
            <div style={styles.planIntroCopy}>
              This wizard now follows the roadmap’s startup loop: pick a template, generate the C++23 project files, then continue into AI setup and the live viewport workflow inside ShadowIDE.
            </div>
            {planSummary?.firstLaunchSteps && planSummary.firstLaunchSteps.length > 0 && (
              <div style={styles.planFlowList}>
                {planSummary.firstLaunchSteps.slice(1, 6).map((step) => (
                  <div key={step} style={styles.planFlowItem}>{step}</div>
                ))}
              </div>
            )}
            <div style={styles.planArtifactWrap}>
              {(planSummary?.firstLaunchArtifacts.length ? planSummary.firstLaunchArtifacts : [
                "C++23 src/ with example component + system",
                "compile_commands.json (auto-generated)",
                ".shadow scene file",
                ".shadow_project.toml (build config)",
              ]).map((artifact) => (
                <div key={artifact} style={styles.planArtifactChip}>{artifact}</div>
              ))}
            </div>
          </div>

          {/* Template picker */}
          <div style={styles.label}>Template</div>
          <div style={styles.templateGrid}>
            {orderedTemplates.map((t) => (
              <button
                key={t.id}
                style={{
                  ...styles.templateCard,
                  ...(selectedTemplate === t.id ? styles.templateCardActive : {}),
                }}
                onClick={() => setSelectedTemplate(t.id)}
              >
                <div style={styles.templateBadge}>
                  {TEMPLATE_ORDER.indexOf(t.id) <= 1 ? "Plan Starter" : "Foundation"}
                </div>
                <div style={{ ...styles.templateIcon, color: selectedTemplate === t.id ? "var(--accent, #6366f1)" : "var(--text-muted, #6b7280)" }}>
                  {TEMPLATE_ICONS[t.id] ?? TEMPLATE_ICONS["empty_3d"]}
                </div>
                <div style={styles.templateName}>{t.name}</div>
                <div style={styles.templateDesc}>{t.description}</div>
              </button>
            ))}
          </div>
          {selectedTemplateInfo && (
            <div style={styles.templatePlanNote}>
              <strong>{selectedTemplateInfo.name}:</strong> {selectedTemplateNote}
            </div>
          )}

          {/* Project name */}
          <div style={styles.label}>Project Name</div>
          <input
            style={styles.input}
            value={projectName}
            onChange={(e) => setProjectName(e.target.value)}
            placeholder="MyGame"
            spellCheck={false}
          />

          {/* Parent directory */}
          <div style={styles.label}>Location</div>
          <div style={styles.dirRow}>
            <input
              style={{ ...styles.input, flex: 1 }}
              value={parentDir}
              onChange={(e) => setParentDir(e.target.value)}
              placeholder="/home/user/projects"
              spellCheck={false}
            />
            <button style={styles.browseBtn} onClick={pickDir}>Browse…</button>
          </div>

          {parentDir && projectName && (
            <div style={styles.pathPreview}>
              Will create: <code style={{ color: "var(--accent, #6366f1)" }}>{parentDir}/{projectName}</code>
            </div>
          )}

          {error && <div style={styles.error}>{error}</div>}
        </div>

        <div style={styles.footer}>
          <button style={styles.cancelBtn} onClick={onClose}>Cancel</button>
          <button
            style={{ ...styles.createBtn, opacity: creating ? 0.5 : 1 }}
            disabled={creating}
            onClick={handleCreate}
          >
            {creating ? "Creating…" : "Create Project"}
          </button>
        </div>
      </div>
    </div>
  );
}

const styles: Record<string, React.CSSProperties> = {
  overlay: {
    position: "fixed", inset: 0, background: "rgba(0,0,0,0.6)",
    display: "flex", alignItems: "center", justifyContent: "center", zIndex: 9999,
  },
  modal: {
    background: "var(--bg-secondary, #111827)", border: "1px solid var(--border-color, #1f2937)",
    borderRadius: 8, width: 560, maxWidth: "95vw", maxHeight: "90vh",
    display: "flex", flexDirection: "column", overflow: "hidden",
    boxShadow: "0 20px 60px rgba(0,0,0,0.5)",
  },
  header: {
    display: "flex", alignItems: "center", justifyContent: "space-between",
    padding: "14px 18px", borderBottom: "1px solid var(--border-color, #1f2937)",
  },
  title: { fontSize: 15, fontWeight: 700, color: "var(--text-primary, #f9fafb)" },
  closeBtn: {
    background: "transparent", border: "none", color: "var(--text-muted, #6b7280)",
    fontSize: 16, cursor: "pointer", padding: "2px 6px", borderRadius: 4,
  },
  body: { flex: 1, overflowY: "auto", padding: "18px 18px 12px" },
  planIntroCard: {
    padding: "14px 14px 12px",
    borderRadius: 8,
    border: "1px solid rgba(233,170,95,0.2)",
    background: "linear-gradient(180deg, rgba(233,170,95,0.08), rgba(17,24,39,0.55))",
    marginBottom: 12,
  },
  planIntroKicker: {
    fontSize: 10,
    fontWeight: 700,
    color: "#e9aa5f",
    textTransform: "uppercase",
    letterSpacing: 1,
    marginBottom: 4,
  },
  planIntroTitle: {
    fontSize: 14,
    fontWeight: 700,
    color: "var(--text-primary, #f9fafb)",
    marginBottom: 6,
  },
  planIntroCopy: {
    fontSize: 11,
    color: "var(--text-secondary, #94a3b8)",
    lineHeight: 1.55,
    marginBottom: 10,
  },
  planFlowList: {
    display: "flex",
    flexDirection: "column",
    gap: 6,
    marginBottom: 10,
  },
  planFlowItem: {
    fontSize: 11,
    color: "var(--text-primary, #f9fafb)",
    lineHeight: 1.45,
  },
  planArtifactWrap: {
    display: "flex",
    flexWrap: "wrap",
    gap: 6,
  },
  planArtifactChip: {
    padding: "4px 8px",
    borderRadius: 999,
    border: "1px solid rgba(142,184,212,0.18)",
    background: "rgba(10,14,23,0.45)",
    color: "var(--text-secondary, #94a3b8)",
    fontSize: 10,
    fontWeight: 600,
  },
  footer: {
    display: "flex", justifyContent: "flex-end", gap: 8,
    padding: "12px 18px", borderTop: "1px solid var(--border-color, #1f2937)",
  },
  label: { fontSize: 11, fontWeight: 600, color: "var(--text-muted, #6b7280)", textTransform: "uppercase", letterSpacing: 0.8, marginBottom: 8, marginTop: 14 },
  templateGrid: { display: "grid", gridTemplateColumns: "repeat(2, minmax(0, 1fr))", gap: 8, marginBottom: 4 },
  templateCard: {
    background: "var(--bg-primary, #0a0e17)", border: "1px solid var(--border-color, #1f2937)",
    borderRadius: 6, padding: "12px 8px", cursor: "pointer", textAlign: "left",
    display: "flex", flexDirection: "column", gap: 4, transition: "border-color 0.15s",
    position: "relative",
  },
  templateCardActive: { borderColor: "var(--accent, #6366f1)", background: "rgba(99,102,241,0.06)" },
  templateBadge: {
    alignSelf: "flex-start",
    padding: "2px 7px",
    borderRadius: 999,
    border: "1px solid rgba(142,184,212,0.18)",
    background: "rgba(142,184,212,0.08)",
    color: "var(--text-secondary, #94a3b8)",
    fontSize: 9,
    fontWeight: 700,
    letterSpacing: 0.4,
    textTransform: "uppercase",
    marginBottom: 4,
  },
  templateIcon: { marginBottom: 4 },
  templateName: { fontSize: 12, fontWeight: 600, color: "var(--text-primary, #f9fafb)" },
  templateDesc: { fontSize: 10, color: "var(--text-muted, #6b7280)", lineHeight: 1.4 },
  templatePlanNote: {
    fontSize: 11,
    color: "var(--text-secondary, #94a3b8)",
    lineHeight: 1.5,
    marginTop: 8,
    padding: "8px 10px",
    borderRadius: 6,
    border: "1px solid var(--border-color, #1f2937)",
    background: "rgba(30,41,59,0.28)",
  },
  input: {
    width: "100%", background: "var(--bg-primary, #0a0e17)", border: "1px solid var(--border-color, #1f2937)",
    borderRadius: 4, color: "var(--text-primary, #f9fafb)", fontSize: 13, padding: "7px 10px",
    outline: "none", boxSizing: "border-box",
  },
  dirRow: { display: "flex", gap: 8, alignItems: "stretch" },
  browseBtn: {
    padding: "7px 12px", background: "var(--bg-hover, #1f2937)", border: "1px solid var(--border-color, #1f2937)",
    borderRadius: 4, color: "var(--text-primary, #f9fafb)", cursor: "pointer", fontSize: 12, flexShrink: 0,
  },
  pathPreview: { fontSize: 11, color: "var(--text-muted, #6b7280)", marginTop: 8 },
  error: { fontSize: 12, color: "#f87171", marginTop: 10, padding: "8px 10px", background: "rgba(248,113,113,0.08)", borderRadius: 4 },
  cancelBtn: {
    padding: "7px 16px", background: "transparent", border: "1px solid var(--border-color, #1f2937)",
    borderRadius: 4, color: "var(--text-muted, #6b7280)", cursor: "pointer", fontSize: 13,
  },
  createBtn: {
    padding: "7px 20px", background: "var(--accent, #6366f1)", border: "none",
    borderRadius: 4, color: "#fff", cursor: "pointer", fontSize: 13, fontWeight: 600,
  },
};
