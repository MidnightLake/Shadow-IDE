import React, { useState, useMemo } from "react";

interface Keybinding {
  command: string;
  keys: string;
  category: string;
  description: string;
}

const KEYBINDINGS: Keybinding[] = [
  // Editor
  { command: "Save File", keys: "Ctrl+S", category: "Editor", description: "Save the current file" },
  { command: "Undo", keys: "Ctrl+Z", category: "Editor", description: "Undo last change" },
  { command: "Redo", keys: "Ctrl+Y", category: "Editor", description: "Redo last undone change" },
  { command: "Toggle Comment", keys: "Ctrl+/", category: "Editor", description: "Toggle line comment" },
  { command: "Select Next Occurrence", keys: "Ctrl+D", category: "Editor", description: "Add selection to next match" },
  { command: "Go to Line", keys: "Ctrl+G", category: "Editor", description: "Navigate to a specific line" },
  { command: "Go to Definition", keys: "F12", category: "Editor", description: "Jump to symbol definition" },
  { command: "Find References", keys: "Shift+F12", category: "Editor", description: "Show all references to symbol" },
  { command: "Rename Symbol", keys: "F2", category: "Editor", description: "Rename symbol under cursor" },
  // AI
  { command: "AI Action", keys: "Ctrl+K", category: "AI", description: "Open AI action popup for selected text" },
  { command: "AI Chat", keys: "Ctrl+Shift+A", category: "AI", description: "Open AI chat panel" },
  { command: "Generate Docs", keys: "Alt+D", category: "AI", description: "Generate documentation for symbol" },
  // View
  { command: "Quick Open", keys: "Ctrl+P", category: "View", description: "Quick open files by name" },
  { command: "Command Palette", keys: "Ctrl+Shift+P", category: "View", description: "Open command palette" },
  { command: "Toggle Terminal", keys: "Ctrl+`", category: "View", description: "Show or hide the terminal" },
  { command: "Toggle Sidebar", keys: "Ctrl+B", category: "View", description: "Show or hide the sidebar" },
  { command: "Explorer Panel", keys: "Ctrl+Shift+E", category: "View", description: "Show file explorer panel" },
  { command: "Search Panel", keys: "Ctrl+Shift+F", category: "View", description: "Open project-wide search" },
  // Git
  { command: "Git Panel", keys: "Ctrl+Shift+G", category: "Git", description: "Open Git graph panel" },
  // Debug
  { command: "Debug Panel", keys: "Ctrl+Shift+D", category: "Debug", description: "Open debug panel" },
  // Test
  { command: "Test Explorer", keys: "Ctrl+Shift+T", category: "Test", description: "Open test explorer panel" },
  // Extensions
  { command: "Extensions", keys: "Ctrl+Shift+X", category: "View", description: "Open extensions / languages panel" },
  // Remote
  { command: "Remote Settings", keys: "Ctrl+Shift+R", category: "View", description: "Open remote settings panel" },
];

/** Split a key combo string into individual key chips */
function parseKeys(keys: string): string[] {
  return keys.split("+").map((k) => k.trim());
}

function KeyChip({ label }: { label: string }) {
  return (
    <span style={{
      display: "inline-block",
      padding: "1px 6px",
      marginRight: 2,
      background: "var(--bg-surface, #181825)",
      border: "1px solid var(--border-color, #45475a)",
      borderBottom: "3px solid var(--border-color, #313244)",
      borderRadius: 4,
      fontSize: 11,
      fontFamily: "system-ui, sans-serif",
      color: "var(--text-secondary, #bac2de)",
      lineHeight: "1.5",
    }}>
      {label}
    </span>
  );
}

export default function KeybindingsPanel() {
  const [query, setQuery] = useState("");

  const categories = useMemo(() => {
    const filtered = query.trim()
      ? KEYBINDINGS.filter((kb) =>
          kb.command.toLowerCase().includes(query.toLowerCase()) ||
          kb.keys.toLowerCase().includes(query.toLowerCase()) ||
          kb.description.toLowerCase().includes(query.toLowerCase())
        )
      : KEYBINDINGS;

    const map = new Map<string, Keybinding[]>();
    for (const kb of filtered) {
      const arr = map.get(kb.category) ?? [];
      arr.push(kb);
      map.set(kb.category, arr);
    }
    return map;
  }, [query]);

  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column", fontFamily: "monospace", fontSize: 12, color: "var(--text-primary, #cdd6f4)" }}>
      {/* Header */}
      <div style={{ padding: "8px 10px", borderBottom: "1px solid var(--border-color, #313244)", flexShrink: 0 }}>
        <div style={{ fontWeight: 700, color: "var(--accent, #89b4fa)", marginBottom: 6 }}>Keybindings</div>
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search commands, keys, or descriptions..."
          style={{
            width: "100%",
            fontSize: 12,
            padding: "4px 8px",
            background: "var(--bg-secondary, #181825)",
            border: "1px solid var(--border-color, #313244)",
            borderRadius: 4,
            color: "var(--text-primary, #cdd6f4)",
            outline: "none",
            boxSizing: "border-box",
          }}
        />
      </div>

      <div style={{ flex: 1, overflowY: "auto" }}>
        {categories.size === 0 && (
          <div style={{ padding: 16, color: "var(--text-muted)", textAlign: "center" }}>No keybindings match your search.</div>
        )}

        {Array.from(categories.entries()).map(([category, bindings]) => (
          <div key={category}>
            <div style={{
              padding: "4px 10px",
              background: "var(--bg-surface, #181825)",
              fontSize: 10,
              fontWeight: 700,
              color: "var(--accent, #89b4fa)",
              textTransform: "uppercase",
              letterSpacing: "0.08em",
              borderBottom: "1px solid var(--border-color, #313244)",
              position: "sticky",
              top: 0,
            }}>
              {category}
            </div>
            {bindings.map((kb) => (
              <div key={kb.command} style={{
                display: "flex",
                alignItems: "center",
                gap: 8,
                padding: "5px 10px",
                borderBottom: "1px solid var(--border-color, #1e1e2e)",
              }}>
                <div style={{ flex: 1 }}>
                  <div style={{ color: "var(--text-primary, #cdd6f4)", marginBottom: 1 }}>{kb.command}</div>
                  <div style={{ fontSize: 10, color: "var(--text-muted, #6c7086)" }}>{kb.description}</div>
                </div>
                <div style={{ flexShrink: 0 }}>
                  {parseKeys(kb.keys).map((key, i) => (
                    <React.Fragment key={i}>
                      {i > 0 && <span style={{ fontSize: 10, color: "var(--text-muted)", margin: "0 1px" }}>+</span>}
                      <KeyChip label={key} />
                    </React.Fragment>
                  ))}
                </div>
              </div>
            ))}
          </div>
        ))}
      </div>
    </div>
  );
}
