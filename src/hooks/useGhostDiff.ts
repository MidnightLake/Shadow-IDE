import { useEffect, useRef, useCallback, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { editor } from "monaco-editor";

interface FileChange {
  path: string;
  action: string; // "created" | "patched" | "deleted"
  preview?: string;
  oldContent?: string;
  newContent?: string;
}

interface GhostDiffState {
  path: string;
  additions: DiffLine[];
  deletions: DiffLine[];
  decorationIds: string[];
}

interface DiffLine {
  lineNumber: number;
  content: string;
}

interface UseGhostDiffReturn {
  /** Whether a ghost diff is currently shown */
  hasDiff: boolean;
  /** Accept the proposed changes */
  acceptDiff: () => void;
  /** Reject/dismiss the proposed changes */
  rejectDiff: () => void;
  /** Number of added lines */
  addedLines: number;
  /** Number of removed lines */
  removedLines: number;
}

/**
 * Ghost diff overlay for Monaco editor.
 * Shows proposed AI changes as inline decorations:
 * - Green background for added/modified lines
 * - Red strikethrough for deleted lines
 * - Accept/reject controls
 */
export function useGhostDiff(
  editorInstance: editor.IStandaloneCodeEditor | null,
  activeFilePath: string | undefined,
  onAccept?: (path: string, newContent: string) => void
): UseGhostDiffReturn {
  const [diffState, setDiffState] = useState<GhostDiffState | null>(null);
  const pendingContent = useRef<string>("");


  // Compute line-level diff between old and new content
  const computeDiff = useCallback(
    (oldText: string, newText: string): { additions: DiffLine[]; deletions: DiffLine[] } => {
      const oldLines = oldText.split("\n");
      const newLines = newText.split("\n");
      const additions: DiffLine[] = [];
      const deletions: DiffLine[] = [];

      // Simple LCS-based diff
      const maxLen = Math.max(oldLines.length, newLines.length);
      let oldIdx = 0;
      let newIdx = 0;

      while (oldIdx < oldLines.length || newIdx < newLines.length) {
        if (oldIdx >= oldLines.length) {
          // Remaining new lines are additions
          additions.push({ lineNumber: newIdx + 1, content: newLines[newIdx] });
          newIdx++;
        } else if (newIdx >= newLines.length) {
          // Remaining old lines are deletions
          deletions.push({ lineNumber: oldIdx + 1, content: oldLines[oldIdx] });
          oldIdx++;
        } else if (oldLines[oldIdx] === newLines[newIdx]) {
          // Lines match — advance both
          oldIdx++;
          newIdx++;
        } else {
          // Look ahead to find if old line appears later in new (deletion)
          // or if new line appears later in old (addition)
          const lookAhead = Math.min(10, maxLen - Math.max(oldIdx, newIdx));
          let foundOldInNew = -1;
          let foundNewInOld = -1;

          for (let k = 1; k <= lookAhead; k++) {
            if (foundOldInNew < 0 && newIdx + k < newLines.length && newLines[newIdx + k] === oldLines[oldIdx]) {
              foundOldInNew = k;
            }
            if (foundNewInOld < 0 && oldIdx + k < oldLines.length && oldLines[oldIdx + k] === newLines[newIdx]) {
              foundNewInOld = k;
            }
          }

          if (foundNewInOld >= 0 && (foundOldInNew < 0 || foundNewInOld <= foundOldInNew)) {
            // Lines were deleted from old
            for (let k = 0; k < foundNewInOld; k++) {
              deletions.push({ lineNumber: oldIdx + k + 1, content: oldLines[oldIdx + k] });
            }
            oldIdx += foundNewInOld;
          } else if (foundOldInNew >= 0) {
            // Lines were added in new
            for (let k = 0; k < foundOldInNew; k++) {
              additions.push({ lineNumber: newIdx + k + 1, content: newLines[newIdx + k] });
            }
            newIdx += foundOldInNew;
          } else {
            // Modified line — both a deletion and an addition
            deletions.push({ lineNumber: oldIdx + 1, content: oldLines[oldIdx] });
            additions.push({ lineNumber: newIdx + 1, content: newLines[newIdx] });
            oldIdx++;
            newIdx++;
          }
        }
      }

      return { additions, deletions };
    },
    []
  );

  // Apply decorations to the Monaco editor
  const applyDecorations = useCallback(
    (additions: DiffLine[], deletions: DiffLine[]) => {
      if (!editorInstance) return [];

      const model = editorInstance.getModel();
      if (!model) return [];

      const decorations: editor.IModelDeltaDecoration[] = [];

      // Green background for additions (on new line numbers in current content)
      for (const add of additions) {
        if (add.lineNumber <= model.getLineCount()) {
          decorations.push({
            range: {
              startLineNumber: add.lineNumber,
              startColumn: 1,
              endLineNumber: add.lineNumber,
              endColumn: model.getLineMaxColumn(add.lineNumber),
            },
            options: {
              isWholeLine: true,
              className: "ghost-diff-addition",
              glyphMarginClassName: "ghost-diff-glyph-add",
              overviewRuler: {
                color: "#3fb950",
                position: 1, // Right
              },
            },
          });
        }
      }

      // Red strikethrough for deletions (shown as margin decorations near the line)
      for (const del of deletions) {
        const targetLine = Math.min(del.lineNumber, model.getLineCount());
        decorations.push({
          range: {
            startLineNumber: targetLine,
            startColumn: 1,
            endLineNumber: targetLine,
            endColumn: 1,
          },
          options: {
            isWholeLine: false,
            glyphMarginClassName: "ghost-diff-glyph-del",
            before: {
              content: `- ${del.content}`,
              inlineClassName: "ghost-diff-deletion-text",
            },
          },
        });
      }

      return editorInstance.deltaDecorations([], decorations);
    },
    [editorInstance]
  );

  // Clear all ghost diff decorations
  const clearDecorations = useCallback(() => {
    if (editorInstance && diffState?.decorationIds.length) {
      editorInstance.deltaDecorations(diffState.decorationIds, []);
    }
    setDiffState(null);
    pendingContent.current = "";
  }, [editorInstance, diffState]);

  // Accept: apply the new content and save to disk
  const acceptDiff = useCallback(() => {
    if (!diffState || !activeFilePath) return;
    const content = pendingContent.current;
    const path = diffState.path;
    clearDecorations();
    onAccept?.(path, content);
    // Persist accepted changes to disk
    invoke("write_file_content", { path, content }).catch((err) => {
      console.error("[useGhostDiff] Failed to save accepted diff:", err);
    });
  }, [diffState, activeFilePath, clearDecorations, onAccept]);

  // Reject: just clear decorations
  const rejectDiff = useCallback(() => {
    clearDecorations();
  }, [clearDecorations]);

  // Listen for AI file change events
  useEffect(() => {
    let unlisten: (() => void) | null = null;

    // Listen for file changes from any AI stream
    listen<FileChange>("ai-ghost-diff", (event) => {
      const change = event.payload;
      if (!change || !editorInstance || !activeFilePath) return;

      // Only show ghost diff for the currently active file
      if (change.path !== activeFilePath) return;
      if (change.action !== "patched" && change.action !== "created") return;

      const model = editorInstance.getModel();
      if (!model) return;

      const oldContent = model.getValue();
      const newContent = change.newContent || change.preview || "";
      if (!newContent || oldContent === newContent) return;

      pendingContent.current = newContent;

      const { additions, deletions } = computeDiff(oldContent, newContent);
      if (additions.length === 0 && deletions.length === 0) return;

      // Clear any existing decorations
      if (diffState?.decorationIds.length) {
        editorInstance.deltaDecorations(diffState.decorationIds, []);
      }

      const ids = applyDecorations(additions, deletions);
      setDiffState({
        path: change.path,
        additions,
        deletions,
        decorationIds: ids,
      });
    }).then((fn) => {
      unlisten = fn;
    });

    return () => {
      unlisten?.();
    };
  }, [editorInstance, activeFilePath, computeDiff, applyDecorations, diffState]);

  // Clear diff when active file changes
  useEffect(() => {
    if (diffState && diffState.path !== activeFilePath) {
      clearDecorations();
    }
  }, [activeFilePath, diffState, clearDecorations]);

  return {
    hasDiff: diffState !== null,
    acceptDiff,
    rejectDiff,
    addedLines: diffState?.additions.length ?? 0,
    removedLines: diffState?.deletions.length ?? 0,
  };
}
