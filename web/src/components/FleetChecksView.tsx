import { useEffect, useMemo, useState } from "react";
import { ChevronRight, RefreshCw, Search, Settings2 } from "lucide-react";
import { api, ApiError } from "../api";
import type { NodeMonitoringConfig, ProbeResult, ServiceResult } from "../monitoringTypes";
import type { NodeSnapshot } from "../types";
import { formatDateTime, formatLatency } from "../utils";

type CheckState = "ok" | "bad" | "pending" | "paused" | "unknown" | "offline" | "stale";
const labels: Record<CheckState, string> = { ok: "正常", bad: "故障", pending: "未应用", paused: "已暂停", unknown: "未知", offline: "主机离线", stale: "数据过期" };
interface Props { kind: "services" | "probes"; nodes: NodeSnapshot[]; onOpenNode: (id: string, tab?: "services" | "probes" | "config") => void; onUnauthorized: () => void; refreshKey: number }

export function FleetChecksView({ kind, nodes, onOpenNode, onUnauthorized, refreshKey }: Props) {
  const [configs, setConfigs] = useState<Record<string, NodeMonitoringConfig>>({});
  const [recentChecks, setRecentChecks] = useState<Record<string, { services: ServiceResult[]; probes: ProbeResult[] }>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [loading, setLoading] = useState(true);
  const [reload, setReload] = useState(0);
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState("all");
  const [clock, setClock] = useState(Date.now());
  const nodeKey = JSON.stringify(nodes.map((node) => ({ id: node.id, revision: node.latest?.monitoring?.agent.applied_config_revision ?? null })).sort((left, right) => left.id.localeCompare(right.id)));
  useEffect(() => { const timer = window.setInterval(() => setClock(Date.now()), 5000); return () => window.clearInterval(timer); }, []);
  useEffect(() => { const timer = window.setInterval(() => { if (document.visibilityState === "visible") setReload((value) => value + 1); }, 30_000); return () => window.clearInterval(timer); }, []);
  useEffect(() => {
    const controller = new AbortController(); let index = 0;
    const ids = (JSON.parse(nodeKey) as { id: string }[]).map((node) => node.id);
    const nextConfigs: Record<string, NodeMonitoringConfig> = {}; const nextErrors: Record<string, string> = {};
    setLoading(true);
    const worker = async () => {
      while (index < ids.length && !controller.signal.aborted) {
        const id = ids[index++]!;
        try {
          nextConfigs[id] = await api.monitoringConfig(id, controller.signal);
        }
        catch (error) {
          if (controller.signal.aborted) return;
          if (error instanceof ApiError && error.status === 401) { onUnauthorized(); controller.abort(); return; }
          nextErrors[id] = error instanceof Error ? error.message : "配置载入失败";
        }
      }
    };
    void Promise.all(Array.from({ length: Math.min(4, ids.length) }, worker)).then(() => {
      if (!controller.signal.aborted) { setConfigs(nextConfigs); setErrors(nextErrors); setLoading(false); }
    });
    return () => controller.abort();
  }, [nodeKey, reload, refreshKey, onUnauthorized, kind]);
  useEffect(() => {
    setRecentChecks((current) => {
      const next: typeof current = {};
      for (const node of nodes) {
        const old = current[node.id]; const data = node.latest?.monitoring;
        const services = [...(old?.services ?? []), ...(data?.services ?? [])];
        const probes = [...(old?.probes ?? []), ...(data?.probes ?? [])];
        const latestServices = new Map<string, ServiceResult>(); const latestProbes = new Map<string, ProbeResult>();
        for (const service of services) {
          const key = `${service.id}:${service.config_revision}`; const previous = latestServices.get(key);
          if (!previous || Date.parse(service.checked_at) >= Date.parse(previous.checked_at)) latestServices.set(key, service);
        }
        for (const probe of probes) {
          const key = `${probe.target_id}:${probe.config_revision}`; const previous = latestProbes.get(key);
          if (!previous || Date.parse(probe.completed_at) >= Date.parse(previous.completed_at)) latestProbes.set(key, probe);
        }
        next[node.id] = { services: [...latestServices.values()].slice(-128), probes: [...latestProbes.values()].slice(-128) };
      }
      return next;
    });
  }, [nodes]);
  const rows = useMemo(() => nodes.flatMap((node) => {
    const config = configs[node.id]; if (!config) return [];
    const data = node.latest?.monitoring;
    const stale = !node.last_seen_at || !Number.isFinite(Date.parse(node.last_seen_at)) || clock - Date.parse(node.last_seen_at) + (data?.agent.sample_age_ms ?? 0) > Math.max(30, (data?.report_interval_secs ?? 5) * 3) * 1000;
    return config[kind].map((check) => {
      const result = kind === "services" ? [...(recentChecks[node.id]?.services ?? []), ...(data?.services ?? [])].filter((result) => result.id === check.id && result.config_revision === config.revision).sort((left, right) => Date.parse(right.checked_at) - Date.parse(left.checked_at))[0]
        : [...(recentChecks[node.id]?.probes ?? []), ...(data?.probes ?? [])].filter((result) => result.target_id === check.id && result.config_revision === config.revision).sort((left, right) => Date.parse(right.completed_at) - Date.parse(left.completed_at))[0];
      const checkedAt = result ? ("checked_at" in result ? result.checked_at : result.completed_at) : null;
      const checkExpired = checkedAt && clock - Date.parse(checkedAt) > ("interval_secs" in check ? Math.max(30, check.interval_secs * 2) : 30) * 1000;
      const state: CheckState = !check.enabled ? "paused" : !node.online ? "offline" : stale ? "stale"
        : data?.agent.applied_config_revision !== config.revision ? "pending" : !result ? "unknown" : checkExpired ? "stale"
        : "healthy" in result ? result.status === "ok" && result.healthy !== null ? result.healthy ? "ok" : "bad" : "unknown"
        : result.status === "success" ? "ok" : result.status === "failure" || result.status === "timeout" ? "bad" : "unknown";
      const detail = result ? ("state" in result ? result.state : result.status) : "--";
      return { node, id: check.id, name: check.name, state, checkedAt, error: result?.error ?? null,
        target: "target" in check ? `${check.kind.toUpperCase()} · ${check.target}${check.port ? `:${check.port}` : ""}` : `期望 ${check.expected_state === "running" ? "运行" : "停止"}`,
        latency: result && "latency_ms" in result && state === "ok" ? result.latency_ms : null, detail };
    });
  }), [nodes, configs, kind, clock, recentChecks]);
  const firstNode = nodes[0];
  const filtered = rows.filter((row) => (filter === "all" || row.state === filter) && [row.name, row.node.display_name, row.node.hostname, row.target]
    .some((value) => value.toLowerCase().includes(query.trim().toLowerCase())));
  return <section className="fleet-checks">
    <div className="fleet-summary-strip">{(["ok", "bad", "pending", "unknown", "paused"] as const).map((state) => <button key={state} className={filter === state ? "active" : ""} onClick={() => setFilter(filter === state ? "all" : state)}><span className={`fleet-dot ${state}`} />{labels[state]}<strong>{rows.filter((row) => row.state === state).length}</strong></button>)}<span className="fleet-summary-total">{rows.length} 项</span></div>
    <div className="fleet-toolbar"><label className="search-field"><Search size={15} /><input aria-label={`搜索${kind === "services" ? "服务" : "探测"}`} placeholder="主机、名称或目标" value={query} onChange={(event) => setQuery(event.target.value)} /></label>
      <select aria-label="监测状态" value={filter} onChange={(event) => setFilter(event.target.value)}><option value="all">全部状态</option>{Object.entries(labels).map(([state, label]) => <option key={state} value={state}>{label}</option>)}</select>
      <button className="icon-button" title="刷新监测配置" aria-label="刷新监测配置" disabled={loading} onClick={() => setReload((value) => value + 1)}><RefreshCw size={16} className={loading ? "spin" : ""} /></button>
    </div>
    {Object.entries(errors).map(([id, error]) => <div className="global-error" role="alert" key={id}><span>{nodes.find((node) => node.id === id)?.display_name || id}：{error}</span><button className="icon-button subtle" title="重新载入" aria-label="重新载入" onClick={() => setReload((value) => value + 1)}><RefreshCw size={14} /></button></div>)}
    {loading && !rows.length ? <div className="fleet-empty"><RefreshCw size={18} className="spin" />正在载入配置</div> : !filtered.length ? <div className="fleet-empty">{rows.length ? "没有匹配的监测项目" : "暂无已配置项目"}{!rows.length && firstNode && <button className="secondary-button" onClick={() => onOpenNode(firstNode.id, "config")}><Settings2 size={15} />配置主机</button>}</div> : <div className="fleet-table-scroll"><table className="fleet-table"><thead><tr><th>主机 / 项目</th><th>目标</th><th>状态</th>{kind === "probes" && <th>响应时间</th>}<th>最近检查</th><th><span className="sr-only">操作</span></th></tr></thead><tbody>{filtered.map((row) => <tr key={`${row.node.id}:${row.id}`}><td><button className="fleet-name" onClick={() => onOpenNode(row.node.id, kind)}><strong>{row.name}</strong><small>{row.node.display_name || row.node.hostname}</small></button></td><td className="fleet-target" title={row.target}>{row.target}</td><td><span className={`fleet-state ${row.state}`} title={row.error ?? row.detail}><span className={`fleet-dot ${row.state}`} />{labels[row.state]}</span>{row.error && row.state === "bad" && <small className="fleet-error-detail" title={row.error}>{row.error}</small>}</td>{kind === "probes" && <td>{formatLatency(row.latency)}</td>}<td className="fleet-time">{formatDateTime(row.checkedAt)}</td><td><button className="icon-button subtle" title="打开主机监测" aria-label="打开主机监测" onClick={() => onOpenNode(row.node.id, kind)}><ChevronRight size={16} /></button></td></tr>)}</tbody></table></div>}
  </section>;
}
