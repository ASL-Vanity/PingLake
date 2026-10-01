import { ArrowDown, ArrowUp, ChevronRight, LayoutGrid, List, Monitor, Search, Server } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import type { HostGroup, NodeSnapshot } from "../types";
import { browserLatencyLabel, type BrowserLatency } from "../hooks/useBrowserLatency";
import { agentQuality, formatDateTime, formatOperatingSystem, formatRate, formatRelativeTime, nodeDiskPercent, nodeFreshness, nodeMemoryPercent } from "../utils";
import { EmptyNodes } from "./EmptyNodes";
import styles from "./NodeTable.module.css";
import { filterAndSortNodes, type NodeSortDirection, type NodeSortField, type NodeStatusFilter } from "./NodeTable.helpers";

type ViewMode = "cards" | "table";
interface NodeTableProps {
  nodes: NodeSnapshot[]; groups: HostGroup[]; onSelect: (node: NodeSnapshot) => void;
  onAssignGroup: (nodeId: string, groupId: string | null) => Promise<void>;
  browserLatency?: Record<string, BrowserLatency>; nodeLatencies?: Record<string, BrowserLatency>;
  query?: string; onQueryChange?: (query: string) => void; groupId?: string; onGroupChange?: (groupId: string) => void;
  onManageGroups?: () => void; onCreateGroup?: (name: string) => Promise<HostGroup>; onDeleteGroup?: (id: string) => Promise<void>;
}
const storageKey = "pinglake.hosts.view";
const sortOptions: [NodeSortField, string][] = [["status", "状态"], ["name", "名称"], ["cpu", "CPU"], ["memory", "内存"], ["disk", "磁盘"], ["latency", "访问延迟"]];
function storedValue<T extends string>(key: string, values: readonly T[], fallback: T): T { try { const value = window.localStorage.getItem(`${storageKey}.${key}`); return values.includes(value as T) ? value as T : fallback; } catch { return fallback; } }
function storedGroup() { try { return window.localStorage.getItem(`${storageKey}.group`) ?? "all"; } catch { return "all"; } }

export function NodeTable({ nodes, groups, onSelect, onAssignGroup, browserLatency, nodeLatencies, query, onQueryChange, groupId: suppliedGroup, onGroupChange, onManageGroups }: NodeTableProps) {
  const [localQuery, setLocalQuery] = useState("");
  const [status, setStatus] = useState<NodeStatusFilter>(() => storedValue("status", ["all", "online", "offline"], "all"));
  const [localGroup, setLocalGroup] = useState(storedGroup);
  const [view, setView] = useState<ViewMode>(() => storedValue("mode", ["cards", "table"], "cards"));
  const [sort, setSort] = useState<NodeSortField>(() => storedValue("sort", sortOptions.map(([key]) => key), "status"));
  const [direction, setDirection] = useState<NodeSortDirection>(() => storedValue("direction", ["asc", "desc"], "desc"));
  const [assignmentErrors, setAssignmentErrors] = useState<Record<string, string>>({});
  const [assigning, setAssigning] = useState<Record<string, boolean>>({});
  const activeAssignments = useRef(new Set<string>());
  const activeQuery = query ?? localQuery; const groupId = suppliedGroup ?? localGroup; const latencies = nodeLatencies ?? browserLatency ?? {};
  useEffect(() => { try { for (const [key, value] of Object.entries({ mode: view, sort, direction, status, group: groupId })) window.localStorage.setItem(`${storageKey}.${key}`, value); } catch { /* optional persistence */ } }, [direction, groupId, sort, status, view]);
  useEffect(() => { if (groupId !== "all" && groupId !== "ungrouped" && !groups.some((group) => group.id === groupId)) { setLocalGroup("all"); onGroupChange?.("all"); } }, [groupId, groups, onGroupChange]);
  const filtered = useMemo(() => filterAndSortNodes(nodes, { query: activeQuery, status, groupId, sort, direction, latencies }), [activeQuery, direction, groupId, latencies, nodes, sort, status]);
  const setQuery = (value: string) => { setLocalQuery(value); onQueryChange?.(value); };
  const changeSort = (value: NodeSortField) => { setSort(value); setDirection(value === "name" ? "asc" : "desc"); };
  const assignGroup = async (nodeId: string, value: string) => { if (activeAssignments.current.has(nodeId)) return; activeAssignments.current.add(nodeId); setAssigning((current) => ({ ...current, [nodeId]: true })); setAssignmentErrors((current) => { const next = { ...current }; delete next[nodeId]; return next; }); try { await onAssignGroup(nodeId, value || null); } catch (reason) { setAssignmentErrors((current) => ({ ...current, [nodeId]: reason instanceof Error ? reason.message : "分组更新失败" })); } finally { activeAssignments.current.delete(nodeId); setAssigning((current) => ({ ...current, [nodeId]: false })); } };
  return <section className={styles.hosts} aria-labelledby="hosts-heading">
    <div className={styles.toolbar}><div className={styles.title}><h2 id="hosts-heading">受监主机</h2><span className={styles.count}>{filtered.length} / {nodes.length}</span></div><div className={styles.filters}>
      {query === undefined && <label className={styles.search}><Search size={15} aria-hidden="true" /><input value={activeQuery} onChange={(event) => setQuery(event.target.value)} placeholder="搜索主机或分组" aria-label="搜索主机或分组" /></label>}
      <label className={styles.field}><span>状态</span><select className={styles.select} value={status} onChange={(event) => setStatus(event.target.value as NodeStatusFilter)} aria-label="主机状态"><option value="all">全部</option><option value="online">在线</option><option value="offline">离线</option></select></label>
      <label className={styles.field}><span>排序</span><select className={styles.select} value={sort} onChange={(event) => changeSort(event.target.value as NodeSortField)} aria-label="主机排序">{sortOptions.map(([key, label]) => <option value={key} key={key}>{label}</option>)}</select></label>
      <button type="button" className={styles.direction} onClick={() => setDirection((value) => value === "asc" ? "desc" : "asc")} title={`${sortOptions.find(([key]) => key === sort)?.[1]}${direction === "asc" ? "升序" : "降序"}`} aria-label="切换排序方向">{direction === "asc" ? <ArrowUp size={15} /> : <ArrowDown size={15} />}</button>
      <div className={styles.modes} role="group" aria-label="主机显示方式"><button type="button" aria-label="卡片视图" title="卡片视图" aria-pressed={view === "cards"} onClick={() => setView("cards")}><LayoutGrid size={15} /></button><button type="button" aria-label="表格视图" title="表格视图" aria-pressed={view === "table"} onClick={() => setView("table")}><List size={15} /></button></div>
    </div></div>
    <div className={styles.filterBar}><label className={styles.field}><span>分组</span><select className={styles.select} value={groupId} onChange={(event) => { setLocalGroup(event.target.value); onGroupChange?.(event.target.value); }} aria-label="主机分组"><option value="all">全部分组</option><option value="ungrouped">未分组</option>{groups.map((group) => <option value={group.id} key={group.id}>{group.name}</option>)}</select></label>{onManageGroups && <button type="button" className={`secondary-button ${styles.manage}`} onClick={onManageGroups}>管理分组</button>}</div>
    {nodes.length === 0 ? <EmptyNodes /> : filtered.length === 0 ? <div className={styles.empty}><strong>没有匹配的主机</strong><span>调整搜索、状态或分组筛选</span></div> : view === "cards" ? <div className={styles.grid}>{filtered.map((node) => <HostCard key={node.id} node={node} groups={groups} latency={latencies[node.id]} assignmentError={assignmentErrors[node.id]} assigning={Boolean(assigning[node.id])} onSelect={onSelect} onAssignGroup={assignGroup} />)}</div> : <NodeDataTable nodes={filtered} groups={groups} latencies={latencies} assignmentErrors={assignmentErrors} assigning={assigning} onSelect={onSelect} onAssignGroup={assignGroup} />}
  </section>;
}

function osIcon(node: NodeSnapshot) { return node.os.toLowerCase().includes("windows") ? <Monitor size={17} aria-hidden="true" /> : <Server size={17} aria-hidden="true" />; }
function severity(value: number) { return value >= 90 ? styles.critical : value >= 70 ? styles.warning : ""; }
function Resource({ label, value }: { label: string; value: number | null }) { return <div className={styles.resource}><dt>{label}</dt><dd>{value == null ? "--" : `${value.toFixed(value < 10 ? 1 : 0)}%`}{value != null && <span className={`${styles.meter} ${severity(value)}`}><span style={{ width: `${Math.min(100, Math.max(0, value))}%` }} /></span>}</dd></div>; }
function VisitorLatency({ latency }: { latency?: BrowserLatency }) { const label = browserLatencyLabel(latency); const unavailable = !latency || latency.status !== "ok"; return <div className={styles.visitor}><span className={styles.visitorLabel}>访问延迟</span><strong className={`${styles.visitorValue} ${unavailable ? styles.unavailable : ""}`} title="当前浏览器直接访问此主机 HTTPS 测点的 HTTP 往返耗时">{label}</strong></div>; }

function DataQuality({ node }: { node: NodeSnapshot }) {
  const freshness = nodeFreshness(node);
  const quality = agentQuality(node.latest?.monitoring?.agent);
  return <div className={styles.dataQuality} aria-label="数据质量"><span className={styles[`freshness-${freshness.state}`]}>数据新鲜度：{freshness.label}</span><span className={styles[`quality-${quality.state}`]}>Agent 质量：{quality.label}</span></div>;
}

function HostCard({ node, groups, latency, assignmentError, assigning, onSelect, onAssignGroup }: { node: NodeSnapshot; groups: HostGroup[]; latency?: BrowserLatency; assignmentError?: string; assigning: boolean; onSelect: (node: NodeSnapshot) => void; onAssignGroup: (nodeId: string, value: string) => Promise<void> }) {
  const latest = node.latest; const memory = latest && latest.memory_total_bytes > 0 ? nodeMemoryPercent(node) : null; const disk = latest && latest.disk_total_bytes > 0 ? nodeDiskPercent(node) : null;
  return <article className={`${styles.host} ${!node.online ? styles.offline : ""}`}>
    <button type="button" className={styles.hostHeading} onClick={() => onSelect(node)} aria-label={`打开 ${node.display_name || node.hostname} 的详情`}><span className={styles.osIcon}>{osIcon(node)}</span><span className={styles.identity}><span className={styles.nameRow}><strong title={node.display_name || node.hostname}>{node.display_name || node.hostname}</strong><span className={styles.state}><i className={styles.stateDot} />{node.online ? "在线" : "离线"}</span></span><span className={styles.hostname} title={node.hostname}>{node.hostname}</span></span><ChevronRight size={17} aria-hidden="true" /></button>
    <div className={styles.osName}><span>{formatOperatingSystem(node.os, node.os_version || node.architecture)}</span><span className={styles.architecture}>{node.architecture || "未知架构"}</span>{node.group_name && <span className={styles.groupBadge} title={node.group_name}>{node.group_name}</span>}</div>
    <dl className={styles.resources}><Resource label="CPU" value={latest?.cpu_percent ?? null} /><Resource label="内存" value={memory} /><Resource label="磁盘" value={disk} /></dl>
    <dl className={styles.network}><div><dt><ArrowDown size={14} />下行</dt><dd>{formatRate(latest?.network_received_bytes_per_sec)}</dd></div><div><dt><ArrowUp size={14} />上行</dt><dd>{formatRate(latest?.network_transmitted_bytes_per_sec)}</dd></div></dl>
    <VisitorLatency latency={latency} />
    <DataQuality node={node} />
    <footer className={styles.footer}><label className={styles.assignment}><span>分组</span><select disabled={assigning} value={node.group_id ?? ""} aria-label={`为 ${node.display_name || node.hostname} 分配分组`} onChange={(event) => void onAssignGroup(node.id, event.target.value)}><option value="">未分组</option>{groups.map((group) => <option value={group.id} key={group.id}>{group.name}</option>)}</select></label><div className={styles.heartbeat}><span>{node.online ? "心跳" : "最后心跳"}</span><span title={formatDateTime(node.last_seen_at)}>{formatRelativeTime(node.last_seen_at)}</span></div>{!node.online && <p className={styles.lastSample}>最后采样资源 · 资源数值为最后采样</p>}{assignmentError && <p className={styles.error} role="alert">{assignmentError}</p>}</footer>
  </article>;
}

function NodeDataTable({ nodes, groups, latencies, assignmentErrors, assigning, onSelect, onAssignGroup }: { nodes: NodeSnapshot[]; groups: HostGroup[]; latencies: Record<string, BrowserLatency>; assignmentErrors: Record<string, string>; assigning: Record<string, boolean>; onSelect: (node: NodeSnapshot) => void; onAssignGroup: (nodeId: string, value: string) => Promise<void> }) {
  return <div className={styles.scroll}><table className={styles.table}><thead><tr><th>主机</th><th>状态</th><th>CPU</th><th>内存</th><th>磁盘</th><th>下行</th><th>上行</th><th>访问延迟</th><th>数据新鲜度</th><th>Agent 质量</th><th>分组</th><th>心跳</th></tr></thead><tbody>{nodes.map((node) => { const latest = node.latest; const memory = latest && latest.memory_total_bytes > 0 ? nodeMemoryPercent(node) : null; const disk = latest && latest.disk_total_bytes > 0 ? nodeDiskPercent(node) : null; const freshness = nodeFreshness(node); const quality = agentQuality(latest?.monitoring?.agent); return <tr key={node.id} className={!node.online ? styles.offline : ""}><td className={styles.tableIdentity}><button type="button" onClick={() => onSelect(node)}><span className={styles.osIcon}>{osIcon(node)}</span><span><strong>{node.display_name || node.hostname}</strong><small>{node.hostname}</small></span></button>{!node.online && <small className={styles.lastSample}>最后采样资源</small>}</td><td><span className={styles.tableStatus}><i className={styles.stateDot} />{node.online ? "在线" : "离线"}</span></td><td className={styles.number}>{latest ? `${latest.cpu_percent.toFixed(1)}%` : "--"}</td><td className={styles.number}>{memory == null ? "--" : `${memory.toFixed(1)}%`}</td><td className={styles.number}>{disk == null ? "--" : `${disk.toFixed(1)}%`}</td><td className={styles.number}>{formatRate(latest?.network_received_bytes_per_sec)}</td><td className={styles.number}>{formatRate(latest?.network_transmitted_bytes_per_sec)}</td><td className={styles.tableLatency}>{browserLatencyLabel(latencies[node.id])}</td><td className={styles[`freshness-${freshness.state}`]}>{freshness.label}</td><td className={styles[`quality-${quality.state}`]}>{quality.label}</td><td><label className={styles.assignment}><span className="sr-only">分组</span><select disabled={Boolean(assigning[node.id])} value={node.group_id ?? ""} aria-label={`为 ${node.display_name || node.hostname} 分配分组`} onChange={(event) => void onAssignGroup(node.id, event.target.value)}><option value="">未分组</option>{groups.map((group) => <option value={group.id} key={group.id}>{group.name}</option>)}</select></label>{assignmentErrors[node.id] && <p className={styles.error} role="alert">{assignmentErrors[node.id]}</p>}</td><td className={styles.tableHeartbeat} title={formatDateTime(node.last_seen_at)}>{node.online ? "心跳 " : "最后 "}{formatRelativeTime(node.last_seen_at)}</td></tr>; })}</tbody></table></div>;
}
