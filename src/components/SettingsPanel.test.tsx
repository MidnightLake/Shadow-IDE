import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import SettingsPanel from "./SettingsPanel";

const defaultProps = {
  visible: true,
  oledMode: false,
  onOledChange: vi.fn(),
  panelZones: {
    explorer: "left" as const,
    ai: "left" as const,
    todos: "left" as const,
    search: "left" as const,
    remote: "left" as const,
    settings: "left" as const,
    llmloader: "right" as const,
    languages: "right" as const,
  },
  onPanelZoneChange: vi.fn(),
  sidebarAutoHide: false,
  onSidebarAutoHideChange: vi.fn(),
  showStatusBar: true,
  onShowStatusBarChange: vi.fn(),
  aiCompletionEnabled: false,
  onAiCompletionChange: vi.fn(),
  fontSize: 14,
  onFontSizeChange: vi.fn(),
  tabSize: 2,
  onTabSizeChange: vi.fn(),
  minimapEnabled: true,
  onMinimapChange: vi.fn(),
  useTabs: false,
  onUseTabsChange: vi.fn(),
  systemPrompt: "",
  onSystemPromptChange: vi.fn(),
};

describe("SettingsPanel", () => {
  it("renders nothing when not visible", () => {
    const { container } = render(
      <SettingsPanel {...defaultProps} visible={false} />
    );
    expect(container.firstChild).toBeNull();
  });

  it("renders the settings header", () => {
    render(<SettingsPanel {...defaultProps} />);
    expect(screen.getByText("SETTINGS")).toBeInTheDocument();
  });

  it("renders appearance section with all controls", () => {
    render(<SettingsPanel {...defaultProps} />);
    expect(screen.getByText("Appearance")).toBeInTheDocument();
    expect(screen.getByText("OLED Mode")).toBeInTheDocument();
    expect(screen.getByText("Panel Positions")).toBeInTheDocument();
    expect(screen.getByText("Auto-Hide Sidebar")).toBeInTheDocument();
    expect(screen.getByText("Font Size")).toBeInTheDocument();
  });

  it("renders editor section", () => {
    render(<SettingsPanel {...defaultProps} />);
    expect(screen.getByText("Editor")).toBeInTheDocument();
    expect(screen.getByText("Tab Size")).toBeInTheDocument();
    expect(screen.getByText("AI Inline Completion")).toBeInTheDocument();
  });

  it("renders keyboard shortcuts section", () => {
    render(<SettingsPanel {...defaultProps} />);
    expect(screen.getByText("Keyboard Shortcuts")).toBeInTheDocument();
    expect(screen.getByText("Toggle Terminal")).toBeInTheDocument();
    // "AI Chat" appears in both keyboard shortcuts and panel zones
    expect(screen.getAllByText("AI Chat").length).toBeGreaterThanOrEqual(1);
  });

  it("calls onOledChange when OLED toggle is clicked", () => {
    const onOledChange = vi.fn();
    render(<SettingsPanel {...defaultProps} onOledChange={onOledChange} />);
    const oledToggle = screen.getByText("OLED Mode")
      .closest("label")!
      .querySelector("input")!;
    fireEvent.click(oledToggle);
    expect(onOledChange).toHaveBeenCalledWith(true);
  });

  it("calls onPanelZoneChange when panel zone select changes", () => {
    const onPanelZoneChange = vi.fn();
    render(
      <SettingsPanel
        {...defaultProps}
        onPanelZoneChange={onPanelZoneChange}
      />
    );
    // All panels default to "left", find the File Explorer row and change it
    const explorerLabel = screen.getByText("File Explorer");
    const select = explorerLabel.closest(".settings-panel-zone")!.querySelector("select")!;
    fireEvent.change(select, { target: { value: "right" } });
    expect(onPanelZoneChange).toHaveBeenCalledWith("explorer", "right");
  });

  it("calls onFontSizeChange when font size input changes", () => {
    const onFontSizeChange = vi.fn();
    render(
      <SettingsPanel {...defaultProps} onFontSizeChange={onFontSizeChange} />
    );
    const input = screen.getByDisplayValue("14");
    fireEvent.change(input, { target: { value: "16" } });
    expect(onFontSizeChange).toHaveBeenCalledWith(16);
  });

  it("calls onTabSizeChange when tab size input changes", () => {
    const onTabSizeChange = vi.fn();
    render(
      <SettingsPanel {...defaultProps} onTabSizeChange={onTabSizeChange} />
    );
    const input = screen.getByDisplayValue("2");
    fireEvent.change(input, { target: { value: "4" } });
    expect(onTabSizeChange).toHaveBeenCalledWith(4);
  });

  it("calls onAiCompletionChange when AI toggle is clicked", () => {
    const onAiCompletionChange = vi.fn();
    render(
      <SettingsPanel
        {...defaultProps}
        onAiCompletionChange={onAiCompletionChange}
      />
    );
    const aiToggle = screen.getByText("AI Inline Completion")
      .closest("label")!
      .querySelector("input")!;
    fireEvent.click(aiToggle);
    expect(onAiCompletionChange).toHaveBeenCalledWith(true);
  });

  it("toggles about section", () => {
    render(<SettingsPanel {...defaultProps} />);
    expect(screen.queryByText(/v0\.84\.0/)).not.toBeInTheDocument();
    fireEvent.click(screen.getByText("About ShadowIDE"));
    expect(screen.getByText(/v0\.84\.0/)).toBeInTheDocument();
    fireEvent.click(screen.getByText("Hide"));
    expect(screen.queryByText(/v0\.84\.0/)).not.toBeInTheDocument();
  });

  it("reflects initial prop values correctly", () => {
    render(
      <SettingsPanel
        {...defaultProps}
        oledMode={true}
        panelZones={{
          ...defaultProps.panelZones,
          explorer: "right",
        }}
        sidebarAutoHide={true}
        aiCompletionEnabled={true}
        fontSize={18}
        tabSize={4}
      />
    );
    const oledToggle = screen.getByText("OLED Mode")
      .closest("label")!
      .querySelector("input")!;
    expect(oledToggle).toBeChecked();

    // File Explorer should show "right" selected
    const explorerLabel = screen.getByText("File Explorer");
    const select = explorerLabel.closest(".settings-panel-zone")!.querySelector("select")!;
    expect(select.value).toBe("right");

    const autoHide = screen.getByText("Auto-Hide Sidebar")
      .closest("label")!
      .querySelector("input")!;
    expect(autoHide).toBeChecked();

    expect(screen.getByDisplayValue("18")).toBeInTheDocument();
    expect(screen.getByDisplayValue("4")).toBeInTheDocument();
  });

  it("renders all panel zone selectors", () => {
    render(<SettingsPanel {...defaultProps} />);
    // Panel zone labels may also appear elsewhere (keyboard shortcuts, header, etc.)
    const zones = ["File Explorer", "AI Chat", "Diagnostics", "Search", "Remote", "Settings"];
    for (const label of zones) {
      expect(screen.getAllByText(label).length).toBeGreaterThanOrEqual(1);
    }
  });

  it("renders system prompt section", () => {
    render(<SettingsPanel {...defaultProps} />);
    expect(screen.getByText("System Prompt")).toBeInTheDocument();
    expect(screen.getByPlaceholderText(/Custom system prompt/)).toBeInTheDocument();
  });

  it("calls onSystemPromptChange when system prompt is edited", () => {
    const onSystemPromptChange = vi.fn();
    render(
      <SettingsPanel {...defaultProps} onSystemPromptChange={onSystemPromptChange} />
    );
    const textarea = screen.getByPlaceholderText(/Custom system prompt/);
    fireEvent.change(textarea, { target: { value: "Be helpful" } });
    expect(onSystemPromptChange).toHaveBeenCalledWith("Be helpful");
  });

  it("shows reset button when system prompt is set", () => {
    render(<SettingsPanel {...defaultProps} systemPrompt="custom prompt" />);
    expect(screen.getByText("Reset to default")).toBeInTheDocument();
  });

  it("resets about panel when visibility changes", () => {
    const { rerender } = render(<SettingsPanel {...defaultProps} />);
    fireEvent.click(screen.getByText("About ShadowIDE"));
    expect(screen.getByText(/v0\.84\.0/)).toBeInTheDocument();

    rerender(<SettingsPanel {...defaultProps} visible={false} />);
    rerender(<SettingsPanel {...defaultProps} visible={true} />);
    expect(screen.queryByText(/v0\.84\.0/)).not.toBeInTheDocument();
  });
});
