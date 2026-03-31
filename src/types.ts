export type SidebarView = "explorer" | "ai" | "collab" | "remote" | "todos" | "search" | "settings" | "llmloader" | "languages" | "logs" | "rag" | "bluetooth" | "gitgraph" | "testexplorer" | "agent" | "database" | "debug" | "deps" | "edithistory" | "keybindings" | "docs" | "profiler" | "glslpreview" | "pr" | "plugins" | "mutation" | "cicd" | "gamedev" | "planengine";
export type PanelZone = "left" | "right";

export interface RecentProject {
  path: string;
  name: string;
  last_opened: number;
}

export interface WorkspaceSettings {
  tab_size?: number;
  use_tabs?: boolean;
  font_size?: number;
  minimap_enabled?: boolean;
  ai_model?: string;
  ai_temperature?: number;
  tools_enabled?: boolean;
}

export const DEFAULT_ZONES: Record<SidebarView, PanelZone> = {
  explorer: "left",
  ai: "left",
  collab: "left",
  todos: "left",
  search: "left",
  remote: "left",
  settings: "left",
  llmloader: "right",
  languages: "right",
  logs: "left",
  rag: "left",
  bluetooth: "left",
  gitgraph: "left",
  testexplorer: "left",
  agent: "right",
  database: "left",
  debug: "left",
  deps: "right",
  edithistory: "left",
  keybindings: "left",
  docs: "right",
  profiler: "right",
  glslpreview: "right",
  pr: "left",
  plugins: "left",
  mutation: "left",
  cicd: "left",
  gamedev: "left",
  planengine: "left",
};
