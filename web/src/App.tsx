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
} from "lucide-react";
import { api, ApiError } from "./api";
import { AlertsView } from "./components/AlertsView";
import { LoginPage } from "./components/LoginPage";
import { NodeTable } from "./components/NodeTable";
import { SettingsDrawer } from "./components/SettingsDrawer";
import { SummaryCards } from "./components/SummaryCards";
import { useMonitoring } from "./hooks/useMonitoring";
import type { ConnectionState } from "./types";

type AuthState = "checking" | "authenticated" | "anonymous" | "unavailable";
type MainView = "overview" | "alerts";
export type ThemePreference = "system" | "light" | "dark" | "midnight" | "circuit";

const themePreferences: ThemePreference[] = ["system", "light", "dark", "midnight", "circuit"];

const NodeDetail = lazy(() =>
  import("./components/NodeDetail").then((module) => ({ default: module.NodeDetail })),
);

export default function App() {
  const [authState, setAuthState] = useState<AuthState>("checking");
  const [view, setView] = useState<MainView>("overview");
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [authError, setAuthError] = useState<string | null>(null);
  const [authRetrying, setAuthRetrying] = useState(false);
  const [themePreference, setThemePreference] = useState<ThemePreference>(() => {
    const saved = window.localStorage.getItem("pinglake.theme");
    return themePreferences.includes(saved as ThemePreference) ? saved as ThemePreference : "system";
  });
  const authRequestId = useRef(0);
  const authRequestAbort = useRef<AbortController | null>(null);

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
    if (selectedNodeId && monitoring.hasLoadedData && !selectedNode) {
      setSelectedNodeId(null);
    }
  }, [monitoring.hasLoadedData, selectedNode, selectedNodeId]);

  useEffect(() => {
    window.scrollTo(0, 0);
  }, [selectedNodeId, view]);

  const logout = async () => {
    invalidateAuthCheck();
    try {
      await api.logout();
    } finally {
      setAuthState("anonymous");
      setSelectedNodeId(null);
      setSettingsOpen(false);
    }
  };

  const openNodeById = (nodeId: string) => {
    if (monitoring.nodes.some((node) => node.id === nodeId)) {
      setSelectedNodeId(nodeId);
      setView("overview");
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
    <div className="app-shell">
      <header className="topbar">
        <div className="topbar-inner">
          <button className="brand-lockup app-brand" type="button" onClick={() => { setView("overview"); setSelectedNodeId(null); }}>
            <span className="brand-mark"><img src="/pinglake-mark.svg" alt="" /></span>
            <span>PingLake</span>
          </button>

          <nav className="primary-nav" aria-label="主导航">
            <button type="button" className={view === "overview" && !selectedNode ? "active" : ""} onClick={() => { setView("overview"); setSelectedNodeId(null); }}>
              <LayoutDashboard size={16} />概览
            </button>
            <button type="button" className={view === "alerts" ? "active" : ""} onClick={() => { setView("alerts"); setSelectedNodeId(null); }}>
              <BellRing size={16} />告警
              {monitoring.summary.active_alerts > 0 && <span className="nav-badge">{monitoring.summary.active_alerts}</span>}
            </button>
          </nav>

          <div className="topbar-actions">
            <ConnectionBadge state={monitoring.connection} />
            <button type="button" className="icon-button dark" onClick={() => void monitoring.refresh(false)} disabled={monitoring.refreshing} title="刷新全部数据" aria-label="刷新全部数据">
              <RefreshCw size={17} className={monitoring.refreshing ? "spin" : ""} />
            </button>
            <button type="button" className="icon-button dark" onClick={() => setSettingsOpen(true)} title="告警设置" aria-label="告警设置">
              <Settings size={17} />
            </button>
            <button type="button" className="icon-button dark" onClick={() => void logout()} title="退出登录" aria-label="退出登录">
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
              node={selectedNode}
              onBack={() => setSelectedNodeId(null)}
              onDelete={monitoring.deleteNode}
              onRename={monitoring.renameNode}
              onUnauthorized={handleUnauthorized}
            />
          </Suspense>
        ) : view === "alerts" ? (
          <>
            <PageHeading title="告警" detail="活动告警与恢复记录" />
            <AlertsView alerts={monitoring.alerts} onOpenNode={openNodeById} />
          </>
        ) : (
          <>
            <PageHeading
              title="运行概览"
              detail={monitoring.lastUpdated ? `更新于 ${monitoring.lastUpdated.toLocaleTimeString("zh-CN", { hour12: false })}` : "等待首个采样"}
            />
            <SummaryCards summary={monitoring.summary} />
            <NodeTable
              nodes={monitoring.nodes}
              groups={monitoring.groups}
              onSelect={(node) => setSelectedNodeId(node.id)}
              onCreateGroup={monitoring.createGroup}
              onAssignGroup={monitoring.assignNodeGroup}
            />
          </>
        )}
      </main>

      <nav className="mobile-nav" aria-label="移动端导航">
        <button type="button" className={view === "overview" ? "active" : ""} onClick={() => { setView("overview"); setSelectedNodeId(null); }}><LayoutDashboard size={19} /><span>概览</span></button>
        <button type="button" className={view === "alerts" ? "active" : ""} onClick={() => { setView("alerts"); setSelectedNodeId(null); }}><BellRing size={19} /><span>告警</span>{monitoring.summary.active_alerts > 0 && <i>{monitoring.summary.active_alerts}</i>}</button>
        <button type="button" onClick={() => setSettingsOpen(true)}><Settings size={19} /><span>设置</span></button>
        <button type="button" onClick={() => void logout()}><LogOut size={19} /><span>退出</span></button>
      </nav>

      <SettingsDrawer
        open={settingsOpen}
        settings={monitoring.settings}
        nodes={monitoring.nodes}
        themePreference={themePreference}
        onThemeChange={setThemePreference}
        onClose={() => setSettingsOpen(false)}
        onSave={monitoring.saveSettings}
      />
    </div>
  );
}

function ConnectionBadge({ state }: { state: ConnectionState }) {
  const label = state === "live" ? "实时" : state === "polling" ? "轮询" : "连接中";
  return <div className={`connection-badge ${state}`} title={state === "live" ? "SSE 实时连接正常" : state === "polling" ? "实时连接中断，已启用轮询" : "正在建立实时连接"}><span />{label}</div>;
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
