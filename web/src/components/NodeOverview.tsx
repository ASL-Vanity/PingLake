import { useCallback } from "react";
import { RefreshCw, Thermometer } from "lucide-react";
import { CartesianGrid, Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { api } from "../api";
import type { NodeSnapshot } from "../types";
import { formatBytes, formatDateTime, formatDuration, formatPercent, formatTemperature, historyDiskPercent, historyMemoryPercent, ratioPercent } from "../utils";
import { useHistoryQuery } from "../hooks/useHistoryQuery";
import { useHistoryRange } from "./HistoryRange";

export function NodeOverview({ node, onUnauthorized }: { node: NodeSnapshot; onUnauthorized: () => void }) {
  const range = useHistoryRange();
  const minutes = range?.minutes ?? 60;
  const fetcher = useCallback((signal: AbortSignal) => api.history(node.id, minutes, signal), [node.id, minutes]);
  const history = useHistoryQuery(`overview:${node.id}:${minutes}`, fetcher, onUnauthorized, `overview:${node.id}`, range?.refreshKey);
  const latest = node.latest;
  const chart = history.data?.map((point) => ({ time: point.collected_at, cpu: point.cpu_percent, memory: historyMemoryPercent(point), disk: historyDiskPercent(point), rx: point.network_received_bytes_per_sec / 1048576, tx: point.network_transmitted_bytes_per_sec / 1048576 })) ?? [];
  return <div className="node-overview">
    <div className="node-section-title"><h3>主机趋势</h3><span>{history.data?.length ?? 0} 个采样点</span><button type="button" className="icon-button" title="刷新主机趋势" aria-label="刷新主机趋势" onClick={history.refresh}><RefreshCw size={15} className={history.loading ? "spin" : ""} /></button></div>
    {history.error && <p className="form-error" role="alert">{history.error}</p>}
    {history.retained && <p className="monitoring-history-count" role="status">正在更新所选范围，显示上一范围数据</p>}
    {!history.data && history.loading ? <div className="chart-empty" role="status">载入趋势中</div> : !chart.length ? <div className="chart-empty">所选时间范围暂无数据</div> : <div className={`node-overview-charts ${history.loading ? "updating" : ""}`} aria-busy={history.loading}>
      <OverviewChart data={chart} title="资源使用率" unit="%" series={[["cpu", "CPU", "var(--chart-cpu)"], ["memory", "内存", "var(--chart-memory)"], ["disk", "磁盘", "var(--chart-disk)"]]} />
      <OverviewChart data={chart} title="网络吞吐" unit="MiB/s" series={[["rx", "下行", "var(--chart-received)"], ["tx", "上行", "var(--chart-transmitted)"]]} />
    </div>}
    <div className="node-section-title"><h3>系统信息</h3></div>
    <dl className="monitoring-readings"><div><dt>操作系统</dt><dd>{node.os} {node.os_version}</dd></div><div><dt>内核</dt><dd>{node.kernel_version || "--"}</dd></div><div><dt>运行时间</dt><dd>{formatDuration(latest?.uptime_seconds)}</dd></div><div><dt>温度</dt><dd><Thermometer size={12} /> {formatTemperature(latest?.temperature_celsius)}</dd></div><div><dt>进程数</dt><dd>{latest?.process_count ?? "--"}</dd></div><div><dt>注册时间</dt><dd>{formatDateTime(node.enrolled_at)}</dd></div></dl>
    <div className="node-section-title"><h3>磁盘容量</h3><span>{latest?.disks.length ?? 0} 个卷</span></div>
    <div className="monitoring-table-scroll"><table className="monitoring-table"><thead><tr><th>卷 / 挂载点</th><th>文件系统</th><th>已用</th><th>总量</th><th>使用率</th></tr></thead><tbody>{latest?.disks.map((disk, index) => <tr key={`${disk.mount_point}:${index}`}><th scope="row">{disk.mount_point}<small className="node-cell-secondary">{disk.name}</small></th><td>{disk.file_system || "--"}</td><td>{formatBytes(disk.used_bytes)}</td><td>{formatBytes(disk.total_bytes)}</td><td>{formatPercent(ratioPercent(disk.used_bytes, disk.total_bytes), 1)}</td></tr>)}</tbody></table>{!latest?.disks.length && <p className="monitoring-empty">暂无磁盘数据</p>}</div>
    <div className="node-section-title"><h3>进程占用</h3><span>{latest?.processes.length ?? 0} / {latest?.process_count ?? 0}</span></div>
    <div className="monitoring-table-scroll"><table className="monitoring-table"><thead><tr><th>进程</th><th>PID</th><th>CPU</th><th>内存</th></tr></thead><tbody>{latest?.processes.map((process) => <tr key={`${process.pid}:${process.name}`}><th scope="row">{process.name}</th><td>{process.pid}</td><td>{formatPercent(process.cpu_percent, 1)}</td><td>{formatBytes(process.memory_bytes)}</td></tr>)}</tbody></table>{!latest?.processes.length && <p className="monitoring-empty">暂无进程明细</p>}</div>
  </div>;
}

function OverviewChart({ data, title, unit, series }: { data: Record<string, number | string | null>[]; title: string; unit: string; series: [string, string, string][] }) {
  return <div className="node-trend"><div className="chart-heading"><strong>{title}</strong><span>{unit}</span></div><div className="node-chart-legend">{series.map(([key, name, color]) => <span key={key}><i style={{ backgroundColor: color }} />{name}</span>)}</div><div className="chart-container"><ResponsiveContainer width="100%" height="100%"><LineChart data={data} margin={{ top: 8, right: 12, bottom: 2, left: 0 }}><CartesianGrid stroke="var(--chart-grid)" vertical={false} strokeDasharray="3 3" /><XAxis dataKey="time" tickFormatter={(value) => new Date(String(value)).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })} minTickGap={40} tick={{ fill: "var(--chart-text)", fontSize: 10 }} /><YAxis width={40} domain={unit === "%" ? [0, 100] : [0, "auto"]} tick={{ fill: "var(--chart-text)", fontSize: 10 }} /><Tooltip labelFormatter={(value) => formatDateTime(String(value))} formatter={(value) => `${Number(value).toFixed(1)} ${unit}`} contentStyle={{ background: "var(--chart-tooltip)", border: "1px solid var(--chart-tooltip-border)", borderRadius: 4, color: "var(--text)" }} />{series.map(([key, name, color]) => <Line key={key} dataKey={key} name={name} stroke={color} strokeWidth={1.8} dot={false} connectNulls={false} isAnimationActive={false} />)}</LineChart></ResponsiveContainer></div></div>;
}
