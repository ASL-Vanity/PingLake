import { ArrowDown, ArrowUp, ChevronRight, Cpu, MemoryStick, Monitor, Plus, Search, Server } from "lucide-react";
import { useMemo, useState } from "react";
import type { HostGroup, NodeSnapshot } from "../types";
import {
  formatDateTime,
  formatLatency,
  formatOperatingSystem,
  formatRate,
  formatRelativeTime,
  nodeDiskPercent,
  nodeMemoryPercent,
  osLabel,
} from "../utils";
import { EmptyNodes } from "./EmptyNodes";
import { MetricBar } from "./MetricBar";

interface NodeTableProps {
  nodes: NodeSnapshot[];
  groups: HostGroup[];
  onSelect: (node: NodeSnapshot) => void;
  onCreateGroup: (name: string) => Promise<HostGroup>;
  onAssignGroup: (nodeId: string, groupId: string | null) => Promise<void>;
}

export function NodeTable({ nodes, groups, onSelect, onCreateGroup, onAssignGroup }: NodeTableProps) {
  const [query, setQuery] = useState("");
  const [status, setStatus] = useState<"all" | "online" | "offline">("all");
  const [groupFilter, setGroupFilter] = useState("all");
  const [newGroupName, setNewGroupName] = useState("");
  const [creatingGroup, setCreatingGroup] = useState(false);
  const [groupError, setGroupError] = useState<string | null>(null);

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

  if (nodes.length === 0) return <EmptyNodes />;

  return (
    <section className="data-panel hosts-panel" aria-labelledby="hosts-heading">
      <div className="panel-toolbar hosts-toolbar">
        <div><h2 id="hosts-heading">受监主机</h2><span>{filtered.length} / {nodes.length}</span></div>
        <div className="table-controls">
          <div className="segmented-control" aria-label="主机状态筛选">
            {(["all", "online", "offline"] as const).map((value) => (
              <button type="button" className={status === value ? "active" : ""} onClick={() => setStatus(value)} key={value}>
                {value === "all" ? "全部" : value === "online" ? "在线" : "离线"}
              </button>
            ))}
          </div>
          <label className="group-filter"><span>分组</span><select value={groupFilter} onChange={(event) => setGroupFilter(event.target.value)}><option value="all">全部分组</option><option value="ungrouped">未分组</option>{groups.map((group) => <option value={group.id} key={group.id}>{group.name}</option>)}</select></label>
          <label className="search-field"><Search size={15} aria-hidden="true" /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索主机" aria-label="搜索主机" /></label>
        </div>
      </div>
      <div className="group-create-row">
        <label><span>新建分组</span><input value={newGroupName} maxLength={64} onChange={(event) => setNewGroupName(event.target.value)} placeholder="例如：邮件、数据库、测试" onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); void createGroup(); } }} /></label>
        <button type="button" className="secondary-button" onClick={() => void createGroup()} disabled={!newGroupName.trim() || creatingGroup}><Plus size={15} />{creatingGroup ? "创建中" : "创建分组"}</button>
        {groupError && <span className="group-error" role="alert">{groupError}</span>}
      </div>
      {filtered.length === 0 ? <div className="filtered-empty">没有匹配的主机</div> : (
        <div className="server-card-grid">
          {filtered.map((node) => <HostCard key={node.id} node={node} groups={groups} onSelect={onSelect} onAssignGroup={onAssignGroup} />)}
        </div>
      )}
    </section>
  );
}

function HostCard({ node, groups, onSelect, onAssignGroup }: {
  node: NodeSnapshot;
  groups: HostGroup[];
  onSelect: (node: NodeSnapshot) => void;
  onAssignGroup: (nodeId: string, groupId: string | null) => Promise<void>;
}) {
  const latest = node.latest;
  const systemIcon = node.os.toLowerCase().includes("windows") ? <Monitor size={16} /> : <Server size={16} />;
  return (
    <article className={`server-card ${node.online ? "online" : "offline"}`}>
      <button type="button" className="server-card-main" onClick={() => onSelect(node)} aria-label={`打开 ${node.display_name || node.hostname} 的详情`}>
        <header>
          <div className="node-identity"><span className={`status-dot ${node.online ? "online" : "offline"}`} /><div><strong>{node.display_name || node.hostname}</strong><span>{node.hostname}</span></div></div>
          <ChevronRight size={17} />
        </header>
        <div className="server-card-context"><span>{systemIcon}{formatOperatingSystem(node.os, node.os_version || node.architecture)}</span>{node.group_name && <b>{node.group_name}</b>}</div>
        <div className="server-card-metrics">
          <MetricBar value={latest?.cpu_percent ?? 0} compact label="CPU" />
          <MetricBar value={nodeMemoryPercent(node)} compact label="内存" />
          <MetricBar value={nodeDiskPercent(node)} compact label="磁盘" />
          <div className="card-network"><span><ArrowDown size={13} /><b>下行</b>{formatRate(latest?.network_received_bytes_per_sec)}</span><span><ArrowUp size={13} /><b>上行</b>{formatRate(latest?.network_transmitted_bytes_per_sec)}</span></div>
          <div className="card-latency"><span>Hub latency</span><strong>{formatLatency(latest?.hub_latency_ms)}</strong></div>
        </div>
      </button>
      <footer>
        <label>分组<select value={node.group_id ?? ""} onClick={(event) => event.stopPropagation()} onChange={(event) => void onAssignGroup(node.id, event.target.value || null)}><option value="">未分组</option>{groups.map((group) => <option value={group.id} key={group.id}>{group.name}</option>)}</select></label>
        <span title={formatDateTime(node.last_seen_at)}>{node.online ? "心跳 " : "最后心跳 "}{formatRelativeTime(node.last_seen_at)}</span>
      </footer>
    </article>
  );
}
