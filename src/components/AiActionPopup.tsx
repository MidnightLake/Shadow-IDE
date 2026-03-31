import React, { useEffect, useRef, useState, useCallback } from "react";

interface AiActionPopupProps {
  selectedText: string;
  language: string;
  position: { x: number; y: number };
  onClose: () => void;
  onAction: (action: string, selectedText: string) => void;
  multiCursorNote?: string;
}

type MenuItem =
  | { type: "action"; label: string; action: string }
  | { type: "submenu"; label: string; children: Array<{ label: string; action: string }> }
  | { type: "custom" };

const TRANSLATE_LANGS = ["Python", "TypeScript", "Rust", "Go", "Java"] as const;

const MENU_ITEMS: MenuItem[] = [
  { type: "action", label: "Explain", action: "explain" },
  { type: "action", label: "Refactor", action: "refactor" },
  { type: "action", label: "Add tests", action: "add_tests" },
  { type: "action", label: "Fix bugs", action: "fix_bugs" },
  { type: "action", label: "Add docs", action: "add_docs" },
  {
    type: "submenu",
    label: "Translate to\u2026",
    children: TRANSLATE_LANGS.map((lang) => ({
      label: lang,
      action: `translate_to_${lang.toLowerCase()}`,
    })),
  },
  { type: "custom" },
];

export default function AiActionPopup({
  selectedText,
  language,
  position,
  onClose,
  onAction,
  multiCursorNote,
}: AiActionPopupProps) {
  const popupRef = useRef<HTMLDivElement>(null);
  const customInputRef = useRef<HTMLInputElement>(null);
  const [activeIndex, setActiveIndex] = useState(0);
  const [openSubmenu, setOpenSubmenu] = useState<number | null>(null);
  const [customPrompt, setCustomPrompt] = useState("");

  // Flat list of focusable items for keyboard navigation
  type FlatItem = { label: string; action: string } | { type: "custom" };
  const flatItems: FlatItem[] = MENU_ITEMS.map(
    (item): FlatItem => {
      if (item.type === "action") return { label: item.label, action: item.action };
      if (item.type === "submenu") return { label: item.label, action: `__submenu_${MENU_ITEMS.indexOf(item)}` };
      return { type: "custom" as const };
    }
  );

  const handleSelect = useCallback(
    (action: string) => {
      if (action.startsWith("__submenu_")) {
        const idx = parseInt(action.slice("__submenu_".length), 10);
        setOpenSubmenu((prev) => (prev === idx ? null : idx));
      } else {
        onAction(action, selectedText);
      }
    },
    [onAction, selectedText]
  );

  const handleCustomSubmit = useCallback(() => {
    if (customPrompt.trim()) {
      onAction(`custom:${customPrompt.trim()}`, selectedText);
    }
  }, [customPrompt, onAction, selectedText]);

  // Close on outside click
  useEffect(() => {
    const handleClick = (e: MouseEvent) => {
      if (popupRef.current && !popupRef.current.contains(e.target as Node)) {
        onClose();
      }
    };
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [onClose]);

  // Keyboard navigation
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        if (openSubmenu !== null) {
          setOpenSubmenu(null);
        } else {
          onClose();
        }
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setActiveIndex((prev) => (prev + 1) % flatItems.length);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setActiveIndex((prev) => (prev - 1 + flatItems.length) % flatItems.length);
      } else if (e.key === "Enter") {
        e.preventDefault();
        const item = flatItems[activeIndex];
        if (!item) return;
        if ("type" in item && item.type === "custom") {
          customInputRef.current?.focus();
        } else if ("action" in item) {
          handleSelect(item.action);
        }
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [activeIndex, flatItems, handleSelect, onClose, openSubmenu]);

  // Clamp position to viewport
  const popupWidth = 220;
  const clampedX = Math.min(position.x, window.innerWidth - popupWidth - 8);
  const clampedY = position.y;

  return (
    <div
      ref={popupRef}
      role="menu"
      aria-label="AI actions"
      style={{
        position: "fixed",
        left: clampedX,
        top: clampedY,
        zIndex: 9999,
        width: popupWidth,
        background: "#1e1e2e",
        border: "1px solid #3d3d5c",
        borderRadius: 8,
        boxShadow: "0 8px 32px rgba(0,0,0,0.5), 0 2px 8px rgba(0,0,0,0.3)",
        padding: "4px 0",
        fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
        fontSize: 13,
        color: "#cdd6f4",
        userSelect: "none",
      }}
    >
      {/* Header */}
      <div style={{
        padding: "4px 12px 6px",
        borderBottom: "1px solid #313244",
        fontSize: 11,
        color: "#6c7086",
        letterSpacing: "0.05em",
        textTransform: "uppercase",
      }}>
        AI Actions · <span style={{ fontStyle: "italic", color: "#7f849c" }}>{language}</span>
        {multiCursorNote && (
          <div style={{ color: "#fab387", marginTop: 2 }}>{multiCursorNote}</div>
        )}
      </div>

      {MENU_ITEMS.map((item, menuIdx) => {
        const flatIdx = MENU_ITEMS.slice(0, menuIdx).reduce((acc, m) => {
          if (m.type === "action" || m.type === "custom") return acc + 1;
          if (m.type === "submenu") return acc + 1;
          return acc;
        }, 0);

        if (item.type === "action") {
          return (
            <button
              key={item.action}
              role="menuitem"
              style={menuItemStyle(activeIndex === flatIdx)}
              onMouseEnter={() => setActiveIndex(flatIdx)}
              onClick={() => handleSelect(item.action)}
            >
              {item.label}
            </button>
          );
        }

        if (item.type === "submenu") {
          return (
            <div key={item.label} style={{ position: "relative" }}>
              <button
                role="menuitem"
                aria-haspopup="menu"
                aria-expanded={openSubmenu === menuIdx}
                style={{
                  ...menuItemStyle(activeIndex === flatIdx),
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "center",
                }}
                onMouseEnter={() => { setActiveIndex(flatIdx); setOpenSubmenu(menuIdx); }}
                onMouseLeave={() => setOpenSubmenu(null)}
                onClick={() => setOpenSubmenu((prev) => (prev === menuIdx ? null : menuIdx))}
              >
                {item.label}
                <span style={{ opacity: 0.6, fontSize: 10 }}>&#9654;</span>
              </button>
              {openSubmenu === menuIdx && (
                <div
                  role="menu"
                  onMouseEnter={() => setOpenSubmenu(menuIdx)}
                  onMouseLeave={() => setOpenSubmenu(null)}
                  style={{
                    position: "absolute",
                    left: "100%",
                    top: 0,
                    background: "#1e1e2e",
                    border: "1px solid #3d3d5c",
                    borderRadius: 8,
                    boxShadow: "0 8px 24px rgba(0,0,0,0.4)",
                    padding: "4px 0",
                    minWidth: 140,
                    zIndex: 10000,
                  }}
                >
                  {item.children.map((child) => (
                    <button
                      key={child.action}
                      role="menuitem"
                      style={menuItemStyle(false)}
                      onMouseEnter={(e) => { (e.currentTarget as HTMLElement).style.background = "#313244"; }}
                      onMouseLeave={(e) => { (e.currentTarget as HTMLElement).style.background = "transparent"; }}
                      onClick={() => { onAction(child.action, selectedText); }}
                    >
                      {child.label}
                    </button>
                  ))}
                </div>
              )}
            </div>
          );
        }

        // custom prompt
        return (
          <div key="custom" style={{ padding: "6px 8px 4px", borderTop: "1px solid #313244", marginTop: 2 }}>
            <div style={{ display: "flex", gap: 4 }}>
              <input
                ref={customInputRef}
                type="text"
                placeholder="Custom prompt\u2026"
                value={customPrompt}
                onChange={(e) => setCustomPrompt(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") { e.preventDefault(); handleCustomSubmit(); }
                  e.stopPropagation();
                }}
                style={{
                  flex: 1,
                  background: "#313244",
                  border: "1px solid #45475a",
                  borderRadius: 4,
                  color: "#cdd6f4",
                  padding: "3px 6px",
                  fontSize: 12,
                  outline: "none",
                  fontFamily: "inherit",
                }}
              />
              <button
                onClick={handleCustomSubmit}
                style={{
                  background: "#89b4fa",
                  color: "#1e1e2e",
                  border: "none",
                  borderRadius: 4,
                  padding: "3px 8px",
                  fontSize: 12,
                  cursor: "pointer",
                  fontFamily: "inherit",
                  fontWeight: 600,
                }}
              >
                Go
              </button>
            </div>
          </div>
        );
      })}
    </div>
  );
}

function menuItemStyle(active: boolean): React.CSSProperties {
  return {
    display: "block",
    width: "100%",
    textAlign: "left",
    background: active ? "#313244" : "transparent",
    border: "none",
    color: active ? "#cdd6f4" : "#bac2de",
    padding: "5px 12px",
    cursor: "pointer",
    fontFamily: "inherit",
    fontSize: 13,
    transition: "background 0.1s",
  };
}
