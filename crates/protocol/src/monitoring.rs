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

/// Status vocabulary shared by every active check.  `MetricStatus` remains the
/// status used by resource metrics and capabilities; this type adds the
/// check-specific policy outcome while keeping the same wire spellings for
/// the common states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Ok,
    WarmingUp,
    Unsupported,
    PermissionDenied,
    Unavailable,
    Stale,
    PolicyDenied,
}

impl Default for CheckStatus {
    fn default() -> Self {
        Self::Unavailable
    }
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
    #[serde(default)]
    pub listening_port_numbers: Vec<u16>,
    #[serde(default)]
    pub udp_listening_sockets: u64,
    #[serde(default)]
    pub udp_listening_ports: u64,
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
    /// Optional extension checks.  These fields default to empty so reports
    /// produced by older Agents remain valid.
    #[serde(default)]
    pub dns_checks: Vec<DnsResult>,
    #[serde(default)]
    pub process_checks: Vec<ProcessResult>,
    #[serde(default)]
    pub local_port_checks: Vec<LocalPortResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ServiceCheck {
    pub id: Uuid,
    pub name: String,
    #[serde(default = "enabled")]
    pub enabled: bool,
    #[serde(default = "running")]
    pub expected_state: String,
}

impl Default for ServiceCheck {
    fn default() -> Self {
        Self {
            id: Uuid::nil(),
            name: String::new(),
            enabled: true,
            expected_state: running(),
        }
    }
}

/// A DNS lookup check. `record_type` is intentionally a string so Agents can
/// add record types without making old Hub binaries reject the configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DnsCheck {
    pub id: Uuid,
    pub name: String,
    pub hostname: String,
    pub record_type: String,
    pub expected_value: Option<String>,
    pub enabled: bool,
    pub interval_secs: u64,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DnsResult {
    pub id: Uuid,
    pub name: String,
    pub checked_at: Option<DateTime<Utc>>,
    pub status: CheckStatus,
    pub hostname: String,
    pub record_type: String,
    pub answers: Vec<String>,
    pub latency_ms: Option<f64>,
    pub error: Option<String>,
}

/// A process existence/health check, identified by a stable process name or
/// executable pattern. The protocol does not prescribe how an Agent locates
/// processes on a platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ProcessCheck {
    pub id: Uuid,
    pub name: String,
    pub process_name: String,
    pub expected_count: Option<u32>,
    pub enabled: bool,
    pub expected_state: String,
    pub interval_secs: u64,
    pub timeout_ms: u64,
}

impl ProcessCheck {
    pub fn expects_running(&self) -> bool {
        self.expected_state == "running"
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProcessResult {
    pub id: Uuid,
    pub name: String,
    pub checked_at: Option<DateTime<Utc>>,
    pub status: CheckStatus,
    pub process_name: String,
    pub count: Option<u32>,
    pub expected_count: Option<u32>,
    pub error: Option<String>,
}

/// A local listening-port check. `address` defaults to loopback semantics at
/// the Agent and is carried explicitly when a platform exposes a bind address.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalPortCheck {
    pub id: Uuid,
    pub name: String,
    pub address: Option<String>,
    pub port: u16,
    pub enabled: bool,
    pub interval_secs: u64,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalPortResult {
    pub id: Uuid,
    pub name: String,
    pub checked_at: Option<DateTime<Utc>>,
    pub status: CheckStatus,
    pub address: Option<String>,
    pub port: u16,
    pub latency_ms: Option<f64>,
    pub error: Option<String>,
}

/// Short aliases used by clients that call this check simply a port check.
pub type PortCheck = LocalPortCheck;
pub type PortResult = LocalPortResult;

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
    #[serde(rename = "icmp")]
    Icmp,
    Tcp,
    Http,
    Dns,
}

impl Default for ProbeKind {
    fn default() -> Self {
        Self::Icmp
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
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

impl Default for ProbeTarget {
    fn default() -> Self {
        Self {
            id: Uuid::nil(),
            name: String::new(),
            kind: ProbeKind::default(),
            target: String::new(),
            port: None,
            enabled: enabled(),
            interval_secs: interval(),
            timeout_ms: timeout(),
            expected_status: None,
            response_contains: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeMonitoringConfig {
    pub revision: u64,
    pub browser_latency_url: Option<String>,
    pub services: Vec<ServiceCheck>,
    pub probes: Vec<ProbeTarget>,
    #[serde(default)]
    pub dns_checks: Vec<DnsCheck>,
    #[serde(default, alias = "processes")]
    pub process_checks: Vec<ProcessCheck>,
    #[serde(default)]
    pub local_port_checks: Vec<LocalPortCheck>,
}

impl Default for ProcessCheck {
    fn default() -> Self {
        Self {
            id: Uuid::nil(),
            name: String::new(),
            process_name: String::new(),
            expected_count: None,
            enabled: enabled(),
            expected_state: running(),
            interval_secs: interval(),
            timeout_ms: timeout(),
        }
    }
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
    WarmingUp,
    Unavailable,
    Stale,
}

impl From<ProbeStatus> for CheckStatus {
    fn from(value: ProbeStatus) -> Self {
        match value {
            ProbeStatus::Success => Self::Ok,
            ProbeStatus::PermissionDenied => Self::PermissionDenied,
            ProbeStatus::Unsupported => Self::Unsupported,
            ProbeStatus::PolicyDenied => Self::PolicyDenied,
            ProbeStatus::Timeout | ProbeStatus::Failure => Self::Unavailable,
            ProbeStatus::WarmingUp => Self::WarmingUp,
            ProbeStatus::Unavailable => Self::Unavailable,
            ProbeStatus::Stale => Self::Stale,
        }
    }
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

    #[test]
    fn v2_checks_use_shared_status_and_allow_future_fields() {
        let value = serde_json::json!({
            "revision": 4,
            "services": [],
            "probes": [{
                "id": Uuid::nil(), "name": "dns", "kind": "dns",
                "target": "example.test", "enabled": true,
                "future_probe_option": "ignored"
            }],
            "dns_checks": [{
                "id": Uuid::nil(), "name": "authoritative",
                "hostname": "example.test", "record_type": "A",
                "status_hint": "future"
            }],
            "process_checks": [{
                "id": Uuid::nil(), "name": "worker", "process_name": "worker.exe"
            }],
            "local_port_checks": [{
                "id": Uuid::nil(), "name": "http", "port": 8080
            }],
            "future_section": true
        });
        let config: NodeMonitoringConfig = serde_json::from_value(value).unwrap();
        assert_eq!(config.probes[0].kind, ProbeKind::Dns);
        assert_eq!(config.dns_checks[0].record_type, "A");
        assert_eq!(
            CheckStatus::from(ProbeStatus::Timeout),
            CheckStatus::Unavailable
        );
        assert_eq!(
            CheckStatus::from(ProbeStatus::PolicyDenied),
            CheckStatus::PolicyDenied
        );
    }

    #[test]
    fn new_result_fields_are_optional_for_old_stored_json() {
        let result: DnsResult =
            serde_json::from_str(r#"{"id":"00000000-0000-0000-0000-000000000000","name":"dns"}"#)
                .unwrap();
        assert_eq!(result.status, CheckStatus::Unavailable);
        assert!(result.answers.is_empty());
    }

    #[test]
    fn v2_fixture_is_wire_compatible() {
        let data: MonitoringData =
            serde_json::from_str(include_str!("../fixtures/monitoring-v2.json")).unwrap();
        assert_eq!(data.schema_version, 2);
        assert_eq!(data.dns_checks[0].status, CheckStatus::Ok);
        assert_eq!(data.process_checks[0].status, CheckStatus::PermissionDenied);
        assert_eq!(data.local_port_checks[0].status, CheckStatus::Unavailable);
    }
}
