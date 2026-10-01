use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricStatus {
    Ok,
    WarmingUp,
    #[default]
    Unsupported,
    PermissionDenied,
    Unavailable,
    Stale,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Capability {
    pub status: MetricStatus,
    pub source: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CpuCore {
    pub id: String,
    pub usage_percent: f32,
    pub frequency_mhz: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CpuTimes {
    pub user_percent: Option<f64>,
    pub system_percent: Option<f64>,
    pub iowait_percent: Option<f64>,
    pub steal_percent: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MemoryDetails {
    pub available_bytes: Option<u64>,
    pub cached_bytes: Option<u64>,
    pub buffers_bytes: Option<u64>,
    pub swap_in_bytes: Option<String>,
    pub swap_out_bytes: Option<String>,
    pub swap_in_bytes_per_sec: Option<f64>,
    pub swap_out_bytes_per_sec: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DiskIo {
    pub id: String,
    pub name: String,
    pub status: MetricStatus,
    pub read_bytes: Option<String>,
    pub written_bytes: Option<String>,
    pub reads: Option<String>,
    pub writes: Option<String>,
    pub read_bytes_per_sec: Option<f64>,
    pub write_bytes_per_sec: Option<f64>,
    pub read_iops: Option<f64>,
    pub write_iops: Option<f64>,
    pub read_latency_ms: Option<f64>,
    pub write_latency_ms: Option<f64>,
    pub utilization_percent: Option<f64>,
    pub queue_depth: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct InodeMetric {
    pub id: String,
    pub mount_point: String,
    pub status: MetricStatus,
    pub total: Option<String>,
    pub used: Option<String>,
    pub free: Option<String>,
    pub used_percent: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NetworkHealth {
    pub id: String,
    pub name: String,
    pub status: MetricStatus,
    pub link_up: Option<bool>,
    pub received_bytes: String,
    pub transmitted_bytes: String,
    pub received_packets: String,
    pub transmitted_packets: String,
    pub receive_errors: String,
    pub transmit_errors: String,
    pub receive_drops: Option<String>,
    pub transmit_drops: Option<String>,
    pub receive_errors_per_sec: Option<f64>,
    pub transmit_errors_per_sec: Option<f64>,
    pub receive_drops_per_sec: Option<f64>,
    pub transmit_drops_per_sec: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct TcpMetrics {
    pub states: BTreeMap<String, u64>,
    pub listening_sockets: u64,
    pub listening_ports: u64,
    pub scope: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentHealth {
    pub collection_duration_ms: f64,
    pub send_duration_ms: Option<f64>,
    pub upload_attempts: u64,
    pub upload_successes: u64,
    pub upload_failures: u64,
    pub retries: u64,
    pub consecutive_failures: u64,
    pub success_rate_percent: Option<f64>,
    pub queue_length: usize,
    pub dropped_reports: u64,
    pub sample_age_ms: f64,
    pub last_success_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub applied_config_revision: Option<u64>,
    pub config_error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MonitoringData {
    pub schema_version: u32,
    pub session_id: Uuid,
    pub sample_sequence: u64,
    pub report_interval_secs: u64,
    pub capabilities: BTreeMap<String, Capability>,
    pub cpu_cores: Vec<CpuCore>,
    pub cpu_times: CpuTimes,
    pub memory: MemoryDetails,
    pub disk_io: Vec<DiskIo>,
    pub inodes: Vec<InodeMetric>,
    pub network_health: Vec<NetworkHealth>,
    pub tcp: TcpMetrics,
    pub agent: AgentHealth,
    pub services: Vec<ServiceResult>,
    pub probes: Vec<ProbeResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceCheck {
    pub id: Uuid,
    pub name: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default = "running")]
    pub expected_state: String,
}

fn enabled() -> bool {
    true
}
fn running() -> String {
    "running".to_owned()
}
fn interval() -> u64 {
    30
}
fn timeout() -> u64 {
    5_000
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeKind {
    Icmp,
    Tcp,
    Http,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeTarget {
    pub id: Uuid,
    pub name: String,
    pub kind: ProbeKind,
    pub target: String,
    pub port: Option<u16>,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default = "interval")]
    pub interval_secs: u64,
    #[serde(default = "timeout")]
    pub timeout_ms: u64,
    pub expected_status: Option<u16>,
    pub response_contains: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct NodeMonitoringConfig {
    pub revision: u64,
    pub browser_latency_url: Option<String>,
    pub services: Vec<ServiceCheck>,
    pub probes: Vec<ProbeTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceResult {
    pub id: Uuid,
    pub name: String,
    pub checked_at: DateTime<Utc>,
    pub status: MetricStatus,
    pub state: String,
    pub healthy: Option<bool>,
    pub error: Option<String>,
    pub config_revision: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProbeStatus {
    Success,
    Failure,
    Timeout,
    PermissionDenied,
    Unsupported,
    PolicyDenied,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeResult {
    pub sample_id: Uuid,
    pub target_id: Uuid,
    pub config_revision: u64,
    pub kind: ProbeKind,
    pub scheduled_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub status: ProbeStatus,
    pub latency_ms: Option<f64>,
    pub http_status: Option<u16>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitoringHistoryPoint {
    pub collected_at: DateTime<Utc>,
    pub received_at: DateTime<Utc>,
    pub monitoring: MonitoringData,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProbeStatistics {
    pub target_id: Uuid,
    pub name: String,
    pub kind: ProbeKind,
    pub successful: u64,
    pub failed: u64,
    pub unknown: u64,
    pub expected: u64,
    pub success_rate_percent: Option<f64>,
    pub coverage_percent: Option<f64>,
    pub p50_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
    #[serde(default)]
    pub latency_samples: u64,
    pub observed_seconds: f64,
    pub available_seconds: f64,
    pub unknown_seconds: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_monitoring_and_large_counters_round_trip() {
        let data = MonitoringData {
            memory: MemoryDetails {
                swap_in_bytes: Some(u64::MAX.to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let encoded = serde_json::to_string(&data).unwrap();
        let decoded: MonitoringData = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded.memory.swap_in_bytes, Some(u64::MAX.to_string()));
        assert_eq!(
            serde_json::from_str::<MonitoringData>("{}")
                .unwrap()
                .cpu_cores
                .len(),
            0
        );
    }
    #[test]
    fn legacy_reports_without_monitoring_remain_readable() {
        let value = serde_json::json!({
            "collected_at": "2026-10-01T00:00:00Z", "cpu_percent": 12.0,
            "memory_used_bytes": 100, "memory_total_bytes": 200, "swap_used_bytes": 0,
            "swap_total_bytes": 0, "disk_used_bytes": 20, "disk_total_bytes": 100,
            "network_received_bytes_per_sec": 0, "network_transmitted_bytes_per_sec": 0,
            "load_one": null, "load_five": null, "load_fifteen": null, "temperature_celsius": null,
            "uptime_seconds": 5, "process_count": 1, "disks": [], "interfaces": []
        });
        let report: crate::MetricReport = serde_json::from_value(value).unwrap();
        assert!(report.monitoring.is_none());
        assert_eq!(report.cpu_percent, 12.0);
    }
}
