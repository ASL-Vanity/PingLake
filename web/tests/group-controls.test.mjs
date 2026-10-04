import assert from "node:assert/strict";
import test, { after } from "node:test";
import { mkdir, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { build } from "esbuild";
import { JSDOM } from "jsdom";
import React, { act } from "react";
import { createRoot } from "react-dom/client";

const output = new URL("../.tmp/group-controls-fixture.mjs", import.meta.url);
const bundled = await build({
  stdin: {
    contents: 'export { default as App } from "./src/App"; export { GroupView } from "./src/components/GroupView";',
    resolveDir: fileURLToPath(new URL("..", import.meta.url)), loader: "tsx",
  },
  bundle: true, write: false, format: "esm", platform: "node", packages: "external", jsx: "automatic",
  plugins: [{ name: "ignore-fixture-styles", setup(builder) {
    builder.onLoad({ filter: /\.module\.css$/ }, () => ({ contents: "export default {}", loader: "js" }));
    builder.onLoad({ filter: /\.css$/ }, () => ({ contents: "", loader: "js" }));
  } }],
});
await mkdir(new URL("../.tmp", import.meta.url), { recursive: true });
await writeFile(output, bundled.outputFiles[0].text);
const { App, GroupView } = await import(output.href);
const dom = new JSDOM('<!doctype html><html><body><div id="test"></div></body></html>', { url: "http://fixture.test/" });
after(() => dom.window.close());
for (const name of ["window", "document", "HTMLElement", "HTMLButtonElement", "Event", "MouseEvent", "KeyboardEvent"]) {
  Object.defineProperty(globalThis, name, { value: dom.window[name], configurable: true });
}
Object.defineProperty(globalThis, "navigator", { value: dom.window.navigator, configurable: true });
dom.window.HTMLElement.prototype.attachEvent ??= () => {};
dom.window.HTMLElement.prototype.detachEvent ??= () => {};
globalThis.IS_REACT_ACT_ENVIRONMENT = true;
globalThis.ResizeObserver = class { observe() {} unobserve() {} disconnect() {} };
globalThis.EventSource = class { addEventListener() {} close() {} };
window.matchMedia = () => ({ matches: false, addEventListener() {}, removeEventListener() {} });
window.scrollTo = () => {};
const container = document.getElementById("test");
const group = { id: "group-db", name: "数据库", created_at: new Date().toISOString() };
const sample = {
  id: "node-alpha", display_name: "Node Alpha", hostname: "alpha.example", os: "Linux", os_version: "Debian",
  kernel_version: "6.1", architecture: "x86_64", agent_version: "0.3.0", enrolled_at: new Date().toISOString(),
  group_id: group.id, group_name: group.name, last_seen_at: new Date().toISOString(),
  browser_latency_url: null, online: true, latest: null,
};
const settings = {
  cpu_percent: 85, memory_percent: 90, disk_percent: 85, temperature_celsius: 85,
  offline_after_seconds: 20, sustained_for_seconds: 60,
  offline_enabled: true, cpu_enabled: true, memory_enabled: true, disk_enabled: true, temperature_enabled: true,
  webhook_enabled: false, webhook_url: "", email_enabled: false, email_recipients: [],
};
let root;
let requests;

function button(text, scope = container) {
  return [...scope.querySelectorAll("button")].find((element) => element.textContent.trim() === text);
}
async function click(element) {
  assert.ok(element, "expected control exists");
  await act(async () => element.dispatchEvent(new MouseEvent("click", { bubbles: true })));
}
async function mount(component) {
  window.history.replaceState(null, "", "/");
  window.localStorage.clear();
  requests = [];
  globalThis.fetch = async (path, options) => {
    const url = String(path);
    requests.push({ path: url, options });
    if (options?.method === "DELETE") return new Response(null, { status: 204 });
    const value = url.endsWith("/auth/me") ? { authenticated: true }
      : url.endsWith("/nodes") ? [sample]
        : url.endsWith("/groups") ? [group]
          : url.endsWith("/settings") ? settings
            : url.endsWith("/summary") ? { total_nodes: 1, online_nodes: 1, offline_nodes: 0, active_alerts: 0, average_cpu_percent: null, average_memory_percent: null }
              : [];
    return new Response(JSON.stringify(value), { headers: { "Content-Type": "application/json" } });
  };
  root = createRoot(container);
  await act(async () => root.render(component));
}
async function cleanup() {
  await act(async () => root.unmount());
  window.history.replaceState(null, "", "/");
}
function groupProps(onDelete) {
  return { groups: [group], nodes: [sample], onCreate: async () => group, onDelete, onAssign: async () => {}, onOpenNode() {}, onOpenHosts() {} };
}

test("desktop navigation and host toolbar expose group management; mobile drawer reaches all workflows", async () => {
  await mount(React.createElement(App));
  try {
    const desktop = container.querySelector(".topbar-nav");
    assert.deepEqual([...desktop.querySelectorAll("button")].map((item) => item.textContent), ["概览", "主机", "服务", "探测", "告警", "分组"]);
    await click(button("管理分组", container.querySelector("main")));
    assert.equal(window.location.hash, "#groups");
    assert.ok(container.querySelector(".group-workspace"));
    await click(button("主机", desktop));
    await click(button("管理分组", container.querySelector("main")));
    assert.equal(window.location.hash, "#groups");
    await click(button("概览", desktop));
    await click(button("更多", container.querySelector(".mobile-nav")));
    assert.ok(container.querySelector(".app-sidebar.open"));
    const drawer = container.querySelector(".sidebar-nav");
    assert.equal(drawer.querySelectorAll("button").length, 6);
    await click(button("分组", drawer));
    assert.equal(window.location.hash, "#groups");
    assert.equal(container.querySelector(".app-sidebar.open"), null);
    assert.deepEqual([...container.querySelectorAll(".mobile-nav button")].map((item) => item.textContent), ["概览", "主机", "告警", "更多"]);
  } finally { await cleanup(); }
});

test("selected-group deletion has a text action and cancellation restores focus without a request", async () => {
  const deleted = [];
  await mount(React.createElement(GroupView, groupProps(async (id) => deleted.push(id))));
  try {
    await click([...container.querySelectorAll(".group-directory button")].find((item) => item.textContent.includes(group.name)));
    const remove = button("删除分组", container.querySelector(".group-members-toolbar"));
    remove.focus();
    await click(remove);
    const dialog = container.querySelector('[role="alertdialog"]');
    assert.match(dialog.textContent, /1 台主机将移入未分组，主机和历史数据会保留/);
    assert.equal(document.activeElement.textContent, "取消");
    await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    assert.equal(container.querySelector('[role="alertdialog"]'), null);
    assert.equal(document.activeElement, remove);
    assert.deepEqual(deleted, []);
    assert.equal(container.querySelectorAll(".group-member-row").length, 1);
  } finally { await cleanup(); }
});

test("failed group deletion remains retryable and successful API deletion preserves the member node", async () => {
  let rejectDelete;
  await mount(React.createElement(GroupView, groupProps(() => new Promise((resolve, reject) => { rejectDelete = reject; }))));
  try {
    await click(container.querySelector(`[aria-label="删除分组 ${group.name}"]`));
    await click(button("确认删除"));
    assert.ok(button("正在删除").disabled);
    await act(async () => document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
    assert.ok(container.querySelector('[role="alertdialog"]'));
    await act(async () => rejectDelete(new Error("删除被拒绝")));
    assert.equal(container.querySelector('[role="alertdialog"] [role="alert"]').textContent, "删除被拒绝");
    assert.equal(button("确认删除").disabled, false);
    assert.equal(container.querySelectorAll(".group-member-row").length, 1);
    await click(button("取消"));
  } finally { await cleanup(); }

  await mount(React.createElement(App));
  try {
    await click(button("分组", container.querySelector(".topbar-nav")));
    await click(container.querySelector(`[aria-label="删除分组 ${group.name}"]`));
    await click(button("确认删除"));
    assert.equal(container.querySelector('[role="alertdialog"]'), null);
    assert.equal(container.querySelectorAll(".group-directory-row").length, 0);
    assert.equal(container.querySelectorAll(".group-member-row").length, 1);
    assert.equal(container.querySelector(".group-member-row select").value, "");
    assert.equal(requests.filter(({ options }) => options?.method === "DELETE").length, 1);
    assert.equal(requests.find(({ options }) => options?.method === "DELETE").path, `/api/v1/groups/${group.id}`);
    await click(button("主机", container.querySelector(".topbar-nav")));
    assert.match(container.querySelector("main").textContent, /Node Alpha/);
  } finally { await cleanup(); }
});
