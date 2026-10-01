export type MetricStatus = "ok" | "warming_up" | "unsupported" | "permission_denied" | "unavailable" | "stale";
export interface Capability { status: MetricStatus; source: string; error: string | null }
export interface CpuCore { id: string; usage_percent: number; frequency_mhz: number | null }
export interface DiskIo {
  id: string; name: string; status: MetricStatus;
  read_bytes: string | null; written_bytes: string | null; reads: string | null; writes: string | null;
  read_bytes_per_sec: number | null; write_bytes_per_sec: number | null;
  read_iops: number | null; write_iops: number | null;
  read_latency_ms: number | null; write_latency_ms: number | null;
  utilization_percent: number | null; queue_depth: number | null;
}
export interface InodeMetric { id: string; mount_point: string; status: MetricStatus; total: string | null; used: string | null; free: string | null; used_percent: number | null }
export interface NetworkHealth {
  id: string; name: string; status: MetricStatus; link_up: boolean | null;
  received_bytes: string; transmitted_bytes: string; received_packets: string; transmitted_packets: string;
  receive_errors: string; transmit_errors: string; receive_drops: string | null; transmit_drops: string | null;
  receive_errors_per_sec: number | null; transmit_errors_per_sec: number | null;
  receive_drops_per_sec: number | null; transmit_drops_per_sec: number | null;
}
export interface AgentHealth {
  collection_duration_ms: number; send_duration_ms: number | null;
  upload_attempts: number; upload_successes: number; upload_failures: number; retries: number;
  consecutive_failures: number; success_rate_percent: number | null; queue_length: number; dropped_reports: number;
  sample_age_ms: number; last_success_at: string | null; last_error: string | null;
  applied_config_revision: number | null; config_error: string | null;
}
export interface MonitoringData {
  schema_version: number; session_id: string; sample_sequence: number; report_interval_secs: number;
  capabilities: Record<string, Capability>; cpu_cores: CpuCore[];
  cpu_times: { user_percent: number | null; system_percent: number | null; iowait_percent: number | null; steal_percent: number | null };
  memory: { available_bytes: number | null; cached_bytes: number | null; buffers_bytes: number | null; swap_in_bytes: string | null; swap_out_bytes: string | null; swap_in_bytes_per_sec: number | null; swap_out_bytes_per_sec: number | null };
  disk_io: DiskIo[]; inodes: InodeMetric[]; network_health: NetworkHealth[];
  tcp: { states: Record<string, number>; listening_sockets: number; listening_ports: number; scope: string };
  agent: AgentHealth; services: ServiceResult[]; probes: ProbeResult[];
}
export interface ServiceCheck { id: string; name: string; enabled: boolean; expected_state: string }
export type ProbeKind = "icmp" | "tcp" | "http";
export interface ProbeTarget {
  id: string; name: string; kind: ProbeKind; target: string; port: number | null; enabled: boolean;
  interval_secs: number; timeout_ms: number; expected_status: number | null; response_contains: string | null;
}
export interface NodeMonitoringConfig { revision: number; browser_latency_url: string | null; services: ServiceCheck[]; probes: ProbeTarget[] }
export interface ServiceResult { id: string; name: string; checked_at: string; status: MetricStatus; state: string; healthy: boolean | null; error: string | null; config_revision: number }
export type ProbeStatus = "success" | "failure" | "timeout" | "permission_denied" | "unsupported" | "policy_denied";
export interface ProbeResult { sample_id: string; target_id: string; config_revision: number; kind: ProbeKind; scheduled_at: string; completed_at: string; status: ProbeStatus; latency_ms: number | null; http_status: number | null; error: string | null }
export interface MonitoringHistoryPoint { collected_at: string; received_at: string; monitoring: MonitoringData }
export interface ProbeStatistics {
  latency_samples?: number | null;
  target_id: string; name: string; kind: ProbeKind; successful: number; failed: number; unknown: number; expected: number;
  success_rate_percent: number | null; coverage_percent: number | null;
  p50_ms: number | null; p95_ms: number | null; p99_ms: number | null;
  observed_seconds: number; available_seconds: number; unknown_seconds: number;
}
export type MonitoringSection = "cpu" | "memory" | "disk" | "network" | "tcp" | "agent" | "services" | "probes" | "all";
