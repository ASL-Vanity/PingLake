import { Activity, BellRing, Cpu, MemoryStick, Server, ServerOff } from "lucide-react";
import type { DashboardSummary } from "../types";
import { formatPercent } from "../utils";

interface SummaryCardsProps {
  summary: DashboardSummary;
}

const summaryItems = (summary: DashboardSummary) => [
  { label: "受监主机", value: String(summary.total_nodes), icon: Server, tone: "neutral" },
  { label: "在线", value: String(summary.online_nodes), icon: Activity, tone: "success" },
  { label: "离线", value: String(summary.offline_nodes), icon: ServerOff, tone: summary.offline_nodes ? "danger" : "neutral" },
  { label: "活动告警", value: String(summary.active_alerts), icon: BellRing, tone: summary.active_alerts ? "warning" : "neutral" },
  { label: "在线平均 CPU", value: formatPercent(summary.average_cpu_percent, 1), icon: Cpu, tone: "neutral" },
  { label: "在线平均内存", value: formatPercent(summary.average_memory_percent, 1), icon: MemoryStick, tone: "neutral" },
];

export function SummaryCards({ summary }: SummaryCardsProps) {
  return (
    <section className="summary-grid" aria-label="运行概览">
      {summaryItems(summary).map((item) => {
        const Icon = item.icon;
        return (
          <article className={`summary-item ${item.tone}`} key={item.label}>
            <div>
              <span>{item.label}</span>
              <strong>{item.value}</strong>
            </div>
            <Icon size={19} strokeWidth={1.8} aria-hidden="true" />
          </article>
        );
      })}
    </section>
  );
}
