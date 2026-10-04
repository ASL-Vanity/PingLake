import { useCallback, useMemo, useState, type ReactNode } from "react";
import { AlertTriangle, BellRing, CheckCircle2, ChevronRight, CircleHelp, Clock3, Server, ServerOff } from "lucide-react";
import { Area, AreaChart, CartesianGrid, ResponsiveContainer, Tooltip, XAxis, YAxis } from "recharts";
import { api } from "../api";
import { useHistoryQuery } from "../hooks/useHistoryQuery";
import type { AlertRecord, DashboardSummary, HistoryPoint, NodeSnapshot } from "../types";
import { formatDateTime, historyMemoryPercent } from "../utils";
import styles from "./OverviewDashboard.module.css";

type Range = 60 | 360 | 1440 | 10080;
type Metric = "cpu" | "memory" | "disk" | "network";

type OverviewTab = "overview" | "resources" | "network" | "services" | "probes" | "quality" | "config";
interface Props {
  nodes: NodeSnapshot[];
  alerts: AlertRecord[];
  summary: DashboardSummary;
  onOpenNode: (id: string, tab?: OverviewTab) => void;
  onViewAlerts: () => void;
  onViewHosts: () => void;
  onUnauthorized: () => void;
}

const ranges: Array<[Range, string]> = [[60, "1 小时"], [360, "6 小时"], [1440, "24 小时"], [10080, "7 天"]];

function freshness(node: NodeSnapshot): { stale: boolean; label: string; ageSeconds: number | null } {
  const received = node.last_seen_at ? Date.parse(node.last_seen_at) : NaN;
  const collected = node.latest?.collected_at ? Date.parse(node.latest.collected_at) : NaN;
  const sampleAge = node.latest?.monitoring?.agent?.sample_age_ms;
  const ageSeconds = Number.isFinite(received) ? Math.max(0, (Date.now() - received) / 1000) : null;
  const reportInterval = node.latest?.monitoring?.report_interval_secs ?? 5;
  const stale = !node.online || !Number.isFinite(received) || ageSeconds! > Math.max(reportInterval * 3, 45) ||
    (Number.isFinite(collected) && Date.now() - collected > Math.max(reportInterval * 3, 45) * 1000);
  if (!node.online) return { stale: true, label: "离线", ageSeconds };
  if (!Number.isFinite(received)) return { stale: true, label: "无接收时间", ageSeconds: null };
  if (stale) return { stale: true, label: `${Math.round(ageSeconds ?? 0)} 秒未更新`, ageSeconds };
  if (sampleAge != null && sampleAge > reportInterval * 3000) return { stale: true, label: "采样延迟", ageSeconds };
  return { stale: false, label: ageSeconds != null && ageSeconds < 10 ? "刚刚更新" : `${Math.round(ageSeconds ?? 0)} 秒前`, ageSeconds };
}

function chartValue(point: HistoryPoint, metric: Metric): number | null {
  if (metric === "cpu") return Number.isFinite(point.cpu_percent) ? point.cpu_percent : null;
  if (metric === "memory") return historyMemoryPercent(point);
  if (metric === "disk") return point.disk_total_bytes > 0 ? point.disk_used_bytes * 100 / point.disk_total_bytes : null;
  const rx = point.network_received_bytes_per_sec;
  const tx = point.network_transmitted_bytes_per_sec;
  return Number.isFinite(rx + tx) ? (rx + tx) / 1024 / 1024 : null;
}

function metricLabel(metric: Metric): string {
  return metric === "cpu" ? "CPU 使用率" : metric === "memory" ? "内存使用率" : metric === "disk" ? "磁盘使用率" : "网络总吞吐";
}

function metricUnit(metric: Metric): string {
  return metric === "network" ? "MiB/s" : "%";
}

export function OverviewDashboard({ nodes, alerts, summary, onOpenNode, onViewAlerts, onViewHosts, onUnauthorized }: Props) {
  const [selectedId, setSelectedId] = useState(nodes.find((node) => node.online)?.id ?? nodes[0]?.id ?? "");
  const [range, setRange] = useState<Range>(60);
  const [metric, setMetric] = useState<Metric>("cpu");
  const selected = nodes.find((node) => node.id === selectedId) ?? nodes.find((node) => node.online) ?? nodes[0] ?? null;
  const fetcher = useCallback((signal: AbortSignal) => selected ? api.history(selected.id, range, signal) : Promise.resolve([]), [range, selected?.id]);
  const history = useHistoryQuery<HistoryPoint[]>(selected ? `overview:${selected.id}:${range}` : "overview:empty", fetcher, onUnauthorized, selected?.id ?? "overview", 0);
  const series = useMemo(() => (history.data ?? []).map((point) => ({ ...point, value: chartValue(point, metric) })), [history.data, metric]);
  const staleCount = nodes.filter((node) => freshness(node).stale).length;
  const activeAlerts = alerts.filter((alert) => alert.active);
  const recentAlerts = [...alerts].sort((a, b) => Date.parse(b.opened_at) - Date.parse(a.opened_at)).slice(0, 5);
  const onlineRate = summary.total_nodes ? Math.round(summary.online_nodes * 100 / summary.total_nodes) : 0;

  if (!nodes.length) return <section className={styles.emptyState}><Server size={23} /><h2>还没有受监主机</h2><p>完成 Agent 注册后，实时资源和异常状态会显示在这里。</p><button type="button" className="primary-button" onClick={onViewHosts}>打开主机管理<ChevronRight size={16} /></button></section>;

  return <div className={styles.dashboard}>
    <section className={styles.statBand} aria-label="概览摘要">
      <Stat icon={<Server size={17} />} label="受监主机" value={summary.total_nodes} detail={`${summary.online_nodes} 台在线`} />
      <Stat icon={<CheckCircle2 size={17} />} label="在线率" value={`${onlineRate}%`} detail={summary.offline_nodes ? `${summary.offline_nodes} 台离线` : "全部在线"} tone={summary.offline_nodes ? "warning" : "good"} />
      <Stat icon={<BellRing size={17} />} label="活动告警" value={summary.active_alerts} detail={summary.active_alerts ? "需要处理" : "无活动告警"} tone={summary.active_alerts ? "critical" : "good"} />
      <Stat icon={<Clock3 size={17} />} label="采集状态" value={staleCount ? `${staleCount} 台` : "正常"} detail={staleCount ? "数据过期或离线" : "数据新鲜"} tone={staleCount ? "warning" : "good"} />
    </section>

    <div className={styles.mainGrid}>
      <section className={styles.trendSection} aria-label="资源趋势">
        <div className={styles.sectionHeader}><div><p className={styles.eyebrow}>实时资源</p><h2>{selected ? selected.display_name || selected.hostname : "资源趋势"}</h2></div><button type="button" className={styles.linkButton} onClick={onViewHosts}>查看全部主机<ChevronRight size={15} /></button></div>
        <div className={styles.controls}>
          <label>主机<select value={selected?.id ?? ""} onChange={(event) => setSelectedId(event.target.value)}>{nodes.map((node) => <option key={node.id} value={node.id}>{node.display_name || node.hostname}</option>)}</select></label>
          <div className={styles.segmented} role="group" aria-label="历史范围">{ranges.map(([value, label]) => <button type="button" key={value} className={range === value ? styles.active : ""} onClick={() => setRange(value)}>{label}</button>)}</div>
          <label>指标<select value={metric} onChange={(event) => setMetric(event.target.value as Metric)}><option value="cpu">CPU</option><option value="memory">内存</option><option value="disk">磁盘</option><option value="network">网络</option></select></label>
        </div>
        <div className={styles.chartFrame} aria-busy={history.loading}>
          {!selected || (history.loading && !history.data) ? <div className={styles.chartEmpty}><CircleHelp size={19} /><span>载入历史数据</span></div> : !series.some((point) => point.value != null) ? <div className={styles.chartEmpty}><CircleHelp size={19} /><span>{history.error || "所选范围内没有可用历史数据"}</span></div> : <ResponsiveContainer width="100%" height="100%"><AreaChart data={series} margin={{ top: 12, right: 14, left: -16, bottom: 2 }}><defs><linearGradient id="overviewTrendFill" x1="0" y1="0" x2="0" y2="1"><stop offset="0%" stopColor="var(--blue)" stopOpacity={0.2} /><stop offset="100%" stopColor="var(--blue)" stopOpacity={0} /></linearGradient></defs><CartesianGrid stroke="var(--border)" vertical={false} strokeDasharray="3 3" /><XAxis dataKey="collected_at" tickFormatter={(value: string) => new Date(value).toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit" })} minTickGap={50} tick={{ fill: "var(--text-muted)", fontSize: 10 }} /><YAxis domain={metric === "network" ? [0, "auto"] : [0, 100]} tick={{ fill: "var(--text-muted)", fontSize: 10 }} width={42} /><Tooltip labelFormatter={(value) => formatDateTime(String(value))} formatter={(value) => [`${Number(value).toFixed(1)} ${metricUnit(metric)}`, metricLabel(metric)]} contentStyle={{ background: "var(--surface)", border: "1px solid var(--border)", borderRadius: 6, color: "var(--text)", fontSize: 12 }} /><Area type="monotone" dataKey="value" stroke="var(--blue)" fill="url(#overviewTrendFill)" strokeWidth={2} dot={false} connectNulls={false} isAnimationActive={false} /></AreaChart></ResponsiveContainer>}
        </div>
        <div className={styles.trendFooter}><span>{history.retained ? "正在更新范围，暂显示上一次结果" : `${series.length} 个历史点`}</span><span>{selected ? freshness(selected).label : "--"}</span></div>
      </section>

      <section className={styles.fleetSection} aria-label="主机状态">
        <div className={styles.sectionHeader}><div><p className={styles.eyebrow}>主机状态</p><h2>Fleet 状态</h2></div><button type="button" className={styles.linkButton} onClick={onViewHosts}>主机列表<ChevronRight size={15} /></button></div>
        <div className={styles.fleetMatrix}>{nodes.map((node) => { const state = freshness(node); const cpu = node.latest?.cpu_percent ?? 0; return <button type="button" className={styles.fleetRow} key={node.id} onClick={() => onOpenNode(node.id)}><span className={`${styles.statusDot} ${node.online && !state.stale ? styles.ok : node.online ? styles.warn : styles.bad}`} /><span className={styles.fleetName}><strong>{node.display_name || node.hostname}</strong><small>{node.group_name || node.hostname}</small></span><span className={styles.fleetBar}><i style={{ width: `${Math.min(100, Math.max(3, cpu))}%` }} /></span><span className={styles.fleetValue}>{node.latest ? `${Math.round(cpu)}%` : "--"}</span><ChevronRight size={14} /></button>; })}</div>
      </section>
    </div>

    <div className={styles.lowerGrid}>
      <section className={styles.alertSection} aria-label="最近告警"><div className={styles.sectionHeader}><div><p className={styles.eyebrow}>异常活动</p><h2>最近告警</h2></div><button type="button" className={styles.linkButton} onClick={onViewAlerts}>全部告警<ChevronRight size={15} /></button></div>{recentAlerts.length ? <div className={styles.alertList}>{recentAlerts.map((alert) => <button type="button" className={styles.alertRow} key={alert.id} onClick={() => onOpenNode(alert.node_id)}><span className={`${styles.alertIcon} ${alert.active ? styles.alertActive : styles.alertResolved}`}>{alert.active ? <AlertTriangle size={15} /> : <CheckCircle2 size={15} />}</span><span><strong>{alert.message}</strong><small>{alert.node_name} · {formatDateTime(alert.opened_at)}</small></span><span className={styles.alertState}>{alert.active ? "活动" : "已恢复"}</span></button>)}</div> : <div className={styles.clearState}><CheckCircle2 size={19} /><span>没有告警活动，当前状态清洁。</span></div>}</section>
      <section className={styles.exceptionSection} aria-label="异常摘要"><div className={styles.sectionHeader}><div><p className={styles.eyebrow}>需要关注</p><h2>异常摘要</h2></div></div><Exception icon={<ServerOff size={17} />} label="离线主机" value={summary.offline_nodes} onClick={onViewHosts} /><Exception icon={<Clock3 size={17} />} label="采集过期" value={staleCount} onClick={onViewHosts} /><Exception icon={<AlertTriangle size={17} />} label="活动告警" value={activeAlerts.length} onClick={onViewAlerts} /></section>
    </div>
  </div>;
}

function Stat({ icon, label, value, detail, tone = "neutral" }: { icon: ReactNode; label: string; value: ReactNode; detail: string; tone?: string }) {
  return <div className={`${styles.stat} ${styles[tone]}`}><span className={styles.statIcon}>{icon}</span><span className={styles.statCopy}><small>{label}</small><strong>{value}</strong><em>{detail}</em></span></div>;
}

function Exception({ icon, label, value, onClick }: { icon: ReactNode; label: string; value: number; onClick: () => void }) {
  return <button type="button" className={styles.exception} onClick={onClick}><span>{icon}</span><span><small>{label}</small><strong>{value}</strong></span><ChevronRight size={15} /></button>;
}
