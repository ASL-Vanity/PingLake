import assert from "node:assert/strict";
import test, { after } from "node:test";
import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";

const output = new URL("../.tmp/ui-fixture.mjs", import.meta.url);
const bundled = await build({
  stdin: { contents: 'export {default as App} from "./src/App"; export {NodeTable} from "./src/components/NodeTable"; export {GroupView} from "./src/components/GroupView"; export {SettingsDrawer} from "./src/components/SettingsDrawer"; export {NodeDetail} from "./src/components/NodeDetail"; export {MonitoringConfig} from "./src/components/MonitoringConfig"; export {FleetChecksView} from "./src/components/FleetChecksView"; export {useHistoryQuery, clearHistoryCache} from "./src/hooks/useHistoryQuery"; export {buildHistorySeries} from "./src/components/MonitoringHistory.helpers";', resolveDir: fileURLToPath(new URL("..", import.meta.url)), loader: "tsx" },
  bundle: true, write: false, format: "esm", platform: "node", packages: "external", jsx: "automatic",
  plugins: [{ name: "fixture-style-modules", setup(builder) {
    builder.onLoad({ filter: /\.module\.css$/ }, () => ({ contents: "export default {}", loader: "js" }));
    builder.onLoad({ filter: /\.css$/ }, () => ({ contents: "", loader: "js" }));
  } }],
});
await mkdir(new URL("../.tmp", import.meta.url), { recursive: true });
await writeFile(output, bundled.outputFiles[0].text);
const { App, NodeTable, GroupView, SettingsDrawer, NodeDetail, MonitoringConfig, FleetChecksView, useHistoryQuery, clearHistoryCache, buildHistorySeries } = await import(output.href);
const dom = new JSDOM('<!doctype html><html><body><div id="test"></div></body></html>', { url: "http://fixture.test/" });
after(() => dom.window.close());
for (const name of ["window", "document", "HTMLElement", "HTMLButtonElement", "HTMLInputElement", "HTMLSelectElement", "Event", "MouseEvent", "KeyboardEvent"]) Object.defineProperty(globalThis, name, { value: dom.window[name], configurable: true });
Object.defineProperty(globalThis, "navigator", { value: dom.window.navigator, configurable: true });
if (!dom.window.HTMLElement.prototype.attachEvent) dom.window.HTMLElement.prototype.attachEvent = () => {};
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
globalThis.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
globalThis.EventSource = class { addEventListener() {} close() {} };
window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
window.scrollTo = () => {};
const container = document.getElementById("test");
let root;
let requests;
const nodeId = "00000000-0000-4000-8000-000000000001";
const group = { id: "00000000-0000-4000-8000-000000000099", name: "数据库", created_at: new Date().toISOString() };
const settings = { cpu_percent: 85, memory_percent: 90, disk_percent: 85, temperature_celsius: 85, offline_after_seconds: 20, sustained_for_seconds: 60, offline_enabled: true, cpu_enabled: true, memory_enabled: true, disk_enabled: true, temperature_enabled: true, webhook_enabled: false, webhook_url: "", email_enabled: false, email_recipients: [] };
const config = { revision: 1, browser_latency_url: null, services: [{ id: "service1", name: "dbus.service", enabled: true, expected_state: "running" }], probes: [{ id: "probe1", name: "网站", kind: "http", target: "https://example.com", port: null, enabled: true, interval_secs: 30, timeout_ms: 1000, expected_status: 200, response_contains: null }], dns_checks: [], process_checks: [], local_port_checks: [] };
function node(override = {}) {
  const now = new Date().toISOString();
  return { id: nodeId, display_name: "Node Alpha", hostname: "alpha.example", os: "Linux", os_version: "Debian", kernel_version: "6.1", architecture: "x86_64", agent_version: "0.1.0", enrolled_at: now, group_id: group.id, group_name: group.name, last_seen_at: now, browser_latency_url: null, online: true, latest: { collected_at: now, cpu_percent: 12, memory_used_bytes: 100, memory_total_bytes: 200, swap_used_bytes: 0, swap_total_bytes: 0, disk_used_bytes: 20, disk_total_bytes: 100, network_received_bytes_per_sec: 1024, network_transmitted_bytes_per_sec: 512, temperature_celsius: null, load_one: 0.1, load_five: 0.1, load_fifteen: 0.1, hub_latency_ms: 20, uptime_seconds: 100, process_count: 1, disks: [], processes: [], interfaces: [], monitoring: { schema_version: 1, session_id: "session1", sample_sequence: 1, report_interval_secs: 5, capabilities: {}, cpu_cores: [], cpu_times: {}, memory: {}, disk_io: [], inodes: [], network_health: [], tcp: { states: {}, listening_ports: 0, listening_sockets: 0 }, services: [{ id: "service1", name: "dbus.service", status: "ok", state: "running", healthy: true, checked_at: now, config_revision: 1 }], probes: [{ sample_id: "sample1", target_id: "probe1", config_revision: 1, status: "success", kind: "http", completed_at: now, scheduled_at: now, latency_ms: 20 }], agent: { applied_config_revision: 1, config_error: null, sample_age_ms: 0, collection_duration_ms: 1, upload_attempts: 1, upload_successes: 1, upload_failures: 0, consecutive_failures: 0, retries: 0, dropped_reports: 0, queue_length: 0, last_error: null } } }, ...override };
}
function fixtureFetch() {
  requests = [];
  globalThis.fetch = async (path, options) => {
    requests.push({ path: String(path), options });
    const url = String(path);
    const data = url.endsWith("/auth/me") ? { authenticated: true } : url.endsWith("/nodes") ? [node()]
      : url.endsWith("/summary") ? { total_nodes: 1, online_nodes: 1, offline_nodes: 0, active_alerts: 0, average_cpu_percent: 12, average_memory_percent: 50 }
      : url.endsWith("/groups") ? [group] : url.endsWith("/settings") ? settings
      : url.endsWith("/monitoring") ? config : [];
    return new Response(JSON.stringify(data), { headers: { "Content-Type": "application/json" } });
  };
}
function button(text) { return [...container.querySelectorAll("button")].find((button) => button.textContent.trim() === text); }
async function click(element) { assert.ok(element, "expected control exists"); await act(async () => element.dispatchEvent(new MouseEvent("click", { bubbles: true }))); }
async function change(element, value) {
  assert.ok(element);
  await act(async () => {
    const proto = element instanceof HTMLSelectElement ? HTMLSelectElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, "value").set.call(element, value);
    element.dispatchEvent(new Event(element instanceof HTMLSelectElement ? "change" : "input", { bubbles: true }));
  });
}
async function mount(component) {
  window.localStorage.clear(); fixtureFetch(); clearHistoryCache();
  root = createRoot(container); await act(async () => root.render(component));
}
async function cleanup() { await act(async () => root.unmount()); clearHistoryCache(); window.history.replaceState(null, "", "/"); }

test("main navigation exposes all monitoring workflows and restores hash history", async () => {
  await mount(React.createElement(App));
  try {
    const nav = container.querySelector('[aria-label="主导航"]');
    assert.equal(nav.querySelectorAll("button").length, 6);
    for (const label of ["主机", "服务", "探测", "告警", "分组"]) {
      await click([...nav.querySelectorAll("button")].find((button) => button.textContent === label));
      assert.ok(window.location.hash.length > 1);
      assert.ok(container.querySelector("main").textContent.length > 0);
    }
    await click(button("告警设置"));
    assert.ok(container.querySelector('[role="dialog"]'));
    await click(container.querySelector('[aria-label="关闭设置"]'));
    await act(async () => { window.history.replaceState(null, "", "#hosts?group=ungrouped"); window.dispatchEvent(new Event("popstate")); });
    assert.equal(container.querySelector('[aria-label="主导航"] button.active').textContent, "主机");
    assert.match(container.textContent, /没有匹配的主机/);
  } finally { await cleanup(); }
});

test("card/table view and offline filter preserve labels and catch assignment failures", async () => {
  await mount(React.createElement(NodeTable, { nodes: [node(), node({ id: "b", display_name: "Offline Beta", online: false })], groups: [group], onSelect() {}, onAssignGroup: async () => { throw new Error("assignment denied"); }, browserLatency: {} }));
  try {
    await click(container.querySelector('[aria-label="表格视图"]'));
    assert.ok(container.querySelector(".server-list"));
    for (const name of ["CPU", "内存", "磁盘"]) assert.match(container.textContent, new RegExp(name));
    assert.equal(container.querySelectorAll('[role="meter"]').length, 6);
    await click([...container.querySelectorAll('[aria-label="主机状态"] button')].find((button) => button.textContent === "离线"));
    assert.equal(container.querySelectorAll('[role="listitem"]').length, 1);
    assert.match(container.textContent, /最后心跳/);
    await change(container.querySelector('select[aria-label^="为 Offline Beta"]'), "");
    assert.match(container.querySelector('[role="alert"]').textContent, /assignment denied/);
  } finally { await cleanup(); }
});

test("deleting a populated group confirms preservation and cancellation has no effect", async () => {
  const deleted = [];
  await mount(React.createElement(GroupView, { nodes: [node()], groups: [group], onCreate: async () => group, onDelete: async (id) => deleted.push(id), onAssign: async () => {}, onOpenNode() {}, onOpenHosts() {} }));
  try {
    await click(container.querySelector('[aria-label="删除分组 数据库"]'));
    assert.match(container.querySelector('[role="alertdialog"]').textContent, /1 台主机.*历史数据会保留/);
    await click(button("取消")); assert.deepEqual(deleted, []);
    await click(container.querySelector('[aria-label="删除分组 数据库"]'));
    await click(button("确认删除")); assert.deepEqual(deleted, [group.id]);
  } finally { await cleanup(); }
});

test("node detail has one navigation and shared range survives tab changes", async () => {
  const tabs = [];
  await mount(React.createElement(NodeDetail, { node: node(), onBack() {}, onDelete: async () => {}, onRename: async () => node(), onUnauthorized() {}, onConfigSaved() {}, onTabChange: (tab) => tabs.push(tab) }));
  try {
    assert.equal(container.querySelectorAll('[role="tablist"]').length, 1);
    assert.equal(container.querySelectorAll('[role="tab"]').length, 7);
    assert.equal(container.querySelector('[aria-label="监测时间范围"]').querySelectorAll("button").length, 3);
    await click(button("24 小时"));
    await click(container.querySelector("#monitoring-tab-network"));
    assert.equal(button("24 小时").getAttribute("aria-pressed"), "true");
    assert.deepEqual(tabs, ["network"]);
    assert.ok(requests.some(({ path }) => path.includes("minutes=1440") && path.includes("section=network")));
    assert.equal(container.querySelectorAll('.node-overview').length, 0);
    await click(container.querySelector("#monitoring-tab-config"));
    assert.equal(container.querySelector('[aria-label="监测时间范围"]'), null);
    assert.ok(container.querySelector(".monitoring-config"));
  } finally { await cleanup(); }
});

test("v1 nodes remain readable while v2 checks expose DNS, process and local-port states", async () => {
  const v2 = node();
  v2.latest.monitoring.schema_version = 2;
  const checkedAt = new Date().toISOString();
  v2.latest.monitoring.probes.push({ sample_id: "dns-sample", target_id: "dns1", config_revision: 1, kind: "dns", status: "policy_denied", completed_at: checkedAt, scheduled_at: checkedAt, latency_ms: null, http_status: null, error: "blocked", healthy: null, dns: { record_type: "A", rcode: 5, answers: [] } });
  v2.latest.monitoring.process_checks = [{ id: "proc1", name: "worker", config_revision: 1, status: "permission_denied", process_name: "worker", count: null, expected_count: 1, checked_at: checkedAt, error: "denied", reason: null, healthy: null }];
  v2.latest.monitoring.local_port_checks = [{ id: "port1", name: "HTTP", config_revision: 1, status: "unavailable", address_scope: { scope: "any_local" }, address_family: "any", protocol: "tcp", port: 8080, checked_at: checkedAt, observed_addresses: [], latency_ms: null, error: "missing", reason: null, healthy: null }];
  const v2Config = { ...config, probes: [...config.probes, { id: "dns1", name: "DNS", kind: "dns", target: "example.com", port: null, enabled: true, interval_secs: 30, timeout_ms: 5000, expected_status: null, response_contains: null, dns: { record_type: "A", expected_value: null } }], process_checks: [{ id: "proc1", name: "worker", process_name: "worker", expected_count: 1, enabled: true, expected_state: "running", interval_secs: 30, timeout_ms: 5000 }], local_port_checks: [{ id: "port1", name: "HTTP", address_scope: { scope: "any_local" }, address_family: "any", protocol: "tcp", port: 8080, enabled: true, interval_secs: 30, timeout_ms: 5000 }] };
  Object.assign(config, v2Config);
  await mount(React.createElement(NodeDetail, { node: v2, initialTab: "probes", onBack() {}, onDelete: async () => {}, onRename: async () => v2, onUnauthorized() {}, onConfigSaved() {} }));
  try {
    assert.match(container.textContent, /DNS/);
    assert.match(container.textContent, /策略禁止/);
    assert.match(container.textContent, /本机端口/);
    assert.match(container.textContent, /采集失败/);
  } finally { await cleanup(); }
  await mount(React.createElement(MonitoringConfig, { nodeId, config: v2Config, onSaved() {}, onUnauthorized() {} }));
  try {
    assert.ok(container.querySelector('input[aria-label="不存在"]') == null);
    assert.match(container.textContent, /DNS/);
    assert.match(container.textContent, /进程检查/);
    assert.match(container.textContent, /本机端口检查/);
    const firstText = container.querySelectorAll("input")[0];
    firstText.focus();
    assert.equal(document.activeElement, firstText);
    await act(async () => container.querySelector("form").dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    assert.ok(requests.some(({ options }) => options?.method === "PUT"));
  } finally { Object.assign(config, { dns_checks: [], process_checks: [], local_port_checks: [] }); await cleanup(); }
});

test("history ignores obsolete responses and retains same-scope data during refresh", async () => {
  const pending = [];
  const fetcher = (signal) => new Promise((resolve) => pending.push({ signal, resolve }));
  const unauthorized = () => {};
  function Fixture({ scope }) {
    const history = useHistoryQuery(scope, fetcher, unauthorized);
    return React.createElement("div", null, React.createElement("span", { id: "history" }, history.data ?? "loading"), React.createElement("button", { onClick: history.refresh }, "刷新"));
  }
  await mount(React.createElement(Fixture, { scope: "A" }));
  try {
    await act(async () => root.render(React.createElement(Fixture, { scope: "B" })));
    assert.equal(pending[0].signal.aborted, true);
    await act(async () => { pending[1].resolve("B data"); });
    await act(async () => { pending[0].resolve("obsolete A"); });
    assert.equal(container.querySelector("#history").textContent, "B data");
    await click(button("刷新"));
    assert.equal(container.querySelector("#history").textContent, "B data");
    await act(async () => { pending[2].resolve("B updated"); });
    assert.equal(container.querySelector("#history").textContent, "B updated");
  } finally { await cleanup(); }
});

test("history retains a previous range only for the same node and device", async () => {
  const pending = [];
  const fetcher = () => new Promise((resolve) => pending.push(resolve));
  const unauthorized = () => {};
  function Fixture({ range, device }) {
    const history = useHistoryQuery(`${device}:${range}`, fetcher, unauthorized, device);
    return React.createElement("div", null, React.createElement("span", { id: "history" }, history.data ?? "loading"), history.retained ? "previous range" : "");
  }
  await mount(React.createElement(Fixture, { range: 60, device: "disk-a" }));
  try {
    await act(async () => pending[0]("range 60"));
    await act(async () => root.render(React.createElement(Fixture, { range: 360, device: "disk-a" })));
    assert.match(container.textContent, /range 60previous range/);
    await act(async () => root.render(React.createElement(Fixture, { range: 360, device: "disk-b" })));
    assert.equal(container.querySelector("#history").textContent, "loading");
    await act(async () => pending[2]("disk-b data"));
    await act(async () => pending[1]("obsolete disk-a"));
    assert.equal(container.querySelector("#history").textContent, "disk-b data");
  } finally { await cleanup(); }
});

test("a failed request for a new device exposes the error instead of reporting no data", async () => {
  const pending = [];
  const fetcher = () => new Promise((resolve, reject) => pending.push({ resolve, reject }));
  const unauthorized = () => {};
  function Fixture({ device }) {
    const history = useHistoryQuery(device, fetcher, unauthorized);
    return React.createElement("div", null, history.data ?? "loading", history.error && React.createElement("p", { role: "alert" }, history.error));
  }
  await mount(React.createElement(Fixture, { device: "disk-a" }));
  try {
    await act(async () => pending[0].resolve("disk-a value"));
    await act(async () => root.render(React.createElement(Fixture, { device: "disk-b" })));
    await act(async () => pending[1].reject(new Error("device query failed")));
    assert.equal(container.querySelector('[role="alert"]').textContent, "device query failed");
    assert.doesNotMatch(container.textContent, /disk-a value/);
  } finally { await cleanup(); }
});

test("sparse event series and outage gaps retain all real observations", () => {
  const sample = (seconds, reading) => ({ collected_at: new Date(seconds * 1000).toISOString(), received_at: new Date((seconds + 10) * 1000).toISOString(), reading, monitoring: { session_id: "same", report_interval_secs: 0 } });
  const points = [sample(0, 10), sample(300, 20), sample(900, 30)];
  const normal = buildHistorySeries(points, { events: true, intervalSecs: 300, minutes: 60 }, (point) => point.reading);
  assert.deepEqual(normal.map((point) => point.value), [10, 20, 30]);
  assert.equal(normal[1].time, points[1].collected_at);
  const gap = buildHistorySeries([sample(0, 10), sample(2000, 20)], { events: true, intervalSecs: 300, minutes: 60 }, (point) => point.reading);
  assert.deepEqual(gap.map((point) => point.value), [10, null, 20]);
});

test("global checks show stale samples and unconfigured agent revisions distinctly", async () => {
  const sample = node();
  sample.latest.monitoring.agent.sample_age_ms = 60_000;
  const unauthorized = () => {};
  const openNode = () => {};
  await mount(React.createElement(FleetChecksView, { kind: "services", nodes: [sample], onUnauthorized: unauthorized, onOpenNode: openNode, refreshKey: 0 }));
  try {
    assert.equal(container.querySelector("tbody .fleet-state").textContent, "数据过期");
    const fresh = node(); fresh.latest.monitoring.agent.applied_config_revision = 0;
    await act(async () => root.render(React.createElement(FleetChecksView, { kind: "services", nodes: [fresh], onUnauthorized: unauthorized, onOpenNode: openNode, refreshKey: 0 })));
    assert.equal(container.querySelector("tbody .fleet-state").textContent, "未应用");
  } finally { await cleanup(); }
});

test("node detail never displays an expired service result as healthy", async () => {
  const sample = node(); sample.latest.monitoring.services[0].checked_at = new Date(Date.now() - 120_000).toISOString();
  await mount(React.createElement(NodeDetail, { node: sample, initialTab: "services", onBack() {}, onDelete: async () => {}, onRename: async () => node(), onUnauthorized() {}, onConfigSaved() {} }));
  try {
    assert.ok(container.querySelector("tbody .monitoring-status.stale"));
    assert.equal(container.querySelector("tbody .monitoring-status.ok"), null);
  } finally { await cleanup(); }
});

test("manual detail refresh fetches configuration and trend data together", async () => {
  const sample = node();
  const props = { node: sample, onBack() {}, onDelete: async () => {}, onRename: async () => node(), onUnauthorized() {}, onConfigSaved() {} };
  await mount(React.createElement(NodeDetail, { ...props, refreshKey: 0 }));
  try {
    const configRequests = () => requests.filter(({ path }) => path.endsWith("/monitoring")).length;
    const historyRequests = () => requests.filter(({ path }) => path.includes("/history?")).length;
    const oldConfigs = configRequests(); const oldHistory = historyRequests();
    await act(async () => root.render(React.createElement(NodeDetail, { ...props, refreshKey: 1 })));
    assert.equal(configRequests(), oldConfigs + 1);
    assert.equal(historyRequests(), oldHistory + 1);
  } finally { await cleanup(); }
});

test("settings fields are locked until save completes and page mode is not a modal", async () => {
  let resolveSave;
  await mount(React.createElement(SettingsDrawer, { open: true, inline: true, settings, nodes: [node()], themePreference: "light", onThemeChange() {}, onClose() {}, onSave: (value) => new Promise((resolve) => { resolveSave = () => resolve(value); }) }));
  try {
    await act(async () => container.querySelector("form").dispatchEvent(new Event("submit", { bubbles: true, cancelable: true })));
    assert.equal(container.querySelector('[role="dialog"]'), null);
    assert.equal(container.querySelector("form").getAttribute("aria-busy"), "true");
    assert.ok([...container.querySelectorAll("input, select")].every((input) => input.matches(":disabled")));
    await act(async () => resolveSave());
    assert.equal(container.querySelector("form").getAttribute("aria-busy"), "false");
    assert.match(container.textContent, /设置已保存/);
  } finally { await cleanup(); }
});
