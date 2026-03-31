// Map file extensions and special names to emoji icons
const FILE_ICONS: Record<string, string> = {
  // Languages
  rs: "🦀", ts: "🔷", tsx: "⚛️", js: "🟨", jsx: "⚛️",
  py: "🐍", go: "🐹", java: "☕", kt: "🟣", swift: "🍊",
  c: "🔵", cpp: "🔵", h: "🔵", cs: "🟦", rb: "💎",
  php: "🐘", lua: "🌙", zig: "⚡", dart: "🎯",
  // Web
  html: "🌐", css: "🎨", scss: "🎨", sass: "🎨", vue: "💚",
  svelte: "🔥", astro: "🚀",
  // Data
  json: "📋", yaml: "📋", yml: "📋", toml: "📋", xml: "📋",
  csv: "📊", sql: "🗄️",
  // Config
  env: "🔑", gitignore: "🙈", dockerfile: "🐳",
  // Docs
  md: "📝", mdx: "📝", txt: "📄", pdf: "📕", org: "📓",
  // Media
  png: "🖼️", jpg: "🖼️", jpeg: "🖼️", gif: "🖼️", svg: "🖼️", webp: "🖼️",
  mp3: "🎵", wav: "🎵", ogg: "🎵", mp4: "🎬", webm: "🎬",
  // Game dev
  tscn: "🎬", scn: "🎬", tres: "⚙️", gd: "👾", gdshader: "🔷",
  glsl: "🔷", frag: "🔷", vert: "🔷", wgsl: "🔷",
  // Archives
  zip: "📦", tar: "📦", gz: "📦",
  // Special names (full filename)
  "Cargo.toml": "🦀", "package.json": "📦", "Makefile": "⚙️",
  ".env": "🔑", "README.md": "📖", "Dockerfile": "🐳",
};

const FOLDER_ICONS: Record<string, string> = {
  src: "📂", lib: "📚", tests: "🧪", docs: "📖",
  assets: "🎨", public: "🌐", dist: "📦", build: "📦",
  ".git": "🔀", node_modules: "📦", target: "⚙️",
};

export function getFileIcon(name: string): string {
  // Check special full names first
  if (FILE_ICONS[name]) return FILE_ICONS[name];
  // Then extension
  const ext = name.split(".").pop()?.toLowerCase() ?? "";
  return FILE_ICONS[ext] ?? "📄";
}

export function getFolderIcon(name: string): string {
  return FOLDER_ICONS[name] ?? "📁";
}
