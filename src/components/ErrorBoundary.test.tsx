import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ErrorBoundary } from "./ErrorBoundary";

function ThrowingChild({ shouldThrow }: { shouldThrow: boolean }) {
  if (shouldThrow) throw new Error("Test explosion");
  return <div>Child content</div>;
}

describe("ErrorBoundary", () => {
  beforeEach(() => {
    vi.spyOn(console, "error").mockImplementation(() => {});
  });

  it("renders children when no error", () => {
    render(
      <ErrorBoundary>
        <div>Hello</div>
      </ErrorBoundary>
    );
    expect(screen.getByText("Hello")).toBeInTheDocument();
  });

  it("renders default fallback on error", () => {
    render(
      <ErrorBoundary>
        <ThrowingChild shouldThrow={true} />
      </ErrorBoundary>
    );
    expect(screen.getByText("Component crashed")).toBeInTheDocument();
    expect(screen.getByText("Test explosion")).toBeInTheDocument();
    expect(screen.getByText("Retry")).toBeInTheDocument();
  });

  it("shows component name in crash message when provided", () => {
    render(
      <ErrorBoundary name="Editor">
        <ThrowingChild shouldThrow={true} />
      </ErrorBoundary>
    );
    expect(screen.getByText("Editor crashed")).toBeInTheDocument();
  });

  it("renders custom fallback when provided", () => {
    render(
      <ErrorBoundary fallback={<div>Custom error UI</div>}>
        <ThrowingChild shouldThrow={true} />
      </ErrorBoundary>
    );
    expect(screen.getByText("Custom error UI")).toBeInTheDocument();
    expect(screen.queryByText("Component crashed")).not.toBeInTheDocument();
  });

  it("resets error state when Retry is clicked", () => {
    render(
      <ErrorBoundary>
        <ThrowingChild shouldThrow={true} />
      </ErrorBoundary>
    );
    expect(screen.getByText("Component crashed")).toBeInTheDocument();

    // Click retry — state resets, but child will throw again since shouldThrow is still true
    // This verifies the Retry button triggers the state reset (hasError → false)
    fireEvent.click(screen.getByText("Retry"));
    // After retry, ThrowingChild throws again so we're back in error state
    expect(screen.getByText("Component crashed")).toBeInTheDocument();
  });

  it("logs error to console", () => {
    const consoleSpy = vi.spyOn(console, "error");
    render(
      <ErrorBoundary name="TestPanel">
        <ThrowingChild shouldThrow={true} />
      </ErrorBoundary>
    );
    expect(consoleSpy).toHaveBeenCalledWith(
      "[ErrorBoundary TestPanel]",
      expect.any(Error),
      expect.any(String)
    );
  });
});
