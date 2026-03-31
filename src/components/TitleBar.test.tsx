import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { TitleBar } from "./TitleBar";

const mockWindow = {
  minimize: vi.fn(),
  toggleMaximize: vi.fn(),
  close: vi.fn(),
};

const defaultProps = {
  rootPath: "/home/user/my-project",
  activeFile: undefined,
  isMobileDevice: false,
  appWindow: mockWindow,
};

describe("TitleBar", () => {
  it("renders project name from rootPath", () => {
    render(<TitleBar {...defaultProps} />);
    expect(screen.getAllByText("my-project").length).toBeGreaterThan(0);
  });

  it("shows / when rootPath is /", () => {
    render(<TitleBar {...defaultProps} rootPath="/" />);
    expect(screen.getByText("/")).toBeInTheDocument();
  });

  it("renders version label", () => {
    render(<TitleBar {...defaultProps} />);
    expect(screen.getByText("ShadowIDE")).toBeInTheDocument();
    expect(screen.getByText("v0.84.0")).toBeInTheDocument();
  });

  it("renders project name from Windows paths", () => {
    render(<TitleBar {...defaultProps} rootPath={"C:\\Users\\amine\\Documents\\CLI\\shadow-ide"} />);
    expect(screen.getAllByText("shadow-ide").length).toBeGreaterThan(0);
  });

  it("shows active file name when file is open", () => {
    render(
      <TitleBar
        {...defaultProps}
        activeFile={{ name: "main.rs", path: "/home/user/project/src/main.rs", content: "", modified: false, language: "rust" }}
      />
    );
    expect(screen.getByText("main.rs")).toBeInTheDocument();
  });

  it("shows M indicator when file is modified", () => {
    render(
      <TitleBar
        {...defaultProps}
        activeFile={{ name: "lib.rs", path: "/p/lib.rs", content: "", modified: true, language: "rust" }}
      />
    );
    expect(screen.getByText("M")).toBeInTheDocument();
  });

  it("does not show M indicator when file is not modified", () => {
    render(
      <TitleBar
        {...defaultProps}
        activeFile={{ name: "lib.rs", path: "/p/lib.rs", content: "", modified: false, language: "rust" }}
      />
    );
    expect(screen.queryByText("M")).not.toBeInTheDocument();
  });

  it("renders window controls on desktop", () => {
    render(<TitleBar {...defaultProps} />);
    expect(screen.getByTitle("Minimize")).toBeInTheDocument();
    expect(screen.getByTitle("Maximize")).toBeInTheDocument();
    expect(screen.getByTitle("Close")).toBeInTheDocument();
  });

  it("hides window controls on mobile", () => {
    render(<TitleBar {...defaultProps} isMobileDevice={true} />);
    expect(screen.queryByTitle("Minimize")).not.toBeInTheDocument();
    expect(screen.queryByTitle("Maximize")).not.toBeInTheDocument();
    expect(screen.queryByTitle("Close")).not.toBeInTheDocument();
  });

  it("calls minimize on button click", () => {
    render(<TitleBar {...defaultProps} />);
    fireEvent.click(screen.getByTitle("Minimize"));
    expect(mockWindow.minimize).toHaveBeenCalled();
  });

  it("calls toggleMaximize on button click", () => {
    render(<TitleBar {...defaultProps} />);
    fireEvent.click(screen.getByTitle("Maximize"));
    expect(mockWindow.toggleMaximize).toHaveBeenCalled();
  });

  it("calls close on button click", () => {
    render(<TitleBar {...defaultProps} />);
    fireEvent.click(screen.getByTitle("Close"));
    expect(mockWindow.close).toHaveBeenCalled();
  });

  it("hides controls when appWindow is null", () => {
    render(<TitleBar {...defaultProps} appWindow={null} />);
    expect(screen.queryByTitle("Minimize")).not.toBeInTheDocument();
  });
});
