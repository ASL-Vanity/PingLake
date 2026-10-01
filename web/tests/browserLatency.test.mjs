import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { transform } from "esbuild";

const source = await readFile(new URL("../src/browserLatency.ts", import.meta.url), "utf8");
const { code } = await transform(source, { loader: "ts", format: "esm", target: "es2022" });
const { measureBrowserEndpoint } = await import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}`);
const response = (body = "ok", headers = {}) => new Response(body, { headers: { "Cache-Control": "no-store", ...headers } });

test("each endpoint is measured independently and receives no Hub credentials", async () => {
  const requests = [];
  const sample = async (endpoint, duration) => {
    let clock = 10;
    const fake = async (url, options) => { requests.push({ url, options }); clock += duration; return response(); };
    return measureBrowserEndpoint(endpoint, new AbortController().signal, fake, () => clock);
  };
  assert.deepEqual(await Promise.all([sample("https://a.example/pinglake/latency", 12), sample("https://b.example/pinglake/latency", 81)]), [12, 81]);
  assert.notEqual(requests[0].url.origin, requests[1].url.origin);
  assert.notEqual(requests[0].url.searchParams.get("_pinglake"), requests[1].url.searchParams.get("_pinglake"));
  for (const { options } of requests) {
    assert.equal(options.credentials, "omit"); assert.equal(options.cache, "no-store");
    assert.equal(options.redirect, "error"); assert.equal(options.referrerPolicy, "no-referrer");
    assert.equal(options.mode, "cors"); assert.equal(options.headers, undefined);
  }
});

test("network, HTTP and cache-policy failures reject instead of returning zero", async () => {
  const signal = new AbortController().signal;
  await assert.rejects(measureBrowserEndpoint("https://a.example/latency", signal, async () => { throw new TypeError("fetch failed"); }), /fetch failed/);
  await assert.rejects(measureBrowserEndpoint("https://a.example/latency", signal, async () => new Response("error", { status: 503 })), /503/);
  await assert.rejects(measureBrowserEndpoint("https://a.example/latency", signal, async () => new Response("cached")), /disable caching/);
  await assert.rejects(measureBrowserEndpoint("http://a.example/latency", signal), /Invalid HTTPS/);
  await assert.rejects(measureBrowserEndpoint("https://user:pass@a.example/latency", signal), /Invalid HTTPS/);
});

test("abort prevents requests and propagates cancellation during a stream", async () => {
  const before = new AbortController(); before.abort();
  await assert.rejects(measureBrowserEndpoint("https://a.example/latency", before.signal, async () => { assert.fail("fetch called"); }), { name: "AbortError" });
  const during = new AbortController(); let cancelled = false;
  const stream = new ReadableStream({ pull(controller) { during.abort(); controller.enqueue(new Uint8Array([1])); }, cancel() { cancelled = true; } });
  await assert.rejects(measureBrowserEndpoint("https://a.example/latency", during.signal, async () => response(stream)), { name: "AbortError" });
  assert.equal(cancelled, true);
});

test("oversized declared and streaming bodies are stopped", async () => {
  const signal = new AbortController().signal;
  await assert.rejects(measureBrowserEndpoint("https://a.example/latency", signal, async () => response("", { "Content-Length": "65537" })), /too large/);
  let cancelled = false;
  const body = new ReadableStream({ pull(controller) { controller.enqueue(new Uint8Array(16384)); }, cancel() { cancelled = true; } });
  await assert.rejects(measureBrowserEndpoint("https://a.example/latency", signal, async () => response(body)), /too large/);
  assert.equal(cancelled, true);
});
