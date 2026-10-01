import type { MonitoringHistoryPoint } from "../monitoringTypes";

export function buildHistorySeries(points: MonitoringHistoryPoint[], options: { events: boolean; intervalSecs?: number; minutes: number }, reading: (point: MonitoringHistoryPoint) => number | null) {
  return points.flatMap((point, index) => {
    const time = options.events ? point.collected_at : point.received_at;
    const previous = points[index - 1];
    const previousTime = previous ? options.events ? previous.collected_at : previous.received_at : null;
    const cadence = options.events ? (options.intervalSecs ?? 30) * 3000 : Math.max(point.monitoring.report_interval_secs * 3000, options.minutes * 60_000 / 240 * 3);
    const gap = previous && ((!options.events && previous.monitoring.session_id !== point.monitoring.session_id) || Date.parse(time) - Date.parse(previousTime!) > cadence);
    const actual = { time, value: reading(point) };
    return gap ? [{ time: new Date(Date.parse(previousTime!) + 1).toISOString(), value: null }, actual] : [actual];
  });
}
