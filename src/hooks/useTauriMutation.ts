import { useState, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";

export interface MutationResult<TArgs, TResult> {
  mutate: (args: TArgs) => Promise<TResult>;
  loading: boolean;
  error: string | null;
  reset: () => void;
}

export function useTauriMutation<TArgs extends Record<string, unknown>, TResult>(
  command: string,
): MutationResult<TArgs, TResult> {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const mutate = useCallback(
    async (args: TArgs): Promise<TResult> => {
      setLoading(true);
      setError(null);
      try {
        const result = await invoke<TResult>(command, args);
        return result;
      } catch (err) {
        const errStr = String(err);
        setError(errStr);
        throw new Error(errStr);
      } finally {
        setLoading(false);
      }
    },
    [command],
  );

  const reset = useCallback(() => {
    setError(null);
  }, []);

  return { mutate, loading, error, reset };
}
