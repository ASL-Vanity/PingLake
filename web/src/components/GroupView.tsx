import { useEffect, useMemo, useState, type FormEvent } from "react";
import { ChevronRight, Folder, Plus, Search, Trash2 } from "lucide-react";
import type { HostGroup, NodeSnapshot } from "../types";
import { ConfirmDialog } from "./ConfirmDialog";

interface Props {
  groups: HostGroup[];
  nodes: NodeSnapshot[];
  onCreate: (name: string) => Promise<HostGroup>;
  onDelete: (id: string) => Promise<void>;
  onAssign: (id: string, group: string | null) => Promise<void>;
  onOpenNode: (id: string) => void;
  onOpenHosts: (group: string) => void;
}

export function GroupView({ groups, nodes, onCreate, onDelete, onAssign, onOpenNode, onOpenHosts }: Props) {
  const [name, setName] = useState("");
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState("all");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<HostGroup | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  useEffect(() => {
    if (selected !== "all" && selected !== "ungrouped" && !groups.some((group) => group.id === selected)) setSelected("all");
  }, [groups, selected]);
  const members = useMemo(() => nodes.filter((node) => (selected === "all" || (selected === "ungrouped" ? !node.group_id : node.group_id === selected))
    && [node.display_name, node.hostname, node.group_name ?? ""].some((value) => value.toLowerCase().includes(query.trim().toLowerCase()))), [nodes, selected, query]);
  const create = async (event: FormEvent) => {
    event.preventDefault(); if (!name.trim() || busy) return;
    if (new TextEncoder().encode(name.trim()).length > 64) { setError("分组名称不能超过 64 字节"); return; }
    setBusy("create"); setError(null);
    try { const group = await onCreate(name.trim()); setName(""); setSelected(group.id); }
    catch (reason) { setError(reason instanceof Error ? reason.message : "创建分组失败"); }
    finally { setBusy(null); }
  };
  const remove = async () => {
    if (!deleting || busy) return; setBusy("delete"); setDeleteError(null);
    try { await onDelete(deleting.id); setDeleting(null); }
    catch (reason) { setDeleteError(reason instanceof Error ? reason.message : "删除分组失败"); }
    finally { setBusy(null); }
  };
  const assign = async (id: string, group: string) => {
    if (busy) return; setBusy(id); setError(null);
    try { await onAssign(id, group || null); }
    catch (reason) { setError(reason instanceof Error ? reason.message : "修改分组失败"); }
    finally { setBusy(null); }
  };
  return <section className="group-workspace">
    <form className="group-create" onSubmit={(event) => void create(event)}>
      <label htmlFor="new-group">新建分组</label>
      <input id="new-group" placeholder="分组名称" value={name} onChange={(event) => setName(event.target.value)} maxLength={64} />
      <button className="primary-button" type="submit" disabled={!!busy || !name.trim()}><Plus size={16} />{busy === "create" ? "正在创建" : "创建"}</button>
    </form>
    {error && <p className="form-error" role="alert">{error}</p>}
    <div className="group-layout">
      <div className="group-directory" aria-label="分组目录">
        <button className={selected === "all" ? "active" : ""} onClick={() => setSelected("all")}><Folder size={16} /><span>全部主机</span><strong>{nodes.length}</strong></button>
        <button className={selected === "ungrouped" ? "active" : ""} onClick={() => setSelected("ungrouped")}><Folder size={16} /><span>未分组</span><strong>{nodes.filter((node) => !node.group_id).length}</strong></button>
        {groups.map((group) => <div className="group-directory-row" key={group.id}>
          <button className={selected === group.id ? "active" : ""} onClick={() => setSelected(group.id)}><Folder size={16} /><span>{group.name}</span><strong>{nodes.filter((node) => node.group_id === group.id).length}</strong></button>
          <button className="icon-button subtle" title={`删除分组 ${group.name}`} aria-label={`删除分组 ${group.name}`} disabled={!!busy} onClick={() => { setDeleting(group); setDeleteError(null); }}><Trash2 size={15} /></button>
        </div>)}
        {!groups.length && <p className="group-empty-note">暂无自定义分组</p>}
      </div>
      <div className="group-members">
        <div className="group-members-toolbar"><h2>{selected === "all" ? "全部主机" : selected === "ungrouped" ? "未分组" : groups.find((group) => group.id === selected)?.name}<small>{members.length} 台</small></h2>
          <label className="search-field"><Search size={15} /><input aria-label="搜索分组主机" placeholder="搜索主机" value={query} onChange={(event) => setQuery(event.target.value)} /></label>
          {selected !== "all" && <button className="secondary-button" onClick={() => onOpenHosts(selected)}>主机视图<ChevronRight size={14} /></button>}
        </div>
        {members.length ? members.map((node) => <div className="group-member-row" key={node.id}>
          <button onClick={() => onOpenNode(node.id)}><span className={`fleet-dot ${node.online ? "ok" : "bad"}`} /><span><strong>{node.display_name || node.hostname}</strong><small>{node.hostname}</small></span><ChevronRight size={14} /></button>
          <select aria-label={`${node.display_name || node.hostname} 的分组`} value={node.group_id ?? ""} disabled={!!busy} onChange={(event) => void assign(node.id, event.target.value)}><option value="">未分组</option>{groups.map((group) => <option key={group.id} value={group.id}>{group.name}</option>)}</select>
        </div>) : <div className="fleet-empty">没有匹配的主机</div>}
      </div>
    </div>
    {deleting && <ConfirmDialog title={`删除分组「${deleting.name}」？`} description={`该分组下的 ${nodes.filter((node) => node.group_id === deleting.id).length} 台主机将移入未分组，主机和历史数据会保留。`} busy={busy === "delete"} error={deleteError} onCancel={() => { if (!busy) setDeleting(null); }} onConfirm={() => void remove()} />}
  </section>;
}
