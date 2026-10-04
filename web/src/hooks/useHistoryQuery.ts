import { useEffect, useState } from "react";
import { ApiError } from "../api";

const cache = new Map<string, { data: unknown; fetchedAt: number }>();
const CACHE_TTL = 20_000;
export function clearHistoryCache() { cache.clear(); }

export function useHistoryQuery<T>(key: string, fetcher: (signal: AbortSignal) => Promise<T>, onUnauthorized: () => void, retainKey = key, refreshKey = 0) {
  const [result, setResult] = useState<{ key: string; retainKey: string; data?: T }>({ key, retainKey });
  const [queryError, setQueryError] = useState<{ key: string; message: string | null }>({ key, message: null });
  const [loading, setLoading] = useState(false);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    const cached = cache.get(key);
    setQueryError({ key, message: null });
    if (cached) setResult({ key, retainKey, data: cached.data as T });
    setLoading(true);
    if (cached && revision === 0 && refreshKey === 0 && Date.now() - cached.fetchedAt < CACHE_TTL) {
      setLoading(false);
      return () => controller.abort();
    }
    void fetcher(controller.signal).then((data) => {
      if (controller.signal.aborted) return;
      if (cache.size >= 32 && !cache.has(key)) cache.delete(cache.keys().next().value!);
      cache.set(key, { data, fetchedAt: Date.now() });
      setResult({ key, retainKey, data });
    }).catch((reason) => {
      if (controller.signal.aborted) return;
      if (reason instanceof ApiError && reason.status === 401) onUnauthorized();
      else setQueryError({ key, message: reason instanceof Error ? reason.message : "无法载入历史数据" });
    }).finally(() => { if (!controller.signal.aborted) setLoading(false); });
    return () => controller.abort();
  }, [key, retainKey, revision, refreshKey, fetcher, onUnauthorized]);
  useEffect(() => {
    const timer = window.setInterval(() => { if (!document.hidden) setRevision((current) => current + 1); }, 30_000);
    return () => window.clearInterval(timer);
  }, []);
  const cached = cache.get(key)?.data as T | undefined;
  const retained = result.key !== key && result.retainKey === retainKey && result.data !== undefined && cached === undefined;
  return { data: result.key === key ? result.data : cached ?? (retained ? result.data : undefined), retained,
    error: queryError.key === key ? queryError.message : null, loading, refresh: () => setRevision((current) => current + 1) };
}
