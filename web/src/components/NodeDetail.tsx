import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  CartesianGrid,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { api, ApiError } from "../api";
import type { HistoryPoint, NodeSnapshot } from "../types";
import {
  formatBytes,
  formatDateTime,
  formatDuration,
  formatLatency,
  formatOperatingSystem,
  formatPercent,
  formatRate,
  formatRelativeTime,
  formatTemperature,
  historyDiskPercent,
  historyMemoryPercent,
  nodeDiskPercent,
  nodeMemoryPercent,
  ratioPercent,
} from "../utils";
import { MetricBar } from "./MetricBar";
import { AppIcon } from "./AppIcon";

interface NodeDetailProps {
  node: NodeSnapshot;
  onBack: () => void;
  onDelete: (nodeId: string) => Promise<void>;
  onRename: (nodeId: string, displayName: string) => Promise<NodeSnapshot>;
  onUnauthorized: () => void;
}

type HistoryRange = 60 | 360 | 1440;

export function NodeDetail({ node, onBack, onDelete, onRename, onUnauthorized }: NodeDetailProps) {
  const [range, setRange] = useState<HistoryRange>(60);
  const [history, setHistory] = useState<HistoryPoint[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [deleteConfirmationOpen, setDeleteConfirmationOpen] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [editingName, setEditingName] = useState(false);
  const [nameDraft, setNameDraft] = useState(node.display_name || node.hostname);
  const [nameSaving, setNameSaving] = useState(false);
  const [nameError, setNameError] = useState<string | null>(null);
  const historyRequestId = useRef(0);
  const historyRequestAbort = useRef<AbortController | null>(null);

  const loadHistory = useCallback(async (quiet = false) => {
    const requestId = ++historyRequestId.current;
    historyRequestAbort.current?.abort();
    const controller = new AbortController();
    historyRequestAbort.current = controller;
    if (!quiet) setLoading(true);
    try {
      const points = await api.history(node.id, range, controller.signal);
      if (requestId !== historyRequestId.current) return;
      setHistory(points);
      setError(null);
    } catch (reason) {
      if (controller.signal.aborted || requestId !== historyRequestId.current) return;
      if (reason instanceof ApiError && reason.status === 401) {
        onUnauthorized();
        return;
      }
      setError(reason instanceof Error ? reason.message : "无法载入历史数据");
    } finally {
      if (requestId === historyRequestId.current) {
        historyRequestAbort.current = null;
        setLoading(false);
      }
    }
  }, [node.id, onUnauthorized, range]);

  useEffect(() => {
    void loadHistory();
    const interval = window.setInterval(() => void loadHistory(true), 30_000);
    return () => {
      window.clearInterval(interval);
      historyRequestId.current += 1;
      historyRequestAbort.current?.abort();
      historyRequestAbort.current = null;
    };
  }, [loadHistory]);

  const confirmDelete = async () => {
    if (deleting) return;
    setDeleting(true);
    setDeleteError(null);
    try {
      await onDelete(node.id);
      onBack();
    } catch (reason) {
      setDeleteError(reason instanceof Error ? reason.message : "删除节点失败");
    } finally {
      setDeleting(false);
    }
  };

  const saveName = async () => {
    const value = nameDraft.trim();
    if (!value || nameSaving) return;
    setNameSaving(true);
    setNameError(null);
    try {
      await onRename(node.id, value);
      setEditingName(false);
    } catch (reason) {
      setNameError(reason instanceof Error ? reason.message : "Unable to update server name.");
    } finally {
      setNameSaving(false);
    }
  };

  const chartData = useMemo(() => history.map((point) => ({
    time: point.collected_at,
    cpu: point.cpu_percent,
    memory: historyMemoryPercent(point),
    disk: historyDiskPercent(point),
    received: point.network_received_bytes_per_sec / 1024 / 1024,
    transmitted: point.network_transmitted_bytes_per_sec / 1024 / 1024,
    temperature: point.temperature_celsius,
  })), [history]);

  const latest = node.latest;
  const memoryPercent = nodeMemoryPercent(node);
  const diskPercent = nodeDiskPercent(node);
  const hubLatency = latest?.hub_latency_ms;

  return (
    <div className="detail-view">
      <header className="detail-header">
        <button type="button" className="icon-button" onClick={onBack} aria-label="返回节点列表" title="返回节点列表">
          <AppIcon name="back" size={18} />
        </button>
        <div className="detail-title">
          <div className="detail-title-line">
            <span className={`status-dot ${node.online ? "online" : "offline"}`} />
            {editingName ? (
              <div className="node-name-editor">
                <input value={nameDraft} maxLength={64} onChange={(event) => setNameDraft(event.target.value)} onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); void saveName(); } }} autoFocus />
                <button type="button" className="icon-button" onClick={() => void saveName()} disabled={nameSaving || !nameDraft.trim()} title="保存名称" aria-label="保存名称"><AppIcon name="check" size={16} /></button>
                <button type="button" className="icon-button" onClick={() => { setEditingName(false); setNameDraft(node.display_name || node.hostname); setNameError(null); }} disabled={nameSaving} title="取消编辑" aria-label="取消编辑"><AppIcon name="close" size={16} /></button>
              </div>
            ) : (
              <><h1>{node.display_name || node.hostname}</h1><button type="button" className="icon-button name-edit-button" onClick={() => setEditingName(true)} title="编辑服务器名称" aria-label="编辑服务器名称"><AppIcon name="edit" size={15} /></button></>
            )}
            <span className={`status-label ${node.online ? "online" : "offline"}`}>{node.online ? "在线" : "离线"}</span>
          </div>
          <p>{node.hostname} · {formatOperatingSystem(node.os, node.os_version)} · {node.architecture}</p>
        </div>
        <div className="detail-header-actions">
          <div className="detail-heartbeat">
            <span>最后心跳</span>
            <strong title={formatDateTime(node.last_seen_at)}>{formatRelativeTime(node.last_seen_at)}</strong>
          </div>
          <div className="detail-heartbeat latency-heartbeat">
            <span>Hub 延迟</span>
            <strong title="节点到新加坡 PingLake Hub 的 HTTPS 往返耗时">{formatLatency(hubLatency)}</strong>
          </div>
          <button
            type="button"
            className="danger-button detail-delete-button"
            onClick={() => { setDeleteError(null); setDeleteConfirmationOpen(true); }}
          >
            <AppIcon name="trash" size={16} />删除节点
          </button>
        </div>
      </header>

      <section className="current-metrics" aria-label="当前指标">
        <article>
          <div className="metric-heading"><AppIcon name="cpu" size={17} /><span>CPU</span></div>
          <strong>{formatPercent(latest?.cpu_percent, 1)}</strong>
          <MetricBar value={latest?.cpu_percent} />
          <small>{latest?.load_one != null ? `负载 ${latest.load_one.toFixed(2)} / ${latest.load_five?.toFixed(2) ?? "--"}` : "暂无负载数据"}</small>
        </article>
        <article>
          <div className="metric-heading"><AppIcon name="memory" size={17} /><span>内存</span></div>
          <strong>{formatPercent(memoryPercent, 1)}</strong>
          <MetricBar value={latest ? memoryPercent : null} />
          <small>{formatBytes(latest?.memory_used_bytes)} / {formatBytes(latest?.memory_total_bytes)}</small>
        </article>
        <article>
          <div className="metric-heading"><AppIcon name="disk" size={17} /><span>磁盘</span></div>
          <strong>{formatPercent(diskPercent, 1)}</strong>
          <MetricBar value={latest ? diskPercent : null} />
          <small>{formatBytes(latest?.disk_used_bytes)} / {formatBytes(latest?.disk_total_bytes)}</small>
        </article>
        <article>
          <div className="metric-heading"><AppIcon name="network" size={17} /><span>网络</span></div>
          <div className="network-current">
            <strong><AppIcon name="download" size={15} />{formatRate(latest?.network_received_bytes_per_sec)}</strong>
            <strong><AppIcon name="upload" size={15} />{formatRate(latest?.network_transmitted_bytes_per_sec)}</strong>
          </div>
          <small>{latest?.interfaces.length ?? 0} 个活动接口</small>
        </article>
      </section>

      <section className="data-panel history-panel" aria-labelledby="history-heading">
        <div className="panel-toolbar">
          <div><h2 id="history-heading">历史趋势</h2><span>{history.length} 个采样点</span></div>
          <div className="chart-actions">
            <div className="segmented-control" aria-label="历史范围">
              {([60, 360, 1440] as const).map((minutes) => (
                <button type="button" className={range === minutes ? "active" : ""} onClick={() => setRange(minutes)} key={minutes}>
                  {minutes === 60 ? "1 小时" : minutes === 360 ? "6 小时" : "24 小时"}
                </button>
              ))}
            </div>
            <button type="button" className="icon-button" onClick={() => void loadHistory()} title="刷新历史数据" aria-label="刷新历史数据">
              <AppIcon name="refresh" size={16} className={loading ? "spin" : ""} />
            </button>
          </div>
        </div>
        {error ? (
          <div className="chart-empty error-state">{error}</div>
        ) : loading ? (
          <div className="chart-loading"><span /><span /><span /></div>
        ) : chartData.length === 0 ? (
          <div className="chart-empty">所选时间范围内暂无采样数据</div>
        ) : (
          <div className="chart-grid">
            <article className="chart-block">
              <div className="chart-heading"><strong>资源使用率</strong><span><i className="legend cpu" />CPU <i className="legend memory" />内存 <i className="legend disk" />磁盘</span></div>
              <div className="chart-container">
                <ResponsiveContainer width="100%" height="100%">
                  <LineChart data={chartData} margin={{ top: 8, right: 12, bottom: 2, left: -18 }}>
                    <CartesianGrid stroke="var(--chart-grid)" strokeDasharray="3 3" vertical={false} />
                    <XAxis dataKey="time" tickFormatter={formatChartTime} tick={{ fontSize: 11, fill: "var(--chart-text)" }} minTickGap={42} axisLine={false} tickLine={false} />
                    <YAxis domain={[0, 100]} tickFormatter={(value: number) => `${value}%`} tick={{ fontSize: 11, fill: "var(--chart-text)" }} axisLine={false} tickLine={false} />
                    <Tooltip labelFormatter={(value) => formatDateTime(String(value))} formatter={(value, name) => [`${Number(value).toFixed(1)}%`, metricName(String(name))]} contentStyle={tooltipStyle} />
                    <Line type="monotone" dataKey="cpu" stroke="var(--chart-cpu)" strokeWidth={2} dot={false} activeDot={{ r: 3 }} isAnimationActive={false} />
                    <Line type="monotone" dataKey="memory" stroke="var(--chart-memory)" strokeWidth={2} dot={false} activeDot={{ r: 3 }} isAnimationActive={false} />
                    <Line type="monotone" dataKey="disk" stroke="var(--chart-disk)" strokeWidth={1.6} dot={false} activeDot={{ r: 3 }} isAnimationActive={false} />
                  </LineChart>
                </ResponsiveContainer>
              </div>
            </article>
            <article className="chart-block">
              <div className="chart-heading"><strong>网络吞吐</strong><span><i className="legend received" />下行 <i className="legend transmitted" />上行</span></div>
              <div className="chart-container">
                <ResponsiveContainer width="100%" height="100%">
                  <LineChart data={chartData} margin={{ top: 8, right: 12, bottom: 2, left: -12 }}>
                    <CartesianGrid stroke="var(--chart-grid)" strokeDasharray="3 3" vertical={false} />
                    <XAxis dataKey="time" tickFormatter={formatChartTime} tick={{ fontSize: 11, fill: "var(--chart-text)" }} minTickGap={42} axisLine={false} tickLine={false} />
                    <YAxis tickFormatter={(value: number) => `${value.toFixed(value < 1 ? 1 : 0)}`} tick={{ fontSize: 11, fill: "var(--chart-text)" }} axisLine={false} tickLine={false} width={44} />
                    <Tooltip labelFormatter={(value) => formatDateTime(String(value))} formatter={(value, name) => [`${Number(value).toFixed(2)} MB/s`, metricName(String(name))]} contentStyle={tooltipStyle} />
                    <Line type="monotone" dataKey="received" stroke="var(--chart-received)" strokeWidth={2} dot={false} activeDot={{ r: 3 }} isAnimationActive={false} />
                    <Line type="monotone" dataKey="transmitted" stroke="var(--chart-transmitted)" strokeWidth={2} dot={false} activeDot={{ r: 3 }} isAnimationActive={false} />
                  </LineChart>
                </ResponsiveContainer>
              </div>
              <span className="chart-unit">MB/s</span>
            </article>
          </div>
        )}
      </section>

      <div className="detail-lower-grid">
        <section className="data-panel system-panel" aria-labelledby="system-heading">
          <div className="panel-toolbar"><div><h2 id="system-heading">系统信息</h2></div></div>
          <dl className="system-info-grid">
            <div><dt>操作系统</dt><dd>{node.os} {node.os_version}</dd></div>
            <div><dt>内核</dt><dd>{node.kernel_version || "--"}</dd></div>
            <div><dt>架构</dt><dd>{node.architecture || "--"}</dd></div>
            <div><dt>Agent</dt><dd>{node.agent_version || "--"}</dd></div>
            <div><dt>运行时间</dt><dd>{formatDuration(latest?.uptime_seconds)}</dd></div>
            <div><dt>进程数</dt><dd>{latest?.process_count ?? "--"}</dd></div>
            <div><dt>温度</dt><dd><AppIcon name="temperature" size={15} />{formatTemperature(latest?.temperature_celsius)}</dd></div>
            <div><dt>注册时间</dt><dd>{formatDateTime(node.enrolled_at)}</dd></div>
          </dl>
        </section>

        <section className="data-panel disks-panel" aria-labelledby="disks-heading">
          <div className="panel-toolbar"><div><h2 id="disks-heading">磁盘</h2><span>{latest?.disks.length ?? 0} 个卷</span></div></div>
          {!latest?.disks.length ? (
            <div className="small-empty">暂无磁盘明细</div>
          ) : (
            <div className="disk-list">
              {latest.disks.map((disk, index) => {
                const percent = ratioPercent(disk.used_bytes, disk.total_bytes);
                return (
                  <div className="disk-row" key={`${disk.name}-${disk.mount_point}-${index}`}>
                    <AppIcon name="box" size={16} />
                    <div className="disk-main">
                      <div><strong>{disk.name || disk.mount_point}</strong><span>{disk.mount_point} · {disk.file_system || "--"}</span></div>
                      <MetricBar value={percent} compact />
                    </div>
                    <span>{formatBytes(disk.used_bytes)} / {formatBytes(disk.total_bytes)}</span>
                  </div>
                );
              })}
            </div>
          )}
        </section>
      </div>

      <section className="data-panel process-panel" aria-labelledby="processes-heading">
        <div className="panel-toolbar"><div><h2 id="processes-heading">进程占用</h2><span>{latest?.processes.length ?? 0} / {latest?.process_count ?? 0} 个进程</span></div><span className="process-note"><AppIcon name="sort" size={14} />按 CPU、内存排序</span></div>
        {!latest?.processes.length ? (
          <div className="small-empty">等待更新后的 Agent 上报进程明细</div>
        ) : (
          <div className="process-table-scroll">
            <table className="process-table">
              <thead><tr><th>进程</th><th>PID</th><th>CPU</th><th>内存</th></tr></thead>
              <tbody>{latest.processes.map((process) => <tr key={`${process.pid}-${process.name}`}><td title={process.name}>{process.name || "--"}</td><td>{process.pid}</td><td>{process.cpu_percent.toFixed(1)}%</td><td>{formatBytes(process.memory_bytes)}</td></tr>)}</tbody>
            </table>
          </div>
        )}
      </section>

      {deleteConfirmationOpen && (
        <DeleteConfirmation
          nodeName={node.display_name || node.hostname}
          deleting={deleting}
          error={deleteError}
          onClose={() => { if (!deleting) setDeleteConfirmationOpen(false); }}
          onConfirm={() => void confirmDelete()}
        />
      )}
    </div>
  );
}

function DeleteConfirmation({
  nodeName,
  deleting,
  error,
  onClose,
  onConfirm,
}: {
  nodeName: string;
  deleting: boolean;
  error: string | null;
  onClose: () => void;
  onConfirm: () => void;
}) {
  const dialogRef = useRef<HTMLElement>(null);
  const cancelButton = useRef<HTMLButtonElement>(null);
  const previousFocus = useRef<HTMLElement | null>(null);
  const deletingRef = useRef(deleting);
  const onCloseRef = useRef(onClose);
  deletingRef.current = deleting;
  onCloseRef.current = onClose;

  useEffect(() => {
    previousFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    const focusTimer = window.setTimeout(() => cancelButton.current?.focus(), 0);
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === "Escape" && !deletingRef.current) onCloseRef.current();
      if (event.key !== "Tab") return;
      const focusable = Array.from(
        dialogRef.current?.querySelectorAll<HTMLElement>(
          'button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [href], [tabindex]:not([tabindex="-1"])',
        ) ?? [],
      ).filter((element) => !element.hasAttribute("disabled") && element.getClientRects().length > 0);
      if (focusable.length === 0) {
        event.preventDefault();
        dialogRef.current?.focus();
        return;
      }
      const first = focusable[0]!;
      const last = focusable[focusable.length - 1]!;
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    };
    window.addEventListener("keydown", handleKey);
    return () => {
      window.clearTimeout(focusTimer);
      window.removeEventListener("keydown", handleKey);
      previousFocus.current?.focus();
    };
  }, []);

  return (
    <div className="confirm-layer">
      <button type="button" className="confirm-backdrop" aria-label="关闭删除确认" onClick={onClose} disabled={deleting} tabIndex={-1} />
      <section ref={dialogRef} className="confirm-dialog" role="alertdialog" aria-modal="true" aria-labelledby="delete-node-title" aria-describedby="delete-node-description" tabIndex={-1}>
        <h2 id="delete-node-title">删除节点？</h2>
        <p id="delete-node-description">将删除“{nodeName}”及其历史指标和告警记录。此操作无法撤销。</p>
        {error && <div className="form-error" role="alert">{error}</div>}
        <footer>
          <button ref={cancelButton} type="button" className="secondary-button" onClick={onClose} disabled={deleting}>取消</button>
          <button type="button" className="danger-button" onClick={onConfirm} disabled={deleting}>
            <AppIcon name="trash" size={16} />{deleting ? "正在删除" : "确认删除"}
          </button>
        </footer>
      </section>
    </div>
  );
}

const tooltipStyle = {
  border: "1px solid var(--chart-tooltip-border)",
  borderRadius: 6,
  boxShadow: "0 8px 24px rgba(18, 23, 20, 0.12)",
  fontSize: 12,
  color: "var(--text)",
  background: "var(--chart-tooltip)",
};

function formatChartTime(value: string): string {
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "--";
  return new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit", hour12: false }).format(date);
}

function metricName(name: string): string {
  return ({ cpu: "CPU", memory: "内存", disk: "磁盘", received: "下行", transmitted: "上行" } as Record<string, string>)[name] ?? name;
}
