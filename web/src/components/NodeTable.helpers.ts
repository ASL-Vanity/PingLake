import type { NodeSnapshot } from "../types";
import type { BrowserLatency } from "../hooks/useBrowserLatency";

export type NodeStatusFilter = "all" | "online" | "offline";
export type NodeSortField = "status" | "name" | "cpu" | "memory" | "disk" | "latency";
export type NodeSortDirection = "asc" | "desc";
export interface NodeQuery { query: string; status: NodeStatusFilter; groupId: string; sort: NodeSortField; direction: NodeSortDirection; latencies?: Record<string, BrowserLatency | undefined> }
const collator = new Intl.Collator("zh-CN", { numeric: true, sensitivity: "base" });
function finite(value: number | null | undefined): number | null { return value != null && Number.isFinite(value) ? value : null; }
function percentage(used?: number, total?: number): number | null { return used != null && total != null && total > 0 ? finite(used / total) : null; }
function metric(node: NodeSnapshot, options: NodeQuery): number | null {
  if (options.sort === "cpu") return finite(node.latest?.cpu_percent);
  if (options.sort === "memory") return percentage(node.latest?.memory_used_bytes, node.latest?.memory_total_bytes);
  if (options.sort === "disk") return percentage(node.latest?.disk_used_bytes, node.latest?.disk_total_bytes);
  const latency = options.latencies?.[node.id];
  return latency?.status === "ok" ? finite(latency.milliseconds) : null;
}
export function filterAndSortNodes(nodes: NodeSnapshot[], options: NodeQuery): NodeSnapshot[] {
  const term = options.query.trim().toLocaleLowerCase("zh-CN");
  return nodes.filter((node) => {
    if (options.status === "online" && !node.online || options.status === "offline" && node.online) return false;
    if (options.groupId === "ungrouped" && node.group_id) return false;
    if (options.groupId !== "all" && options.groupId !== "ungrouped" && node.group_id !== options.groupId) return false;
    return !term || [node.display_name, node.hostname, node.os, node.os_version, node.group_name ?? ""].some((value) => value.toLocaleLowerCase("zh-CN").includes(term));
  }).sort((left, right) => {
    let result: number;
    if (options.sort === "status") result = Number(left.online) - Number(right.online);
    else if (options.sort === "name") result = collator.compare(left.display_name || left.hostname, right.display_name || right.hostname);
    else {
      const leftMetric = metric(left, options); const rightMetric = metric(right, options);
      // Unknown values stay last in both directions.
      if (leftMetric == null && rightMetric != null) return 1;
      if (rightMetric == null && leftMetric != null) return -1;
      result = leftMetric != null && rightMetric != null ? leftMetric - rightMetric : 0;
    }
    if (options.direction === "desc") result *= -1;
    return result || Number(right.online) - Number(left.online) || collator.compare(left.display_name || left.hostname, right.display_name || right.hostname) || collator.compare(left.id, right.id);
  });
}
