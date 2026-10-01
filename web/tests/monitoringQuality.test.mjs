import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { transform } from "esbuild";

const source = await readFile(new URL("../src/utils.ts", import.meta.url), "utf8");
const { code } = await transform(source, { loader: "ts", format: "esm", target: "es2022" });
const { nodeFreshness, agentQuality } = await import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}`);

const base = { online: true, last_seen_at: "2026-10-01T00:00:00.000Z", latest: { monitoring: { report_interval_secs: 5, agent: { sample_age_ms: 0 } } } };

test("freshness includes agent sample age and distinguishes stale data", () => {
  const now = Date.parse("2026-10-01T00:00:03.000Z");
  assert.equal(nodeFreshness(base, now).state, "fresh");
  assert.equal(nodeFreshness({ ...base, latest: { monitoring: { report_interval_secs: 5, agent: { sample_age_ms: 50_000 } } } }, now).state, "stale");
  assert.equal(nodeFreshness({ ...base, online: false }, now).label, "离线 · 最后数据");
});

test("agent quality exposes upload, queue and configuration degradation", () => {
  assert.equal(agentQuality(undefined).state, "unknown");
  assert.equal(agentQuality({ consecutive_failures: 0, upload_failures: 0, success_rate_percent: 100, queue_length: 0, dropped_reports: 0 }).state, "good");
  assert.equal(agentQuality({ consecutive_failures: 0, upload_failures: 0, success_rate_percent: 98, queue_length: 1, dropped_reports: 0 }).state, "warning");
  assert.equal(agentQuality({ consecutive_failures: 1, upload_failures: 0, success_rate_percent: 100, queue_length: 0, dropped_reports: 0 }).state, "bad");
});
