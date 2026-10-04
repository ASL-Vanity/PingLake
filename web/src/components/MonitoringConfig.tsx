import { useRef, useState } from "react";
import { Plus, RefreshCw, Save, Trash2 } from "lucide-react";
import { api, ApiError } from "../api";
import type { LocalPortCheck, NodeMonitoringConfig, ProbeTarget, ProcessCheck, ServiceCheck } from "../monitoringTypes";
import { ConfirmDialog } from "./ConfirmDialog";

function origin(url: string | null): string | null {
  try { return url ? new URL(url).origin : null; } catch { return null; }
}

export function MonitoringConfig({ nodeId, config, onSaved, onUnauthorized }: { nodeId: string; config: NodeMonitoringConfig; onSaved: (config: NodeMonitoringConfig) => void; onUnauthorized: () => void }) {
  const [draft, setDraft] = useState<NodeMonitoringConfig>(config);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [reloadRequired, setReloadRequired] = useState(false);
  const [remove, setRemove] = useState<{ kind: "services" | "probes" | "process_checks" | "local_port_checks"; id: string; name: string } | null>(null);
  const documentOrigin = useRef(origin(config.browser_latency_url));
  const change = (value: Partial<NodeMonitoringConfig>) => { setDraft((current) => ({ ...current, ...value })); setSaved(false); };
  const changeService = (id: string, value: Partial<ServiceCheck>) => change({ services: draft.services.map((item) => item.id === id ? { ...item, ...value } : item) });
  const changeProbe = (id: string, value: Partial<ProbeTarget>) => change({ probes: draft.probes.map((item) => item.id === id ? { ...item, ...value } : item) });
  const changeProcess = (id: string, value: Partial<ProcessCheck>) => change({ process_checks: (draft.process_checks ?? []).map((item) => item.id === id ? { ...item, ...value } : item) });
  const changePort = (id: string, value: Partial<LocalPortCheck>) => change({ local_port_checks: (draft.local_port_checks ?? []).map((item) => item.id === id ? { ...item, ...value } : item) });
  const save = async () => {
    if (saving) return;
    setError(null); setSaving(true); setSaved(false);
    try {
      const url = draft.browser_latency_url?.trim() || null;
      if (url) {
        const parsed = new URL(url);
        if (parsed.protocol !== "https:" || parsed.username || parsed.password || parsed.hash) throw new Error("测点必须使用无凭证、无片段的 HTTPS URL");
      }
      const value = await api.saveMonitoringConfig(nodeId, { ...draft, browser_latency_url: url, services: draft.services.map((item) => ({ ...item, name: item.name.trim() })), probes: draft.probes.map((item) => ({ ...item, name: item.name.trim(), target: item.target.trim(), port: item.kind === "tcp" ? item.port : null, expected_status: item.kind === "http" ? item.expected_status : null, response_contains: item.kind === "http" ? item.response_contains?.trim() || null : null, dns: item.kind === "dns" ? item.dns : null })), process_checks: (draft.process_checks ?? []).map((item) => ({ ...item, name: item.name.trim(), process_name: item.process_name.trim() })), local_port_checks: (draft.local_port_checks ?? []).map((item) => ({ ...item, name: item.name.trim() })) });
      setDraft(value); setSaved(true); onSaved(value);
      if (value.browser_latency_url && origin(value.browser_latency_url) !== documentOrigin.current) setReloadRequired(true);
    } catch (reason) {
      if (reason instanceof ApiError && reason.status === 401) onUnauthorized();
      else setError(reason instanceof Error ? reason.message : "无法保存配置");
    } finally { setSaving(false); }
  };
  return <form className="monitoring-config" onSubmit={(event) => { event.preventDefault(); void save(); }}>
    {draft.revision !== config.revision && <div className="monitoring-reload" role="status"><span>配置已更新为版本 {config.revision}，当前编辑版本为 {draft.revision}。</span><button type="button" className="secondary-button" disabled={saving} onClick={() => { setDraft(config); setError(null); setSaved(false); }}><RefreshCw size={15} />载入最新配置</button></div>}
    <fieldset disabled={saving}>
      <legend>浏览器访问延迟</legend>
      <p className="monitoring-help" id="browser-latency-description">从当前打开控制台的浏览器直接请求受监主机的 HTTPS 测点，展示完整请求的往返耗时。每位访问者的结果只在自己的浏览器中测量。</p>
      <label>节点 HTTPS 测点 URL<input type="url" aria-describedby="browser-latency-description" value={draft.browser_latency_url ?? ""} placeholder="https://node.example.com/pinglake/latency" onChange={(event) => change({ browser_latency_url: event.target.value || null })} /></label>
      <details className="browser-latency-help">
        <summary>如何启用浏览器访问延迟</summary>
        <p>在受监主机上启用 Agent 的延迟端点，并用 HTTPS 反向代理开放 <code>/pinglake/latency</code>。地址必须由当前浏览器直接访问，不能填写 Hub 的地址。</p>
        <pre><code>{`latency_bind = "127.0.0.1:18091"\ndashboard_origin = "${window.location.origin}"`}</code></pre>
        <p>测点应返回成功响应，允许控制台来源 <code>{window.location.origin}</code> 的 CORS 请求，并保留 <code>Cache-Control: no-store</code>。请关闭该路径的 CDN 缓存。测点地址首次保存或切换到新的来源后，刷新控制台以更新安全策略。</p>
        <p>未配置、节点离线或访问受限会显示对应状态；浏览器测量使用 HTTPS 请求，无法直接执行 ICMP ping。</p>
      </details>
    </fieldset>
    <fieldset disabled={saving}>
      <legend>指定服务</legend>
      {draft.services.length === 0 && <p className="monitoring-empty">尚未配置服务</p>}
      {draft.services.map((service) => <div className="monitoring-config-row service-config-row" key={service.id}>
        <label className="check-label"><input type="checkbox" checked={service.enabled} onChange={(event) => changeService(service.id, { enabled: event.target.checked })} />启用</label>
        <label>服务名称<input required maxLength={128} value={service.name} placeholder="nginx.service / Spooler" onChange={(event) => changeService(service.id, { name: event.target.value })} /></label>
        <label>预期状态<select value={service.expected_state} onChange={(event) => changeService(service.id, { expected_state: event.target.value })}><option value="running">运行</option><option value="stopped">停止</option></select></label>
        <button type="button" className="icon-button" title={`删除服务 ${service.name || "配置"}`} aria-label={`删除服务 ${service.name || "配置"}`} onClick={() => setRemove({ kind: "services", id: service.id, name: service.name || "服务" })}><Trash2 size={16} /></button>
      </div>)}
      <button type="button" className="secondary-button" onClick={() => change({ services: [...draft.services, { id: crypto.randomUUID(), name: "", expected_state: "running", enabled: true }] })}><Plus size={15} />添加服务</button>
    </fieldset>
    <fieldset disabled={saving}>
      <legend>主动探测目标</legend>
      {draft.probes.length === 0 && <p className="monitoring-empty">尚未配置探测目标</p>}
      {draft.probes.map((probe) => <div className="monitoring-config-row probe-config-row" key={probe.id}>
        <div className="probe-config-heading"><label className="check-label"><input type="checkbox" checked={probe.enabled} onChange={(event) => changeProbe(probe.id, { enabled: event.target.checked })} />启用</label><button type="button" className="icon-button" title={`删除探测 ${probe.name || "配置"}`} aria-label={`删除探测 ${probe.name || "配置"}`} onClick={() => setRemove({ kind: "probes", id: probe.id, name: probe.name || "探测目标" })}><Trash2 size={16} /></button></div>
        <label>名称<input required maxLength={64} value={probe.name} onChange={(event) => changeProbe(probe.id, { name: event.target.value })} /></label>
        <label>类型<select value={probe.kind} onChange={(event) => changeProbe(probe.id, { kind: event.target.value as ProbeTarget["kind"] })}><option value="icmp">ICMP</option><option value="tcp">TCP</option><option value="http">HTTP / HTTPS</option><option value="dns">DNS</option></select></label>
        <label className="probe-target-field">{probe.kind === "http" ? "URL" : probe.kind === "dns" ? "主机名" : "主机 / IP"}<input required type={probe.kind === "http" ? "url" : "text"} value={probe.target} onChange={(event) => changeProbe(probe.id, { target: event.target.value })} /></label>
        {probe.kind === "tcp" && <label>端口<input required type="number" min={1} max={65535} value={probe.port ?? ""} onChange={(event) => changeProbe(probe.id, { port: event.target.value === "" ? null : Number(event.target.value) })} /></label>}
        {probe.kind === "dns" && <><label>记录类型<select value={probe.dns?.record_type ?? "A"} onChange={(event) => changeProbe(probe.id, { dns: { record_type: event.target.value as "A" | "AAAA", expected_value: probe.dns?.expected_value ?? null } })}><option value="A">A</option><option value="AAAA">AAAA</option></select></label><label>预期值<input maxLength={253} value={probe.dns?.expected_value ?? ""} placeholder="可选" onChange={(event) => changeProbe(probe.id, { dns: { record_type: probe.dns?.record_type ?? "A", expected_value: event.target.value || null } })} /></label></>}
        <label>间隔（秒）<input required type="number" min={10} max={86400} value={probe.interval_secs} onChange={(event) => changeProbe(probe.id, { interval_secs: Number(event.target.value) })} /></label>
        <label>超时（毫秒）<input required type="number" min={1} max={Math.min(10000, probe.interval_secs * 1000 - 1)} value={probe.timeout_ms} onChange={(event) => changeProbe(probe.id, { timeout_ms: Number(event.target.value) })} /></label>
        {probe.kind === "http" && <><label>预期状态码<input type="number" min={100} max={599} value={probe.expected_status ?? ""} placeholder="默认 2xx" onChange={(event) => changeProbe(probe.id, { expected_status: event.target.value === "" ? null : Number(event.target.value) })} /></label><label>响应包含<input maxLength={1024} value={probe.response_contains ?? ""} onChange={(event) => changeProbe(probe.id, { response_contains: event.target.value || null })} /></label></>}
      </div>)}
      <button type="button" className="secondary-button" onClick={() => change({ probes: [...draft.probes, { id: crypto.randomUUID(), name: "", kind: "http", target: "", port: null, enabled: true, interval_secs: 30, timeout_ms: 5000, expected_status: null, response_contains: null, dns: null }] })}><Plus size={15} />添加探测</button>
    </fieldset>
    <fieldset disabled={saving}>
      <legend>进程检查</legend>
      {(draft.process_checks ?? []).map((check) => <div className="monitoring-config-row service-config-row" key={check.id}>
        <label className="check-label"><input type="checkbox" checked={check.enabled} onChange={(event) => changeProcess(check.id, { enabled: event.target.checked })} />启用</label>
        <label>名称<input required maxLength={128} value={check.name} onChange={(event) => changeProcess(check.id, { name: event.target.value })} /></label>
        <label>进程名<input required maxLength={128} value={check.process_name} onChange={(event) => changeProcess(check.id, { process_name: event.target.value })} /></label>
        <label>预期实例数<input type="number" min={0} max={100000} value={check.expected_count ?? ""} placeholder="按运行/停止" onChange={(event) => changeProcess(check.id, { expected_count: event.target.value === "" ? null : Number(event.target.value) })} /></label>
        <label>预期<select value={check.expected_state} onChange={(event) => changeProcess(check.id, { expected_state: event.target.value })}><option value="running">运行</option><option value="stopped">停止</option></select></label>
        <label>间隔（秒）<input required type="number" min={10} max={86400} value={check.interval_secs} onChange={(event) => changeProcess(check.id, { interval_secs: Number(event.target.value) })} /></label>
        <label>超时（毫秒）<input required type="number" min={1} max={Math.min(10000, check.interval_secs * 1000 - 1)} value={check.timeout_ms} onChange={(event) => changeProcess(check.id, { timeout_ms: Number(event.target.value) })} /></label>
        <button type="button" className="icon-button" title={`删除进程检查 ${check.name || "配置"}`} aria-label={`删除进程检查 ${check.name || "配置"}`} onClick={() => setRemove({ kind: "process_checks", id: check.id, name: check.name || "进程检查" })}><Trash2 size={16} /></button>
      </div>)}
      <button type="button" className="secondary-button" onClick={() => change({ process_checks: [...(draft.process_checks ?? []), { id: crypto.randomUUID(), name: "", process_name: "", expected_count: null, enabled: true, expected_state: "running", interval_secs: 30, timeout_ms: 5000 }] })}><Plus size={15} />添加进程检查</button>
    </fieldset>
    <fieldset disabled={saving}>
      <legend>本机端口检查</legend>
      {(draft.local_port_checks ?? []).map((check) => <div className="monitoring-config-row service-config-row" key={check.id}>
        <label className="check-label"><input type="checkbox" checked={check.enabled} onChange={(event) => changePort(check.id, { enabled: event.target.checked })} />启用</label>
        <label>名称<input required maxLength={128} value={check.name} onChange={(event) => changePort(check.id, { name: event.target.value })} /></label>
        <label>协议<select value={check.protocol ?? "tcp"} onChange={(event) => changePort(check.id, { protocol: event.target.value as LocalPortCheck["protocol"] })}><option value="tcp">TCP</option><option value="udp">UDP</option></select></label>
        <label>地址族<select value={check.address_family ?? "any"} onChange={(event) => changePort(check.id, { address_family: event.target.value as LocalPortCheck["address_family"] })}><option value="any">任意</option><option value="ipv4">IPv4</option><option value="ipv6">IPv6</option></select></label>
        <label>范围<select value={check.address_scope?.scope ?? "any_local"} onChange={(event) => changePort(check.id, { address_scope: event.target.value === "exact" ? { scope: "exact", address: "" } : { scope: event.target.value as "any_local" | "loopback" } })}><option value="any_local">本机地址</option><option value="loopback">回环</option><option value="exact">指定地址</option></select></label>
        {check.address_scope?.scope === "exact" && <label>指定地址<input maxLength={253} value={check.address_scope.address} onChange={(event) => changePort(check.id, { address_scope: { scope: "exact", address: event.target.value } })} /></label>}
        <label>端口<input required type="number" min={1} max={65535} value={check.port} onChange={(event) => changePort(check.id, { port: Number(event.target.value) })} /></label>
        <label>间隔（秒）<input required type="number" min={10} max={86400} value={check.interval_secs} onChange={(event) => changePort(check.id, { interval_secs: Number(event.target.value) })} /></label>
        <label>超时（毫秒）<input required type="number" min={1} max={Math.min(10000, check.interval_secs * 1000 - 1)} value={check.timeout_ms} onChange={(event) => changePort(check.id, { timeout_ms: Number(event.target.value) })} /></label>
        <button type="button" className="icon-button" title={`删除端口检查 ${check.name || "配置"}`} aria-label={`删除端口检查 ${check.name || "配置"}`} onClick={() => setRemove({ kind: "local_port_checks", id: check.id, name: check.name || "端口检查" })}><Trash2 size={16} /></button>
      </div>)}
      <button type="button" className="secondary-button" onClick={() => change({ local_port_checks: [...(draft.local_port_checks ?? []), { id: crypto.randomUUID(), name: "", address_scope: { scope: "any_local" }, address_family: "any", protocol: "tcp", port: 80, enabled: true, interval_secs: 30, timeout_ms: 5000 }] })}><Plus size={15} />添加端口检查</button>
    </fieldset>
    {error && <p className="form-error" role="alert">{error}</p>}
    {saved && <p className="monitoring-save-status" role="status">配置版本 {draft.revision} 已保存，等待 Agent 确认</p>}
    {reloadRequired && <div className="monitoring-reload" role="status"><span>测点安全策略已更新，刷新页面后启用访问延迟。</span><button type="button" className="secondary-button" onClick={() => window.location.reload()}><RefreshCw size={15} />刷新页面</button></div>}
    <button type="submit" className="primary-button" disabled={saving}><Save size={16} />{saving ? "正在保存" : "保存配置"}</button>
    {remove && <ConfirmDialog title="删除监测配置？" description={`将从配置中移除“${remove.name}”，保存后生效。`} busy={false} error={null} onCancel={() => setRemove(null)} onConfirm={() => { if (remove.kind === "services") change({ services: draft.services.filter((item) => item.id !== remove.id) }); else if (remove.kind === "probes") change({ probes: draft.probes.filter((item) => item.id !== remove.id) }); else if (remove.kind === "process_checks") change({ process_checks: (draft.process_checks ?? []).filter((item) => item.id !== remove.id) }); else change({ local_port_checks: (draft.local_port_checks ?? []).filter((item) => item.id !== remove.id) }); setRemove(null); }} />}
  </form>;
}
