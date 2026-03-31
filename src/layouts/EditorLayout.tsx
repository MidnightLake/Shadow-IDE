import React, { Suspense } from "react";
import { ErrorBoundary } from "../components/ErrorBoundary";
import MarkdownPreview from "../components/MarkdownPreview";
import JupyterPanel from "../components/JupyterPanel";
import MeshViewer from "../components/MeshViewer";
import type { OpenFile, CursorInfo, DiagnosticCounts, DiagnosticItem } from "../components/Editor";

const Editor = React.lazy(() => import("../components/Editor"));
const OrgModePanel = React.lazy(() => import("../components/OrgModePanel"));

// TODO: extract to layout — terminal and resize logic is tightly coupled to App.tsx state

interface EditorLayoutProps {
  activeFile: OpenFile | undefined;
  openFiles: OpenFile[];
  activeFileIndex: number;
  terminalVisible: boolean;
  terminalHeight: number;
  aiCompletionEnabled: boolean;
  rootPath: string | null;
  minimapEnabled?: boolean;
  fontSize?: number;
  tabSize?: number;
  onActiveFileChange: (i: number) => void;
  onFileClose: (i: number) => void;
  onFileContentChange: (i: number, content: string) => void;
  onFileReorder: (from: number, to: number) => void;
  onMinimapToggle: (v: boolean) => void;
  onCursorChange: (info: CursorInfo) => void;
  onDiagnosticsChange: (counts: DiagnosticCounts) => void;
  onDiagnosticItems: (items: DiagnosticItem[]) => void;
}

export function EditorLayout({
  activeFile,
  openFiles,
  activeFileIndex,
  terminalVisible,
  terminalHeight,
  aiCompletionEnabled,
  rootPath,
  minimapEnabled,
  fontSize,
  tabSize,
  onActiveFileChange,
  onFileClose,
  onFileContentChange,
  onFileReorder,
  onMinimapToggle,
  onCursorChange,
  onDiagnosticsChange,
  onDiagnosticItems,
}: EditorLayoutProps) {
  return (
    <div
      className="editor-area"
      style={{ height: terminalVisible ? `calc(100% - ${terminalHeight}px - 4px)` : "100%" }}
    >
      {activeFile && (activeFile.name.endsWith(".md") || activeFile.name.endsWith(".mdx")) ? (
        <ErrorBoundary name="MarkdownPreview">
          <MarkdownPreview content={activeFile.content} filePath={activeFile.path} />
        </ErrorBoundary>
      ) : activeFile && activeFile.name.endsWith(".ipynb") ? (
        <ErrorBoundary name="JupyterPanel">
          <JupyterPanel filePath={activeFile.path} />
        </ErrorBoundary>
      ) : activeFile && activeFile.name.endsWith(".org") ? (
        <ErrorBoundary name="OrgModePanel">
          <Suspense fallback={<div>Loading...</div>}>
            <OrgModePanel filePath={activeFile.path} />
          </Suspense>
        </ErrorBoundary>
      ) : activeFile && (activeFile.name.endsWith(".obj") || activeFile.name.endsWith(".gltf")) ? (
        <ErrorBoundary name="MeshViewer">
          <MeshViewer filePath={activeFile.path} />
        </ErrorBoundary>
      ) : (
        <ErrorBoundary name="Editor">
          <Suspense fallback={<div>Loading editor...</div>}>
            <Editor
              files={openFiles}
              activeFileIndex={activeFileIndex}
              onActiveFileChange={onActiveFileChange}
              onFileClose={onFileClose}
              onFileContentChange={onFileContentChange}
              onFileReorder={onFileReorder}
              aiCompletionEnabled={aiCompletionEnabled}
              rootPath={rootPath ?? undefined}
              minimapEnabled={minimapEnabled}
              fontSize={fontSize}
              tabSize={tabSize}
              onMinimapToggle={onMinimapToggle}
              onCursorChange={onCursorChange}
              onDiagnosticsChange={onDiagnosticsChange}
              onDiagnosticItems={onDiagnosticItems}
            />
          </Suspense>
        </ErrorBoundary>
      )}
    </div>
  );
}
