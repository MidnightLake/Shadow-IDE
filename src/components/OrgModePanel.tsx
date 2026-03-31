import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import Editor from "@monaco-editor/react";

interface OrgHeading {
  id: string;
  level: number;
  text: string;
  todo?: string;
  tags?: string[];
  collapsed: boolean;
  children: OrgHeading[];
  body: string;
}

interface OrgModePanelProps {
  filePath: string;
}

const TODO_COLORS: Record<string, string> = {
  TODO: "#f38ba8",
  DONE: "#a6e3a1",
  "IN-PROGRESS": "#f9e2af",
  WAITING: "#fab387",
  CANCELLED: "#6c7086",
};

function parseOrgHeadings(content: string): OrgHeading[] {
  const lines = content.split("\n");
  const root: OrgHeading[] = [];
  const stack: OrgHeading[] = [];
  let idCounter = 0;

  const flush = () => { /* body accumulated below */ };

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const headingMatch = line.match(/^(\*+)\s+(.*)/);
    if (headingMatch) {
      const level = headingMatch[1].length;
      let rest = headingMatch[2];

      // Extract TODO keyword
      let todo: string | undefined;
      const todoMatch = rest.match(/^(TODO|DONE|IN-PROGRESS|WAITING|CANCELLED)\s+(.*)/);
      if (todoMatch) {
        todo = todoMatch[1];
        rest = todoMatch[2];
      }

      // Extract tags :tag1:tag2:
      let tags: string[] | undefined;
      const tagsMatch = rest.match(/^(.*?)\s+:([\w:]+):\s*$/);
      if (tagsMatch) {
        rest = tagsMatch[1].trim();
        tags = tagsMatch[2].split(":").filter(Boolean);
      }

      flush();

      const heading: OrgHeading = {
        id: `h${idCounter++}`,
        level,
        text: rest.trim(),
        todo,
        tags,
        collapsed: false,
        children: [],
        body: "",
      };

      // Pop stack until we find a parent
      while (stack.length > 0 && stack[stack.length - 1].level >= level) {
        stack.pop();
      }

      if (stack.length === 0) {
        root.push(heading);
      } else {
        stack[stack.length - 1].children.push(heading);
      }
      stack.push(heading);
    } else {
      // Body line — append to current heading
      if (stack.length > 0) {
        const h = stack[stack.length - 1];
        h.body = h.body ? h.body + "\n" + line : line;
      }
    }
  }

  return root;
}

function serializeOrg(headings: OrgHeading[], level = 1): string {
  let out = "";
  for (const h of headings) {
    const stars = "*".repeat(h.level ?? level);
    const todo = h.todo ? `${h.todo} ` : "";
    const tags = h.tags && h.tags.length ? ` :${h.tags.join(":")}:` : "";
    out += `${stars} ${todo}${h.text}${tags}\n`;
    if (h.body) out += h.body + "\n";
    if (h.children.length) out += serializeOrg(h.children, (h.level ?? level) + 1);
  }
  return out;
}

function toggleCheckbox(body: string, lineIndex: number): string {
  const lines = body.split("\n");
  lines[lineIndex] = lines[lineIndex].replace(
    /^(\s*- \[)([ X])(\].*)$/,
    (_, pre, check, post) => `${pre}${check === " " ? "X" : " "}${post}`
  );
  return lines.join("\n");
}

export default function OrgModePanel({ filePath }: OrgModePanelProps) {
  const [headings, setHeadings] = useState<OrgHeading[]>([]);
  const [rawContent, setRawContent] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const [editingBodyId, setEditingBodyId] = useState<string | null>(null);

  useEffect(() => {
    invoke<string>("read_file_content", { path: filePath })
      .then((content) => {
        setRawContent(content);
        setHeadings(parseOrgHeadings(content));
      })
      .catch((e) => setError(String(e)));
  }, [filePath]);

  const save = useCallback(async (hs: OrgHeading[]) => {
    setSaving(true);
    try {
      const content = serializeOrg(hs);
      await invoke("write_file_content", { path: filePath, content });
      setRawContent(content);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }, [filePath]);

  const toggleCollapse = useCallback((id: string) => {
    const toggle = (hs: OrgHeading[]): OrgHeading[] =>
      hs.map((h) =>
        h.id === id
          ? { ...h, collapsed: !h.collapsed }
          : { ...h, children: toggle(h.children) }
      );
    setHeadings((prev) => toggle(prev));
  }, []);

  const updateBody = useCallback((id: string, body: string) => {
    const update = (hs: OrgHeading[]): OrgHeading[] =>
      hs.map((h) =>
        h.id === id ? { ...h, body } : { ...h, children: update(h.children) }
      );
    setHeadings((prev) => {
      const next = update(prev);
      save(next);
      return next;
    });
  }, [save]);

  const toggleCheck = useCallback((id: string, lineIndex: number) => {
    const update = (hs: OrgHeading[]): OrgHeading[] =>
      hs.map((h) =>
        h.id === id
          ? { ...h, body: toggleCheckbox(h.body, lineIndex) }
          : { ...h, children: update(h.children) }
      );
    setHeadings((prev) => {
      const next = update(prev);
      save(next);
      return next;
    });
  }, [save]);

  if (error) {
    return (
      <div style={{ padding: 16, color: "#f38ba8", fontFamily: "monospace" }}>
        Error: {error}
      </div>
    );
  }

  if (!rawContent && headings.length === 0) {
    return (
      <div style={{ padding: 16, color: "#6c7086", fontFamily: "monospace" }}>
        Loading...
      </div>
    );
  }

  return (
    <div style={{ height: "100%", overflowY: "auto", background: "#1e1e2e", color: "#cdd6f4", fontFamily: "monospace", fontSize: 13 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "8px 12px", borderBottom: "1px solid #313244", position: "sticky", top: 0, background: "#1e1e2e", zIndex: 10 }}>
        <span style={{ fontWeight: "bold", color: "#89b4fa", fontSize: 12 }}>
          {filePath.split("/").pop()}
        </span>
        <button
          onClick={() => save(headings)}
          disabled={saving}
          style={{ background: "#313244", color: saving ? "#6c7086" : "#f9e2af", border: "1px solid #45475a", borderRadius: 4, padding: "3px 10px", fontSize: 11, cursor: "pointer", marginLeft: "auto" }}
        >
          {saving ? "Saving..." : "Save"}
        </button>
      </div>
      <div style={{ padding: "8px 12px" }}>
        {headings.length === 0 && (
          <div style={{ color: "#6c7086", padding: 12 }}>No headings found.</div>
        )}
        {headings.map((h) => (
          <HeadingView
            key={h.id}
            heading={h}
            editingBodyId={editingBodyId}
            onToggleCollapse={toggleCollapse}
            onUpdateBody={updateBody}
            onToggleCheck={toggleCheck}
            onEditBody={(id) => setEditingBodyId(id)}
            onBlurBody={() => setEditingBodyId(null)}
          />
        ))}
      </div>
    </div>
  );
}

interface HeadingViewProps {
  heading: OrgHeading;
  editingBodyId: string | null;
  onToggleCollapse: (id: string) => void;
  onUpdateBody: (id: string, body: string) => void;
  onToggleCheck: (id: string, lineIndex: number) => void;
  onEditBody: (id: string) => void;
  onBlurBody: () => void;
}

function HeadingView({
  heading,
  editingBodyId,
  onToggleCollapse,
  onUpdateBody,
  onToggleCheck,
  onEditBody,
  onBlurBody,
}: HeadingViewProps) {
  const indent = (heading.level - 1) * 16;
  const hasContent = heading.body || heading.children.length > 0;

  return (
    <div style={{ marginBottom: 2 }}>
      <div
        style={{
          display: "flex",
          alignItems: "center",
          gap: 6,
          paddingLeft: indent,
          padding: `4px 8px 4px ${indent + 8}px`,
          cursor: "pointer",
          borderRadius: 4,
          background: "transparent",
        }}
        onMouseEnter={(e) => (e.currentTarget.style.background = "#313244")}
        onMouseLeave={(e) => (e.currentTarget.style.background = "transparent")}
        onClick={() => hasContent && onToggleCollapse(heading.id)}
      >
        {hasContent && (
          <span style={{ color: "#6c7086", fontSize: 10, width: 12, textAlign: "center", flexShrink: 0 }}>
            {heading.collapsed ? "▶" : "▼"}
          </span>
        )}
        {!hasContent && <span style={{ width: 12, flexShrink: 0 }} />}
        <span style={{ color: "#89b4fa", fontWeight: "bold", fontSize: 11 + (7 - heading.level) }}>
          {"*".repeat(heading.level)}
        </span>
        {heading.todo && (
          <span style={{
            background: TODO_COLORS[heading.todo] ?? "#6c7086",
            color: "#1e1e2e",
            borderRadius: 3,
            padding: "1px 5px",
            fontSize: 10,
            fontWeight: 700,
            flexShrink: 0,
          }}>
            {heading.todo}
          </span>
        )}
        <span style={{ flex: 1, color: "#cdd6f4", fontWeight: heading.level === 1 ? 700 : 400 }}>
          {heading.text}
        </span>
        {heading.tags && heading.tags.length > 0 && (
          <div style={{ display: "flex", gap: 3, flexShrink: 0 }}>
            {heading.tags.map((t) => (
              <span key={t} style={{ background: "#45475a", color: "#a6adc8", borderRadius: 8, padding: "1px 6px", fontSize: 10 }}>
                {t}
              </span>
            ))}
          </div>
        )}
      </div>

      {!heading.collapsed && (
        <>
          {heading.body && (
            <div style={{ paddingLeft: indent + 24, paddingRight: 8, marginBottom: 4 }}>
              {editingBodyId === heading.id ? (
                <div style={{ position: "relative" }}>
                  <Editor
                    height="120px"
                    language="markdown"
                    value={heading.body}
                    theme="vs-dark"
                    onChange={(v) => onUpdateBody(heading.id, v ?? "")}
                    options={{
                      minimap: { enabled: false },
                      lineNumbers: "off",
                      fontSize: 12,
                      wordWrap: "on",
                      scrollBeyondLastLine: false,
                    }}
                  />
                  <button
                    onClick={onBlurBody}
                    style={{ position: "absolute", top: 4, right: 4, background: "#45475a", color: "#cdd6f4", border: "none", borderRadius: 3, padding: "2px 8px", fontSize: 10, cursor: "pointer", zIndex: 5 }}
                  >
                    Done
                  </button>
                </div>
              ) : (
                <div
                  style={{ padding: "4px 0", color: "#a6adc8", fontSize: 12, lineHeight: 1.6, cursor: "text" }}
                  onClick={() => onEditBody(heading.id)}
                  title="Click to edit body"
                >
                  <BodyRenderer body={heading.body} headingId={heading.id} onToggleCheck={onToggleCheck} />
                </div>
              )}
            </div>
          )}

          {heading.children.map((child) => (
            <HeadingView
              key={child.id}
              heading={child}
              editingBodyId={editingBodyId}
              onToggleCollapse={onToggleCollapse}
              onUpdateBody={onUpdateBody}
              onToggleCheck={onToggleCheck}
              onEditBody={onEditBody}
              onBlurBody={onBlurBody}
            />
          ))}
        </>
      )}
    </div>
  );
}

function BodyRenderer({
  body,
  headingId,
  onToggleCheck,
}: {
  body: string;
  headingId: string;
  onToggleCheck: (id: string, lineIndex: number) => void;
}) {
  const lines = body.split("\n");
  return (
    <div>
      {lines.map((line, i) => {
        const checkboxMatch = line.match(/^(\s*)- \[([ X])\](.*)$/);
        if (checkboxMatch) {
          const checked = checkboxMatch[2] === "X";
          return (
            <div key={i} style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <input
                type="checkbox"
                checked={checked}
                onChange={() => onToggleCheck(headingId, i)}
                style={{ cursor: "pointer", accentColor: "#89b4fa" }}
              />
              <span style={{ textDecoration: checked ? "line-through" : "none", color: checked ? "#6c7086" : "#a6adc8" }}>
                {checkboxMatch[3]}
              </span>
            </div>
          );
        }
        return (
          <div key={i} style={{ whiteSpace: "pre-wrap" }}>
            {line || "\u00a0"}
          </div>
        );
      })}
    </div>
  );
}
