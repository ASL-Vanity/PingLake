import { useCallback, useEffect, useRef, useState } from "react";
import { api, ApiError, eventStreamUrl } from "../api";
import type {
  AlertRecord,
  AlertSettings,
  ConnectionState,
  DashboardSummary,
  HostGroup,
  LiveEvent,
  NodeSnapshot,
} from "../types";
import { deriveSummary } from "../utils";

interface MonitoringState {
  nodes: NodeSnapshot[];
  alerts: AlertRecord[];
  groups: HostGroup[];
  settings: AlertSettings | null;
  summary: DashboardSummary;
  connection: ConnectionState;
  loading: boolean;
  hasLoadedData: boolean;
  refreshing: boolean;
  lastUpdated: Date | null;
  error: string | null;
}

const emptySummary: DashboardSummary = {
  total_nodes: 0,
  online_nodes: 0,
  offline_nodes: 0,
  active_alerts: 0,
  average_cpu_percent: 0,
  average_memory_percent: 0,
};

export function useMonitoring(authenticated: boolean, onUnauthorized: () => void) {
  const [state, setState] = useState<MonitoringState>({
    nodes: [],
    alerts: [],
    groups: [],
    settings: null,
    summary: emptySummary,
    connection: "connecting",
    loading: true,
    hasLoadedData: false,
    refreshing: false,
    lastUpdated: null,
    error: null,
  });
  const refreshInFlight = useRef(false);
  const liveRevision = useRef(0);

  const handleError = useCallback(
    (error: unknown) => {
      if (error instanceof ApiError && error.status === 401) {
        onUnauthorized();
        return;
      }
      const message = error instanceof Error ? error.message : "无法连接 PingLake Hub";
      setState((current) => ({ ...current, error: message }));
    },
    [onUnauthorized],
  );

  const refresh = useCallback(
    async (silent = false) => {
      if (!authenticated || refreshInFlight.current) return;
      refreshInFlight.current = true;
      const liveRevisionAtStart = liveRevision.current;
      let retryAfterLiveEvent = false;
      if (!silent) setState((current) => ({ ...current, refreshing: true, error: null }));
      try {
        const [summary, nodes, alerts, settings, groups] = await Promise.all([
          api.summary(),
          api.nodes(),
          api.alerts(),
          api.settings(),
          api.groups(),
        ]);
        if (liveRevisionAtStart !== liveRevision.current) {
          retryAfterLiveEvent = true;
          return;
        }
        setState((current) => ({
          ...current,
          summary,
          nodes,
          alerts,
          groups,
          settings,
          loading: false,
          hasLoadedData: true,
          refreshing: false,
          lastUpdated: new Date(),
          error: null,
        }));
      } catch (error) {
        handleError(error);
        setState((current) => ({
          ...current,
          loading: false,
          refreshing: false,
        }));
      } finally {
        refreshInFlight.current = false;
        if (retryAfterLiveEvent) void refresh(true);
      }
    },
    [authenticated, handleError],
  );

  const applyLiveEvent = useCallback((event: LiveEvent) => {
    liveRevision.current += 1;
    setState((current) => {
      if (event.type === "snapshot") {
        const index = current.nodes.findIndex((node) => node.id === event.payload.id);
        const nodes = [...current.nodes];
        if (index >= 0) nodes[index] = event.payload;
        else nodes.unshift(event.payload);
        const activeAlerts = current.alerts.filter((alert) => alert.active).length;
        return {
          ...current,
          nodes,
          summary: deriveSummary(nodes, activeAlerts),
          lastUpdated: new Date(),
        };
      }

      if (event.type === "alert") {
        const alerts = [event.payload, ...current.alerts.filter((alert) => alert.id !== event.payload.id)];
        const nodes = event.payload.kind === "offline"
          ? current.nodes.map((node) => (
            node.id === event.payload.node_id ? { ...node, online: !event.payload.active } : node
          ))
          : current.nodes;
        return {
          ...current,
          nodes,
          alerts,
          summary: deriveSummary(nodes, alerts.filter((alert) => alert.active).length),
          lastUpdated: new Date(),
        };
      }

      if (event.type === "node_removed") {
        const nodes = current.nodes.filter((node) => node.id !== event.payload.id);
        const alerts = current.alerts.filter((alert) => alert.node_id !== event.payload.id);
        return {
          ...current,
          nodes,
          alerts,
          summary: deriveSummary(nodes, alerts.filter((alert) => alert.active).length),
          lastUpdated: new Date(),
        };
      }

      return { ...current, settings: event.payload, lastUpdated: new Date() };
    });
  }, []);

  useEffect(() => {
    if (!authenticated) return;
    setState((current) => ({
      ...current,
      nodes: [],
      alerts: [],
      groups: [],
      settings: null,
      summary: emptySummary,
      connection: "connecting",
      loading: true,
      hasLoadedData: false,
      refreshing: false,
      lastUpdated: null,
      error: null,
    }));
    void refresh(false);
  }, [authenticated, refresh]);

  useEffect(() => {
    if (!authenticated) return;
    let stream: EventSource | null = null;
    let reconnectTimer: number | null = null;
    let closed = false;
    let retryMs = 1_000;

    const connect = () => {
      if (closed) return;
      setState((current) => ({ ...current, connection: "connecting" }));
      stream = new EventSource(eventStreamUrl, { withCredentials: true });

      stream.onopen = () => {
        retryMs = 1_000;
        setState((current) => ({ ...current, connection: "live", error: null }));
      };

      const handleMessage = (message: MessageEvent<string>) => {
        try {
          applyLiveEvent(JSON.parse(message.data) as LiveEvent);
        } catch {
          setState((current) => ({ ...current, error: "收到无法解析的实时事件" }));
        }
      };

      stream.onmessage = handleMessage;
      stream.addEventListener("pinglake", handleMessage as EventListener);
      stream.onerror = () => {
        stream?.close();
        stream = null;
        if (closed) return;
        setState((current) => ({ ...current, connection: "polling" }));
        reconnectTimer = window.setTimeout(connect, retryMs);
        retryMs = Math.min(retryMs * 2, 30_000);
      };
    };

    connect();
    return () => {
      closed = true;
      stream?.close();
      if (reconnectTimer != null) window.clearTimeout(reconnectTimer);
    };
  }, [applyLiveEvent, authenticated]);

  useEffect(() => {
    if (!authenticated) return;
    const interval = window.setInterval(
      () => void refresh(true),
      state.connection === "live" ? 30_000 : 8_000,
    );
    return () => window.clearInterval(interval);
  }, [authenticated, refresh, state.connection]);

  const saveSettings = useCallback(
    async (settings: AlertSettings) => {
      try {
        const saved = await api.updateSettings(settings);
        liveRevision.current += 1;
        setState((current) => ({ ...current, settings: saved, error: null }));
        return saved;
      } catch (error) {
        handleError(error);
        throw error;
      }
    },
    [handleError],
  );

  const deleteNode = useCallback(
    async (nodeId: string) => {
      try {
        await api.deleteNode(nodeId);
        liveRevision.current += 1;
        setState((current) => {
          const nodes = current.nodes.filter((node) => node.id !== nodeId);
          const alerts = current.alerts.filter((alert) => alert.node_id !== nodeId);
          return {
            ...current,
            nodes,
            alerts,
            summary: deriveSummary(nodes, alerts.filter((alert) => alert.active).length),
            lastUpdated: new Date(),
            error: null,
          };
        });
      } catch (error) {
        handleError(error);
        throw error;
      }
    },
    [handleError],
  );

  const createGroup = useCallback(
    async (name: string) => {
      try {
        const group = await api.createGroup(name);
        setState((current) => ({ ...current, groups: [...current.groups, group].sort((left, right) => left.name.localeCompare(right.name)) }));
        return group;
      } catch (error) {
        handleError(error);
        throw error;
      }
    },
    [handleError],
  );

  const assignNodeGroup = useCallback(
    async (nodeId: string, groupId: string | null) => {
      try {
        const snapshot = await api.assignNodeGroup(nodeId, groupId);
        setState((current) => ({
          ...current,
          nodes: current.nodes.map((node) => node.id === nodeId ? snapshot : node),
          lastUpdated: new Date(),
        }));
      } catch (error) {
        handleError(error);
        throw error;
      }
    },
    [handleError],
  );

  const renameNode = useCallback(
    async (nodeId: string, displayName: string) => {
      try {
        const snapshot = await api.renameNode(nodeId, displayName);
        liveRevision.current += 1;
        setState((current) => ({
          ...current,
          nodes: current.nodes.map((node) => node.id === nodeId ? snapshot : node),
          lastUpdated: new Date(),
        }));
        return snapshot;
      } catch (error) {
        handleError(error);
        throw error;
      }
    },
    [handleError],
  );

  const dismissError = useCallback(() => {
    setState((current) => ({ ...current, error: null }));
  }, []);

  return { ...state, refresh, saveSettings, deleteNode, createGroup, assignNodeGroup, renameNode, dismissError };
}
