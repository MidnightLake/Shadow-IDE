import React, { createContext, useContext, useEffect, useState, useCallback, useRef } from "react";
import type { MarketplaceTheme } from "../components/ThemeMarketplace";

export type ColorFilter = "none" | "deuteranopia" | "protanopia" | "tritanopia" | "high-contrast";

const COLOR_FILTER_STORAGE_KEY = "shadow-ide-color-filter";

const CSS_FILTERS: Record<ColorFilter, string> = {
  none: "",
  // eslint-disable-next-line no-useless-escape -- SVG data URI requires these quotes
  deuteranopia: "url(\"data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg'><filter id='d'><feColorMatrix type='matrix' values='0.625 0.375 0 0 0  0.7 0.3 0 0 0  0 0.3 0.7 0 0  0 0 0 1 0'/></filter></svg>#d\")",
  // eslint-disable-next-line no-useless-escape -- SVG data URI requires these quotes
  protanopia: "url(\"data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg'><filter id='p'><feColorMatrix type='matrix' values='0.567 0.433 0 0 0  0.558 0.442 0 0 0  0 0.242 0.758 0 0  0 0 0 1 0'/></filter></svg>#p\")",
  // eslint-disable-next-line no-useless-escape -- SVG data URI requires these quotes
  tritanopia: "url(\"data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg'><filter id='t'><feColorMatrix type='matrix' values='0.95 0.05 0 0 0  0 0.433 0.567 0 0  0 0.475 0.525 0 0  0 0 0 1 0'/></filter></svg>#t\")",
  "high-contrast": "contrast(1.5) saturate(1.2)",
};

export type ThemeName =
  | "dark"
  | "light"
  | "dracula"
  | "nord"
  | "solarized-dark"
  | "solarized-light"
  | "gruvbox"
  | "catppuccin"
  | "tokyo-night"
  | "one-dark-pro"
  | "github-dark"
  | "github-light";

interface ThemeColors {
  background: string;
  surface: string;
  border: string;
  text: string;
  textMuted: string;
  accent: string;
  accentHover: string;
  error: string;
  warning: string;
  success: string;
  editorBackground: string;
  editorForeground: string;
}

export type FontScale = "normal" | "large" | "x-large";

const FONT_SCALE_PX: Record<FontScale, string> = {
  normal: "14px",
  large: "17px",
  "x-large": "20px",
};

const FONT_SCALE_KEY = "shadow-ide-font-scale";

interface ThemeContextValue {
  theme: ThemeName;
  colors: ThemeColors;
  setTheme: (theme: ThemeName) => void;
  monacoTheme: string;
  colorFilter: ColorFilter;
  setColorFilter: (filter: ColorFilter) => void;
  fontSize: FontScale;
  setFontSize: (scale: FontScale) => void;
}

const THEME_DEFINITIONS: Record<ThemeName, ThemeColors> = {
  dark: {
    background: "#1e1e2e",
    surface: "#181825",
    border: "#313244",
    text: "#cdd6f4",
    textMuted: "#6c7086",
    accent: "#89b4fa",
    accentHover: "#b4d0ff",
    error: "#f38ba8",
    warning: "#fab387",
    success: "#a6e3a1",
    editorBackground: "#1e1e2e",
    editorForeground: "#cdd6f4",
  },
  light: {
    background: "#eff1f5",
    surface: "#e6e9ef",
    border: "#bcc0cc",
    text: "#4c4f69",
    textMuted: "#8c8fa1",
    accent: "#1e66f5",
    accentHover: "#0a59e8",
    error: "#d20f39",
    warning: "#df8e1d",
    success: "#40a02b",
    editorBackground: "#eff1f5",
    editorForeground: "#4c4f69",
  },
  dracula: {
    background: "#282a36",
    surface: "#21222c",
    border: "#44475a",
    text: "#f8f8f2",
    textMuted: "#6272a4",
    accent: "#bd93f9",
    accentHover: "#caa9ff",
    error: "#ff5555",
    warning: "#ffb86c",
    success: "#50fa7b",
    editorBackground: "#282a36",
    editorForeground: "#f8f8f2",
  },
  nord: {
    background: "#2e3440",
    surface: "#272c36",
    border: "#3b4252",
    text: "#eceff4",
    textMuted: "#4c566a",
    accent: "#88c0d0",
    accentHover: "#8fbcbb",
    error: "#bf616a",
    warning: "#ebcb8b",
    success: "#a3be8c",
    editorBackground: "#2e3440",
    editorForeground: "#eceff4",
  },
  "solarized-dark": {
    background: "#002b36",
    surface: "#073642",
    border: "#586e75",
    text: "#839496",
    textMuted: "#657b83",
    accent: "#268bd2",
    accentHover: "#2aa198",
    error: "#dc322f",
    warning: "#cb4b16",
    success: "#859900",
    editorBackground: "#002b36",
    editorForeground: "#839496",
  },
  "solarized-light": {
    background: "#fdf6e3",
    surface: "#eee8d5",
    border: "#93a1a1",
    text: "#657b83",
    textMuted: "#93a1a1",
    accent: "#268bd2",
    accentHover: "#2aa198",
    error: "#dc322f",
    warning: "#cb4b16",
    success: "#859900",
    editorBackground: "#fdf6e3",
    editorForeground: "#657b83",
  },
  gruvbox: {
    background: "#282828",
    surface: "#1d2021",
    border: "#3c3836",
    text: "#ebdbb2",
    textMuted: "#928374",
    accent: "#458588",
    accentHover: "#83a598",
    error: "#cc241d",
    warning: "#d79921",
    success: "#98971a",
    editorBackground: "#282828",
    editorForeground: "#ebdbb2",
  },
  catppuccin: {
    background: "#1e1e2e",
    surface: "#181825",
    border: "#313244",
    text: "#cdd6f4",
    textMuted: "#6c7086",
    accent: "#cba6f7",
    accentHover: "#d5bffe",
    error: "#f38ba8",
    warning: "#fab387",
    success: "#a6e3a1",
    editorBackground: "#1e1e2e",
    editorForeground: "#cdd6f4",
  },
  "tokyo-night": {
    background: "#1a1b26",
    surface: "#16161e",
    border: "#292e42",
    text: "#a9b1d6",
    textMuted: "#565f89",
    accent: "#7aa2f7",
    accentHover: "#89b4fa",
    error: "#f7768e",
    warning: "#e0af68",
    success: "#9ece6a",
    editorBackground: "#1a1b26",
    editorForeground: "#a9b1d6",
  },
  "one-dark-pro": {
    background: "#282c34",
    surface: "#21252b",
    border: "#3b4048",
    text: "#abb2bf",
    textMuted: "#5c6370",
    accent: "#61afef",
    accentHover: "#56b6c2",
    error: "#e06c75",
    warning: "#e5c07b",
    success: "#98c379",
    editorBackground: "#282c34",
    editorForeground: "#abb2bf",
  },
  "github-dark": {
    background: "#0d1117",
    surface: "#161b22",
    border: "#30363d",
    text: "#e6edf3",
    textMuted: "#484f58",
    accent: "#388bfd",
    accentHover: "#58a6ff",
    error: "#f85149",
    warning: "#d29922",
    success: "#3fb950",
    editorBackground: "#0d1117",
    editorForeground: "#e6edf3",
  },
  "github-light": {
    background: "#ffffff",
    surface: "#f6f8fa",
    border: "#d0d7de",
    text: "#24292f",
    textMuted: "#57606a",
    accent: "#0969da",
    accentHover: "#0550ae",
    error: "#cf222e",
    warning: "#9a6700",
    success: "#1a7f37",
    editorBackground: "#ffffff",
    editorForeground: "#24292f",
  },
};

const MONACO_THEME_MAP: Record<ThemeName, string> = {
  dark: "vs-dark",
  light: "vs",
  dracula: "vs-dark",
  nord: "vs-dark",
  "solarized-dark": "vs-dark",
  "solarized-light": "vs",
  gruvbox: "vs-dark",
  catppuccin: "vs-dark",
  "tokyo-night": "vs-dark",
  "one-dark-pro": "vs-dark",
  "github-dark": "vs-dark",
  "github-light": "vs",
};

const THEME_STORAGE_KEY = "shadowide-theme";
const CUSTOM_THEMES_KEY = "shadow-custom-themes";

const ThemeContext = createContext<ThemeContextValue | null>(null);

// Map from hex background to rgb triple for glass effect
function hexToRgbTriple(hex: string): string {
  const h = hex.replace("#", "");
  if (h.length === 6) {
    const r = parseInt(h.slice(0, 2), 16);
    const g = parseInt(h.slice(2, 4), 16);
    const b = parseInt(h.slice(4, 6), 16);
    return `${r},${g},${b}`;
  }
  return "30,30,30";
}

function applyThemeToDom(colors: ThemeColors): void {
  const root = document.documentElement;
  root.style.setProperty("--theme-background", colors.background);
  root.style.setProperty("--theme-surface", colors.surface);
  root.style.setProperty("--theme-border", colors.border);
  root.style.setProperty("--theme-text", colors.text);
  root.style.setProperty("--theme-text-muted", colors.textMuted);
  root.style.setProperty("--theme-accent", colors.accent);
  root.style.setProperty("--theme-accent-hover", colors.accentHover);
  root.style.setProperty("--theme-error", colors.error);
  root.style.setProperty("--theme-warning", colors.warning);
  root.style.setProperty("--theme-success", colors.success);
  root.style.setProperty("--theme-editor-bg", colors.editorBackground);
  root.style.setProperty("--theme-editor-fg", colors.editorForeground);
  root.style.setProperty("--bg-rgb", hexToRgbTriple(colors.background));
}

export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const [theme, setThemeState] = useState<ThemeName>(() => {
    try {
      const stored = localStorage.getItem(THEME_STORAGE_KEY);
      if (stored && stored in THEME_DEFINITIONS) return stored as ThemeName;
    } catch { /* ignore */ }
    return "dark";
  });

  const [colorFilter, setColorFilterState] = useState<ColorFilter>(() => {
    try {
      const stored = localStorage.getItem(COLOR_FILTER_STORAGE_KEY);
      if (stored && stored in CSS_FILTERS) return stored as ColorFilter;
    } catch { /* ignore */ }
    return "none";
  });

  const [fontSize, setFontSizeState] = useState<FontScale>(() => {
    try {
      const stored = localStorage.getItem(FONT_SCALE_KEY);
      if (stored === "normal" || stored === "large" || stored === "x-large") return stored;
    } catch { /* ignore */ }
    return "normal";
  });

  const highContrastStyleRef = useRef<HTMLStyleElement | null>(null);

  const colors = THEME_DEFINITIONS[theme];

  useEffect(() => {
    applyThemeToDom(colors);
  }, [colors]);

  // Apply color filter to document root
  useEffect(() => {
    document.documentElement.style.filter = CSS_FILTERS[colorFilter];
    try {
      localStorage.setItem(COLOR_FILTER_STORAGE_KEY, colorFilter);
    } catch { /* ignore */ }

    // Inject or remove high-contrast override styles
    if (colorFilter === "high-contrast") {
      if (!highContrastStyleRef.current) {
        const style = document.createElement("style");
        style.id = "high-contrast-override";
        style.textContent = `
          * { outline: 1px solid white !important; }
          .panel-bg { background: #000 !important; color: #fff !important; }
        `;
        document.head.appendChild(style);
        highContrastStyleRef.current = style;
      }
    } else {
      if (highContrastStyleRef.current) {
        highContrastStyleRef.current.remove();
        highContrastStyleRef.current = null;
      }
    }
  }, [colorFilter]);

  const setTheme = useCallback((newTheme: ThemeName) => {
    setThemeState(newTheme);
    try {
      localStorage.setItem(THEME_STORAGE_KEY, newTheme);
    } catch { /* ignore */ }
  }, []);

  const setColorFilter = useCallback((filter: ColorFilter) => {
    setColorFilterState(filter);
  }, []);

  const setFontSize = useCallback((scale: FontScale) => {
    setFontSizeState(scale);
    document.documentElement.style.fontSize = FONT_SCALE_PX[scale];
    try { localStorage.setItem(FONT_SCALE_KEY, scale); } catch { /* ignore */ }
  }, []);

  // Apply font size on mount
  useEffect(() => {
    document.documentElement.style.fontSize = FONT_SCALE_PX[fontSize];
  }, [fontSize]);

  // Listen for marketplace theme-install events
  useEffect(() => {
    const handler = (e: Event) => {
      const mTheme = (e as CustomEvent<MarketplaceTheme>).detail;
      if (!mTheme) return;
      // Build a ThemeColors from the marketplace theme
      const colors: ThemeColors = {
        background: mTheme.colors.background,
        surface: mTheme.colors.surface,
        border: mTheme.colors.border,
        text: mTheme.colors.text,
        textMuted: mTheme.colors.textMuted,
        accent: mTheme.colors.accent,
        accentHover: mTheme.colors.accentHover,
        error: mTheme.colors.error,
        warning: mTheme.colors.warning,
        success: mTheme.colors.success,
        editorBackground: mTheme.colors.editorBackground,
        editorForeground: mTheme.colors.editorForeground,
      };
      // Persist the custom theme
      try {
        const existing = JSON.parse(localStorage.getItem(CUSTOM_THEMES_KEY) ?? "{}") as Record<string, ThemeColors>;
        existing[mTheme.id] = colors;
        localStorage.setItem(CUSTOM_THEMES_KEY, JSON.stringify(existing));
      } catch { /* ignore */ }
      // Apply to DOM immediately
      applyThemeToDom(colors);
    };
    window.addEventListener("theme-install", handler);
    return () => window.removeEventListener("theme-install", handler);
  }, []);

  const value: ThemeContextValue = {
    theme,
    colors,
    setTheme,
    monacoTheme: MONACO_THEME_MAP[theme],
    colorFilter,
    setColorFilter,
    fontSize,
    setFontSize,
  };

  return <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>;
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error("useTheme must be used inside ThemeProvider");
  return ctx;
}
