import { useState, useCallback } from "react";
import { useTheme } from "../contexts/ThemeContext";

interface ThemeEditorProps {
  onClose: () => void;
}

const COLOR_KEYS = [
  "background", "surface", "border", "text", "textMuted",
  "accent", "accentHover", "error", "warning", "success",
  "editorBackground", "editorForeground",
] as const;

type ColorKey = typeof COLOR_KEYS[number];

const CSS_VAR_MAP: Record<ColorKey, string> = {
  background: "--theme-background",
  surface: "--theme-surface",
  border: "--theme-border",
  text: "--theme-text",
  textMuted: "--theme-text-muted",
  accent: "--theme-accent",
  accentHover: "--theme-accent-hover",
  error: "--theme-error",
  warning: "--theme-warning",
  success: "--theme-success",
  editorBackground: "--theme-editor-bg",
  editorForeground: "--theme-editor-fg",
};

const LABEL_MAP: Record<ColorKey, string> = {
  background: "Background",
  surface: "Surface",
  border: "Border",
  text: "Text",
  textMuted: "Text Muted",
  accent: "Accent",
  accentHover: "Accent Hover",
  error: "Error",
  warning: "Warning",
  success: "Success",
  editorBackground: "Editor Background",
  editorForeground: "Editor Foreground",
};

const CUSTOM_THEME_KEY = "shadow-ide-custom-theme";

export default function ThemeEditor({ onClose }: ThemeEditorProps) {
  const { colors } = useTheme();

  // Local mutable color state (starts from current theme)
  const [localColors, setLocalColors] = useState<Record<ColorKey, string>>(() => {
    // Try loading saved custom theme first
    try {
      const saved = localStorage.getItem(CUSTOM_THEME_KEY);
      if (saved) {
        const parsed = JSON.parse(saved) as { colors?: Record<ColorKey, string> };
        if (parsed.colors) return parsed.colors;
      }
    } catch { /* ignore */ }
    return { ...colors } as Record<ColorKey, string>;
  });

  const handleColorChange = useCallback((key: ColorKey, value: string) => {
    setLocalColors((prev) => {
      const updated = { ...prev, [key]: value };
      // Immediately apply to DOM
      document.documentElement.style.setProperty(CSS_VAR_MAP[key], value);
      return updated;
    });
  }, []);

  const handleSave = useCallback(() => {
    try {
      localStorage.setItem(CUSTOM_THEME_KEY, JSON.stringify({ name: "Custom", colors: localColors }));
    } catch { /* ignore */ }
  }, [localColors]);

  const handleReset = useCallback(() => {
    // Revert to base theme colors from context
    for (const key of COLOR_KEYS) {
      const baseColor = (colors as Record<ColorKey, string>)[key];
      document.documentElement.style.setProperty(CSS_VAR_MAP[key], baseColor);
    }
    setLocalColors({ ...colors } as Record<ColorKey, string>);
  }, [colors]);

  const handleExport = useCallback(() => {
    const json = JSON.stringify({ name: "Custom", colors: localColors }, null, 2);
    const blob = new Blob([json], { type: "application/json" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = "shadow-ide-theme.json";
    a.click();
    URL.revokeObjectURL(url);
  }, [localColors]);

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label="Theme Editor"
      style={{
        position: "fixed",
        inset: 0,
        background: "rgba(0,0,0,0.6)",
        zIndex: 20000,
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
      }}
      onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div
        style={{
          background: "#1e1e2e",
          border: "1px solid #313244",
          borderRadius: 8,
          width: 420,
          maxWidth: "95vw",
          maxHeight: "85vh",
          display: "flex",
          flexDirection: "column",
          overflow: "hidden",
          boxShadow: "0 20px 60px rgba(0,0,0,0.5)",
          fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
          color: "#cdd6f4",
        }}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div style={{ padding: "10px 14px", borderBottom: "1px solid #313244", display: "flex", justifyContent: "space-between", alignItems: "center", flexShrink: 0 }}>
          <span style={{ fontWeight: 700, fontSize: 13, color: "#89b4fa" }}>Customize Theme</span>
          <button
            onClick={onClose}
            style={{ background: "transparent", border: "none", color: "#6c7086", cursor: "pointer", fontSize: 16, padding: "0 4px" }}
            aria-label="Close theme editor"
          >
            ×
          </button>
        </div>

        {/* Color grid */}
        <div style={{ flex: 1, overflowY: "auto", padding: "12px 14px" }}>
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 10 }}>
            {COLOR_KEYS.map((key) => (
              <label key={key} style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer" }}>
                <input
                  type="color"
                  value={localColors[key]}
                  onChange={(e) => handleColorChange(key, e.target.value)}
                  style={{
                    width: 28,
                    height: 28,
                    border: "1px solid #45475a",
                    borderRadius: 4,
                    padding: 2,
                    cursor: "pointer",
                    background: "transparent",
                    flexShrink: 0,
                  }}
                  aria-label={LABEL_MAP[key]}
                />
                <span style={{ fontSize: 11, color: "#a6adc8", userSelect: "none" }}>
                  {LABEL_MAP[key]}
                </span>
              </label>
            ))}
          </div>
        </div>

        {/* Footer buttons */}
        <div style={{ padding: "10px 14px", borderTop: "1px solid #313244", display: "flex", gap: 8, flexShrink: 0 }}>
          <button
            onClick={handleSave}
            style={{ background: "#313244", border: "1px solid #45475a", color: "#cdd6f4", borderRadius: 4, padding: "5px 12px", cursor: "pointer", fontSize: 12, fontFamily: "inherit" }}
          >
            Save as Custom
          </button>
          <button
            onClick={handleReset}
            style={{ background: "transparent", border: "1px solid #45475a", color: "#a6adc8", borderRadius: 4, padding: "5px 12px", cursor: "pointer", fontSize: 12, fontFamily: "inherit" }}
          >
            Reset
          </button>
          <button
            onClick={handleExport}
            style={{ background: "transparent", border: "1px solid #45475a", color: "#a6adc8", borderRadius: 4, padding: "5px 12px", cursor: "pointer", fontSize: 12, fontFamily: "inherit" }}
          >
            Export .json
          </button>
        </div>
      </div>
    </div>
  );
}
