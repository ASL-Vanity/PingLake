const MAX_RESPONSE_BYTES = 64 * 1024;

export async function measureBrowserEndpoint(endpoint: string, signal: AbortSignal, fetcher: typeof fetch = fetch, now: () => number = () => performance.now()): Promise<number> {
  const url = new URL(endpoint);
  if (url.protocol !== "https:" || url.username || url.password || url.hash) throw new Error("Invalid HTTPS endpoint");
  url.searchParams.set("_pinglake", crypto.randomUUID());
  signal.throwIfAborted();
  const started = now();
  const response = await fetcher(url, { mode: "cors", credentials: "omit", cache: "no-store", redirect: "error", referrerPolicy: "no-referrer", signal });
  if (!response.ok) { await response.body?.cancel(); throw new Error(`HTTP ${response.status}`); }
  if (!/no-store/i.test(response.headers.get("Cache-Control") ?? "")) { await response.body?.cancel(); throw new Error("Endpoint must disable caching"); }
  const length = Number(response.headers.get("Content-Length"));
  if (length > MAX_RESPONSE_BYTES) { await response.body?.cancel(); throw new Error("Response too large"); }
  const reader = response.body?.getReader();
  if (reader) {
    let total = 0;
    try {
      for (;;) {
        signal.throwIfAborted();
        const chunk = await reader.read();
        signal.throwIfAborted();
        if (chunk.done) break;
        total += chunk.value.byteLength;
        if (total > MAX_RESPONSE_BYTES) throw new Error("Response too large");
      }
    } catch (error) { await reader.cancel().catch(() => {}); throw error; }
    finally { reader.releaseLock(); }
  }
  signal.throwIfAborted();
  return now() - started;
}
