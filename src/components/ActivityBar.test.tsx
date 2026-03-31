import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ActivityBar } from "./ActivityBar";
import { DEFAULT_ZONES } from "../types";

const defaultProps = {
  leftView: null as string | null,
  rightView: null as string | null,
  panelZones: DEFAULT_ZONES,
  sidebarAutoHide: false,
  sidebarHidden: false,
  sidebarTimerRef: { current: null },
  onActivityClick: vi.fn(),
  onPanelContextMenu: vi.fn(),
  onSidebarHiddenChange: vi.fn(),
  showRecentMenu: false,
  recentProjects: [],
  onRecentMenuToggle: vi.fn(),
  onRecentProjectClick: vi.fn(),
  aiCompletionEnabled: false,
  onAiCompletionToggle: vi.fn(),
  terminalVisible: false,
  onTerminalToggle: vi.fn(),
};

describe("ActivityBar", () => {
  it("renders all panel buttons", () => {
    render(<ActivityBar {...defaultProps} />);
    expect(screen.getByTitle("Explorer")).toBeInTheDocument();
    expect(screen.getByTitle("ShadowAI (Ctrl+Shift+A)")).toBeInTheDocument();
    expect(screen.getByTitle("Search (Ctrl+Shift+F)")).toBeInTheDocument();
    expect(screen.getByTitle("Remote (Ctrl+Shift+R)")).toBeInTheDocument();
    expect(screen.getByTitle("Settings")).toBeInTheDocument();
  });

  it("marks active panel button", () => {
    render(<ActivityBar {...defaultProps} leftView={"explorer" as any} />);
    const explorerBtn = screen.getByTitle("Explorer");
    expect(explorerBtn.classList.contains("active")).toBe(true);
  });

  it("calls onActivityClick when panel button clicked", () => {
    const handler = vi.fn();
    render(<ActivityBar {...defaultProps} onActivityClick={handler} />);
    fireEvent.click(screen.getByTitle("Explorer"));
    expect(handler).toHaveBeenCalledWith("explorer");
  });

  it("calls onActivityClick for settings", () => {
    const handler = vi.fn();
    render(<ActivityBar {...defaultProps} onActivityClick={handler} />);
    fireEvent.click(screen.getByTitle("Settings"));
    expect(handler).toHaveBeenCalledWith("settings");
  });

  it("toggles AI completion on button click", () => {
    const handler = vi.fn();
    render(<ActivityBar {...defaultProps} onAiCompletionToggle={handler} />);
    fireEvent.click(screen.getByTitle("AI Completion: OFF"));
    expect(handler).toHaveBeenCalled();
  });

  it("shows AI completion ON state", () => {
    render(<ActivityBar {...defaultProps} aiCompletionEnabled={true} />);
    expect(screen.getByTitle("AI Completion: ON")).toBeInTheDocument();
  });

  it("toggles terminal on button click", () => {
    const handler = vi.fn();
    render(<ActivityBar {...defaultProps} onTerminalToggle={handler} />);
    fireEvent.click(screen.getByTitle("Toggle Terminal (Ctrl+`)"));
    expect(handler).toHaveBeenCalled();
  });

  it("shows recent projects menu when toggled", () => {
    render(
      <ActivityBar
        {...defaultProps}
        showRecentMenu={true}
        recentProjects={[
          { path: "/home/user/project-a", name: "project-a", last_opened: 1000 },
          { path: "/home/user/project-b", name: "project-b", last_opened: 900 },
        ]}
      />
    );
    expect(screen.getByText("Recent Projects")).toBeInTheDocument();
    expect(screen.getByText("project-a")).toBeInTheDocument();
    expect(screen.getByText("project-b")).toBeInTheDocument();
  });

  it("does not show recent menu when no projects", () => {
    render(<ActivityBar {...defaultProps} showRecentMenu={true} recentProjects={[]} />);
    expect(screen.queryByText("Recent Projects")).not.toBeInTheDocument();
  });

  it("calls onRecentProjectClick when project selected", () => {
    const handler = vi.fn();
    render(
      <ActivityBar
        {...defaultProps}
        showRecentMenu={true}
        recentProjects={[{ path: "/home/user/proj", name: "proj", last_opened: 1000 }]}
        onRecentProjectClick={handler}
      />
    );
    fireEvent.click(screen.getByText("proj"));
    expect(handler).toHaveBeenCalledWith("/home/user/proj");
  });

  it("calls onRecentMenuToggle when recent button clicked", () => {
    const handler = vi.fn();
    render(<ActivityBar {...defaultProps} onRecentMenuToggle={handler} />);
    fireEvent.click(screen.getByTitle("Recent Projects"));
    expect(handler).toHaveBeenCalled();
  });
});
