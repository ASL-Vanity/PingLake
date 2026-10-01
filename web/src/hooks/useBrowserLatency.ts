import { useEffect, useMemo, useRef, useState } from "react";
import type { NodeSnapshot } from "../types";
import { measureBrowserEndpoint } from "../browserLatency";

export type BrowserLatencyStatus = "unconfigured" | "measuring" | "ok" | "unreachable" | "offline" | "paused" | "reload_required" | "invalid";
export interface BrowserLatency { status: BrowserLatencyStatus; milliseconds?: number; measuredAt?: number }

export function useBrowserLatency(nodes: NodeSnapshot[]) {
  const [results, setResults] = useState<Record<string, BrowserLatency>>({});
  const initialOrigins = useRef<Set<string> | null>(null);
  const descriptors = JSON.stringify(nodes.map((node) => ({ id: node.id, url: node.browser_latency_url ?? null, online: node.online })).sort((a, b) => a.id.localeCompare(b.id)));
  const targets = useMemo(() => JSON.parse(descriptors) as { id: string; url: string | null; online: boolean }[], [descriptors]);

  useEffect(() => {
    if (targets.length && initialOrigins.current === null) {
      initialOrigins.current = new Set(targets.flatMap((target) => {
        try { return target.url ? [new URL(target.url).origin] : []; } catch { return []; }
      }));
    }
    let closed = false;
    let running = false;
    let scheduled: number | undefined;
    const controllers = new Set<AbortController>();
    const publish = (id: string, result: BrowserLatency) => {
      if (!closed) setResults((current) => ({ ...current, [id]: result }));
    };
    const measure = async (target: (typeof targets)[number]) => {
      if (!target.url) { publish(target.id, { status: "unconfigured" }); return; }
      if (!target.online) { publish(target.id, { status: "offline" }); return; }
      let url: URL;
      try {
        url = new URL(target.url);
        if (url.protocol !== "https:" || url.username || url.password) throw new Error("invalid URL");
      } catch { publish(target.id, { status: "invalid" }); return; }
      if (!initialOrigins.current?.has(url.origin)) { publish(target.id, { status: "reload_required" }); return; }
      if (document.hidden) { publish(target.id, { status: "paused" }); return; }
      publish(target.id, { status: "measuring" });
      const controller = new AbortController();
      controllers.add(controller);
      const timeout = window.setTimeout(() => controller.abort(), 5_000);
      try {
        const milliseconds = await measureBrowserEndpoint(url.href, controller.signal);
        if (!document.hidden) publish(target.id, { status: "ok", milliseconds, measuredAt: Date.now() });
      } catch {
        if (!closed) publish(target.id, { status: document.hidden ? "paused" : "unreachable", measuredAt: Date.now() });
      } finally { clearTimeout(timeout); controllers.delete(controller); }
    };
    const cycle = async () => {
      if (closed || running || document.hidden) return;
      running = true;
      let index = 0;
      // A single queue bounds concurrent requests across all node cards.
      await Promise.all(Array.from({ length: Math.min(4, targets.length) }, async () => {
        while (!closed && !document.hidden && index < targets.length) await measure(targets[index++]!);
      }));
      running = false;
      if (!closed && !document.hidden) scheduled = window.setTimeout(() => void cycle(), 30_000);
    };
    const onVisibility = () => {
      clearTimeout(scheduled);
      if (document.hidden) {
        controllers.forEach((controller) => controller.abort());
        targets.filter((target) => target.url && target.online).forEach((target) => publish(target.id, { status: "paused" }));
      } else if (!running) void cycle();
    };
    targets.forEach((target) => publish(target.id, { status: !target.url ? "unconfigured" : !target.online ? "offline" : document.hidden ? "paused" : "measuring" }));
    void cycle();
    document.addEventListener("visibilitychange", onVisibility);
    return () => { closed = true; clearTimeout(scheduled); controllers.forEach((controller) => controller.abort()); document.removeEventListener("visibilitychange", onVisibility); };
  }, [targets]);
  return results;
}

export function browserLatencyLabel(value?: BrowserLatency): string {
  if (!value) return "测量中";
  if (value.status === "ok") return `${value.milliseconds?.toFixed(0)} ms`;
  return ({ unconfigured: "未配置", measuring: "测量中", unreachable: "不可达 / 访问受限", offline: "节点离线", paused: "已暂停", reload_required: "需刷新页面", invalid: "测点地址无效" })[value.status];
}
