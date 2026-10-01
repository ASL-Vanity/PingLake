import type {
  AlertRecord,
  AlertSettings,
  DashboardSummary,
  HistoryPoint,
  HostGroup,
  NodeSnapshot,
} from "./types";
import type { MonitoringHistoryPoint, MonitoringSection, NodeMonitoringConfig, ProbeStatistics } from "./monitoringTypes";

const API_BASE = "/api/v1";

export class ApiError extends Error {
  status: number;

  constructor(message: string, status: number) {
    super(message);
    this.name = "ApiError";
    this.status = status;
  }
}

async function request<T>(path: string, options: RequestInit = {}): Promise<T> {
  const headers = new Headers(options.headers);
  if (options.body && !headers.has("Content-Type")) {
    headers.set("Content-Type", "application/json");
  }

  const response = await fetch(`${API_BASE}${path}`, {
    ...options,
    headers,
    credentials: "same-origin",
    cache: "no-store",
  });

  if (!response.ok) {
    let message = `请求失败 (${response.status})`;
    try {
      const body = (await response.json()) as { error?: string };
      if (body.error) message = body.error;
    } catch {
      // Preserve the status-based fallback when the server has no JSON body.
    }
    throw new ApiError(message, response.status);
  }

  if (response.status === 204) return undefined as T;
  return (await response.json()) as T;
}

export const api = {
  monitoringConfig: (id: string, signal?: AbortSignal) => request<NodeMonitoringConfig>(`/nodes/${encodeURIComponent(id)}/monitoring`, { signal }),
  saveMonitoringConfig: (id: string, config: NodeMonitoringConfig) => request<NodeMonitoringConfig>(`/nodes/${encodeURIComponent(id)}/monitoring`, { method: "PUT", body: JSON.stringify(config) }),
  monitoringHistory: (id: string, minutes: number, section: MonitoringSection, device: string | undefined, signal?: AbortSignal) => {
    const query = new URLSearchParams({ minutes: String(minutes), section });
    if (device) query.set("device", device);
    return request<MonitoringHistoryPoint[]>(`/nodes/${encodeURIComponent(id)}/monitoring/history?${query}`, { signal });
  },
  probeStatistics: (id: string, minutes: number, signal?: AbortSignal) => request<ProbeStatistics[]>(`/nodes/${encodeURIComponent(id)}/probes/statistics?minutes=${minutes}`, { signal }),
  me: (signal?: AbortSignal) => request<{ authenticated: boolean }>("/auth/me", { signal }),
  login: (password: string) =>
    request<{ authenticated: boolean }>("/auth/login", {
      method: "POST",
      body: JSON.stringify({ password }),
    }),
  logout: () => request<void>("/auth/logout", { method: "POST" }),
  summary: () => request<DashboardSummary>("/summary"),
  nodes: () => request<NodeSnapshot[]>("/nodes"),
  history: (id: string, minutes: number, signal?: AbortSignal) =>
    request<HistoryPoint[]>(`/nodes/${encodeURIComponent(id)}/history?minutes=${minutes}`, { signal }),
  deleteNode: (id: string) =>
    request<void>(`/nodes/${encodeURIComponent(id)}`, { method: "DELETE" }),
  alerts: () => request<AlertRecord[]>("/alerts"),
  settings: () => request<AlertSettings>("/settings"),
  updateSettings: (settings: AlertSettings) =>
    request<AlertSettings>("/settings", {
      method: "PUT",
      body: JSON.stringify(settings),
    }),
  groups: () => request<HostGroup[]>("/groups"),
  createGroup: (name: string) => request<HostGroup>("/groups", {
    method: "POST",
    body: JSON.stringify({ name }),
  }),
  deleteGroup: (id: string) => request<void>(`/groups/${encodeURIComponent(id)}`, { method: "DELETE" }),
  assignNodeGroup: (nodeId: string, groupId: string | null) =>
    request<NodeSnapshot>(`/nodes/${encodeURIComponent(nodeId)}/group`, {
      method: "PUT",
      body: JSON.stringify({ group_id: groupId }),
    }),
  renameNode: (nodeId: string, displayName: string) =>
    request<NodeSnapshot>(`/nodes/${encodeURIComponent(nodeId)}/name`, {
      method: "PUT",
      body: JSON.stringify({ display_name: displayName }),
    }),
};

export const eventStreamUrl = `${API_BASE}/events`;
