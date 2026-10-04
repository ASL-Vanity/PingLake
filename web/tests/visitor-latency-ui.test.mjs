import assert from "node:assert/strict";
import test, { after } from "node:test";
import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";

const output = new URL("../.tmp/visitor-latency-fixture.mjs", import.meta.url);
const bundled = await build({
  stdin: {
    contents: 'export { NodeTable } from "./src/components/NodeTable"; export { NodeDetail } from "./src/components/NodeDetail"; export { MonitoringConfig } from "./src/components/MonitoringConfig";',
    resolveDir: fileURLToPath(new URL("..", import.meta.url)),
    loader: "tsx",
  },
  bundle: true,
  write: false,
  format: "esm",
  platform: "node",
  packages: "external",
  jsx: "automatic",
});
await mkdir(new URL("../.tmp", import.meta.url), { recursive: true });
await writeFile(output, bundled.outputFiles[0].text);
const { NodeTable, NodeDetail, MonitoringConfig } = await import(output.href);

const dom = new JSDOM('<!doctype html><html><body><div id="test"></div></body></html>', { url: "https://monitor.example.test/" });
after(() => dom.window.close());
for (const name of ["window", "document", "HTMLElement", "HTMLButtonElement", "HTMLInputElement", "HTMLSelectElement", "Event", "MouseEvent", "KeyboardEvent"]) {
  Object.defineProperty(globalThis, name, { value: dom.window[name], configurable: true });
}
Object.defineProperty(globalThis, "navigator", { value: dom.window.navigator, configurable: true });
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
globalThis.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
const container = document.getElementById("test");
const config = { revision: 1, browser_latency_url: "https://node.example.test/pinglake/latency", services: [], probes: [], dns_checks: [], process_checks: [], local_port_checks: [] };

function node(overrides = {}) {
  const now = new Date().toISOString();
  return {
    id: "visitor-node", display_name: "Visitor node", hostname: "node.example.test", os: "Linux", os_version: "Debian", kernel_version: "6.1", architecture: "x86_64", agent_version: "0.3.0",
    enrolled_at: now, group_id: null, group_name: null, last_seen_at: now, browser_latency_url: config.browser_latency_url, online: true,
    latest: {
      collected_at: now, cpu_percent: 12, memory_used_bytes: 100, memory_total_bytes: 200, swap_used_bytes: 0, swap_total_bytes: 0, disk_used_bytes: 20, disk_total_bytes: 100,
      network_received_bytes_per_sec: 1024, network_transmitted_bytes_per_sec: 512, temperature_celsius: null, load_one: 0.1, load_five: 0.1, load_fifteen: 0.1,
      hub_latency_ms: 20, uptime_seconds: 100, process_count: 1, disks: [], processes: [], interfaces: [],
    },
    ...overrides,
  };
}

const tableProps = { nodes: [node()], groups: [], onSelect() {}, onCreateGroup: async () => ({}), onAssignGroup: async () => {} };
const detailProps = { node: node(), onBack() {}, onDelete: async () => {}, onRename: async () => node(), onUnauthorized() {}, onConfigSaved() {} };
let root;
async function mount(component) {
  window.localStorage.clear();
  globalThis.fetch = async (path) => new Response(JSON.stringify(String(path).endsWith("/monitoring") ? config : []), { headers: { "Content-Type": "application/json" } });
  root = createRoot(container);
  await act(async () => root.render(component));
}
async function cleanup() { await act(async () => root.unmount()); }
async function render(component) { await act(async () => root.render(component)); }
async function click(element) {
  assert.ok(element, "expected latency view control exists");
  await act(async () => element.dispatchEvent(new MouseEvent("click", { bubbles: true })));
}
function reading(scope, label) {
  const row = [...scope.querySelectorAll(".latency-reading")].find((element) => element.firstElementChild.textContent === label);
  assert.ok(row, `${label} should have its own reading`);
  return row.querySelector("strong").textContent;
}

test("host cards and compact list show distinct Hub and current-browser latency", async () => {
  const browserLatency = { "visitor-node": { status: "ok", milliseconds: 136.6 } };
  await mount(React.createElement(NodeTable, { ...tableProps, browserLatency }));
  try {
    assert.equal(reading(container.querySelector(".card-latency"), "Hub 延迟"), "20 ms");
    assert.equal(reading(container.querySelector(".card-latency"), "浏览器访问"), "137 ms");
    await click(container.querySelector('[aria-label="表格视图"]'));
    const footer = container.querySelector(".server-list-footer");
    assert.equal(reading(footer, "Hub 延迟"), "20 ms");
    assert.equal(reading(footer, "浏览器访问"), "137 ms");
    assert.equal(footer.querySelector('[data-status="ok"]').title.includes("当前浏览器"), true);
  } finally { await cleanup(); }
});

test("card and list browser readings keep missing, unavailable and offline states honest", async () => {
  const states = [
    [undefined, "测量中"],
    [{ status: "unconfigured" }, "未配置"],
    [{ status: "unreachable", milliseconds: 0 }, "不可达 / 访问受限"],
    [{ status: "offline", milliseconds: 0 }, "节点离线"],
    [{ status: "paused" }, "已暂停"],
    [{ status: "reload_required" }, "需刷新页面"],
    [{ status: "invalid" }, "测点地址无效"],
    [{ status: "ok" }, "暂无测量"],
    [{ status: "ok", milliseconds: Number.NaN }, "暂无测量"],
    [{ status: "ok", milliseconds: Number.POSITIVE_INFINITY }, "暂无测量"],
    [{ status: "ok", milliseconds: 0 }, "0 ms"],
  ];
  await mount(React.createElement(NodeTable, tableProps));
  try {
    for (const mode of ["cards", "list"]) {
      if (mode === "list") await click(container.querySelector('[aria-label="表格视图"]'));
      for (const [value, expected] of states) {
        await render(React.createElement(NodeTable, { ...tableProps, nodes: [node({ online: value?.status !== "offline" })], browserLatency: value ? { "visitor-node": value } : {} }));
        const scope = container.querySelector(mode === "cards" ? ".card-latency" : ".server-list-footer");
        assert.equal(reading(scope, "浏览器访问"), expected);
        assert.equal(reading(scope, "Hub 延迟"), "20 ms");
      }
    }
  } finally { await cleanup(); }
});

test("detail header separates browser measurements from Hub and preserves missing sample states", async () => {
  await mount(React.createElement(NodeDetail, { ...detailProps, browserLatency: { status: "ok", milliseconds: 63.2 } }));
  try {
    assert.equal(container.querySelector(".latency-heartbeat:not(.browser-latency-heartbeat) strong").textContent, "20 ms");
    assert.equal(container.querySelector(".browser-latency-heartbeat strong").textContent, "63 ms");
    for (const [browserLatency, expected] of [[{ status: "unreachable" }, "不可达 / 访问受限"], [{ status: "offline" }, "节点离线"], [{ status: "ok" }, "暂无测量"]]) {
      await render(React.createElement(NodeDetail, { ...detailProps, browserLatency }));
      assert.equal(container.querySelector(".browser-latency-heartbeat strong").textContent, expected);
    }
    await render(React.createElement(NodeDetail, { ...detailProps, node: node({ latest: null }), browserLatency: { status: "unconfigured" } }));
    assert.equal(container.querySelector(".latency-heartbeat:not(.browser-latency-heartbeat) strong").textContent, "--");
    assert.equal(container.querySelector(".browser-latency-heartbeat strong").textContent, "未配置");
  } finally { await cleanup(); }
});

test("browser measurement setup explains its node endpoint and uses the current dashboard Origin", async () => {
  await mount(React.createElement(MonitoringConfig, { nodeId: "visitor-node", config, onSaved() {}, onUnauthorized() {} }));
  try {
    const help = container.querySelector(".browser-latency-help");
    assert.ok(help);
    assert.match(help.textContent, /\/pinglake\/latency/);
    assert.match(help.textContent, /dashboard_origin = "https:\/\/monitor\.example\.test"/);
    assert.match(help.textContent, /CORS/);
    assert.match(help.textContent, /Cache-Control: no-store/);
    assert.match(container.querySelector("#browser-latency-description").textContent, /当前打开控制台的浏览器/);
  } finally { await cleanup(); }
});
