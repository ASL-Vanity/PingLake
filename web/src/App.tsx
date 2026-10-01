import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  Activity,
  BellRing,
  LayoutDashboard,
  LoaderCircle,
  LogOut,
  RefreshCw,
  Settings,
  WifiOff,
  X,
  Server, Folder, Radio, HeartPulse, ChevronRight, Search, AlertTriangle, CheckCircle2,
} from "lucide-react";
import { api, ApiError } from "./api";
import { AlertsView } from "./components/AlertsView";
import { LoginPage } from "./components/LoginPage";
import { NodeTable } from "./components/NodeTable";
import { SettingsDrawer } from "./components/SettingsDrawer";
import { OverviewDashboard } from "./components/OverviewDashboard";
import { GroupView } from "./components/GroupView";
import { FleetChecksView } from "./components/FleetChecksView";
import { formatDateTime, formatRelativeTime } from "./utils";
import { useMonitoring } from "./hooks/useMonitoring";
import { useBrowserLatency } from "./hooks/useBrowserLatency";
import { clearHistoryCache } from "./hooks/useHistoryQuery";
import type { AlertRecord, ConnectionState, NodeSnapshot } from "./types";

type AuthState = "checking" | "authenticated" | "anonymous" | "unavailable";
type MainView = "overview" | "hosts" | "services" | "probes" | "alerts" | "groups" | "settings";
type DetailTab = "overview" | "resources" | "network" | "services" | "probes" | "quality" | "config";
interface Route { view: MainView; nodeId: string | null; tab: DetailTab; query: string; groupId?: string }
const navigation = [
  { id: "overview", label: "概览", icon: LayoutDashboard }, { id: "hosts", label: "主机", icon: Server },
  { id: "services", label: "服务", icon: HeartPulse }, { id: "probes", label: "探测", icon: Radio },
  { id: "alerts", label: "告警", icon: BellRing }, { id: "groups", label: "分组", icon: Folder },
  { id: "settings", label: "设置", icon: Settings },
] as const;
function readRoute(): Route {
  const hash = window.location.hash.slice(1);
  if (hash.length > 2048) return { view: "overview", nodeId: null, tab: "overview", query: "" };
  const [path, queryString] = hash.split("?", 2);
  const params = new URLSearchParams(queryString);
  const view = navigation.some((item) => item.id === path) ? path as MainView : "overview";
  const node = params.get("node");
  const tab = params.get("tab");
  const group = params.get("group");
  return { view, nodeId: node && /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(node) ? node : null,
    tab: ["overview", "resources", "network", "services", "probes", "quality", "config"].includes(tab ?? "") ? tab as DetailTab : "overview",
    query: (params.get("q") ?? "").slice(0, 256), groupId: group === "all" || group === "ungrouped" || /^[0-9a-f-]{36}$/i.test(group ?? "") ? group! : "all" };
}
function routeHash(route: Route) {
  const params = new URLSearchParams();
  if (route.nodeId) { params.set("node", route.nodeId); if (route.tab !== "overview") params.set("tab", route.tab); }
  if (route.query) params.set("q", route.query.slice(0, 256));
  if (route.groupId) params.set("group", route.groupId);
  return `#${route.view}${params.size ? `?${params}` : ""}`;
}
export type ThemePreference = "system" | "light" | "dark" | "midnight" | "circuit";

const themePreferences: ThemePreference[] = ["system", "light", "dark", "midnight", "circuit"];

const NodeDetail = lazy(() =>
  import("./components/NodeDetail").then((module) => ({ default: module.NodeDetail })),
);

export default function App() {
  const [authState, setAuthState] = useState<AuthState>("checking");
  const [route, setRoute] = useState<Route>(readRoute);
  const { view, nodeId: selectedNodeId } = route;
  const [configRefresh, setConfigRefresh] = useState(0);
  const [authError, setAuthError] = useState<string | null>(null);
  const [authRetrying, setAuthRetrying] = useState(false);
  const [themePreference, setThemePreference] = useState<ThemePreference>(() => {
    const saved = window.localStorage.getItem("pinglake.theme");
    return themePreferences.includes(saved as ThemePreference) ? saved as ThemePreference : "system";
  });
  const authRequestId = useRef(0);
  const authRequestAbort = useRef<AbortController | null>(null);
  useEffect(() => {
    const update = () => setRoute(readRoute());
    window.addEventListener("hashchange", update);
    window.addEventListener("popstate", update);
    return () => { window.removeEventListener("hashchange", update); window.removeEventListener("popstate", update); };
  }, []);
  const navigate = useCallback((next: Route, replace = false) => {
    window.history[replace ? "replaceState" : "pushState"](null, "", routeHash(next));
    setRoute(next);
  }, []);
  const openView = (next: MainView) => navigate({ view: next, nodeId: null, tab: "overview", query: next === "hosts" ? route.query : "" });

  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const applyTheme = () => {
      const resolved = themePreference === "system" ? (media.matches ? "dark" : "light") : themePreference;
      document.documentElement.dataset.theme = resolved;
      document.documentElement.style.colorScheme = resolved === "light" ? "light" : "dark";
      window.localStorage.setItem("pinglake.theme", themePreference);
    };
    applyTheme();
    media.addEventListener("change", applyTheme);
    return () => media.removeEventListener("change", applyTheme);
  }, [themePreference]);

  const checkAuth = useCallback(async (initial = false) => {
    const requestId = ++authRequestId.current;
    authRequestAbort.current?.abort();
    const controller = new AbortController();
    authRequestAbort.current = controller;
    if (initial) setAuthState("checking");
    else setAuthRetrying(true);
    setAuthError(null);
    try {
      const result = await api.me(controller.signal);
      if (controller.signal.aborted || requestId !== authRequestId.current) return;
      if (result.authenticated) {
        setAuthState("authenticated");
      } else {
        setAuthState("unavailable");
        setAuthError("Hub 返回了无效的认证状态，请稍后重试。");
      }
    } catch (error) {
      if (controller.signal.aborted || requestId !== authRequestId.current) return;
      if (error instanceof ApiError && error.status === 401) {
        setAuthState("anonymous");
      } else {
        setAuthState("unavailable");
        setAuthError(error instanceof Error ? error.message : "无法连接 PingLake Hub");
      }
    } finally {
      if (requestId === authRequestId.current) {
        authRequestAbort.current = null;
        setAuthRetrying(false);
      }
    }
  }, []);

  useEffect(() => {
    void checkAuth(true);
  }, [checkAuth]);

  const invalidateAuthCheck = useCallback(() => {
    authRequestId.current += 1;
    authRequestAbort.current?.abort();
    authRequestAbort.current = null;
    setAuthRetrying(false);
  }, []);

  const handleUnauthorized = useCallback(() => {
    invalidateAuthCheck();
    clearHistoryCache();
    setAuthState("anonymous");
    navigate({ view: "overview", nodeId: null, tab: "overview", query: "" }, true);
  }, [invalidateAuthCheck, navigate]);

  const monitoring = useMonitoring(authState === "authenticated", handleUnauthorized);
  const browserLatency = useBrowserLatency(authState === "authenticated" ? monitoring.nodes : []);
  const selectedNode = useMemo(
    () => monitoring.nodes.find((node) => node.id === selectedNodeId) ?? null,
    [monitoring.nodes, selectedNodeId],
  );

  useEffect(() => {
    if (selectedNodeId && monitoring.hasLoadedData && !selectedNode) {
      navigate({ ...route, nodeId: null }, true);
    }
  }, [monitoring.hasLoadedData, selectedNode, selectedNodeId, navigate, route]);

  useEffect(() => {
    window.scrollTo(0, 0);
  }, [selectedNodeId, view]);

  const logout = async () => {
    invalidateAuthCheck();
    try {
      await api.logout();
    } finally {
      clearHistoryCache();
      setAuthState("anonymous");
      navigate({ view: "overview", nodeId: null, tab: "overview", query: "" }, true);
    }
  };

  const openNodeById = (nodeId: string, tab: DetailTab = "overview") => {
    if (monitoring.nodes.some((node) => node.id === nodeId)) {
      navigate({ ...route, nodeId, tab });
    }
  };

  if (authState === "checking") {
    return (
      <main className="boot-screen">
        <span className="brand-mark"><img src="/pinglake-mark.svg" alt="" /></span>
        <strong>PingLake</strong>
        <LoaderCircle className="spin" size={20} />
      </main>
    );
  }

  if (authState === "anonymous") {
    return <LoginPage onAuthenticated={() => { invalidateAuthCheck(); setAuthError(null); setAuthState("authenticated"); }} />;
  }

  if (authState === "unavailable") {
    return <ConnectionFailurePage error={authError} retrying={authRetrying} onRetry={() => void checkAuth()} />;
  }

  return (
    <div className="app-shell workspace-shell">
      <aside className="workspace-sidebar">
        <button className="brand-lockup workspace-brand" onClick={() => openView("overview")}><span className="brand-mark"><img src="/pinglake-mark.svg" alt="" /></span><span>PingLake<small>主机监控</small></span></button>
        <span className="workspace-nav-caption">监控空间</span>
        <nav className="workspace-navigation" aria-label="主导航">{navigation.map(({ id, label, icon: Icon }) => <button key={id} className={view === id ? "active" : ""} aria-current={view === id ? "page" : undefined} onClick={() => openView(id)}><Icon size={18} /><span>{label}</span>{id === "alerts" && monitoring.summary.active_alerts > 0 && <strong className="workspace-nav-count">{monitoring.summary.active_alerts}</strong>}</button>)}</nav>
        <div className="workspace-sidebar-footer"><ConnectionBadge state={monitoring.connection} /><span>{monitoring.nodes.length} 台主机</span><button className="icon-button subtle" title="退出登录" aria-label="退出登录" onClick={() => void logout()}><LogOut size={16} /></button></div>
      </aside>
      <header className="topbar">
        <div className="topbar-inner">
          <button className="brand-lockup app-brand" type="button" onClick={() => openView("overview")}>
            <span className="brand-mark"><img src="/pinglake-mark.svg" alt="" /></span>
            <span>PingLake</span>
          </button>

          <div className="workspace-breadcrumb"><span>{navigation.find((item) => item.id === view)?.label}</span>{selectedNode && <><ChevronRight size={14} /><strong>{selectedNode.display_name || selectedNode.hostname}</strong></>}</div>
          <label className="search-field workspace-search"><Search size={15} /><input placeholder="搜索主机" aria-label="全局搜索主机" value={route.query} onChange={(event) => navigate({ view: "hosts", nodeId: null, tab: "overview", query: event.target.value.slice(0, 256) }, true)} /></label>

          <div className="topbar-actions">
            <ConnectionBadge state={monitoring.connection} />
            <button type="button" className="icon-button" onClick={() => { setConfigRefresh((value) => value + 1); void monitoring.refresh(false); }} disabled={monitoring.refreshing} title="刷新全部数据" aria-label="刷新全部数据">
              <RefreshCw size={17} className={monitoring.refreshing ? "spin" : ""} />
            </button>
            <button type="button" className="icon-button" onClick={() => void logout()} title="退出登录" aria-label="退出登录">
              <LogOut size={17} />
            </button>
          </div>
        </div>
      </header>

      <main className="main-content">
        {monitoring.error && monitoring.hasLoadedData && (
          <div className="global-error" role="alert">
            <span>{monitoring.error}</span>
            <button type="button" className="icon-button subtle" onClick={() => void monitoring.refresh(false)} title="重试" aria-label="重试"><RefreshCw size={15} /></button>
            <button type="button" className="icon-button subtle" onClick={monitoring.dismissError} title="关闭提示" aria-label="关闭提示"><X size={15} /></button>
          </div>
        )}

        {monitoring.loading ? (
          <DashboardSkeleton />
        ) : !monitoring.hasLoadedData ? (
          <ConnectionFailurePage
            compact
            error={monitoring.error}
            retrying={monitoring.refreshing}
            onRetry={() => void monitoring.refresh(false)}
          />
        ) : selectedNode ? (
          <Suspense fallback={<DetailSkeleton />}>
            <NodeDetail
              key={selectedNode.id}
              node={selectedNode}
              onBack={() => navigate({ ...route, nodeId: null, tab: "overview" })}
              initialTab={route.tab}
              refreshKey={configRefresh}
              onTabChange={(tab) => navigate({ ...route, tab }, true)}
              onDelete={monitoring.deleteNode}
              onRename={monitoring.renameNode}
              onUnauthorized={handleUnauthorized}
              browserLatency={browserLatency[selectedNode.id]}
              onConfigSaved={() => { setConfigRefresh((value) => value + 1); void monitoring.refresh(true); }}
            />
          </Suspense>
        ) : view === "alerts" ? (
          <>
            <PageHeading title="告警" detail={`${monitoring.summary.active_alerts} 条活动告警`} />
            <AlertsView alerts={monitoring.alerts} onOpenNode={openNodeById} />
          </>
        ) : view === "hosts" ? <><PageHeading title="主机" detail={`${monitoring.nodes.length} 台主机`} /><NodeTable nodes={monitoring.nodes} groups={monitoring.groups} onSelect={(node) => openNodeById(node.id)} onAssignGroup={monitoring.assignNodeGroup} browserLatency={browserLatency} query={route.query} onQueryChange={(query) => navigate({ ...route, query }, true)} groupId={route.groupId ?? "all"} onGroupChange={(groupId) => navigate({ ...route, groupId }, true)} onManageGroups={() => openView("groups")} /></>
        : view === "services" || view === "probes" ? <><PageHeading title={view === "services" ? "服务" : "主动探测"} detail={`${monitoring.nodes.length} 台主机`} /><FleetChecksView kind={view} nodes={monitoring.nodes} onOpenNode={openNodeById} onUnauthorized={handleUnauthorized} refreshKey={configRefresh} /></>
        : view === "groups" ? <><PageHeading title="分组" detail={`${monitoring.groups.length} 个分组`} /><GroupView groups={monitoring.groups} nodes={monitoring.nodes} onCreate={monitoring.createGroup} onDelete={monitoring.deleteGroup} onAssign={monitoring.assignNodeGroup} onOpenNode={openNodeById} onOpenHosts={(groupId) => navigate({ view: "hosts", nodeId: null, tab: "overview", query: "", groupId })} /></>
        : view === "settings" ? <SettingsDrawer inline open settings={monitoring.settings} nodes={monitoring.nodes} themePreference={themePreference} onThemeChange={setThemePreference} onClose={() => openView("overview")} onSave={monitoring.saveSettings} />
        : (
          <>
            <PageHeading
              title="运行概览"
              detail={monitoring.lastUpdated ? `更新于 ${monitoring.lastUpdated.toLocaleTimeString("zh-CN", { hour12: false })}` : "等待首个采样"}
            />
            <OverviewDashboard nodes={monitoring.nodes} alerts={monitoring.alerts} summary={monitoring.summary} onOpenNode={openNodeById} onViewAlerts={() => openView("alerts")} onViewHosts={() => openView("hosts")} onUnauthorized={handleUnauthorized} />
          </>
        )}
      </main>

      <nav className="mobile-nav" aria-label="移动端导航">
        {navigation.map(({ id, label, icon: Icon }) => <button key={id} type="button" className={view === id ? "active" : ""} aria-current={view === id ? "page" : undefined} onClick={() => openView(id)}><Icon size={19} /><span>{label}</span>{id === "alerts" && monitoring.summary.active_alerts > 0 && <i>{monitoring.summary.active_alerts}</i>}</button>)}
      </nav>
    </div>
  );
}

function ConnectionBadge({ state }: { state: ConnectionState }) {
  const label = state === "live" ? "实时" : state === "polling" ? "轮询" : "连接中";
  return <div className={`connection-badge ${state}`} title={state === "live" ? "SSE 实时连接正常" : state === "polling" ? "实时连接中断，已启用轮询" : "正在建立实时连接"}><span />{label}</div>;
}

function OverviewActivity({ nodes, alerts, onOpenNode, onViewAlerts, onViewHosts }: { nodes: NodeSnapshot[]; alerts: AlertRecord[]; onOpenNode: (id: string, tab?: DetailTab) => void; onViewAlerts: () => void; onViewHosts: () => void }) {
  const [now, setNow] = useState(Date.now());
  useEffect(() => { const timer = window.setInterval(() => setNow(Date.now()), 5000); return () => window.clearInterval(timer); }, []);
  const exceptions = nodes.flatMap((node) => {
    const data = node.latest?.monitoring;
    const reasons: string[] = []; let tab: DetailTab = "overview";
    const recent = (at: string, seconds: number) => Number.isFinite(Date.parse(at)) && now - Date.parse(at) <= seconds * 1000;
    if (!node.online) reasons.push("主机离线");
    else if (!node.latest || !node.last_seen_at || !Number.isFinite(Date.parse(node.last_seen_at)) || now - Date.parse(node.last_seen_at) + (data?.agent.sample_age_ms ?? 0) > Math.max(30, (data?.report_interval_secs ?? 5) * 3) * 1000) reasons.push("数据过期");
    if (node.online && data) {
      const revision = data.agent.applied_config_revision;
      if (data.services.some((service) => service.config_revision === revision && service.status === "ok" && service.healthy === false && recent(service.checked_at, 30))) { reasons.push("服务故障"); tab = "services"; }
      const latestProbes = new Map<string, (typeof data.probes)[number]>();
      for (const probe of data.probes) {
        if (probe.config_revision !== revision) continue;
        const previous = latestProbes.get(probe.target_id);
        if (!previous || Date.parse(probe.completed_at) >= Date.parse(previous.completed_at)) latestProbes.set(probe.target_id, probe);
      }
      if ([...latestProbes.values()].some((probe) => ["failure", "timeout"].includes(probe.status) && recent(probe.completed_at, 60))) { reasons.push("探测失败"); if (tab === "overview") tab = "probes"; }
      if (data.agent.consecutive_failures > 0 || data.agent.config_error) { reasons.push("上报异常"); if (tab === "overview") tab = "quality"; }
    }
    return reasons.length ? [{ node, reasons, tab }] : [];
  });
  const latestAlerts = [...alerts].sort((left, right) => Date.parse(right.opened_at) - Date.parse(left.opened_at)).slice(0, 5);
  return <div className="overview-activity">
    <section className="overview-section"><header><h2>异常主机 <span>{exceptions.length}</span></h2><button className="overview-more" onClick={onViewHosts}>全部主机<ChevronRight size={15} /></button></header>
      {!nodes.length ? <div className="overview-empty">暂无主机</div> : !exceptions.length ? <div className="overview-empty"><CheckCircle2 size={20} />当前没有检测到主机异常</div> : exceptions.slice(0, 8).map(({ node, reasons, tab }) => <button className="overview-node-row" key={node.id} onClick={() => onOpenNode(node.id, tab)}><span className={`fleet-dot ${node.online ? "pending" : "bad"}`} /><span className="overview-row-title"><strong>{node.display_name || node.hostname}</strong><small>{node.group_name || "未分组"} · {node.hostname}</small></span><span className="overview-reasons">{reasons.join(" · ")}</span><ChevronRight size={16} /></button>)}
    </section>
    <section className="overview-section"><header><h2>最近告警 <span>{alerts.length}</span></h2><button className="overview-more" onClick={onViewAlerts}>全部告警<ChevronRight size={15} /></button></header>
      {!latestAlerts.length ? <div className="overview-empty"><CheckCircle2 size={20} />暂无告警记录</div> : latestAlerts.map((alert) => <button className="overview-alert-row" key={alert.id} onClick={() => onOpenNode(alert.node_id, alert.kind === "service" ? "services" : alert.kind === "probe" ? "probes" : "overview")}><span className={`overview-alert-icon ${alert.active ? "bad" : "ok"}`}>{alert.active ? <AlertTriangle size={17} /> : <CheckCircle2 size={17} />}</span><span className="overview-row-title"><strong>{alert.node_name}<small>{alert.active ? "活动" : "已恢复"}</small></strong><span>{alert.message}</span></span><time title={formatDateTime(alert.opened_at)}>{formatRelativeTime(alert.opened_at)}</time><ChevronRight size={15} /></button>)}
    </section>
  </div>;
}

function PageHeading({ title, detail }: { title: string; detail: string }) {
  return <header className="page-heading"><div><h1>{title}</h1><p>{detail}</p></div></header>;
}

function DashboardSkeleton() {
  return (
    <div className="dashboard-skeleton" aria-label="正在载入监控数据">
      <div className="skeleton-heading"><span /><span /></div>
      <div className="skeleton-summary">{Array.from({ length: 6 }, (_, index) => <span key={index} />)}</div>
      <div className="skeleton-table"><i /><i /><i /><i /><i /></div>
    </div>
  );
}

function DetailSkeleton() {
  return (
    <div className="detail-skeleton" aria-label="正在载入节点详情">
      <div className="skeleton-heading"><span /><span /></div>
      <div className="skeleton-detail-metrics">{Array.from({ length: 4 }, (_, index) => <span key={index} />)}</div>
      <div className="skeleton-detail-chart" />
    </div>
  );
}

function ConnectionFailurePage({
  error,
  retrying,
  onRetry,
  compact = false,
}: {
  error: string | null;
  retrying: boolean;
  onRetry: () => void;
  compact?: boolean;
}) {
  return (
    <section className={`connection-failure${compact ? " compact" : ""}`} role="alert" aria-live="assertive">
      <span className="connection-failure-icon"><WifiOff size={22} /></span>
      <div>
        <h1>无法连接 PingLake Hub</h1>
        <p>{retrying ? "正在重新连接…" : error || "请检查 Hub 服务和网络连接后重试。"}</p>
      </div>
      <button type="button" className="primary-button" onClick={onRetry} disabled={retrying}>
        <RefreshCw size={16} className={retrying ? "spin" : ""} />
        {retrying ? "正在重试" : "重试连接"}
      </button>
    </section>
  );
}
