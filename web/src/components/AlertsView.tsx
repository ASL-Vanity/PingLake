import { useMemo, useState } from "react";
import { AppIcon } from "./AppIcon";
import type { AlertKind, AlertRecord } from "../types";
import { formatDateTime, formatRelativeTime } from "../utils";

interface AlertsViewProps {
  alerts: AlertRecord[];
  onOpenNode: (nodeId: string) => void;
}

const kindLabels: Record<AlertKind, string> = {
  offline: "离线",
  cpu: "CPU",
  memory: "内存",
  disk: "磁盘",
  temperature: "温度",
};

export function AlertsView({ alerts, onOpenNode }: AlertsViewProps) {
  const [filter, setFilter] = useState<"active" | "all" | "resolved">("active");
  const [query, setQuery] = useState("");
  const filtered = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    return alerts.filter((alert) => {
      if (filter === "active" && !alert.active) return false;
      if (filter === "resolved" && alert.active) return false;
      return !normalized || [alert.node_name, alert.message, kindLabels[alert.kind]].some((value) => value.toLowerCase().includes(normalized));
    });
  }, [alerts, filter, query]);

  const activeCount = alerts.filter((alert) => alert.active).length;

  return (
    <section className="data-panel alerts-panel" aria-labelledby="alerts-heading">
      <div className="panel-toolbar">
        <div><h2 id="alerts-heading">告警</h2><span>{activeCount ? `${activeCount} 条活动告警` : "当前无活动告警"}</span></div>
        <div className="table-controls">
          <div className="segmented-control" aria-label="告警筛选">
            {(["active", "all", "resolved"] as const).map((value) => (
              <button type="button" className={filter === value ? "active" : ""} aria-pressed={filter === value} onClick={() => setFilter(value)} key={value}>
                {value === "active" ? "活动" : value === "all" ? "全部" : "已恢复"}
              </button>
            ))}
          </div>
          <label className="search-field">
            <AppIcon name="search" size={15} />
            <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索告警" aria-label="搜索告警" />
          </label>
        </div>
      </div>
      {filtered.length === 0 ? (
        <div className="alerts-empty">
          <AppIcon name="check" size={24} />
          <strong>{filter === "active" ? "当前没有活动告警" : "没有匹配的告警"}</strong>
        </div>
      ) : (
        <div className="alert-list">
          {filtered.map((alert) => (
            <button type="button" className={`alert-row ${alert.active ? "active" : "resolved"} alert-${alert.kind}`} onClick={() => onOpenNode(alert.node_id)} key={alert.id} aria-label={`${alert.node_name} ${kindLabels[alert.kind]}告警，${alert.active ? "活动" : "已恢复"}`}>
              <span className={`alert-icon ${alert.active ? "active" : "resolved"}`} aria-hidden="true">
                {alert.active ? <AppIcon name="warning" size={17} /> : <AppIcon name="check" size={17} />}
              </span>
              <span className="alert-state">
                <strong>{alert.active ? "活动" : "已恢复"}</strong>
                <small>{kindLabels[alert.kind]}</small>
              </span>
              <span className="alert-node"><strong>{alert.node_name}</strong><small>{alert.message}</small></span>
              <span className="alert-reading">
                {alert.value != null ? <strong>{formatAlertValue(alert.kind, alert.value)}</strong> : <strong>--</strong>}
                {alert.threshold != null && <small>阈值 {formatAlertValue(alert.kind, alert.threshold)}</small>}
              </span>
              <span className="alert-time" title={formatDateTime(alert.opened_at)}>
                <AppIcon name="timeline" size={12} /><span>{formatRelativeTime(alert.opened_at)}</span>
                <small>{alert.active ? "触发" : alert.resolved_at ? `恢复于 ${formatDateTime(alert.resolved_at)}` : "已恢复"}</small>
              </span>
            </button>
          ))}
        </div>
      )}
    </section>
  );
}

function formatAlertValue(kind: AlertKind, value: number): string {
  if (kind === "temperature") return `${value.toFixed(1)} °C`;
  if (kind === "offline") return `${value.toFixed(0)} 秒`;
  return `${value.toFixed(1)}%`;
}
