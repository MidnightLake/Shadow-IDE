import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

interface LanguagesPanelProps {
  visible: boolean;
}

interface LspServer {
  language: string;
  command: string;
  available: boolean;
}

const LANGUAGE_INFO: Record<string, { name: string; icon: string; install: string; docs: string; extensions: string[] }> = {
  rust: {
    name: "Rust",
    icon: "Rs",
    install: "rustup component add rust-analyzer",
    docs: "https://doc.rust-lang.org/book/",
    extensions: [".rs"],
  },
  typescript: {
    name: "TypeScript / JavaScript",
    icon: "TS",
    install: "npm i -g typescript-language-server typescript",
    docs: "https://www.typescriptlang.org/docs/",
    extensions: [".ts", ".tsx", ".js", ".jsx"],
  },
  python: {
    name: "Python",
    icon: "Py",
    install: "pip install pyright",
    docs: "https://docs.python.org/3/",
    extensions: [".py"],
  },
  cpp: {
    name: "C / C++",
    icon: "C+",
    install: "apt install clangd  /  brew install llvm",
    docs: "https://en.cppreference.com/",
    extensions: [".c", ".cpp", ".h", ".hpp"],
  },
  go: {
    name: "Go",
    icon: "Go",
    install: "go install golang.org/x/tools/gopls@latest",
    docs: "https://go.dev/doc/",
    extensions: [".go"],
  },
  zig: {
    name: "Zig",
    icon: "Zg",
    install: "install from https://github.com/zigtools/zls",
    docs: "https://ziglang.org/documentation/",
    extensions: [".zig"],
  },
  lua: {
    name: "Lua",
    icon: "Lu",
    install: "install from https://github.com/LuaLS/lua-language-server",
    docs: "https://www.lua.org/manual/5.4/",
    extensions: [".lua"],
  },
};

const EXTRA_LANGUAGES = [
  { name: "HTML / CSS", icon: "HT", extensions: [".html", ".css", ".scss"] },
  { name: "JSON / YAML", icon: "JS", extensions: [".json", ".yaml", ".yml", ".toml"] },
  { name: "Markdown", icon: "MD", extensions: [".md"] },
  { name: "Shell / Bash", icon: "Sh", extensions: [".sh", ".bash", ".zsh"] },
  { name: "SQL", icon: "SQ", extensions: [".sql"] },
  { name: "GLSL / HLSL / WGSL", icon: "GL", extensions: [".glsl", ".hlsl", ".wgsl", ".vert", ".frag"] },
  { name: "Java / Kotlin", icon: "Jv", extensions: [".java", ".kt"] },
  { name: "C#", icon: "C#", extensions: [".cs"] },
  { name: "Swift", icon: "Sw", extensions: [".swift"] },
  { name: "Dart", icon: "Da", extensions: [".dart"] },
  { name: "Ruby", icon: "Rb", extensions: [".rb"] },
  { name: "PHP", icon: "PH", extensions: [".php"] },
  { name: "Elixir", icon: "Ex", extensions: [".ex", ".exs"] },
  { name: "Haskell", icon: "Hs", extensions: [".hs"] },
  { name: "Scala", icon: "Sc", extensions: [".scala"] },
  { name: "Solidity", icon: "So", extensions: [".sol"] },
];

export default function LanguagesPanel({ visible }: LanguagesPanelProps) {
  const [servers, setServers] = useState<LspServer[]>([]);
  const [detecting, setDetecting] = useState(false);
  const [expandedLang, setExpandedLang] = useState<string | null>(null);

  useEffect(() => {
    if (!visible) return;
    detectServers();
  }, [visible]);

  const detectServers = async () => {
    setDetecting(true);
    try {
      const detected = await invoke<LspServer[]>("lsp_detect_servers");
      setServers(detected);
    } catch {
      setServers([]);
    }
    setDetecting(false);
  };

  if (!visible) return null;

  const lspLanguages = Object.keys(LANGUAGE_INFO);

  return (
    <div className="lang-panel">
      <div className="lang-header">
        <span className="lang-title">LANGUAGES</span>
        <button className="llm-btn-icon" onClick={detectServers} title="Detect LSP servers">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><polyline points="23 4 23 10 17 10" /><path d="M20.49 15a9 9 0 11-2.12-9.36L23 10" /></svg>
        </button>
      </div>

      {detecting && <div className="lang-detecting">Detecting language servers...</div>}

      {/* LSP-supported languages */}
      <div className="lang-section-title">LSP SUPPORT</div>
      {lspLanguages.map((key) => {
        const info = LANGUAGE_INFO[key];
        const server = servers.find((s) => s.language === key);
        const isAvailable = server?.available ?? false;
        const isExpanded = expandedLang === key;

        return (
          <div key={key} className="lang-row-wrap">
            <div
              className={`lang-row ${isExpanded ? "expanded" : ""}`}
              onClick={() => setExpandedLang(isExpanded ? null : key)}
            >
              <span className={`lang-icon lang-icon-${key}`}>{info.icon}</span>
              <span className="lang-name">{info.name}</span>
              <span className={`lang-dot ${isAvailable ? "available" : "unavailable"}`} title={isAvailable ? `${server?.command} found` : "Not installed"} />
            </div>
            {isExpanded && (
              <div className="lang-details">
                <div className="lang-detail-row">
                  <span className="lang-detail-label">LSP:</span>
                  <span className={isAvailable ? "lang-available" : "lang-unavailable"}>
                    {server?.command || "not detected"}
                  </span>
                </div>
                <div className="lang-detail-row">
                  <span className="lang-detail-label">Install:</span>
                  <code className="lang-install-cmd">{info.install}</code>
                </div>
                <div className="lang-detail-row">
                  <span className="lang-detail-label">Extensions:</span>
                  <span>{info.extensions.join(", ")}</span>
                </div>
                <div className="lang-detail-row">
                  <span className="lang-detail-label">Docs:</span>
                  <span className="lang-doc-link" title={info.docs}>{info.docs.replace(/^https?:\/\//, "").split("/")[0]}</span>
                </div>
              </div>
            )}
          </div>
        );
      })}

      {/* Syntax-highlighted languages */}
      <div className="lang-section-title" style={{ marginTop: 12 }}>SYNTAX HIGHLIGHTING</div>
      <div className="lang-extra-grid">
        {EXTRA_LANGUAGES.map((lang) => (
          <div key={lang.name} className="lang-extra-row" title={lang.extensions.join(", ")}>
            <span className="lang-icon">{lang.icon}</span>
            <span className="lang-extra-name">{lang.name}</span>
          </div>
        ))}
      </div>
    </div>
  );
}
