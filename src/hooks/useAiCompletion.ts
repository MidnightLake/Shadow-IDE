import { useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { editor, languages, CancellationToken, Position } from "monaco-editor";

export function useAiCompletion(model: string) {
  const debounceTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const lastRequestId = useRef(0);

  const registerProvider = useCallback(
    (monacoInstance: typeof import("monaco-editor")) => {
      const provider: languages.InlineCompletionsProvider = {
        provideInlineCompletions: async (
          editorModel: editor.ITextModel,
          position: Position,
          _context: languages.InlineCompletionContext,
          token: CancellationToken
        ): Promise<languages.InlineCompletions> => {
          // Debounce — wait 500ms after the user stops typing
          if (debounceTimer.current) {
            clearTimeout(debounceTimer.current);
          }

          const requestId = ++lastRequestId.current;

          return new Promise((resolve) => {
            debounceTimer.current = setTimeout(async () => {
              // If a newer request came in, skip this one
              if (requestId !== lastRequestId.current || token.isCancellationRequested) {
                resolve({ items: [] });
                return;
              }

              const fullText = editorModel.getValue();
              const offset = editorModel.getOffsetAt(position);
              const prefix = fullText.substring(0, offset);
              const suffix = fullText.substring(offset);

              // Don't complete if prefix is too short
              if (prefix.trim().length < 5) {
                resolve({ items: [] });
                return;
              }

              // Get the language
              const language = editorModel.getLanguageId() || "plaintext";

              // Only send last ~2000 chars of prefix and ~500 chars of suffix
              // to avoid overwhelming the model
              const trimmedPrefix = prefix.slice(-2000);
              const trimmedSuffix = suffix.slice(0, 500);

              try {
                const completion = await invoke<string>("ai_complete_code", {
                  prefix: trimmedPrefix,
                  suffix: trimmedSuffix,
                  language,
                  model: model || null,
                });

                if (
                  requestId !== lastRequestId.current ||
                  token.isCancellationRequested
                ) {
                  resolve({ items: [] });
                  return;
                }

                if (completion && completion.trim()) {
                  resolve({
                    items: [
                      {
                        insertText: completion,
                        range: {
                          startLineNumber: position.lineNumber,
                          startColumn: position.column,
                          endLineNumber: position.lineNumber,
                          endColumn: position.column,
                        },
                      },
                    ],
                  });
                } else {
                  resolve({ items: [] });
                }
              } catch {
                resolve({ items: [] });
              }
            }, 500);
          });
        },

        freeInlineCompletions: () => {},
      };

      const disposable = monacoInstance.languages.registerInlineCompletionsProvider(
        { pattern: "**" },
        provider
      );

      return disposable;
    },
    [model]
  );

  return { registerProvider };
}
