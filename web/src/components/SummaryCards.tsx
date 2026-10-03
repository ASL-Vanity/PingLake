import { AppIcon, type AppIconName } from "./AppIcon";
import type { DashboardSummary, NodeSnapshot } from "../types";
import { formatPercent } from "../utils";

interface SummaryCardsProps {
  summary: DashboardSummary;
  nodes: NodeSnapshot[];
}

const summaryItems = (summary: DashboardSummary, nodes: NodeSnapshot[]) => {
  const hasOnlineCpuSample = nodes.some((node) => node.online && node.latest !== null);
  const hasOnlineMemorySample = nodes.some((node) => node.online && node.latest !== null && node.latest.memory_total_bytes > 0);
  return [
  { label: "离线节点", value: String(summary.offline_nodes), detail: summary.offline_nodes ? "需要关注" : "全部节点在线", icon: "offline" as AppIconName, tone: summary.offline_nodes ? "danger" : "success" },
  { label: "活动告警", value: String(summary.active_alerts), detail: summary.active_alerts ? "存在未处理事件" : "当前无活动告警", icon: "alert" as AppIconName, tone: summary.active_alerts ? "warning" : "success" },
  { label: "在线平均 CPU", value: hasOnlineCpuSample ? formatPercent(summary.average_cpu_percent, 1) : "暂无采样", detail: "在线节点平均值", icon: "cpu" as AppIconName, tone: "neutral" },
  { label: "在线平均内存", value: hasOnlineMemorySample ? formatPercent(summary.average_memory_percent, 1) : "暂无采样", detail: "在线节点平均值", icon: "memory" as AppIconName, tone: "neutral" },
  ];
};

export function SummaryCards({ summary, nodes }: SummaryCardsProps) {
  const healthPercent = summary.total_nodes ? Math.round((summary.online_nodes / summary.total_nodes) * 100) : null;
  return (
    <section className="summary-grid" aria-label="运行概览">
      <article className={`summary-item summary-health ${summary.total_nodes === 0 ? "neutral" : summary.offline_nodes ? "warning" : "success"}`}>
        <div className="summary-health-copy">
          <span className="summary-kicker">运行状态</span>
          <strong>{summary.online_nodes}<small> / {summary.total_nodes}</small></strong>
          <span className="summary-detail">{summary.total_nodes === 0 ? "暂无节点" : `${healthPercent}% 健康度 · ${summary.offline_nodes ? `${summary.offline_nodes} 个节点离线` : "所有节点运行正常"}`}</span>
        </div>
        <div className="summary-health-icon" aria-hidden="true"><AppIcon name="activity" size={21} /></div>
      </article>
      {summaryItems(summary, nodes).map((item) => {
        return (
          <article className={`summary-item ${item.tone}`} key={item.label}>
            <div>
              <span className="summary-kicker">{item.label}</span>
              <strong>{item.value}</strong>
              <small className="summary-detail">{item.detail}</small>
            </div>
            <AppIcon name={item.icon} size={19} />
          </article>
        );
      })}
    </section>
  );
}
