import React, { useEffect, useState, useRef, useCallback, memo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useFocusTrap } from "../hooks/useFocusTrap";

interface Command {
  id: string;
  label: string;
  category: string;
  keybinding?: string;
  handler?: () => void;
}

const BUILTIN_COMMANDS: Command[] = [
  // File
  { id: "file.newFile", label: "New File", category: "File", keybinding: "Ctrl+N" },
  { id: "file.openProject", label: "Open Project Folder", category: "File" },
  { id: "file.saveFile", label: "Save File", category: "File", keybinding: "Ctrl+S" },
  // Edit
  { id: "edit.undo", label: "Undo", category: "Edit", keybinding: "Ctrl+Z" },
  { id: "edit.redo", label: "Redo", category: "Edit", keybinding: "Ctrl+Y" },
  { id: "edit.find", label: "Find in Files", category: "Edit", keybinding: "Ctrl+Shift+F" },
  // AI
  { id: "ai.chat", label: "Open AI Chat", category: "AI", keybinding: "Ctrl+Shift+A" },
  { id: "ai.completeCode", label: "Complete Code (AI)", category: "AI" },
  { id: "ai.explainSelection", label: "Explain Selection", category: "AI" },
  // Git
  { id: "git.status", label: "Git: Show Status", category: "Git" },
  { id: "git.commit", label: "Git: Commit", category: "Git" },
  { id: "git.push", label: "Git: Push", category: "Git" },
  { id: "git.graph", label: "Git: Show Graph", category: "Git" },
  // View
  { id: "view.toggleTerminal", label: "Toggle Terminal", category: "View", keybinding: "Ctrl+`" },
  { id: "view.toggleSidebar", label: "Toggle Sidebar", category: "View" },
  { id: "view.explorerPanel", label: "Show Explorer", category: "View" },
  { id: "view.searchPanel", label: "Show Search", category: "View", keybinding: "Ctrl+Shift+F" },
  { id: "view.aiPanel", label: "Show AI Panel", category: "View", keybinding: "Ctrl+Shift+A" },
  { id: "view.gamedevPanel", label: "Show ShadowEditor Game Panel", category: "View" },
  { id: "view.planenginePanel", label: "Show PlanEngine Roadmap", category: "View" },
  { id: "view.planengineAudit", label: "Show PlanEngine Finish Audit", category: "View" },
  { id: "view.planengineBuildWorkflow", label: "PlanEngine: Build Runtime Workflow", category: "View" },
  { id: "view.planengineReflectWorkflow", label: "PlanEngine: Reflection Workflow", category: "View" },
  { id: "view.planengineAiWorkflow", label: "PlanEngine: AI Workflow", category: "View" },
  { id: "view.planengineViewportWorkflow", label: "PlanEngine: Viewport Workflow", category: "View" },
  { id: "view.gitGraph", label: "Show Git Graph", category: "View" },
  { id: "view.testExplorer", label: "Show Test Explorer", category: "View" },
  { id: "view.agentPanel", label: "Show Agent Panel", category: "View" },
  // Terminal
  { id: "terminal.new", label: "New Terminal", category: "Terminal" },
  { id: "terminal.clear", label: "Clear Terminal", category: "Terminal" },
  // Settings
  { id: "settings.open", label: "Open Settings", category: "Settings" },
  { id: "settings.theme", label: "Change Theme", category: "Settings" },
  // Editor
  { id: "editor.toggleBlame", label: "Toggle Git Blame", category: "Editor", handler: () => window.dispatchEvent(new CustomEvent("editor-toggle-blame")) },
];

function fuzzyMatch(query: string, label: string): boolean {
  if (!query) return true;
  const q = query.toLowerCase();
  const l = label.toLowerCase();
  if (l.includes(q)) return true;
  // Simple character-order fuzzy match
  let qi = 0;
  for (let li = 0; li < l.length && qi < q.length; li++) {
    if (l[li] === q[qi]) qi++;
  }
  return qi === q.length;
}

const CATEGORY_COLORS: Record<string, string> = {
  File: "#89b4fa",
  Edit: "#a6e3a1",
  AI: "#cba6f7",
  Git: "#f38ba8",
  View: "#fab387",
  Terminal: "#f9e2af",
  Settings: "#94e2d5",
};

interface CommandPaletteProps {
  onCommandExecute?: (commandId: string) => void;
}

const CommandPalette = memo(function CommandPalette({ onCommandExecute }: CommandPaletteProps) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [commands, setCommands] = useState<Command[]>(BUILTIN_COMMANDS);
  const inputRef = useRef<HTMLInputElement>(null);
  const focusTrapRef = useFocusTrap(open);

  // Load additional commands from backend (if available)
  useEffect(() => {
    if (!open) return;
    invoke<{ id: string; label: string; category: string }[]>("get_all_commands")
      .then((cmds) => {
        setCommands([
          ...BUILTIN_COMMANDS,
          ...cmds.map((c) => ({ ...c, handler: undefined })),
        ]);
      })
      .catch(() => { /* command may not exist */ });
  }, [open]);

  // Global Ctrl+Shift+P listener
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.ctrlKey && e.shiftKey && e.key === "P") {
        e.preventDefault();
        setOpen((prev) => !prev);
        setQuery("");
        setSelectedIndex(0);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, []);

  // Focus input when opened
  useEffect(() => {
    if (open) {
      setTimeout(() => inputRef.current?.focus(), 50);
    }
  }, [open]);

  const filteredCommands = commands.filter((c) =>
    fuzzyMatch(query, c.label) || fuzzyMatch(query, c.category)
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (e.key === "Escape") {
        setOpen(false);
      } else if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndex((i) => (i >= filteredCommands.length - 1 ? 0 : i + 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex((i) => (i <= 0 ? filteredCommands.length - 1 : i - 1));
      } else if (e.key === "Home") {
        e.preventDefault();
        setSelectedIndex(0);
      } else if (e.key === "End") {
        e.preventDefault();
        setSelectedIndex(filteredCommands.length - 1);
      } else if (e.key === "Enter") {
        e.preventDefault();
        const cmd = filteredCommands[selectedIndex];
        if (cmd) executeCommand(cmd);
      }
    },
    [filteredCommands, selectedIndex]
  );

  const executeCommand = useCallback(
    (cmd: Command) => {
      setOpen(false);
      if (cmd.handler) {
        cmd.handler();
      } else if (onCommandExecute) {
        onCommandExecute(cmd.id);
      } else {
        invoke("execute_command", { commandId: cmd.id }).catch(() => {
          // Command may not exist on backend; ignore
        });
      }
    },
    [onCommandExecute]
  );

  if (!open) return null;

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Command Palette"
      data-testid="command-palette"
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0,0,0,0.6)",
        zIndex: 10000,
        display: "flex",
        alignItems: "flex-start",
        justifyContent: "center",
        paddingTop: "15vh",
      }}
      onClick={() => setOpen(false)}
    >
      <div
        ref={focusTrapRef}
        style={{
          background: "#1e1e2e",
          border: "1px solid #45475a",
          borderRadius: "8px",
          width: "560px",
          maxWidth: "90vw",
          maxHeight: "60vh",
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
          boxShadow: "0 20px 60px rgba(0,0,0,0.5)",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Input */}
        <div style={{ padding: "10px 12px", borderBottom: "1px solid #313244" }}>
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => {
              setQuery(e.target.value);
              setSelectedIndex(0);
            }}
            onKeyDown={handleKeyDown}
            placeholder="Type a command..."
            aria-label="Search commands"
            aria-autocomplete="list"
            style={{
              width: "100%",
              background: "transparent",
              border: "none",
              outline: "none",
              color: "#cdd6f4",
              fontSize: "14px",
              fontFamily: "monospace",
            }}
          />
        </div>

        {/* Results */}
        <div style={{ overflowY: "auto", flex: 1 }}>
          {filteredCommands.length === 0 && (
            <div style={{ padding: "16px", color: "#6c7086", textAlign: "center", fontSize: "12px" }}>
              No commands found
            </div>
          )}
          {filteredCommands.map((cmd, i) => (
            <div
              key={cmd.id}
              role="option"
              aria-selected={i === selectedIndex}
              style={{
                display: "flex",
                alignItems: "center",
                gap: "8px",
                padding: "8px 12px",
                background: i === selectedIndex ? "#313244" : "transparent",
                cursor: "pointer",
                borderLeft: i === selectedIndex ? "2px solid #89b4fa" : "2px solid transparent",
              }}
              onMouseEnter={() => setSelectedIndex(i)}
              onClick={() => executeCommand(cmd)}
            >
              <span
                style={{
                  background: CATEGORY_COLORS[cmd.category] ?? "#6c7086",
                  color: "#1e1e2e",
                  borderRadius: "3px",
                  padding: "1px 5px",
                  fontSize: "9px",
                  fontWeight: "bold",
                  flexShrink: 0,
                  minWidth: "50px",
                  textAlign: "center",
                }}
              >
                {cmd.category}
              </span>
              <span style={{ flex: 1, color: "#cdd6f4", fontSize: "13px" }}>{cmd.label}</span>
              {cmd.keybinding && (
                <span
                  style={{
                    color: "#6c7086",
                    fontSize: "10px",
                    fontFamily: "monospace",
                    background: "#181825",
                    padding: "1px 5px",
                    borderRadius: "3px",
                    border: "1px solid #45475a",
                    flexShrink: 0,
                  }}
                >
                  {cmd.keybinding}
                </span>
              )}
            </div>
          ))}
        </div>

        {/* Footer hint */}
        <div
          style={{
            padding: "6px 12px",
            borderTop: "1px solid #313244",
            fontSize: "10px",
            color: "#6c7086",
            display: "flex",
            gap: "12px",
          }}
        >
          <span>↑↓ Navigate</span>
          <span>Enter Execute</span>
          <span>Esc Close</span>
        </div>
      </div>
    </div>
  );
});

export default CommandPalette;
