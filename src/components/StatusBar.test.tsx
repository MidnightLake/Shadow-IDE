import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { StatusBar } from "./StatusBar";

const defaultProps = {
  diagnosticCounts: { errors: 0, warnings: 0, infos: 0 },
  cursorInfo: { line: 1, column: 1, selected: 0 },
  currentLanguage: "TypeScript",
  activeFile: true,
  aiCompletionEnabled: false,
  onToggleErrorPanel: vi.fn(),
  onHide: vi.fn(),
};

describe("StatusBar", () => {
  it("renders diagnostic counts", () => {
    render(<StatusBar {...defaultProps} diagnosticCounts={{ errors: 3, warnings: 5, infos: 0 }} />);
    expect(screen.getByText("3")).toBeInTheDocument();
    expect(screen.getByText("5")).toBeInTheDocument();
  });

  it("shows info count only when > 0", () => {
    const { rerender } = render(<StatusBar {...defaultProps} />);
    expect(screen.queryByTitle(/info/)).not.toBeInTheDocument();

    rerender(<StatusBar {...defaultProps} diagnosticCounts={{ errors: 0, warnings: 0, infos: 2 }} />);
    expect(screen.getByTitle("2 info — click to view")).toBeInTheDocument();
  });

  it("shows cursor position when file is active", () => {
    render(<StatusBar {...defaultProps} cursorInfo={{ line: 42, column: 13, selected: 0 }} />);
    expect(screen.getByText("Ln 42, Col 13")).toBeInTheDocument();
  });

  it("hides cursor position when no active file", () => {
    render(<StatusBar {...defaultProps} activeFile={false} />);
    expect(screen.queryByText(/Ln \d/)).not.toBeInTheDocument();
  });

  it("shows selection count when text is selected", () => {
    render(<StatusBar {...defaultProps} cursorInfo={{ line: 1, column: 1, selected: 15 }} />);
    expect(screen.getByText("(15 sel)")).toBeInTheDocument();
  });

  it("hides selection count when nothing selected", () => {
    render(<StatusBar {...defaultProps} />);
    expect(screen.queryByText(/sel\)/)).not.toBeInTheDocument();
  });

  it("shows current language", () => {
    render(<StatusBar {...defaultProps} currentLanguage="Rust" />);
    expect(screen.getByText("Rust")).toBeInTheDocument();
  });

  it("shows AI badge when completion enabled", () => {
    render(<StatusBar {...defaultProps} aiCompletionEnabled={true} />);
    expect(screen.getByText("AI")).toBeInTheDocument();
  });

  it("hides AI badge when completion disabled", () => {
    render(<StatusBar {...defaultProps} aiCompletionEnabled={false} />);
    expect(screen.queryByText("AI")).not.toBeInTheDocument();
  });

  it("calls onToggleErrorPanel when diagnostics clicked", () => {
    const handler = vi.fn();
    render(<StatusBar {...defaultProps} onToggleErrorPanel={handler} diagnosticCounts={{ errors: 1, warnings: 0, infos: 0 }} />);
    fireEvent.click(screen.getByTitle("1 error — click to view"));
    expect(handler).toHaveBeenCalled();
  });

  it("calls onHide when close button clicked", () => {
    const handler = vi.fn();
    render(<StatusBar {...defaultProps} onHide={handler} />);
    fireEvent.click(screen.getByTitle("Hide status bar"));
    expect(handler).toHaveBeenCalled();
  });
});
