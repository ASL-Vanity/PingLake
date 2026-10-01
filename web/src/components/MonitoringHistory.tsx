import { useCallback, useState } from "react";
import { RefreshCw } from "lucide-react";
import { CartesianGrid, Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { api } from "../api";
import type { MonitoringData, MonitoringHistoryPoint, MonitoringSection } from "../monitoringTypes";
import { formatDateTime } from "../utils";
import { useHistoryRange } from "./HistoryRange";
import { useHistoryQuery } from "../hooks/useHistoryQuery";
import { buildHistorySeries } from "./MonitoringHistory.helpers";

type Choice = { key: string; name: string; unit: string };
const choices: Partial<Record<MonitoringSection, Choice[]>> = {
  cpu: [{ key: "usage", name: "核心使用率", unit: "%" }, { key: "user", name: "用户态", unit: "%" }, { key: "system", name: "内核态", unit: "%" }, { key: "iowait", name: "CPU IO 等待", unit: "%" }, { key: "steal", name: "Steal", unit: "%" }],
  memory: [{ key: "available", name: "可用内存", unit: "MiB" }, { key: "cached", name: "缓存", unit: "MiB" }, { key: "swap_in", name: "Swap 换入", unit: "KiB/s" }, { key: "swap_out", name: "Swap 换出", unit: "KiB/s" }],
  disk: [{ key: "read", name: "读取", unit: "MiB/s" }, { key: "write", name: "写入", unit: "MiB/s" }, { key: "read_iops", name: "读取 IOPS", unit: "次/s" }, { key: "write_iops", name: "写入 IOPS", unit: "次/s" }, { key: "read_latency", name: "读取延迟", unit: "ms" }, { key: "write_latency", name: "写入延迟", unit: "ms" }, { key: "utilization", name: "磁盘忙碌", unit: "%" }, { key: "queue", name: "队列深度", unit: "" }, { key: "inode", name: "inode 使用率", unit: "%" }],
  network: [{ key: "rx_errors", name: "接收错误", unit: "包/s" }, { key: "tx_errors", name: "发送错误", unit: "包/s" }, { key: "rx_drops", name: "接收丢弃", unit: "包/s" }, { key: "tx_drops", name: "发送丢弃", unit: "包/s" }],
  tcp: [{ key: "total", name: "TCP 表项总数", unit: "" }, { key: "established", name: "ESTABLISHED", unit: "" }, { key: "time_wait", name: "TIME_WAIT", unit: "" }, { key: "listening", name: "监听端口", unit: "" }],
  agent: [{ key: "collection", name: "采集耗时", unit: "ms" }, { key: "send", name: "上报耗时", unit: "ms" }, { key: "success", name: "上报成功率", unit: "%" }, { key: "age", name: "样本年龄", unit: "ms" }, { key: "failures", name: "连续失败", unit: "" }],
  services: [{ key: "healthy", name: "服务健康", unit: "0 / 1" }],
  probes: [{ key: "latency", name: "探测延迟", unit: "ms" }],
};

function value(data: MonitoringData, section: MonitoringSection, key: string, device: string): number | null {
  if (section === "cpu") {
    if (key === "usage") return data.capabilities?.cpu_cores?.status === "ok" ? data.cpu_cores?.find((core) => core.id === device)?.usage_percent ?? null : null;
    return data.cpu_times?.[`${key}_percent` as keyof MonitoringData["cpu_times"]] ?? null;
  }
  if (section === "memory") {
    if (key === "available" || key === "cached") { const amount = data.memory?.[`${key}_bytes` as "available_bytes" | "cached_bytes"]; return amount == null ? null : amount / 1048576; }
    const rate = data.memory?.[`${key}_bytes_per_sec` as "swap_in_bytes_per_sec" | "swap_out_bytes_per_sec"]; return rate == null ? null : rate / 1024;
  }
  if (section === "disk") {
    if (key === "inode") return data.inodes?.find((inode) => inode.id === device)?.used_percent ?? null;
    const disk = data.disk_io?.find((item) => item.id === device);
    if (!disk || disk.status !== "ok") return null;
    if (key === "read" || key === "write") { const rate = disk[`${key}_bytes_per_sec`]; return rate == null ? null : rate / 1048576; }
    return ({ read_iops: disk.read_iops, write_iops: disk.write_iops, read_latency: disk.read_latency_ms, write_latency: disk.write_latency_ms, utilization: disk.utilization_percent, queue: disk.queue_depth })[key] ?? null;
  }
  if (section === "network") {
    const network = data.network_health?.find((item) => item.id === device);
    if (!network || network.status !== "ok") return null;
    return ({ rx_errors: network.receive_errors_per_sec, tx_errors: network.transmit_errors_per_sec, rx_drops: network.receive_drops_per_sec, tx_drops: network.transmit_drops_per_sec })[key] ?? null;
  }
  if (section === "tcp") {
    if (data.capabilities?.tcp?.status !== "ok") return null;
    const entries = Object.entries(data.tcp?.states ?? {});
    if (key === "total") return entries.reduce((sum, [, amount]) => sum + amount, 0);
    if (key === "listening") return data.tcp?.listening_ports ?? null;
    return entries.find(([name]) => name.toLowerCase() === key)?.[1] ?? 0;
  }
  if (section === "agent") return ({ collection: data.agent?.collection_duration_ms, send: data.agent?.send_duration_ms, success: data.agent?.success_rate_percent, age: data.agent?.sample_age_ms, failures: data.agent?.consecutive_failures })[key] ?? null;
  if (section === "services") { const service = data.services?.find((item) => item.id === device); return service?.status === "ok" && service.healthy != null ? Number(service.healthy) : null; }
  if (section === "probes") { const probe = [...(data.probes ?? [])].reverse().find((item) => item.target_id === device); return probe?.status === "success" ? probe.latency_ms : null; }
  return null;
}

export function MonitoringHistory({ nodeId, section, devices, onUnauthorized }: { nodeId: string; section: MonitoringSection; devices: { id: string; name: string; intervalSecs?: number }[]; onUnauthorized: () => void }) {
  const range = useHistoryRange();
  const minutes = range?.minutes ?? 60;
  const [device, setDevice] = useState(devices[0]?.id ?? "");
  const [metric, setMetric] = useState(choices[section]?.[0]?.key ?? "");
  const selectedDevice = devices.some((item) => item.id === device) ? device : devices[0]?.id ?? "";
  const options = choices[section] ?? [];
  const selectedMetric = options.find((item) => item.key === metric) ?? options[0];
  const fetcher = useCallback((signal: AbortSignal) => api.monitoringHistory(nodeId, minutes, section, selectedDevice || undefined, signal), [nodeId, minutes, section, selectedDevice]);
  const history = useHistoryQuery<MonitoringHistoryPoint[]>(`detail:${nodeId}:${minutes}:${section}:${selectedDevice}`, fetcher, onUnauthorized, `detail:${nodeId}:${section}:${selectedDevice}`, range?.refreshKey);
  const { loading, error, refresh } = history;
  const points = history.data ?? [];
  const data = buildHistorySeries(points, { events: section === "services" || section === "probes", minutes,
    intervalSecs: devices.find((device) => device.id === selectedDevice)?.intervalSecs },
    (point) => value(point.monitoring, section, selectedMetric?.key ?? "", selectedDevice));
  return <section className="monitoring-history" aria-label="设备监测历史">
    <div className="monitoring-history-controls">
      <strong>历史趋势</strong>
      {devices.length > 0 && <label>对象<select value={selectedDevice} onChange={(event) => setDevice(event.target.value)}>{devices.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}</select></label>}
      <label>指标<select value={selectedMetric?.key ?? ""} onChange={(event) => setMetric(event.target.value)}>{options.map((item) => <option value={item.key} key={item.key}>{item.name}</option>)}</select></label>
      <button type="button" className="icon-button" title="刷新监测历史" aria-label="刷新监测历史" onClick={refresh}><RefreshCw size={16} className={loading ? "spin" : ""} /></button>
    </div>
    {error && <p className="form-error" role="alert">{error}</p>}
    {history.retained && <p className="monitoring-history-count" role="status">正在更新所选范围，显示上一范围数据</p>}
    {loading && !history.data ? <div className="chart-empty" role="status">载入中</div> : !data.some((point) => point.value != null) ? <p className="chart-empty">所选范围内无可用数据</p> : <div className={`chart-container ${loading ? "updating" : ""}`} aria-busy={loading}><ResponsiveContainer width="100%" height="100%"><LineChart data={data} margin={{ top: 10, left: 8, right: 18, bottom: 4 }}><CartesianGrid stroke="var(--chart-grid)" vertical={false} strokeDasharray="3 3" /><XAxis dataKey="time" minTickGap={45} tickFormatter={(timestamp: string) => new Date(timestamp).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })} tick={{ fill: "var(--chart-text)", fontSize: 10 }} /><YAxis width={55} tick={{ fill: "var(--chart-text)", fontSize: 10 }} /><Tooltip labelFormatter={(timestamp) => formatDateTime(String(timestamp))} formatter={(amount) => [`${Number(amount).toFixed(2)} ${selectedMetric?.unit}`, selectedMetric?.name]} contentStyle={{ background: "var(--surface)", border: "1px solid var(--border)", color: "var(--text)", borderRadius: 6, fontSize: 11 }} /><Line dataKey="value" stroke="var(--blue)" strokeWidth={2} dot={false} connectNulls={false} isAnimationActive={false} /></LineChart></ResponsiveContainer></div>}
    {!loading && !error && <p className="monitoring-history-count">{points.length} 个历史点 · {selectedMetric?.unit || "计数"}</p>}
  </section>;
}
