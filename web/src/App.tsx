import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import {
  Check,
  LoaderCircle,
  RefreshCw,
  WifiOff,
  X,
} from "lucide-react";
import { api, ApiError } from "./api";
import { AlertsView } from "./components/AlertsView";
import { AppIcon } from "./components/AppIcon";
import { BrandMark } from "./components/BrandMark";
import { LoginPage } from "./components/LoginPage";
import { NodeTable } from "./components/NodeTable";
import { SettingsDrawer } from "./components/SettingsDrawer";
import { SummaryCards } from "./components/SummaryCards";
import {
  applyTheme,
  readThemePreference,
  resolveThemePreference,
  THEME_DEFINITIONS,
  THEME_PREFERENCE_OPTIONS,
} from "./theme";
import type { ThemePreference } from "./theme";
import { useMonitoring } from "./hooks/useMonitoring";
import type { ConnectionState } from "./types";

type AuthState = "checking" | "authenticated" | "anonymous" | "unavailable";
type MainView = "overview" | "alerts";

const NodeDetail = lazy(() =>
  import("./components/NodeDetail").then((module) => ({ default: module.NodeDetail })),
);

export default function App() {
  const [authState, setAuthState] = useState<AuthState>("checking");
  const [view, setView] = useState<MainView>("overview");
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [sidebarOpen, setSidebarOpen] = useState(false);
  const [themePickerOpen, setThemePickerOpen] = useState(false);
  const [themePreference, setThemePreference] = useState<ThemePreference>(() => readThemePreference());
  const [authError, setAuthError] = useState<string | null>(null);
  const [authRetrying, setAuthRetrying] = useState(false);
  const authRequestId = useRef(0);
  const authRequestAbort = useRef<AbortController | null>(null);

  useEffect(() => {
    const media = window.matchMedia("(prefers-color-scheme: dark)");
    const update = () => applyTheme(themePreference, { prefersDark: media.matches });
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
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
      if (result.authenticated) setAuthState("authenticated");
      else {
        setAuthState("unavailable");
        setAuthError("Hub 返回了无效的认证状态，请稍后重试。");
      }
    } catch (error) {
      if (controller.signal.aborted || requestId !== authRequestId.current) return;
      if (error instanceof ApiError && error.status === 401) setAuthState("anonymous");
      else {
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

  useEffect(() => { void checkAuth(true); }, [checkAuth]);

  const invalidateAuthCheck = useCallback(() => {
    authRequestId.current += 1;
    authRequestAbort.current?.abort();
    authRequestAbort.current = null;
    setAuthRetrying(false);
  }, []);

  const handleUnauthorized = useCallback(() => {
    invalidateAuthCheck();
    setAuthState("anonymous");
    setSelectedNodeId(null);
    setSettingsOpen(false);
  }, [invalidateAuthCheck]);

  const monitoring = useMonitoring(authState === "authenticated", handleUnauthorized);
  const selectedNode = useMemo(
    () => monitoring.nodes.find((node) => node.id === selectedNodeId) ?? null,
    [monitoring.nodes, selectedNodeId],
  );

  useEffect(() => {
    if (selectedNodeId && monitoring.hasLoadedData && !selectedNode) setSelectedNodeId(null);
  }, [monitoring.hasLoadedData, selectedNode, selectedNodeId]);

  useEffect(() => { window.scrollTo(0, 0); }, [selectedNodeId, view]);

  useEffect(() => {
    if (!themePickerOpen) return;
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setThemePickerOpen(false);
    };
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [themePickerOpen]);

  const logout = async () => {
    invalidateAuthCheck();
    try { await api.logout(); }
    finally {
      setAuthState("anonymous");
      setSelectedNodeId(null);
      setSettingsOpen(false);
    }
  };

  const openNodeById = (nodeId: string) => {
    if (!monitoring.nodes.some((node) => node.id === nodeId)) return;
    setSelectedNodeId(nodeId);
    setView("overview");
    setSidebarOpen(false);
  };

  const navigate = (nextView: MainView) => {
    setView(nextView);
    setSelectedNodeId(null);
    setSidebarOpen(false);
  };

  if (authState === "checking") {
    return <main className="boot-screen"><BrandMark size="lg" /><strong>PingLake</strong><span className="boot-caption">正在连接监控 Hub</span><LoaderCircle className="spin" size={21} /></main>;
  }

  if (authState === "anonymous") {
    return <LoginPage onAuthenticated={() => { invalidateAuthCheck(); setAuthError(null); setAuthState("authenticated"); }} />;
  }

  if (authState === "unavailable") {
    return <ConnectionFailurePage error={authError} retrying={authRetrying} onRetry={() => void checkAuth()} />;
  }

  const resolvedTheme = resolveThemePreference(themePreference);
  const pageTitle = selectedNode ? "节点详情" : view === "alerts" ? "告警中心" : "运行概览";
  const pageDescription = selectedNode
    ? "查看节点实时状态、历史趋势和系统信息"
    : view === "alerts" ? "追踪活动告警与恢复事件" : "监控集群健康度与资源使用情况";

  return (
    <div className="app-shell">
      <button className={`sidebar-scrim${sidebarOpen ? " visible" : ""}`} type="button" aria-label="关闭导航" onClick={() => setSidebarOpen(false)} />
      <aside className={`app-sidebar${sidebarOpen ? " open" : ""}`} aria-label="主导航">
        <div className="sidebar-brand">
          <button className="brand-button" type="button" onClick={() => navigate("overview")} aria-label="返回 PingLake 概览"><BrandMark size="md" showWordmark /></button>
          <button className="sidebar-close" type="button" onClick={() => setSidebarOpen(false)} aria-label="关闭导航"><AppIcon name="close" size={18} /></button>
        </div>
        <div className="sidebar-context"><span className="eyebrow">MONITORING CONSOLE</span><span className="sidebar-subtitle">实时基础设施观察台</span></div>
        <nav className="sidebar-nav">
          <button type="button" className={view === "overview" && !selectedNode ? "active" : ""} onClick={() => navigate("overview")}><AppIcon name="overview" size={18} /><span>概览</span><AppIcon name="chevron-right" size={15} /></button>
          <button type="button" className={view === "alerts" ? "active" : ""} onClick={() => navigate("alerts")}><AppIcon name="alert" size={18} /><span>告警</span>{monitoring.summary.active_alerts > 0 && <b>{monitoring.summary.active_alerts}</b>}<AppIcon name="chevron-right" size={15} /></button>
        </nav>
        <div className="sidebar-spacer" />
        <div className="sidebar-live-card"><ConnectionBadge state={monitoring.connection} /><span>{monitoring.connection === "live" ? "数据流正常" : monitoring.connection === "polling" ? "已切换轮询" : "正在建立连接"}</span></div>
        <ThemePicker value={themePreference} resolved={resolvedTheme} onChange={setThemePreference} />
        <div className="sidebar-actions">
          <button type="button" onClick={() => void monitoring.refresh(false)} disabled={monitoring.refreshing}><AppIcon name="refresh" size={17} className={monitoring.refreshing ? "spin" : ""} /><span>刷新数据</span></button>
          <button type="button" onClick={() => { setSettingsOpen(true); setSidebarOpen(false); }}><AppIcon name="settings" size={17} /><span>告警设置</span></button>
          <button type="button" onClick={() => void logout()}><AppIcon name="logout" size={17} /><span>退出登录</span></button>
        </div>
        <div className="sidebar-footer"><span className="status-pulse" />PingLake Hub<span className="sidebar-version">v0.2</span></div>
      </aside>

      <div className="app-main">
        <header className="topbar">
          <div className="topbar-mobile-brand"><button type="button" className="mobile-menu-button" onClick={() => setSidebarOpen(true)} aria-label="打开导航"><AppIcon name="menu" size={20} /></button><BrandMark size="sm" showWordmark /></div>
          <nav className="topbar-nav" aria-label="主导航">
            <button type="button" className={view === "overview" && !selectedNode ? "active" : ""} onClick={() => navigate("overview")}><AppIcon name="overview" size={16} /><span>概览</span></button>
            <button type="button" className={view === "alerts" ? "active" : ""} onClick={() => navigate("alerts")}><AppIcon name="alert" size={16} /><span>告警</span>{monitoring.summary.active_alerts > 0 && <b>{monitoring.summary.active_alerts}</b>}</button>
          </nav>
          <div className="topbar-actions"><div className={`theme-control${themePickerOpen ? " open" : ""}`}><button type="button" className="theme-trigger" aria-haspopup="true" aria-expanded={themePickerOpen} onClick={() => setThemePickerOpen((open) => !open)}><AppIcon name="theme" size={16} /><span>{THEME_DEFINITIONS[resolvedTheme].label.split(" ")[0]}</span></button>{themePickerOpen && <div className="theme-popover"><ThemePicker value={themePreference} resolved={resolvedTheme} onChange={(nextTheme) => { setThemePreference(nextTheme); setThemePickerOpen(false); }} /></div>}</div><ConnectionBadge state={monitoring.connection} /><span className="topbar-time">{monitoring.lastUpdated ? `更新于 ${monitoring.lastUpdated.toLocaleTimeString("zh-CN", { hour12: false })}` : "等待首个采样"}</span><button type="button" className="icon-button" onClick={() => void monitoring.refresh(false)} disabled={monitoring.refreshing} title="刷新全部数据" aria-label="刷新全部数据"><AppIcon name="refresh" size={17} className={monitoring.refreshing ? "spin" : ""} /></button><button type="button" className="icon-button topbar-settings" onClick={() => setSettingsOpen(true)} title="告警设置" aria-label="打开告警设置"><AppIcon name="settings" size={17} /></button><button type="button" className="icon-button topbar-logout" onClick={() => void logout()} title="退出登录" aria-label="退出登录"><AppIcon name="logout" size={17} /></button></div>
        </header>

        <main className="main-content">
          <div className="page-heading"><div><span className="eyebrow">PINGLAKE / {selectedNode ? "NODE" : view === "alerts" ? "ALERTS" : "STATUS"}</span><h2>{pageTitle}</h2><p>{pageDescription}</p></div></div>
          {monitoring.error && monitoring.hasLoadedData && <div className="global-error" role="alert"><span>{monitoring.error}</span><button type="button" className="icon-button subtle" onClick={() => void monitoring.refresh(false)} title="重试" aria-label="重试"><RefreshCw size={15} /></button><button type="button" className="icon-button subtle" onClick={monitoring.dismissError} title="关闭提示" aria-label="关闭提示"><X size={15} /></button></div>}
          {monitoring.loading ? <DashboardSkeleton /> : !monitoring.hasLoadedData ? <ConnectionFailurePage compact error={monitoring.error} retrying={monitoring.refreshing} onRetry={() => void monitoring.refresh(false)} /> : selectedNode ? <Suspense fallback={<DetailSkeleton />}><NodeDetail node={selectedNode} onBack={() => setSelectedNodeId(null)} onDelete={monitoring.deleteNode} onRename={monitoring.renameNode} onUnauthorized={handleUnauthorized} /></Suspense> : view === "alerts" ? <AlertsView alerts={monitoring.alerts} onOpenNode={openNodeById} /> : <><SummaryCards summary={monitoring.summary} nodes={monitoring.nodes} /><NodeTable nodes={monitoring.nodes} groups={monitoring.groups} onSelect={(node) => setSelectedNodeId(node.id)} onCreateGroup={monitoring.createGroup} onAssignGroup={monitoring.assignNodeGroup} /></>}
        </main>
      </div>

      <nav className="mobile-nav" aria-label="移动端导航"><button type="button" className={view === "overview" && !selectedNode ? "active" : ""} onClick={() => navigate("overview")}><AppIcon name="overview" size={19} /><span>概览</span></button><button type="button" className={view === "alerts" ? "active" : ""} onClick={() => navigate("alerts")}><AppIcon name="alert" size={19} /><span>告警</span>{monitoring.summary.active_alerts > 0 && <i>{monitoring.summary.active_alerts}</i>}</button><button type="button" onClick={() => setSidebarOpen(true)}><AppIcon name="theme" size={19} /><span>主题</span></button><button type="button" onClick={() => setSettingsOpen(true)}><AppIcon name="settings" size={19} /><span>设置</span></button></nav>
      <SettingsDrawer open={settingsOpen} settings={monitoring.settings} nodes={monitoring.nodes} onClose={() => setSettingsOpen(false)} onSave={monitoring.saveSettings} />
    </div>
  );
}

function ThemePicker({ value, resolved, onChange, className = "" }: { value: ThemePreference; resolved: keyof typeof THEME_DEFINITIONS; onChange: (value: ThemePreference) => void; className?: string }) {
  return <div className={`theme-picker ${className}`.trim()}><div className="theme-picker-heading"><span><AppIcon name="theme" size={15} />界面主题</span><small>{THEME_DEFINITIONS[resolved].label.split(" ")[0]}</small></div><div className="theme-options" role="radiogroup" aria-label="界面主题">{THEME_PREFERENCE_OPTIONS.map((option) => <button key={option.id} type="button" role="radio" aria-checked={value === option.id} className={value === option.id ? "selected" : ""} onClick={() => onChange(option.id)}><span className={`theme-preview ${option.id}`} style={option.swatches ? { "--preview-a": option.swatches[0], "--preview-b": option.swatches[1], "--preview-c": option.swatches[2] } as CSSProperties : undefined}><i /><i /><i /></span><span>{option.label.split(" ")[0]}</span>{value === option.id && <Check size={14} />}</button>)}</div></div>;
}

function ConnectionBadge({ state }: { state: ConnectionState }) {
  const label = state === "live" ? "实时" : state === "polling" ? "轮询" : "连接中";
  return <div className={`connection-badge ${state}`} title={state === "live" ? "SSE 实时连接正常" : state === "polling" ? "实时连接中断，已启用轮询" : "正在建立实时连接"}><span />{label}</div>;
}

function DashboardSkeleton() {
  return <div className="dashboard-skeleton" aria-label="正在载入监控数据"><div className="skeleton-heading"><span /><span /></div><div className="skeleton-summary">{Array.from({ length: 5 }, (_, index) => <span key={index} />)}</div><div className="skeleton-table"><i /><i /><i /><i /></div></div>;
}

function DetailSkeleton() {
  return <div className="detail-skeleton" aria-label="正在载入节点详情"><div className="skeleton-heading"><span /><span /></div><div className="skeleton-detail-metrics">{Array.from({ length: 4 }, (_, index) => <span key={index} />)}</div><div className="skeleton-detail-chart" /></div>;
}

function ConnectionFailurePage({ error, retrying, onRetry, compact = false }: { error: string | null; retrying: boolean; onRetry: () => void; compact?: boolean }) {
  return <section className={`connection-failure${compact ? " compact" : ""}`} role="alert" aria-live="assertive"><span className="connection-failure-icon"><WifiOff size={22} /></span><div><span className="eyebrow">CONNECTION LOST</span><h1>无法连接 PingLake Hub</h1><p>{retrying ? "正在重新连接…" : error || "请检查 Hub 服务和网络连接后重试。"}</p></div><button type="button" className="primary-button" onClick={onRetry} disabled={retrying}><RefreshCw size={16} className={retrying ? "spin" : ""} />{retrying ? "正在重试" : "重试连接"}</button></section>;
}
