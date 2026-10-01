import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { transform } from "esbuild";

const source = await readFile(new URL("../src/components/NodeTable.helpers.ts", import.meta.url), "utf8");
const { code } = await transform(source, { loader: "ts", format: "esm", target: "es2022" });
const { filterAndSortNodes } = await import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}`);
const base = { query: "", status: "all", groupId: "all", sort: "status", direction: "desc" };
function node(id, override = {}) {
  return { id, display_name: `Node ${id}`, hostname: `${id}.example.net`, os: "Linux", os_version: "Debian", online: true, group_id: null, group_name: null, latest: null, ...override };
}
const sample = [
  node("a", { display_name: "Node 2", group_id: "db", group_name: "数据库", latest: { cpu_percent: 12, memory_used_bytes: 8, memory_total_bytes: 10, disk_used_bytes: 1, disk_total_bytes: 10 } }),
  node("b", { display_name: "Node 10", online: false, os: "Windows", latest: { cpu_percent: 93, memory_used_bytes: 3, memory_total_bytes: 10, disk_used_bytes: 9, disk_total_bytes: 10 } }),
  node("c", { display_name: "Node 1", group_id: "db", group_name: "数据库" }),
];
const ids = (items) => items.map((item) => item.id);

test("status, group and search compose without dropping ungrouped nodes", () => {
  assert.deepEqual(ids(filterAndSortNodes(sample, { ...base, status: "offline" })), ["b"]);
  assert.deepEqual(ids(filterAndSortNodes(sample, { ...base, groupId: "ungrouped" })), ["b"]);
  assert.deepEqual(ids(filterAndSortNodes(sample, { ...base, groupId: "db", query: "  数据库  " })), ["c", "a"]);
  assert.deepEqual(ids(filterAndSortNodes(sample, { ...base, query: "WINDOWS" })), ["b"]);
  assert.deepEqual(ids(filterAndSortNodes(sample, { ...base, query: "b.example" })), ["b"]);
  assert.deepEqual(filterAndSortNodes(sample, { ...base, groupId: "deleted" }), []);
});

test("name sorting is natural, deterministic and does not mutate source", () => {
  const before = ids(sample);
  assert.deepEqual(ids(filterAndSortNodes(sample, { ...base, sort: "name", direction: "asc" })), ["c", "a", "b"]);
  assert.deepEqual(ids(filterAndSortNodes(sample, { ...base, sort: "name", direction: "desc" })), ["b", "a", "c"]);
  assert.deepEqual(ids(sample), before);
  assert.deepEqual(ids(filterAndSortNodes(sample, base)), ["c", "a", "b"]);
});

test("metric sorts use ratios and keep unknown measurements last in either direction", () => {
  assert.deepEqual(ids(filterAndSortNodes(sample, { ...base, sort: "cpu" })), ["b", "a", "c"]);
  assert.deepEqual(ids(filterAndSortNodes(sample, { ...base, sort: "cpu", direction: "asc" })), ["a", "b", "c"]);
  assert.deepEqual(ids(filterAndSortNodes(sample, { ...base, sort: "memory" })), ["a", "b", "c"]);
  assert.deepEqual(ids(filterAndSortNodes(sample, { ...base, sort: "disk" })), ["b", "a", "c"]);
  const invalid = [...sample, node("d", { latest: { cpu_percent: NaN, memory_total_bytes: 0, memory_used_bytes: 4 } })];
  assert.deepEqual(ids(filterAndSortNodes(invalid, { ...base, sort: "memory" })).slice(0, 2), ["a", "b"]);
});

test("visitor latency sorts only successful browser samples, never agent latency or failed zeros", () => {
  const latencies = { a: { status: "ok", milliseconds: 120 }, b: { status: "ok", milliseconds: 20 }, c: { status: "unreachable", milliseconds: 0 } };
  assert.deepEqual(ids(filterAndSortNodes(sample, { ...base, sort: "latency", latencies, direction: "asc" })), ["b", "a", "c"]);
  assert.deepEqual(ids(filterAndSortNodes(sample, { ...base, sort: "latency", latencies, direction: "desc" })), ["a", "b", "c"]);
});
