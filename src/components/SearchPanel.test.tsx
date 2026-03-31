import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { invoke } from "@tauri-apps/api/core";
import SearchPanel from "./SearchPanel";

const mockInvoke = vi.mocked(invoke);

const defaultProps = {
  visible: true,
  rootPath: "/home/user/project",
  onFileOpen: vi.fn(),
};

describe("SearchPanel", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("renders nothing when not visible", () => {
    const { container } = render(
      <SearchPanel {...defaultProps} visible={false} />
    );
    expect(container.firstChild).toBeNull();
  });

  it("renders search header and input", () => {
    render(<SearchPanel {...defaultProps} />);
    expect(screen.getByText("SEARCH")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Search...")).toBeInTheDocument();
  });

  it("does not show replace input by default", () => {
    render(<SearchPanel {...defaultProps} />);
    expect(screen.queryByPlaceholderText("Replace...")).not.toBeInTheDocument();
  });

  it("toggles replace input on button click", () => {
    render(<SearchPanel {...defaultProps} />);
    // Toggle button shows replace row
    const toggleBtn = screen.getByTitle("Toggle Replace");
    fireEvent.click(toggleBtn);
    expect(screen.getByPlaceholderText("Replace...")).toBeInTheDocument();
    // Toggle again hides it
    fireEvent.click(toggleBtn);
    expect(screen.queryByPlaceholderText("Replace...")).not.toBeInTheDocument();
  });

  it("renders extension filter input", () => {
    render(<SearchPanel {...defaultProps} />);
    expect(
      screen.getByPlaceholderText("File extensions (e.g. ts,rs)")
    ).toBeInTheDocument();
  });

  it("calls invoke with search query after debounce", async () => {
    mockInvoke.mockResolvedValue([
      {
        file: "/home/user/project/src/main.ts",
        line: 10,
        column: 5,
        text: "const hello = 'world';",
        match_text: "hello",
      },
    ]);

    render(<SearchPanel {...defaultProps} />);
    const input = screen.getByPlaceholderText("Search...");
    fireEvent.change(input, { target: { value: "hello" } });

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("search_in_files", {
        root: "/home/user/project",
        pattern: "hello",
        extensions: null,
      });
    });
  });

  it("displays grouped search results", async () => {
    mockInvoke.mockResolvedValue([
      {
        file: "/home/user/project/src/main.ts",
        line: 10,
        column: 5,
        text: "const hello = 'world';",
        match_text: "hello",
      },
      {
        file: "/home/user/project/src/main.ts",
        line: 20,
        column: 1,
        text: "export { hello };",
        match_text: "hello",
      },
    ]);

    render(<SearchPanel {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Search..."), {
      target: { value: "hello" },
    });

    await waitFor(() => {
      expect(screen.getByText("main.ts")).toBeInTheDocument();
      expect(screen.getByText("2")).toBeInTheDocument(); // count badge
    });
  });

  it("calls onFileOpen when a result is clicked", async () => {
    const onFileOpen = vi.fn();
    mockInvoke.mockResolvedValue([
      {
        file: "/home/user/project/src/app.ts",
        line: 5,
        column: 0,
        text: "function test() {}",
        match_text: "test",
      },
    ]);

    render(<SearchPanel {...defaultProps} onFileOpen={onFileOpen} />);
    fireEvent.change(screen.getByPlaceholderText("Search..."), {
      target: { value: "test" },
    });

    await waitFor(() => {
      expect(screen.getByText("app.ts")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByText("function test() {}"));
    expect(onFileOpen).toHaveBeenCalledWith(
      "/home/user/project/src/app.ts",
      "app.ts"
    );
  });

  it("shows No results when search returns empty", async () => {
    mockInvoke.mockResolvedValue([]);

    render(<SearchPanel {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Search..."), {
      target: { value: "nonexistent" },
    });

    await waitFor(() => {
      expect(screen.getByText("No results")).toBeInTheDocument();
    });
  });

  it("calls replace_in_files when Replace All is clicked", async () => {
    const searchResult = [
      {
        file: "/home/user/project/src/a.ts",
        line: 1,
        column: 0,
        text: "old value",
        match_text: "old",
      },
    ];
    mockInvoke.mockImplementation(async (cmd: string) => {
      if (cmd === "search_in_files") return searchResult;
      if (cmd === "replace_in_files") return 3;
      return null;
    });

    vi.spyOn(window, "alert").mockImplementation(() => {});

    render(<SearchPanel {...defaultProps} />);

    // Search first
    fireEvent.change(screen.getByPlaceholderText("Search..."), {
      target: { value: "old" },
    });

    await waitFor(() => {
      expect(screen.getByText("a.ts")).toBeInTheDocument();
    });

    // Toggle replace, type replacement, click Replace All
    fireEvent.click(screen.getByTitle("Toggle Replace"));
    fireEvent.change(screen.getByPlaceholderText("Replace..."), {
      target: { value: "new" },
    });
    fireEvent.click(screen.getByText("All"));

    await waitFor(() => {
      expect(mockInvoke).toHaveBeenCalledWith("replace_in_files", {
        root: "/home/user/project",
        search: "old",
        replace: "new",
        filePaths: ["/home/user/project/src/a.ts"],
      });
    });
  });
});
