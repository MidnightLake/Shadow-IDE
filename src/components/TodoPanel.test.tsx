import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import TodoPanel from "./TodoPanel";

const mockInvoke = vi.mocked(invoke);

const defaultProps = {
  visible: true,
  rootPath: "/home/user/project",
  onFileOpen: vi.fn(),
};

const sampleTodos = [
  { file: "/home/user/project/src/main.rs", line: 10, marker: "TODO", text: "implement caching", priority: "normal" },
  { file: "/home/user/project/src/main.rs", line: 25, marker: "FIXME", text: "fix memory leak", priority: "high" },
  { file: "/home/user/project/src/lib.rs", line: 5, marker: "TODO", text: "add tests", priority: "normal" },
  { file: "/home/user/project/src/lib.rs", line: 12, marker: "HACK", text: "temporary workaround", priority: "low" },
];

describe("TodoPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    mockInvoke.mockResolvedValue(sampleTodos);
  });

  it("renders nothing when not visible", () => {
    const { container } = render(
      <TodoPanel {...defaultProps} visible={false} />
    );
    expect(container.firstChild).toBeNull();
  });

  it("renders header with title", async () => {
    render(<TodoPanel {...defaultProps} />);
    expect(screen.getByText("DIAGNOSTICS")).toBeInTheDocument();
  });

  it("scans todos on mount when visible", async () => {
    render(<TodoPanel {...defaultProps} />);
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("scan_todos", {
        path: "/home/user/project",
      });
    });
  });

  it("does not scan when rootPath is empty", () => {
    render(<TodoPanel {...defaultProps} rootPath="" />);
    expect(mockInvoke).not.toHaveBeenCalled();
  });

  it("displays total count", async () => {
    render(<TodoPanel {...defaultProps} />);
    await waitFor(() => {
      expect(screen.getByText("4")).toBeInTheDocument();
    });
  });

  it("renders filter buttons with counts", async () => {
    render(<TodoPanel {...defaultProps} />);
    await waitFor(() => {
      expect(screen.getByText("All (4)")).toBeInTheDocument();
      expect(screen.getByText("TODO (2)")).toBeInTheDocument();
      expect(screen.getByText("FIXME (1)")).toBeInTheDocument();
      expect(screen.getByText("HACK (1)")).toBeInTheDocument();
    });
  });

  it("filters by marker type when filter button is clicked", async () => {
    render(<TodoPanel {...defaultProps} />);

    await waitFor(() => {
      expect(screen.getByText("TODO (2)")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByText("FIXME (1)"));

    // Only FIXME items should show
    expect(screen.getByText("fix memory leak")).toBeInTheDocument();
    expect(screen.queryByText("implement caching")).not.toBeInTheDocument();
    expect(screen.queryByText("add tests")).not.toBeInTheDocument();
  });

  it("groups items by file with relative paths", async () => {
    render(<TodoPanel {...defaultProps} />);
    await waitFor(() => {
      expect(screen.getByText("src/main.rs")).toBeInTheDocument();
      expect(screen.getByText("src/lib.rs")).toBeInTheDocument();
    });
  });

  it("collapses and expands file groups", async () => {
    render(<TodoPanel {...defaultProps} />);
    await waitFor(() => {
      expect(screen.getByText("implement caching")).toBeInTheDocument();
    });

    // Click file header to collapse
    fireEvent.click(screen.getByText("src/main.rs"));
    expect(screen.queryByText("implement caching")).not.toBeInTheDocument();
    expect(screen.queryByText("fix memory leak")).not.toBeInTheDocument();

    // Items in other files should still show
    expect(screen.getByText("add tests")).toBeInTheDocument();

    // Click again to expand
    fireEvent.click(screen.getByText("src/main.rs"));
    expect(screen.getByText("implement caching")).toBeInTheDocument();
  });

  it("calls onFileOpen when a todo item is clicked", async () => {
    const onFileOpen = vi.fn();
    render(<TodoPanel {...defaultProps} onFileOpen={onFileOpen} />);

    await waitFor(() => {
      expect(screen.getByText("implement caching")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByText("implement caching"));
    expect(onFileOpen).toHaveBeenCalledWith(
      "/home/user/project/src/main.rs",
      "main.rs"
    );
  });

  it("shows empty message when no items found", async () => {
    mockInvoke.mockResolvedValue([]);
    render(<TodoPanel {...defaultProps} />);
    await waitFor(() => {
      expect(
        screen.getByText("No markers found. Click refresh to scan.")
      ).toBeInTheDocument();
    });
  });

  it("refreshes on button click", async () => {
    render(<TodoPanel {...defaultProps} />);
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledTimes(1);
    });

    fireEvent.click(screen.getByTitle("Refresh"));
    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledTimes(2);
    });
  });
});
