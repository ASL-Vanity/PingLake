import { formatPercent, metricSeverity } from "../utils";

interface MetricBarProps {
  value: number | null | undefined;
  compact?: boolean;
  label?: string;
}

export function MetricBar({ value, compact = false, label }: MetricBarProps) {
  const sampled = value != null && Number.isFinite(value);
  const numericValue = value == null || !Number.isFinite(value) ? 0 : value;
  const severity = sampled ? metricSeverity(numericValue, 70, 90) : "unavailable";
  const displayValue = sampled ? formatPercent(numericValue, numericValue < 10 ? 1 : 0) : "暂无采样";
  return (
    <div
      className={`metric-bar-wrap ${compact ? "compact" : ""} ${sampled ? "sampled" : "unavailable"}`}
      aria-label={`${label ?? "使用率"} ${displayValue}`}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={sampled ? numericValue : undefined}
      role="meter"
    >
      <div className="metric-value">
        {label && <span className="metric-label">{label}</span>}
        <strong>{displayValue}</strong>
      </div>
      <div className="metric-track" aria-hidden="true">
        <span className={`metric-fill ${severity}`} style={{ width: sampled ? `${Math.min(100, Math.max(0, numericValue))}%` : "0%" }} />
      </div>
    </div>
  );
}
