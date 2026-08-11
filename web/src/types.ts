export type ConnectionState = "connecting" | "live" | "polling";

export interface DiskMetric {
  name: string;
  mount_point: string;
  file_system: string;
  total_bytes: number;
  used_bytes: number;
}

export interface InterfaceMetric {
  name: string;
  received_bytes_per_sec: number;
  transmitted_bytes_per_sec: number;
}

export interface ProcessMetric {
  pid: number;
  name: string;
  cpu_percent: number;
  memory_bytes: number;
}

export interface MetricReport {
  collected_at: string;
  cpu_percent: number;
  memory_used_bytes: number;
  memory_total_bytes: number;
  swap_used_bytes: number;
  swap_total_bytes: number;
  disk_used_bytes: number;
  disk_total_bytes: number;
  network_received_bytes_per_sec: number;
  network_transmitted_bytes_per_sec: number;
  hub_latency_ms: number | null;
  load_one: number | null;
  load_five: number | null;
  load_fifteen: number | null;
  temperature_celsius: number | null;
  uptime_seconds: number;
  process_count: number;
  processes: ProcessMetric[];
  disks: DiskMetric[];
  interfaces: InterfaceMetric[];
}

export interface NodeSnapshot {
  id: string;
  hostname: string;
  display_name: string;
  os: string;
  os_version: string;
  kernel_version: string;
  architecture: string;
  agent_version: string;
  group_id: string | null;
  group_name: string | null;
  enrolled_at: string;
  last_seen_at: string | null;
  online: boolean;
  latest: MetricReport | null;
}

export interface HostGroup {
  id: string;
  name: string;
  created_at: string;
}

export interface HistoryPoint {
  collected_at: string;
  cpu_percent: number;
  memory_used_bytes: number;
  memory_total_bytes: number;
  disk_used_bytes: number;
  disk_total_bytes: number;
  network_received_bytes_per_sec: number;
  network_transmitted_bytes_per_sec: number;
  hub_latency_ms: number | null;
  temperature_celsius: number | null;
}

export type AlertKind = "offline" | "cpu" | "memory" | "disk" | "temperature";

export interface AlertRecord {
  id: number;
  node_id: string;
  node_name: string;
  kind: AlertKind;
  message: string;
  value: number | null;
  threshold: number | null;
  active: boolean;
  opened_at: string;
  resolved_at: string | null;
}

export interface AlertSettings {
  cpu_percent: number;
  memory_percent: number;
  disk_percent: number;
  temperature_celsius: number;
  offline_after_seconds: number;
  sustained_for_seconds: number;
  offline_enabled: boolean;
  cpu_enabled: boolean;
  memory_enabled: boolean;
  disk_enabled: boolean;
  temperature_enabled: boolean;
  webhook_enabled: boolean;
  webhook_url: string;
  email_enabled: boolean;
  email_recipients: string[];
}

export interface DashboardSummary {
  total_nodes: number;
  online_nodes: number;
  offline_nodes: number;
  active_alerts: number;
  average_cpu_percent: number;
  average_memory_percent: number;
}

export type LiveEvent =
  | { type: "snapshot"; payload: NodeSnapshot }
  | { type: "alert"; payload: AlertRecord }
  | { type: "node_removed"; payload: { id: string } }
  | { type: "settings_changed"; payload: AlertSettings };
