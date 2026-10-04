import { useEffect, useMemo, useState } from "react";
import type { HostGroup, NodeSnapshot } from "../types";
import {
  formatDateTime,
  formatLatency,
  formatOperatingSystem,
  formatRate,
  formatRelativeTime,
  nodeDiskPercent,
  nodeMemoryPercent,
} from "../utils";
import { EmptyNodes } from "./EmptyNodes";
import { MetricBar } from "./MetricBar";
import { AppIcon } from "./AppIcon";
import type { BrowserLatency } from "../hooks/useBrowserLatency";

interface NodeTableProps {
  nodes: NodeSnapshot[];
  groups: HostGroup[];
  onSelect: (node: NodeSnapshot) => void;
  onCreateGroup: (name: string) => Promise<HostGroup>;
  onAssignGroup: (nodeId: string, groupId: string | null) => Promise<void>;
  browserLatency?: Record<string, BrowserLatency>;
  query?: string;
  onQueryChange?: (query: string) => void;
  groupId?: string;
  onGroupChange?: (groupId: string) => void;
  onManageGroups?: () => void;
}

type NodeViewMode = "cards" | "list";

export function NodeTable({ nodes, groups, onSelect, onCreateGroup, onAssignGroup, browserLatency = {}, query: externalQuery, onQueryChange, groupId: externalGroupId, onGroupChange, onManageGroups }: NodeTableProps) {
  const [query, setQuery] = useState(externalQuery ?? "");
  const [status, setStatus] = useState<"all" | "online" | "offline">("all");
  const [groupFilter, setGroupFilter] = useState(externalGroupId ?? "all");
  const [newGroupName, setNewGroupName] = useState("");
  const [creatingGroup, setCreatingGroup] = useState(false);
  const [groupError, setGroupError] = useState<string | null>(null);
  const [viewMode, setViewMode] = useState<NodeViewMode>(() => {
    try {
      const stored = window.localStorage.getItem("pinglake.node-view");
      return stored === "list" ? "list" : "cards";
    } catch {
      return "cards";
    }
  });

  useEffect(() => {
    try {
      window.localStorage.setItem("pinglake.node-view", viewMode);
    } catch {
      // Storage may be unavailable in privacy-restricted browser contexts.
    }
  }, [viewMode]);

  useEffect(() => { if (externalQuery !== undefined) setQuery(externalQuery); }, [externalQuery]);
  useEffect(() => { if (externalGroupId !== undefined) setGroupFilter(externalGroupId); }, [externalGroupId]);

  const filtered = useMemo(() => {
    const normalized = query.trim().toLowerCase();
    return [...nodes]
      .filter((node) => status === "all" || (status === "online" ? node.online : !node.online))
      .filter((node) => groupFilter === "all" || (groupFilter === "ungrouped" ? !node.group_id : node.group_id === groupFilter))
      .filter((node) => !normalized || [node.display_name, node.hostname, node.os, node.os_version, node.group_name ?? ""]
        .some((value) => value.toLowerCase().includes(normalized)))
      .sort((left, right) => Number(right.online) - Number(left.online)
        || (left.group_name ?? "").localeCompare(right.group_name ?? "")
        || left.display_name.localeCompare(right.display_name));
  }, [groupFilter, nodes, query, status]);

  const createGroup = async () => {
    const name = newGroupName.trim();
    if (!name || creatingGroup) return;
    setCreatingGroup(true);
    setGroupError(null);
    try {
      await onCreateGroup(name);
      setNewGroupName("");
    } catch (reason) {
      setGroupError(reason instanceof Error ? reason.message : "无法创建分组");
    } finally {
      setCreatingGroup(false);
    }
  };

  const assignGroup = async (nodeId: string, groupId: string | null) => {
    setGroupError(null);
    try {
      await onAssignGroup(nodeId, groupId);
    } catch (reason) {
      setGroupError(reason instanceof Error ? reason.message : "无法更新分组");
    }
  };

  if (nodes.length === 0) return <EmptyNodes />;

  return (
    <section className="data-panel hosts-panel" aria-labelledby="hosts-heading">
      <div className="panel-toolbar hosts-toolbar">
        <div><h2 id="hosts-heading">受监主机</h2><span>{filtered.length} / {nodes.length}</span></div>
        <div className="table-controls">
          <div className="segmented-control" aria-label="主机状态">
            {(["all", "online", "offline"] as const).map((value) => (
              <button type="button" className={status === value ? "active" : ""} aria-pressed={status === value} onClick={() => setStatus(value)} key={value}>
                {value === "all" ? "全部" : value === "online" ? "在线" : "离线"}
              </button>
            ))}
          </div>
          <label className="group-filter"><span>分组</span><select value={groupFilter} onChange={(event) => { setGroupFilter(event.target.value); onGroupChange?.(event.target.value); }}><option value="all">全部分组</option><option value="ungrouped">未分组</option>{groups.map((group) => <option value={group.id} key={group.id}>{group.name}</option>)}</select></label>
          {onManageGroups && <button type="button" className="secondary-button group-manage-button" onClick={onManageGroups}>管理分组</button>}
          <label className="search-field"><AppIcon name="search" size={15} /><input value={query} onChange={(event) => { setQuery(event.target.value); onQueryChange?.(event.target.value); }} placeholder="搜索主机" aria-label="搜索主机" /></label>
          <div className="view-toggle" aria-label="节点视图">
            <button type="button" className={viewMode === "cards" ? "active" : ""} aria-label="卡片视图" aria-pressed={viewMode === "cards"} onClick={() => setViewMode("cards")} title="卡片视图"><AppIcon name="grid" size={15} /><span className="sr-only">卡片视图</span></button>
            <button type="button" className={viewMode === "list" ? "active" : ""} aria-label="表格视图" aria-pressed={viewMode === "list"} onClick={() => setViewMode("list")} title="列表视图"><AppIcon name="list" size={15} /><span className="sr-only">列表视图</span></button>
          </div>
        </div>
      </div>
      <div className="group-create-row">
        <label><span>新建分组</span><input value={newGroupName} maxLength={64} onChange={(event) => setNewGroupName(event.target.value)} placeholder="例如：邮件、数据库、测试" onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); void createGroup(); } }} /></label>
        <button type="button" className="secondary-button" onClick={() => void createGroup()} disabled={!newGroupName.trim() || creatingGroup}><AppIcon name="plus" size={15} />{creatingGroup ? "创建中" : "创建分组"}</button>
        {groupError && <span className="group-error" role="alert">{groupError}</span>}
      </div>
      {filtered.length === 0 ? <div className="filtered-empty">没有匹配的主机</div> : (
        viewMode === "cards" ? (
          <div className="server-card-grid" role="list" aria-label="主机卡片">
            {filtered.map((node) => <HostCard key={node.id} node={node} groups={groups} browserLatency={browserLatency[node.id]} onSelect={onSelect} onAssignGroup={assignGroup} />)}
          </div>
        ) : (
          <div className="server-list" role="list" aria-label="主机列表">
            {filtered.map((node) => <HostListRow key={node.id} node={node} groups={groups} browserLatency={browserLatency[node.id]} onSelect={onSelect} onAssignGroup={assignGroup} />)}
          </div>
        )
      )}
    </section>
  );
}

function HostCard({ node, groups, browserLatency, onSelect, onAssignGroup }: {
  node: NodeSnapshot;
  groups: HostGroup[];
  browserLatency?: BrowserLatency;
  onSelect: (node: NodeSnapshot) => void;
  onAssignGroup: (nodeId: string, groupId: string | null) => Promise<void>;
}) {
  const latest = node.latest;
  const systemIcon = node.os.toLowerCase().includes("windows") ? <AppIcon name="monitor" size={16} /> : <AppIcon name="server" size={16} />;
  return (
    <article className={`server-card ${node.online ? "online" : "offline"}`} role="listitem">
      <button type="button" className="server-card-main" onClick={() => onSelect(node)} aria-label={`打开 ${node.display_name || node.hostname} 的详情`}>
        <header>
          <div className="node-identity"><span className={`status-dot ${node.online ? "online" : "offline"}`} /><div><strong>{node.display_name || node.hostname}</strong><span>{node.hostname}</span></div></div>
          <AppIcon name="chevron-right" size={17} />
        </header>
        <div className="server-card-context"><span>{systemIcon}{formatOperatingSystem(node.os, node.os_version || node.architecture)}</span>{node.group_name && <b>{node.group_name}</b>}</div>
        <div className="server-card-metrics">
          <MetricBar value={latest?.cpu_percent} compact label="CPU" />
          <MetricBar value={latest ? nodeMemoryPercent(node) : null} compact label="内存" />
          <MetricBar value={latest ? nodeDiskPercent(node) : null} compact label="磁盘" />
          <div className="card-network"><span><AppIcon name="download" size={13} /><b>下行</b>{formatRate(latest?.network_received_bytes_per_sec)}</span><span><AppIcon name="upload" size={13} /><b>上行</b>{formatRate(latest?.network_transmitted_bytes_per_sec)}</span></div>
          <div className="card-latency"><span>Hub 延迟</span><strong>{formatLatency(latest?.hub_latency_ms)}</strong><small>访问 {browserLatency?.status === "ok" ? `${browserLatency.milliseconds?.toFixed(0)} ms` : browserLatency?.status === "unconfigured" ? "未配置" : "--"}</small></div>
        </div>
      </button>
      <footer>
        <label>分组<select value={node.group_id ?? ""} onClick={(event) => event.stopPropagation()} onChange={(event) => void onAssignGroup(node.id, event.target.value || null)}><option value="">未分组</option>{groups.map((group) => <option value={group.id} key={group.id}>{group.name}</option>)}</select></label>
        <span title={formatDateTime(node.last_seen_at)}>{node.online ? "心跳 " : "最后心跳 "}{formatRelativeTime(node.last_seen_at)}</span>
      </footer>
    </article>
  );
}

function HostListRow({ node, groups, browserLatency, onSelect, onAssignGroup }: {
  node: NodeSnapshot;
  groups: HostGroup[];
  browserLatency?: BrowserLatency;
  onSelect: (node: NodeSnapshot) => void;
  onAssignGroup: (nodeId: string, groupId: string | null) => Promise<void>;
}) {
  const latest = node.latest;
  const systemIcon = node.os.toLowerCase().includes("windows") ? <AppIcon name="monitor" size={15} /> : <AppIcon name="server" size={15} />;
  const nodeName = node.display_name || node.hostname;
  return (
    <article className={`server-list-row ${node.online ? "online" : "offline"}`} role="listitem">
      <button type="button" className="server-list-main" onClick={() => onSelect(node)} aria-label={`打开 ${nodeName} 的详情`}>
        <span className="server-list-identity">
          <span className={`status-dot ${node.online ? "online" : "offline"}`} aria-hidden="true" />
          <span><strong>{nodeName}</strong><small>{node.hostname}</small></span>
        </span>
        <span className="server-list-os">{systemIcon}{formatOperatingSystem(node.os, node.os_version || node.architecture)}</span>
        <span className="server-list-status">{node.online ? "在线" : "离线"}</span>
        <span className="server-list-metric"><MetricBar value={latest?.cpu_percent} compact label="CPU" /></span>
        <span className="server-list-metric"><MetricBar value={latest ? nodeMemoryPercent(node) : null} compact label="内存" /></span>
        <span className="server-list-metric"><MetricBar value={latest ? nodeDiskPercent(node) : null} compact label="磁盘" /></span>
        <span className="server-list-network"><AppIcon name="download" size={13} />{formatRate(latest?.network_received_bytes_per_sec)}<AppIcon name="upload" size={13} />{formatRate(latest?.network_transmitted_bytes_per_sec)}<small>{browserLatency?.status === "ok" ? `${browserLatency.milliseconds?.toFixed(0)} ms` : "--"}</small></span>
        <AppIcon name="chevron-right" size={17} className="row-chevron" />
      </button>
      <footer className="server-list-footer">
        {node.group_name && <span className="node-group-chip">{node.group_name}</span>}
        <label><span className="sr-only">{nodeName} 分组</span><select aria-label={`为 ${nodeName} 分配分组`} value={node.group_id ?? ""} onClick={(event) => event.stopPropagation()} onChange={(event) => void onAssignGroup(node.id, event.target.value || null)}><option value="">未分组</option>{groups.map((group) => <option value={group.id} key={group.id}>{group.name}</option>)}</select></label>
        <span title={formatDateTime(node.last_seen_at)}>{node.online ? "心跳 " : "最后心跳 "}{formatRelativeTime(node.last_seen_at)}</span>
      </footer>
    </article>
  );
}
