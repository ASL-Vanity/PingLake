import { formatPercent, metricSeverity } from "../utils";

interface MetricBarProps {
  value: number;
  compact?: boolean;
  label?: string;
}

export function MetricBar({ value, compact = false, label }: MetricBarProps) {
  const severity = metricSeverity(value, 70, 90);
  return (
    <div className={`metric-bar-wrap ${compact ? "compact" : ""}`} aria-label={`${label ?? "使用率"} ${formatPercent(value, 1)}`}>
      <div className="metric-value">
        {label && <span className="metric-label">{label}</span>}
        <strong>{formatPercent(value, value < 10 ? 1 : 0)}</strong>
      </div>
      <div className="metric-track" aria-hidden="true">
        <span className={`metric-fill ${severity}`} style={{ width: `${Math.min(100, Math.max(2, value))}%` }} />
      </div>
    </div>
  );
}
