import { useState, useEffect } from "react";

export interface MarketplaceTheme {
  id: string;
  name: string;
  author: string;
  colors: {
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
    bgRgb: string;
  };
}

const MARKETPLACE_THEMES: MarketplaceTheme[] = [
  {
    id: "monokai",
    name: "Monokai",
    author: "Wimer Hazenberg",
    colors: {
      background: "#272822",
      surface: "#1e1f1c",
      border: "#3e3d32",
      text: "#f8f8f2",
      textMuted: "#75715e",
      accent: "#a6e22e",
      accentHover: "#c3e870",
      error: "#f92672",
      warning: "#fd971f",
      success: "#a6e22e",
      editorBackground: "#272822",
      editorForeground: "#f8f8f2",
      bgRgb: "39,40,34",
    },
  },
  {
    id: "dracula-official",
    name: "Dracula Official",
    author: "Dracula Theme",
    colors: {
      background: "#282a36",
      surface: "#21222c",
      border: "#44475a",
      text: "#f8f8f2",
      textMuted: "#6272a4",
      accent: "#bd93f9",
      accentHover: "#d0b3ff",
      error: "#ff5555",
      warning: "#ffb86c",
      success: "#50fa7b",
      editorBackground: "#282a36",
      editorForeground: "#f8f8f2",
      bgRgb: "40,42,54",
    },
  },
  {
    id: "material-dark",
    name: "Material Dark",
    author: "Mattia Astorino",
    colors: {
      background: "#212121",
      surface: "#1a1a1a",
      border: "#303030",
      text: "#eeffff",
      textMuted: "#546e7a",
      accent: "#82aaff",
      accentHover: "#a9c4ff",
      error: "#f07178",
      warning: "#ffcb6b",
      success: "#c3e88d",
      editorBackground: "#212121",
      editorForeground: "#eeffff",
      bgRgb: "33,33,33",
    },
  },
  {
    id: "atom-one-dark",
    name: "Atom One Dark",
    author: "Atom",
    colors: {
      background: "#282c34",
      surface: "#21252b",
      border: "#3b4048",
      text: "#abb2bf",
      textMuted: "#5c6370",
      accent: "#61afef",
      accentHover: "#80c5f5",
      error: "#e06c75",
      warning: "#e5c07b",
      success: "#98c379",
      editorBackground: "#282c34",
      editorForeground: "#abb2bf",
      bgRgb: "40,44,52",
    },
  },
  {
    id: "ayu-dark",
    name: "Ayu Dark",
    author: "teabyii",
    colors: {
      background: "#0d1017",
      surface: "#0a0e14",
      border: "#1a2435",
      text: "#b3b1ad",
      textMuted: "#3e4b59",
      accent: "#39bae6",
      accentHover: "#60cdff",
      error: "#ff3333",
      warning: "#ffb454",
      success: "#91b362",
      editorBackground: "#0d1017",
      editorForeground: "#b3b1ad",
      bgRgb: "13,16,23",
    },
  },
  {
    id: "palenight",
    name: "Palenight",
    author: "equinusocio",
    colors: {
      background: "#292d3e",
      surface: "#202331",
      border: "#3a3f58",
      text: "#a6accd",
      textMuted: "#676e95",
      accent: "#82aaff",
      accentHover: "#9fbcff",
      error: "#f07178",
      warning: "#ffcb6b",
      success: "#c3e88d",
      editorBackground: "#292d3e",
      editorForeground: "#a6accd",
      bgRgb: "41,45,62",
    },
  },
  {
    id: "cobalt2",
    name: "Cobalt2",
    author: "Wes Bos",
    colors: {
      background: "#193549",
      surface: "#122839",
      border: "#1f4662",
      text: "#ffffff",
      textMuted: "#0088ff",
      accent: "#ffc600",
      accentHover: "#ffd740",
      error: "#ff0080",
      warning: "#ffc600",
      success: "#3ad900",
      editorBackground: "#193549",
      editorForeground: "#ffffff",
      bgRgb: "25,53,73",
    },
  },
  {
    id: "night-owl",
    name: "Night Owl",
    author: "Sarah Drasner",
    colors: {
      background: "#011627",
      surface: "#01111d",
      border: "#122d42",
      text: "#d6deeb",
      textMuted: "#637777",
      accent: "#82aaff",
      accentHover: "#9dbfff",
      error: "#ef5350",
      warning: "#ffcb8b",
      success: "#addb67",
      editorBackground: "#011627",
      editorForeground: "#d6deeb",
      bgRgb: "1,22,39",
    },
  },
  {
    id: "shades-of-purple",
    name: "Shades of Purple",
    author: "Ahmad Awais",
    colors: {
      background: "#2d2b55",
      surface: "#1e1e3f",
      border: "#3d3b6e",
      text: "#ffffff",
      textMuted: "#b2a4e0",
      accent: "#fad000",
      accentHover: "#ffdf00",
      error: "#ff628c",
      warning: "#fad000",
      success: "#3ad900",
      editorBackground: "#2d2b55",
      editorForeground: "#ffffff",
      bgRgb: "45,43,85",
    },
  },
  {
    id: "solarized-dark-marketplace",
    name: "Solarized Dark",
    author: "Ethan Schoonover",
    colors: {
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
      bgRgb: "0,43,54",
    },
  },
];

const INSTALLED_KEY = "shadow-marketplace-installed";

function getInstalledIds(): Set<string> {
  try {
    const raw = localStorage.getItem(INSTALLED_KEY);
    if (raw) return new Set(JSON.parse(raw) as string[]);
  } catch { /* ignore */ }
  return new Set();
}

function saveInstalledIds(ids: Set<string>): void {
  try {
    localStorage.setItem(INSTALLED_KEY, JSON.stringify([...ids]));
  } catch { /* ignore */ }
}

// Global marketplace API exposed on window
declare global {
  interface Window {
    __themeMarketplace: {
      install: (theme: MarketplaceTheme) => void;
    };
  }
}

export default function ThemeMarketplace() {
  const [search, setSearch] = useState("");
  const [installedIds, setInstalledIds] = useState<Set<string>>(getInstalledIds);
  const [previewId, setPreviewId] = useState<string | null>(null);
  const [prevCssVars, setPrevCssVars] = useState<Record<string, string>>({});

  useEffect(() => {
    window.__themeMarketplace = {
      install(theme: MarketplaceTheme) {
        window.dispatchEvent(new CustomEvent("theme-install", { detail: theme }));
      },
    };
    return () => {
      // cleanup
    };
  }, []);

  const filtered = MARKETPLACE_THEMES.filter(
    (t) =>
      t.name.toLowerCase().includes(search.toLowerCase()) ||
      t.author.toLowerCase().includes(search.toLowerCase())
  );

  const applyPreviewCss = (theme: MarketplaceTheme) => {
    const root = document.documentElement;
    const saved: Record<string, string> = {};
    const vars: [string, string][] = [
      ["--theme-background", theme.colors.background],
      ["--theme-surface", theme.colors.surface],
      ["--theme-border", theme.colors.border],
      ["--theme-text", theme.colors.text],
      ["--theme-text-muted", theme.colors.textMuted],
      ["--theme-accent", theme.colors.accent],
      ["--theme-accent-hover", theme.colors.accentHover],
      ["--theme-error", theme.colors.error],
      ["--theme-warning", theme.colors.warning],
      ["--theme-success", theme.colors.success],
      ["--theme-editor-bg", theme.colors.editorBackground],
      ["--theme-editor-fg", theme.colors.editorForeground],
      ["--bg-rgb", theme.colors.bgRgb],
    ];
    for (const [name] of vars) {
      saved[name] = root.style.getPropertyValue(name);
    }
    for (const [name, value] of vars) {
      root.style.setProperty(name, value);
    }
    setPrevCssVars(saved);
    setPreviewId(theme.id);
  };

  const cancelPreview = () => {
    const root = document.documentElement;
    for (const [name, value] of Object.entries(prevCssVars)) {
      root.style.setProperty(name, value);
    }
    setPreviewId(null);
    setPrevCssVars({});
  };

  const handleInstall = (theme: MarketplaceTheme) => {
    if (previewId === theme.id) {
      cancelPreview();
    }
    window.__themeMarketplace.install(theme);
    const next = new Set(installedIds);
    next.add(theme.id);
    setInstalledIds(next);
    saveInstalledIds(next);
  };

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        background: "var(--theme-background, #1e1e2e)",
        color: "var(--theme-text, #cdd6f4)",
        fontFamily: "monospace",
        fontSize: 12,
      }}
    >
      {/* Header */}
      <div
        style={{
          padding: "10px 12px",
          borderBottom: "1px solid var(--theme-border, #313244)",
          flexShrink: 0,
        }}
      >
        <div style={{ fontWeight: 700, color: "var(--theme-accent, #89b4fa)", marginBottom: 8, fontSize: 13 }}>
          Theme Marketplace
        </div>
        <input
          type="text"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder="Search themes..."
          style={{
            width: "100%",
            background: "var(--theme-surface, #181825)",
            border: "1px solid var(--theme-border, #313244)",
            borderRadius: 4,
            color: "var(--theme-text, #cdd6f4)",
            padding: "5px 8px",
            fontSize: 12,
            outline: "none",
            boxSizing: "border-box",
          }}
        />
      </div>

      {/* Grid */}
      <div
        style={{
          flex: 1,
          overflowY: "auto",
          padding: 10,
          display: "grid",
          gridTemplateColumns: "repeat(auto-fill, minmax(180px, 1fr))",
          gap: 10,
          alignContent: "start",
        }}
      >
        {filtered.map((theme) => {
          const installed = installedIds.has(theme.id);
          const isPreviewing = previewId === theme.id;
          const swatchColors = [
            theme.colors.background,
            theme.colors.accent,
            theme.colors.text,
            theme.colors.error,
            theme.colors.success,
          ];

          return (
            <div
              key={theme.id}
              onMouseEnter={() => { if (!isPreviewing) applyPreviewCss(theme); }}
              onMouseLeave={() => { if (isPreviewing) cancelPreview(); }}
              style={{
                background: "var(--theme-surface, #181825)",
                border: `1px solid ${isPreviewing ? "var(--theme-accent, #89b4fa)" : "var(--theme-border, #313244)"}`,
                borderRadius: 6,
                padding: 10,
                display: "flex",
                flexDirection: "column",
                gap: 6,
                cursor: "default",
                transition: "border-color 0.15s",
              }}
            >
              {/* Color swatches */}
              <div style={{ display: "flex", gap: 4 }}>
                {swatchColors.map((color, i) => (
                  <div
                    key={i}
                    style={{
                      flex: 1,
                      height: 16,
                      borderRadius: 3,
                      background: color,
                      border: "1px solid rgba(255,255,255,0.08)",
                    }}
                    title={color}
                  />
                ))}
              </div>

              {/* Name and author */}
              <div>
                <div style={{ fontWeight: 700, fontSize: 12, color: "var(--theme-text, #cdd6f4)" }}>
                  {theme.name}
                </div>
                <div style={{ fontSize: 10, color: "var(--theme-text-muted, #6c7086)", marginTop: 1 }}>
                  by {theme.author}
                </div>
              </div>

              {/* Badges + install button */}
              <div style={{ display: "flex", alignItems: "center", gap: 4, marginTop: 2 }}>
                {installed && (
                  <span
                    style={{
                      fontSize: 9,
                      background: "var(--theme-success, #a6e3a1)",
                      color: "#1e1e2e",
                      borderRadius: 3,
                      padding: "1px 5px",
                      fontWeight: 700,
                    }}
                  >
                    INSTALLED
                  </span>
                )}
                {isPreviewing && (
                  <span
                    style={{
                      fontSize: 9,
                      background: "var(--theme-warning, #fab387)",
                      color: "#1e1e2e",
                      borderRadius: 3,
                      padding: "1px 5px",
                      fontWeight: 700,
                    }}
                  >
                    PREVIEW
                  </span>
                )}
                <div style={{ flex: 1 }} />
                <button
                  onClick={() => handleInstall(theme)}
                  style={{
                    fontSize: 10,
                    padding: "2px 8px",
                    borderRadius: 4,
                    border: "1px solid var(--theme-accent, #89b4fa)",
                    background: installed ? "var(--theme-accent, #89b4fa)" : "transparent",
                    color: installed ? "#1e1e2e" : "var(--theme-accent, #89b4fa)",
                    cursor: "pointer",
                    fontWeight: 600,
                    whiteSpace: "nowrap",
                  }}
                >
                  {installed ? "Apply" : "Install"}
                </button>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
