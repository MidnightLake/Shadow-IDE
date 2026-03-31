import "@testing-library/jest-dom/vitest";
import { vi } from "vitest";

// Mock Tauri APIs — all components import from @tauri-apps/api/core
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(() => Promise.resolve()),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(() => Promise.resolve(() => {})),
  emit: vi.fn(),
}));

vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn(() => ({
    minimize: vi.fn(),
    toggleMaximize: vi.fn(),
    close: vi.fn(),
  })),
}));

vi.mock("@tauri-apps/plugin-dialog", () => ({
  ask: vi.fn(),
  open: vi.fn(),
}));

vi.mock("@tauri-apps/plugin-fs", () => ({
  readFile: vi.fn(),
  writeFile: vi.fn(),
}));

// Mock Monaco Editor — it requires browser APIs not available in jsdom
vi.mock("@monaco-editor/react", () => ({
  default: vi.fn(({ value, onChange }: { value?: string; onChange?: (v: string | undefined) => void }) => {
    const textarea = document.createElement("textarea");
    textarea.setAttribute("data-testid", "monaco-editor");
    if (value !== undefined) textarea.value = value;
    textarea.addEventListener("input", () => onChange?.(textarea.value));
    return textarea;
  }),
}));

// Mock xterm.js
vi.mock("@xterm/xterm", () => ({
  Terminal: vi.fn().mockImplementation(() => ({
    open: vi.fn(),
    write: vi.fn(),
    onData: vi.fn(),
    onResize: vi.fn(),
    dispose: vi.fn(),
    rows: 24,
    cols: 80,
  })),
}));

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: vi.fn().mockImplementation(() => ({
    fit: vi.fn(),
    activate: vi.fn(),
    dispose: vi.fn(),
  })),
}));

vi.mock("@xterm/addon-web-links", () => ({
  WebLinksAddon: vi.fn().mockImplementation(() => ({
    activate: vi.fn(),
    dispose: vi.fn(),
  })),
}));
