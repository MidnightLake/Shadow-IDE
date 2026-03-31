import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { DiagnosticPanel } from "./DiagnosticPanel";

const defaultProps = {
  diagnosticCounts: { errors: 2, warnings: 1, infos: 0 },
  diagnosticItems: [
    { file: "/project/src/main.rs", line: 10, column: 5, message: "unused variable `x`", severity: "warning" as const },
    { file: "/project/src/lib.rs", line: 3, column: 1, message: "cannot find value `foo`", severity: "error" as const },
    { file: "/project/src/lib.rs", line: 20, column: 10, message: "mismatched types", severity: "error" as const },
  ],
  onClose: vi.fn(),
  onFileOpen: vi.fn(),
};

describe("DiagnosticPanel", () => {
  it("renders header with PROBLEMS title", () => {
    render(<DiagnosticPanel {...defaultProps} />);
    expect(screen.getByText("PROBLEMS")).toBeInTheDocument();
  });

  it("shows diagnostic counts summary", () => {
    render(<DiagnosticPanel {...defaultProps} />);
    expect(screen.getByText("2 errors, 1 warnings, 0 info")).toBeInTheDocument();
  });

  it("renders all diagnostic items", () => {
    render(<DiagnosticPanel {...defaultProps} />);
    expect(screen.getByText("unused variable `x`")).toBeInTheDocument();
    expect(screen.getByText("cannot find value `foo`")).toBeInTheDocument();
    expect(screen.getByText("mismatched types")).toBeInTheDocument();
  });

  it("shows file location for each item", () => {
    render(<DiagnosticPanel {...defaultProps} />);
    expect(screen.getByText("main.rs : Ln 10, Col 5")).toBeInTheDocument();
    expect(screen.getByText("lib.rs : Ln 3, Col 1")).toBeInTheDocument();
  });

  it("calls onFileOpen when item is clicked", () => {
    const onFileOpen = vi.fn();
    render(<DiagnosticPanel {...defaultProps} onFileOpen={onFileOpen} />);
    fireEvent.click(screen.getByText("unused variable `x`"));
    expect(onFileOpen).toHaveBeenCalledWith("/project/src/main.rs", "main.rs");
  });

  it("calls onClose when close button is clicked", () => {
    const onClose = vi.fn();
    render(<DiagnosticPanel {...defaultProps} onClose={onClose} />);
    // The close button is the only button in the header
    const closeBtn = document.querySelector(".error-panel-close");
    expect(closeBtn).not.toBeNull();
    fireEvent.click(closeBtn!);
    expect(onClose).toHaveBeenCalled();
  });

  it("shows empty message when no items", () => {
    render(
      <DiagnosticPanel
        {...defaultProps}
        diagnosticCounts={{ errors: 0, warnings: 0, infos: 0 }}
        diagnosticItems={[]}
      />
    );
    expect(screen.getByText("No problems detected")).toBeInTheDocument();
  });

  it("applies severity-specific CSS class", () => {
    render(<DiagnosticPanel {...defaultProps} />);
    const items = document.querySelectorAll(".error-panel-item");
    expect(items.length).toBe(3);
    expect(items[0].classList.contains("error-panel-warning")).toBe(true);
    expect(items[1].classList.contains("error-panel-error")).toBe(true);
  });
});
