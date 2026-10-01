import type { DashboardSummary, HistoryPoint, NodeSnapshot } from "./types";
import type { AgentHealth } from "./monitoringTypes";

export function clampPercent(value: number): number {
  if (!Number.isFinite(value)) return 0;
  return Math.min(100, Math.max(0, value));
}

export function ratioPercent(used: number, total: number): number {
  return total > 0 ? clampPercent((used / total) * 100) : 0;
}

export function formatPercent(value: number | null | undefined, digits = 0): string {
  if (value == null || !Number.isFinite(value)) return "--";
  return `${value.toFixed(digits)}%`;
}

export function formatBytes(value: number | null | undefined, digits = 1): string {
  if (value == null || !Number.isFinite(value)) return "--";
  if (value === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  const index = Math.min(Math.floor(Math.log(Math.abs(value)) / Math.log(1024)), units.length - 1);
  const scaled = value / 1024 ** Math.max(index, 0);
  return `${scaled.toFixed(index <= 1 ? 0 : digits)} ${units[Math.max(index, 0)]}`;
}

export function formatRate(value: number | null | undefined): string {
  const bytes = formatBytes(value);
  return bytes === "--" ? bytes : `${bytes}/s`;
}

export function formatLatency(value: number | null | undefined): string {
  if (value == null || !Number.isFinite(value)) return "--";
  if (value < 1_000) return `${Math.round(value)} ms`;
  return `${(value / 1_000).toFixed(2)} s`;
}

export function formatTemperature(value: number | null | undefined): string {
  return value == null || !Number.isFinite(value) ? "--" : `${value.toFixed(0)} °C`;
}

export function formatDuration(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds)) return "--";
  const days = Math.floor(seconds / 86400);
  const hours = Math.floor((seconds % 86400) / 3600);
  const minutes = Math.floor((seconds % 3600) / 60);
  if (days > 0) return `${days}天 ${hours}小时`;
  if (hours > 0) return `${hours}小时 ${minutes}分`;
  return `${minutes}分钟`;
}

export function formatDateTime(value: string | null | undefined): string {
  if (!value) return "--";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "--";
  return new Intl.DateTimeFormat("zh-CN", {
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  }).format(date);
}

export function formatRelativeTime(value: string | null | undefined): string {
  if (!value) return "从未";
  const time = new Date(value).getTime();
  if (!Number.isFinite(time)) return "未知";
  // Last-seen timestamps originate from the Hub. A small server/browser clock skew must never
  // render as a future heartbeat because a received metric cannot occur in the future.
  const seconds = Math.min(0, Math.round((time - Date.now()) / 1000));
  const formatter = new Intl.RelativeTimeFormat("zh-CN", { numeric: "auto" });
  if (Math.abs(seconds) < 60) return formatter.format(seconds, "second");
  const minutes = Math.round(seconds / 60);
  if (Math.abs(minutes) < 60) return formatter.format(minutes, "minute");
  const hours = Math.round(minutes / 60);
  if (Math.abs(hours) < 24) return formatter.format(hours, "hour");
  return formatter.format(Math.round(hours / 24), "day");
}

export function nodeMemoryPercent(node: NodeSnapshot): number {
  return node.latest ? ratioPercent(node.latest.memory_used_bytes, node.latest.memory_total_bytes) : 0;
}

export function nodeDiskPercent(node: NodeSnapshot): number {
  return node.latest ? ratioPercent(node.latest.disk_used_bytes, node.latest.disk_total_bytes) : 0;
}

export type FreshnessState = "fresh" | "aging" | "stale" | "unknown";

/** Combines Hub receive age with the agent-reported sample age. */
export function nodeFreshness(node: NodeSnapshot, now = Date.now()): { state: FreshnessState; label: string; ageMs: number | null } {
  if (!node.online) return { state: "stale", label: "离线 · 最后数据", ageMs: null };
  const received = Date.parse(node.last_seen_at ?? "");
  if (!Number.isFinite(received)) return { state: "unknown", label: "未知", ageMs: null };
  const sampleAge = node.latest?.monitoring?.agent?.sample_age_ms;
  const ageMs = Math.max(0, now - received) + (Number.isFinite(sampleAge) ? Math.max(0, sampleAge as number) : 0);
  const interval = Math.max(5_000, (node.latest?.monitoring?.report_interval_secs ?? 5) * 1_000);
  if (ageMs > Math.max(interval * 3, 45_000)) return { state: "stale", label: `${Math.round(ageMs / 1_000)} 秒前`, ageMs };
  if (ageMs > interval * 1.5) return { state: "aging", label: `${Math.round(ageMs / 1_000)} 秒前`, ageMs };
  return { state: "fresh", label: ageMs < 10_000 ? "刚刚" : `${Math.round(ageMs / 1_000)} 秒前`, ageMs };
}

export function agentQuality(agent: AgentHealth | undefined): { state: "good" | "warning" | "bad" | "unknown"; label: string } {
  if (!agent) return { state: "unknown", label: "未知" };
  if (agent.config_error || agent.consecutive_failures > 0 || agent.upload_failures > 0) return { state: "bad", label: "异常" };
  if ((agent.success_rate_percent != null && agent.success_rate_percent < 99) || agent.queue_length > 0 || agent.dropped_reports > 0) return { state: "warning", label: "需关注" };
  return { state: "good", label: "良好" };
}

export function historyMemoryPercent(point: HistoryPoint): number {
  return ratioPercent(point.memory_used_bytes, point.memory_total_bytes);
}

export function historyDiskPercent(point: HistoryPoint): number {
  return ratioPercent(point.disk_used_bytes, point.disk_total_bytes);
}

export function deriveSummary(nodes: NodeSnapshot[], activeAlerts: number): DashboardSummary {
  const online = nodes.filter((node) => node.online);
  const cpuTotal = online.reduce((sum, node) => sum + (node.latest?.cpu_percent ?? 0), 0);
  const memoryTotal = online.reduce((sum, node) => sum + nodeMemoryPercent(node), 0);
  return {
    total_nodes: nodes.length,
    online_nodes: online.length,
    offline_nodes: nodes.length - online.length,
    active_alerts: activeAlerts,
    average_cpu_percent: online.length ? cpuTotal / online.length : 0,
    average_memory_percent: online.length ? memoryTotal / online.length : 0,
  };
}

export function metricSeverity(value: number, warning: number, critical: number): "normal" | "warning" | "critical" {
  if (value >= critical) return "critical";
  if (value >= warning) return "warning";
  return "normal";
}

export function osLabel(os: string): string {
  const normalized = os.toLowerCase();
  if (normalized.includes("windows")) return "Windows";
  if (normalized.includes("linux")) return "Linux";
  if (normalized.includes("darwin") || normalized.includes("mac")) return "macOS";
  return os || "未知";
}

export function formatOperatingSystem(os: string, version: string): string {
  const family = osLabel(os);
  const detail = version.trim();
  if (!detail || detail === "unknown") return family;
  return detail.toLowerCase().startsWith(family.toLowerCase()) ? detail : `${family} ${detail}`;
}
