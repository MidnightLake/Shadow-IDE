import { useState, useEffect, useCallback, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";

interface CacheEntry<T> {
  data: T;
  timestamp: number;
}

// Module-level cache shared across all hook instances
const queryCache = new Map<string, CacheEntry<unknown>>();

export interface QueryResult<T> {
  data: T | null;
  loading: boolean;
  error: string | null;
  refetch: () => void;
}

interface QueryOptions<T> {
  enabled?: boolean;
  refetchInterval?: number;
  staleTime?: number;
  onSuccess?: (data: T) => void;
  onError?: (err: string) => void;
}

export function useTauriQuery<T>(
  command: string,
  args?: Record<string, unknown>,
  options?: QueryOptions<T>,
): QueryResult<T> {
  const enabled = options?.enabled ?? true;
  const staleTime = options?.staleTime ?? 30000;
  const refetchInterval = options?.refetchInterval;

  const [data, setData] = useState<T | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const cacheKey = command + JSON.stringify(args ?? {});
  const onSuccessRef = useRef(options?.onSuccess);
  const onErrorRef = useRef(options?.onError);
  onSuccessRef.current = options?.onSuccess;
  onErrorRef.current = options?.onError;

  const fetchData = useCallback(async () => {
    // Check cache freshness
    const cached = queryCache.get(cacheKey) as CacheEntry<T> | undefined;
    if (cached && Date.now() - cached.timestamp < staleTime) {
      setData(cached.data);
      setError(null);
      return;
    }

    setLoading(true);
    setError(null);
    try {
      const result = await invoke<T>(command, args ?? {});
      queryCache.set(cacheKey, { data: result, timestamp: Date.now() });
      setData(result);
      onSuccessRef.current?.(result);
    } catch (err) {
      const errStr = String(err);
      setError(errStr);
      onErrorRef.current?.(errStr);
    } finally {
      setLoading(false);
    }
  }, [cacheKey, command, staleTime]);

  useEffect(() => {
    if (!enabled) return;
    void fetchData();
  }, [enabled, fetchData]);

  useEffect(() => {
    if (!enabled || !refetchInterval) return;
    const id = setInterval(() => {
      void fetchData();
    }, refetchInterval);
    return () => clearInterval(id);
  }, [enabled, refetchInterval, fetchData]);

  return { data, loading, error, refetch: fetchData };
}
