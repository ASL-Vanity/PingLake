import { useCallback, useEffect, useState, type ReactNode } from "react";
import { RefreshCw, Settings } from "lucide-react";
import { api, ApiError } from "../api";
import type { NodeSnapshot } from "../types";
import type { MetricStatus, MonitoringData, MonitoringSection, NodeMonitoringConfig, ProbeStatistics, ProbeStatus } from "../monitoringTypes";
import { formatBytes, formatDateTime, formatLatency, formatPercent, formatRate, formatRelativeTime } from "../utils";
import { browserLatencyLabel, type BrowserLatency } from "../hooks/useBrowserLatency";
import { MonitoringConfig } from "./MonitoringConfig";
import { MonitoringHistory } from "./MonitoringHistory";
import { useHistoryRange } from "./HistoryRange";
import { useHistoryQuery } from "../hooks/useHistoryQuery";

const statusNames: Record<MetricStatus | ProbeStatus, string> = {
  ok: "正常", warming_up: "初次采样", unsupported: "不支持", permission_denied: "权限不足", unavailable: "采集失败", stale: "已过期", unknown: "未知",
  success: "成功", failure: "失败", timeout: "超时", policy_denied: "策略禁止",
};
const capabilityNames: Record<string, string> = { cpu: "CPU", cpu_cores: "每核心 CPU", cpu_times: "CPU 状态", memory: "内存", disk_io: "磁盘 IO", inodes: "inode", network: "网卡", network_health: "网卡健康", tcp: "TCP", services: "服务", probes: "主动探测", agent: "Agent" };
function useClock() {
  const [now, setNow] = useState(Date.now());
  useEffect(() => { const timer = window.setInterval(() => setNow(Date.now()), 5000); return () => window.clearInterval(timer); }, []);
  return now;
}
function checkStale(node: NodeSnapshot, checkedAt: string | undefined, cadence: number, now: number) {
  const data = node.latest?.monitoring;
  const received = Date.parse(node.last_seen_at ?? "");
  const checked = Date.parse(checkedAt ?? "");
  return !node.online || !Number.isFinite(received) || !Number.isFinite(checked)
    || now - received + (data?.agent.sample_age_ms ?? 0) > Math.max(30, (data?.report_interval_secs ?? 5) * 3) * 1000
    || now - checked > Math.max(30, cadence * 2) * 1000;
}
function number(value: number | null | undefined, unit = "", decimals = 1) { return value == null || !Number.isFinite(value) ? "--" : `${value.toFixed(decimals)}${unit}`; }
function counter(value: string | null | undefined) {
  if (value == null) return "--";
  try { return BigInt(value).toLocaleString("zh-CN"); } catch { return "--"; }
}
function stateName(state: string) { return ({ running: "运行", stopped: "停止", unknown: "未知", inactive: "停止", failed: "失败", activating: "启动中", deactivating: "停止中" } as Record<string, string>)[state] ?? state; }
function Badge({ status, error }: { status: MetricStatus | ProbeStatus; error?: string | null }) { return <span className={`monitoring-status ${status}`} title={error ?? undefined}>{statusNames[status] ?? status}</span>; }
function Quantile({ value, count, minimum }: { value: number | null; count: number | null | undefined; minimum: number }) {
  const insufficient = count != null && count > 0 && count < minimum;
  return <>{formatLatency(value)}{count != null && <small className={`monitoring-quantile-count ${insufficient ? "small-sample" : ""}`} title={insufficient ? `${count} 个成功延迟样本，少于 ${minimum} 个样本；百分位对单次观测敏感。` : `${count} 个成功延迟样本`}>n={count}{insufficient ? " · 样本不足" : ""}</small>}</>;
}
function Readings({ items }: { items: [string, ReactNode][] }) { return <dl className="monitoring-readings">{items.map(([name, amount]) => <div key={name}><dt>{name}</dt><dd>{amount}</dd></div>)}</dl>; }
function Table({ headings, children }: { headings: string[]; children: ReactNode }) { return <div className="monitoring-table-scroll"><table className="monitoring-table"><thead><tr>{headings.map((heading) => <th key={heading}>{heading}</th>)}</tr></thead><tbody>{children}</tbody></table></div>; }
function Capabilities({ data, names }: { data: MonitoringData; names: string[] }) {
  const entries = Object.entries(data.capabilities ?? {}).filter(([name]) => names.includes(name));
  return entries.length ? <div className="monitoring-capabilities">{entries.map(([name, capability]) => <span key={name}>{capabilityNames[name] ?? name} <Badge status={capability.status} error={capability.error} />{capability.error && <small>{capability.error}</small>}</span>)}</div> : null;
}

export type DetailTab = "overview" | "resources" | "network" | "services" | "probes" | "quality" | "config";
const tabs: [DetailTab, string][] = [["overview", "概览"], ["resources", "资源"], ["network", "网络"], ["services", "服务"], ["probes", "探测"], ["quality", "采集质量"]];

export function MonitoringPanel({ node, tab, onTabChange: setTab, overview, onUnauthorized, onConfigSaved, browserLatency, toolbar }: { node: NodeSnapshot; tab: DetailTab; onTabChange: (tab: DetailTab) => void; overview: ReactNode; onUnauthorized: () => void; onConfigSaved: () => void; browserLatency?: BrowserLatency; toolbar?: ReactNode }) {
  const [config, setConfig] = useState<NodeMonitoringConfig | null>(null);
  const [configError, setConfigError] = useState<string | null>(null);
  const [configRevision, setConfigRevision] = useState(0);
  const data = node.latest?.monitoring;
  const refreshKey = useHistoryRange()?.refreshKey ?? 0;
  useEffect(() => {
    const controller = new AbortController();
    setConfigError(null);
    void api.monitoringConfig(node.id, controller.signal).then((value) => { if (!controller.signal.aborted) setConfig(value); }).catch((reason) => {
      if (controller.signal.aborted) return;
      if (reason instanceof ApiError && reason.status === 401) onUnauthorized();
      else setConfigError(reason instanceof Error ? reason.message : "无法载入配置");
    });
    return () => controller.abort();
  }, [node.id, configRevision, refreshKey, data?.agent.applied_config_revision, onUnauthorized]);
  const applied = data?.agent.applied_config_revision;
  const pending = config != null && applied !== config.revision;
  return <section className="monitoring-panel" aria-label="详细监测">
    {toolbar && <div className="monitoring-toolbar-shared monitoring-configshared-range"><span>历史范围</span>{toolbar}</div>}
    <div className="monitoring-tabs" role="tablist" aria-label="监测分类">{tabs.map(([id, name]) => <button key={id} role="tab" id={`monitoring-tab-${id}`} aria-controls="node-monitoring-content" aria-selected={tab === id} tabIndex={tab === id || (tab === "config" && id === "overview") ? 0 : -1} type="button" className={tab === id ? "active" : ""} onClick={() => setTab(id)} onKeyDown={(event) => {
      if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
      event.preventDefault(); const index = tabs.findIndex(([key]) => key === tab);
      const next = event.key === "Home" ? 0 : event.key === "End" ? tabs.length - 1 : (index + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
      const id = tabs[next]![0]; setTab(id); document.getElementById(`monitoring-tab-${id}`)?.focus();
    }}>{name}</button>)}</div>
    <div className="monitoring-content" role={tab === "config" ? "region" : "tabpanel"} id="node-monitoring-content" aria-label={tab === "config" ? "监测配置" : undefined} aria-labelledby={tab === "config" ? undefined : `monitoring-tab-${tab}`}>
      {tab !== "config" && !node.online && <p className="monitoring-warning" role="status">节点离线，以下为最后一次上报数据。</p>}
      {tab !== "config" && tab !== "overview" && !data && <p className="monitoring-empty">此 Agent 尚未上报扩展监测数据 · 版本 {node.agent_version}</p>}
      {tab === "overview" && overview}
      {tab === "resources" && data && <Resources nodeId={node.id} data={data} node={node} onUnauthorized={onUnauthorized} />}
      {tab === "network" && data && <Network node={node} data={data} onUnauthorized={onUnauthorized} />}
      {tab === "quality" && data && <Quality node={node} data={data} config={config} browserLatency={browserLatency} onUnauthorized={onUnauthorized} />}
      {(tab === "services" || tab === "probes") && <><div className="monitoring-section-toolbar"><span>{config ? `配置版本 ${config.revision} · ${data?.agent.config_error ? "应用失败" : pending ? "等待 Agent 应用" : "已应用"}` : "配置未载入"}</span><button type="button" className="secondary-button" onClick={() => setTab("config")}><Settings size={14} />配置</button></div>{data?.agent.config_error && <p className="form-error">{data.agent.config_error}</p>}{tab === "services" && data && <Services node={node} data={data} config={config} onUnauthorized={onUnauthorized} />}{tab === "probes" && <Probes node={node} data={data} config={config} onUnauthorized={onUnauthorized} />}</>}
      {tab === "config" && <>{configError && <div className="monitoring-reload"><p className="form-error" role="alert">{configError}</p><button type="button" className="secondary-button" onClick={() => setConfigRevision((current) => current + 1)}><RefreshCw size={14} />重试</button></div>}{!config && !configError && <p className="monitoring-empty">载入配置中</p>}{config && <><div className="monitoring-section-toolbar"><span>配置版本 {config.revision} · Agent 已应用 {applied ?? "--"}</span><Badge status={data?.agent.config_error ? "unavailable" : pending ? "warming_up" : "ok"} /></div><MonitoringConfig nodeId={node.id} config={config} onUnauthorized={onUnauthorized} onSaved={(value) => { setConfig(value); onConfigSaved(); }} /></>}</>}
    </div>
  </section>;
}

function Resources({ nodeId, data, node, onUnauthorized }: { nodeId: string; data: MonitoringData; node: NodeSnapshot; onUnauthorized: () => void }) {
  const [section, setSection] = useState<MonitoringSection>("cpu");
  const cores = data.cpu_cores ?? [];
  const disks = data.disk_io ?? [];
  const inodes = data.inodes ?? [];
  const devices = section === "cpu" ? cores.map((item) => ({ id: item.id, name: item.id })) : section === "disk" ? [...disks.map((item) => ({ id: item.id, name: item.name })), ...inodes.map((item) => ({ id: item.id, name: `${item.mount_point} · inode` }))] : [];
  return <>
    <Capabilities data={data} names={["cpu", "cpu_cores", "cpu_times", "memory", "swap_io", "disk_io", "inodes"]} />
    <h3>CPU</h3>
    <Readings items={[["用户态", number(data.cpu_times?.user_percent, "%")], ["内核态", number(data.cpu_times?.system_percent, "%")], ["CPU IO 等待", number(data.cpu_times?.iowait_percent, "%")], ["Steal", number(data.cpu_times?.steal_percent, "%")]]} />
    {cores.length ? <div className="monitoring-core-grid">{cores.map((core) => <div key={core.id}><span>{core.id}</span><strong>{data.capabilities.cpu_cores?.status === "ok" ? number(core.usage_percent, "%") : "--"}</strong><small>{core.frequency_mhz == null ? "--" : `${core.frequency_mhz} MHz`}</small>{data.capabilities.cpu_cores?.status === "ok" && <progress max={100} value={core.usage_percent} aria-label={`${core.id} 使用率`} />}</div>)}</div> : <p className="monitoring-empty">暂无核心明细</p>}
    <h3>内存 / Swap</h3>
    <Readings items={[["可用内存", formatBytes(data.memory?.available_bytes)], ["缓存", formatBytes(data.memory?.cached_bytes)], ["Buffer", formatBytes(data.memory?.buffers_bytes)], ["Swap 使用", `${formatBytes(node.latest?.swap_used_bytes)} / ${formatBytes(node.latest?.swap_total_bytes)}`], ["Swap 换入", formatRate(data.memory?.swap_in_bytes_per_sec)], ["Swap 换出", formatRate(data.memory?.swap_out_bytes_per_sec)]]} />
    <h3>磁盘 IO</h3>
    {disks.length ? <Table headings={["设备", "状态", "读取", "写入", "读 / 写 IOPS", "读 / 写延迟", "忙碌", "队列"]}>{disks.map((disk) => <tr key={disk.id}><th scope="row" title={disk.id}>{disk.name}</th><td><Badge status={disk.status} /></td><td>{formatRate(disk.read_bytes_per_sec)}</td><td>{formatRate(disk.write_bytes_per_sec)}</td><td>{number(disk.read_iops)} / {number(disk.write_iops)}</td><td>{number(disk.read_latency_ms, " ms")} / {number(disk.write_latency_ms, " ms")}</td><td>{number(disk.utilization_percent, "%")}</td><td>{number(disk.queue_depth)}</td></tr>)}</Table> : <p className="monitoring-empty">暂无磁盘 IO 明细</p>}
    <h3>inode</h3>
    {inodes.length ? <Table headings={["挂载点", "状态", "已用", "总量", "可用", "使用率"]}>{inodes.map((inode) => <tr key={inode.id}><th scope="row">{inode.mount_point}</th><td><Badge status={inode.status} /></td><td>{counter(inode.used)}</td><td>{counter(inode.total)}</td><td>{counter(inode.free)}</td><td>{number(inode.used_percent, "%")}</td></tr>)}</Table> : <p className="monitoring-empty">此平台未提供 inode 明细</p>}
    <div className="monitoring-section-selector" role="group" aria-label="资源历史分类">{([["cpu", "CPU"], ["memory", "内存"], ["disk", "磁盘 / inode"]] as const).map(([id, label]) => <button type="button" key={id} className={section === id ? "active" : ""} onClick={() => setSection(id)}>{label}</button>)}</div>
    <MonitoringHistory key={section} nodeId={nodeId} section={section} devices={devices} onUnauthorized={onUnauthorized} />
  </>;
}

function Network({ node, data, onUnauthorized }: { node: NodeSnapshot; data: MonitoringData; onUnauthorized: () => void }) {
  const [section, setSection] = useState<MonitoringSection>("network");
  const interfaces = data.network_health ?? [];
  return <>
    <Capabilities data={data} names={["network", "network_health", "tcp"]} />
    <h3>网卡吞吐</h3>
    {node.latest?.interfaces.length ? <Table headings={["接口", "下行", "上行"]}>{node.latest.interfaces.map((item) => <tr key={item.name}><th scope="row">{item.name}</th><td>{formatRate(item.received_bytes_per_sec)}</td><td>{formatRate(item.transmitted_bytes_per_sec)}</td></tr>)}</Table> : <p className="monitoring-empty">暂无网卡吞吐数据</p>}
    <h3>网卡健康</h3>
    {interfaces.length ? <Table headings={["接口", "状态", "连接", "累计收 / 发", "接收 / 发送错误", "接收 / 发送丢弃", "错误率", "丢弃率"]}>{interfaces.map((item) => <tr key={item.id}><th scope="row" title={item.id}>{item.name}</th><td><Badge status={item.status} /></td><td>{item.link_up == null ? "--" : item.link_up ? "已连接" : "已断开"}</td><td>{counter(item.received_bytes)} / {counter(item.transmitted_bytes)} B</td><td>{counter(item.receive_errors)} / {counter(item.transmit_errors)}</td><td>{counter(item.receive_drops)} / {counter(item.transmit_drops)}</td><td>{number(item.receive_errors_per_sec)} / {number(item.transmit_errors_per_sec)} 包/s</td><td>{number(item.receive_drops_per_sec)} / {number(item.transmit_drops_per_sec)} 包/s</td></tr>)}</Table> : <p className="monitoring-empty">暂无网卡健康明细</p>}
    <h3>TCP</h3>
    {data.capabilities?.tcp?.status === "ok" ? <><Readings items={[["TCP 表项总数", Object.values(data.tcp.states).reduce((sum, amount) => sum + amount, 0)], ["监听端口", data.tcp.listening_ports], ["监听 Socket", data.tcp.listening_sockets], ["范围", data.tcp.scope || "--"]]} /><Readings items={Object.entries(data.tcp.states).map(([state, amount]) => [state.toUpperCase(), amount])} /></> : <p className="monitoring-empty">TCP 数据 {statusNames[data.capabilities?.tcp?.status ?? "unsupported"]}</p>}
    <div className="monitoring-section-selector" role="group" aria-label="网络历史分类"><button type="button" className={section === "network" ? "active" : ""} onClick={() => setSection("network")}>网卡</button><button type="button" className={section === "tcp" ? "active" : ""} onClick={() => setSection("tcp")}>TCP</button></div>
    <MonitoringHistory key={section} nodeId={node.id} section={section} devices={section === "network" ? interfaces.map((item) => ({ id: item.id, name: item.name })) : []} onUnauthorized={onUnauthorized} />
  </>;
}

function Quality({ node, data, config, browserLatency, onUnauthorized }: { node: NodeSnapshot; data: MonitoringData; config: NodeMonitoringConfig | null; browserLatency?: BrowserLatency; onUnauthorized: () => void }) {
  const agent = data.agent;
  const [capabilityFilter, setCapabilityFilter] = useState<MetricStatus | "all">("all");
  const age = node.last_seen_at ? Math.max(0, Date.now() - Date.parse(node.last_seen_at)) : null;
  const metricAge = age != null && Number.isFinite(agent.sample_age_ms) ? age + Math.max(0, agent.sample_age_ms) : null;
  return <>
    <Readings items={[["最近采集", formatDateTime(node.latest?.collected_at)], ["Hub 最近接收", formatDateTime(node.last_seen_at)], ["上报年龄", age == null ? "--" : `${Math.round(age / 1000)} 秒`], ["指标年龄", metricAge == null ? "--" : `${Math.round(metricAge / 1000)} 秒`], ["上报间隔", `${data.report_interval_secs} 秒`], ["采集耗时", formatLatency(agent.collection_duration_ms)], ["上次发送耗时", formatLatency(agent.send_duration_ms)], ["上报成功率", formatPercent(agent.success_rate_percent, 2)], ["尝试 / 成功 / 失败", `${agent.upload_attempts} / ${agent.upload_successes} / ${agent.upload_failures}`], ["连续失败", agent.consecutive_failures], ["重试次数", agent.retries], ["缓冲报告", agent.queue_length], ["丢弃报告", agent.dropped_reports], ["发送时样本年龄", formatLatency(agent.sample_age_ms)], ["最近成功", formatDateTime(agent.last_success_at)], ["Agent → Hub", formatLatency(node.latest?.hub_latency_ms)], ["访问者 → 此主机", browserLatencyLabel(browserLatency)], ["样本序号", data.sample_sequence], ["配置版本 / 已应用", `${config?.revision ?? "--"} / ${agent.applied_config_revision ?? "--"}`]]} />
    {agent.last_error && <p className="form-error">上次上报错误：{agent.last_error}</p>}
    {agent.config_error && <p className="form-error">配置错误：{agent.config_error}</p>}
    <div className="monitoring-section-toolbar"><h3>采集能力</h3><label>状态<select aria-label="采集能力状态筛选" value={capabilityFilter} onChange={(event) => setCapabilityFilter(event.target.value as MetricStatus | "all")}><option value="all">全部状态</option><option value="ok">正常</option><option value="warming_up">初次采样</option><option value="unsupported">不支持</option><option value="permission_denied">权限不足</option><option value="unavailable">采集失败</option><option value="stale">已过期</option><option value="unknown">未知</option></select></label></div>
    <Table headings={["指标", "状态", "来源", "错误"]}>{Object.entries(data.capabilities ?? {}).filter(([, capability]) => capabilityFilter === "all" || capability.status === capabilityFilter).map(([name, capability]) => <tr key={name}><th scope="row">{capabilityNames[name] ?? name}</th><td><Badge status={capability.status ?? "unknown"} /></td><td>{capability.source || "--"}</td><td>{capability.error || "--"}</td></tr>)}</Table>
    <MonitoringHistory nodeId={node.id} section="agent" devices={[]} onUnauthorized={onUnauthorized} />
  </>;
}

function Services({ node, data, config, onUnauthorized }: { node: NodeSnapshot; data: MonitoringData; config: NodeMonitoringConfig | null; onUnauthorized: () => void }) {
  const nodeId = node.id;
  const now = useClock();
  const checks = config?.services ?? [];
  return <><Capabilities data={data} names={["services"]} />{checks.length ? <Table headings={["服务", "启用", "预期", "状态", "检查结果", "最近检查", "错误"]}>{checks.map((check) => {
    const result = data.services.find((item) => item.id === check.id && item.config_revision === config?.revision);
    return <tr key={check.id}><th scope="row">{check.name}</th><td>{check.enabled ? "是" : "已暂停"}</td><td>{stateName(check.expected_state)}</td><td>{result ? stateName(result.state) : "--"}</td><td>{!check.enabled ? "已暂停" : result ? <Badge status={checkStale(node, result.checked_at, 30, now) ? "stale" : result.status !== "ok" ? result.status : result.healthy == null ? "unavailable" : result.healthy ? "ok" : "failure"} /> : "等待采样"}</td><td>{formatRelativeTime(result?.checked_at)}</td><td>{result?.error ?? "--"}</td></tr>;
  })}</Table> : <p className="monitoring-empty">尚未配置服务</p>}{checks.length > 0 && <MonitoringHistory nodeId={nodeId} section="services" devices={checks.map((item) => ({ id: item.id, name: item.name }))} onUnauthorized={onUnauthorized} />}</>;
}

function Probes({ node, data, config, onUnauthorized }: { node: NodeSnapshot; data?: MonitoringData | null; config: NodeMonitoringConfig | null; onUnauthorized: () => void }) {
  const nodeId = node.id;
  const now = useClock();
  const range = useHistoryRange();
  const minutes = range?.minutes ?? 60;
  const fetcher = useCallback((signal: AbortSignal) => api.probeStatistics(nodeId, minutes, signal), [nodeId, minutes]);
  const { data: stats, loading, error, refresh } = useHistoryQuery<ProbeStatistics[]>(`probes:${nodeId}:${minutes}:${config?.revision}`, fetcher, onUnauthorized, `probes:${nodeId}:${config?.revision}`, range?.refreshKey);
  const statistics = stats ?? [];
  const probes = config?.probes ?? [];
  return <>{data && <Capabilities data={data} names={["probes"]} />}{probes.length ? <Table headings={["目标", "类型", "地址", "状态", "延迟", "HTTP", "最近检查", "错误"]}>{probes.map((target) => {
    const result = [...(data?.probes ?? [])].reverse().find((item) => item.target_id === target.id && item.config_revision === config?.revision);
    const stale = checkStale(node, result?.completed_at, target.interval_secs, now);
    return <tr key={target.id}><th scope="row">{target.name}</th><td>{target.kind.toUpperCase()}</td><td>{target.target}{target.port ? `:${target.port}` : ""}</td><td>{!target.enabled ? "已暂停" : result ? <Badge status={stale ? "stale" : result.status} /> : "等待采样"}</td><td>{result?.status === "success" && !stale ? formatLatency(result.latency_ms) : "--"}</td><td>{result?.http_status ?? "--"}</td><td>{formatRelativeTime(result?.completed_at)}</td><td>{result?.error ?? "--"}</td></tr>;
  })}</Table> : <p className="monitoring-empty">尚未配置探测目标</p>}
    <div className="monitoring-section-toolbar"><h3>可用性 / 延迟统计</h3><button type="button" className="icon-button" title="刷新探测统计" aria-label="刷新探测统计" onClick={refresh}><RefreshCw size={15} className={loading ? "spin" : ""} /></button></div>
    {error ? <p className="form-error" role="alert">{error}</p> : loading ? <p className="monitoring-empty">载入统计中</p> : statistics.length ? <Table headings={["目标", "成功 / 失败 / 未知", "预期样本", "成功率", "可用率（已观测）", "覆盖率", "P50", "P95", "P99", "可用 / 观测 / 未知时长"]}>{statistics.map((item) => <tr key={item.target_id}><th scope="row">{item.name}</th><td>{item.successful} / {item.failed} / {item.unknown}</td><td>{item.expected}</td><td>{formatPercent(item.success_rate_percent, 2)}</td><td>{formatPercent(item.observed_seconds > 0 ? item.available_seconds * 100 / item.observed_seconds : null, 2)}</td><td>{formatPercent(item.coverage_percent, 2)}</td><td><Quantile value={item.p50_ms} count={item.latency_samples} minimum={2} /></td><td><Quantile value={item.p95_ms} count={item.latency_samples} minimum={20} /></td><td><Quantile value={item.p99_ms} count={item.latency_samples} minimum={100} /></td><td>{number(item.available_seconds, "s", 0)} / {number(item.observed_seconds, "s", 0)} / {number(item.unknown_seconds, "s", 0)}</td></tr>)}</Table> : <p className="monitoring-empty">所选范围内暂无探测统计</p>}
    {probes.length > 0 && <MonitoringHistory nodeId={nodeId} section="probes" devices={probes.map((item) => ({ id: item.id, name: item.name, intervalSecs: item.interval_secs }))} onUnauthorized={onUnauthorized} />}
  </>;
}
