import { createContext, useContext, type ReactNode } from "react";

export const historyRanges = [[60, "1 小时"], [360, "6 小时"], [1440, "24 小时"]] as const;
export type HistoryMinutes = (typeof historyRanges)[number][0];
const RangeContext = createContext<{ minutes: HistoryMinutes; setMinutes: (minutes: HistoryMinutes) => void; refreshKey: number } | null>(null);

export function HistoryRangeProvider({ minutes, setMinutes, children, refreshKey = 0 }: { minutes: HistoryMinutes; setMinutes: (minutes: HistoryMinutes) => void; children: ReactNode; refreshKey?: number }) {
  return <RangeContext.Provider value={{ minutes, setMinutes, refreshKey }}>{children}</RangeContext.Provider>;
}

export function useHistoryRange() { return useContext(RangeContext); }

export function HistoryRangeControl({ minutes, onChange }: { minutes: HistoryMinutes; onChange: (minutes: HistoryMinutes) => void }) {
  return <div className="segmented-control detail-range-control" aria-label="监测时间范围">{historyRanges.map(([value, label]) => <button type="button" key={value} className={minutes === value ? "active" : ""} aria-pressed={minutes === value} onClick={() => onChange(value)}>{label}</button>)}</div>;
}
